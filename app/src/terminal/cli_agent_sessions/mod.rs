pub mod auto_continue;
#[cfg(not(target_family = "wasm"))]
#[cfg_attr(not(feature = "local_tty"), allow(dead_code))]
pub mod codex_host_check;
pub mod event;
pub mod listener;
#[cfg(not(target_family = "wasm"))]
pub(crate) mod plugin_manager;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use blocking::unblock;
use event::{CLIAgentEvent, CLIAgentEventSource, CLIAgentEventType};
use warpui::{Entity, EntityId, ModelContext, ModelHandle, SingletonEntity};

use self::listener::CLIAgentSessionListener;
use super::CLIAgent;
use crate::agent_resume::{
    prompt_title, read_prompt_history, AgentPrompt, AgentPromptHistory, AgentResumeProvider,
};
use crate::ai::blocklist::InputConfig;
use crate::channel::ChannelState;

/// The public Clinch channels that own agent-session context chrome and local recovery.
fn session_context_enabled_for(app_id: &str) -> bool {
    matches!(app_id, "sh.clinch.Clinch" | "sh.clinch.ClinchDev")
}

pub(crate) fn session_context_enabled() -> bool {
    session_context_enabled_for(&ChannelState::app_id().to_string())
}

/// Durable identity for prompt history. Entity/view IDs are intentionally not part of the key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CLIAgentSessionKey {
    pub provider: AgentResumeProvider,
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptHistoryLoadState {
    NotRequested,
    Loading {
        key: CLIAgentSessionKey,
        generation: u64,
    },
    Ready,
    Unavailable,
}

impl PromptHistoryLoadState {
    fn matches_loading(&self, key: &CLIAgentSessionKey, generation: u64) -> bool {
        matches!(
            self,
            Self::Loading {
                key: loading_key,
                generation: loading_generation,
            } if loading_key == key && *loading_generation == generation
        )
    }
}

impl Default for PromptHistoryLoadState {
    fn default() -> Self {
        Self::NotRequested
    }
}

/// Status of a tracked CLI agent session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CLIAgentSessionStatus {
    InProgress,
    Success,
    Blocked { message: Option<String> },
}

impl CLIAgentSessionStatus {
    pub fn to_conversation_status(&self) -> crate::ai::agent::conversation::ConversationStatus {
        use crate::ai::agent::conversation::ConversationStatus;
        match self {
            CLIAgentSessionStatus::InProgress => ConversationStatus::InProgress,
            CLIAgentSessionStatus::Success => ConversationStatus::Success,
            CLIAgentSessionStatus::Blocked { message } => ConversationStatus::Blocked {
                blocked_action: message.clone().unwrap_or_default(),
            },
        }
    }
}

/// Rich context accumulated from CLI agent session events.
#[derive(Debug, Clone, Default)]
pub struct CLIAgentSessionContext {
    pub cwd: Option<String>,
    pub project: Option<String>,
    pub session_id: Option<String>,
    pub transcript_path: Option<String>,
    pub tool_name: Option<String>,
    pub tool_input_preview: Option<String>,
    pub summary: Option<String>,
    pub query: Option<String>,
    pub response: Option<String>,
}

/// State of the rich input editor for composing a prompt to send to a CLI agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CLIAgentInputState {
    /// The rich input editor is not open.
    Closed,
    /// The rich input editor is open.
    Open {
        /// How this session was opened (for telemetry).
        entrypoint: CLIAgentInputEntrypoint,
        /// The input config that was active before opening rich input.
        previous_input_config: InputConfig,
        /// Whether the previous lock state was established while the input buffer was empty.
        previous_was_lock_set_with_empty_buffer: bool,
    },
}

/// Why the CLI agent rich input was closed (for telemetry).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum CLIAgentRichInputCloseReason {
    /// User explicitly closed (Escape, Ctrl-G, footer button).
    Manual,
    /// Auto-closed due to agent status change (e.g. Blocked).
    AutoToggle,
    /// Auto-dismissed after submitting a prompt.
    Submit,
    /// Closed for another reason (chip removed, session ended, shared session sync).
    Other,
}

/// How a [`CLIAgentInputState`] was opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum CLIAgentInputEntrypoint {
    /// User pressed Ctrl-G while a CLI agent was active.
    CtrlG,
    /// User clicked the rich input button in the CLI agent footer.
    FooterButton,
    /// Automatically opened when the CLI agent resumed work (left a blocked state)
    /// and the auto-show setting is enabled.
    AutoShow,
    /// Rich input was opened to mirror a shared-session participant's state.
    SharedSessionSync,
}

impl CLIAgentSessionContext {
    pub(crate) fn display_title(&self) -> Option<String> {
        self.latest_user_prompt().or_else(|| self.title_like_text())
    }

    pub(crate) fn latest_user_prompt(&self) -> Option<String> {
        self.query
            .as_deref()
            .map(str::trim)
            .filter(|query| !query.is_empty())
            .map(str::to_owned)
    }

    /// Returns summary text suitable as a fallback title when no user prompt is available.
    pub(crate) fn title_like_text(&self) -> Option<String> {
        self.summary
            .as_deref()
            .map(str::trim)
            .filter(|summary| !summary.is_empty())
            .map(str::to_owned)
    }
}

/// A tracked CLI agent session.
#[derive(Debug, Clone)]
pub struct CLIAgentSession {
    pub agent: CLIAgent,
    pub status: CLIAgentSessionStatus,
    pub session_context: CLIAgentSessionContext,
    /// Rich input editor state.
    pub input_state: CLIAgentInputState,
    /// Whether status-driven auto-toggle is enabled for this session.
    pub should_auto_toggle_input: bool,
    /// Event listener for plugin-backed sessions or Codex OSC9 fallback.
    /// `None` for non-Codex sessions created by command detection alone.
    /// Dropping this handle cleans up the listener's PTY event subscription.
    pub listener: Option<ModelHandle<CLIAgentSessionListener>>,
    /// The plugin version reported by structured plugin events.
    /// `None` if the plugin predates version reporting or Codex is using OSC9 fallback.
    pub plugin_version: Option<String>,
    /// `None` when the session is local.
    /// `Some("user@hostname")` when running over SSH (warpified or legacy).
    /// Used as a key for per-host plugin install failure tracking.
    pub remote_host: Option<String>,
    /// Draft text saved from the rich input composer when it was closed.
    /// Restored into the editor when the composer is reopened.
    pub draft_text: Option<String>,
    /// When the session was detected via a custom toolbar command pattern,
    /// the first word of the command (the binary/alias the user typed).
    /// Used to customize plugin instructions and force manual install mode.
    pub custom_command_prefix: Option<String>,
    /// Set once the session has received any structured OSC 777 (rich)
    /// notification. Codex's OSC 9 fallback never sets it, so this is the
    /// single source of truth for whether the session is plugin-backed.
    pub received_rich_notification: bool,
    /// Exact user-authored messages. Kept out of `session_context` so status events do not clone it.
    pub prompt_history: AgentPromptHistory,
    pub prompt_history_load_state: PromptHistoryLoadState,
    pub(crate) prompt_history_generation: u64,
}

impl CLIAgentSession {
    pub fn first_prompt(&self) -> Option<&AgentPrompt> {
        self.prompt_history.prompts.first()
    }

    pub fn latest_prompt(&self) -> Option<&AgentPrompt> {
        self.prompt_history.prompts.last()
    }

    pub fn prompt_count(&self) -> usize {
        self.prompt_history.prompts.len()
    }

    /// Returns the newest text that is safe to present as a user prompt in session chrome.
    /// Native Codex OSC 9 bodies are opaque completion notices, even though the legacy status
    /// context stores them in `query`, so they are excluded until structured events are active.
    pub fn latest_user_prompt_for_chrome(&self) -> Option<String> {
        self.latest_prompt()
            .map(|prompt| prompt.text.trim().to_owned())
            .filter(|prompt| !prompt.is_empty())
            .or_else(|| {
                (self.agent != CLIAgent::Codex || self.received_rich_notification)
                    .then(|| self.session_context.latest_user_prompt())
                    .flatten()
            })
    }

    pub fn initial_prompt_title(&self) -> Option<String> {
        self.first_prompt()
            .and_then(|prompt| prompt_title(&prompt.text))
    }

    pub fn title_for_tab(&self, use_latest_prompt: bool) -> Option<String> {
        if use_latest_prompt {
            self.latest_user_prompt_for_chrome()
                .or_else(|| self.initial_prompt_title())
                .or_else(|| self.session_context.title_like_text())
        } else {
            self.initial_prompt_title()
                .or_else(|| self.session_context.title_like_text())
        }
    }

    pub fn session_key(&self) -> Option<CLIAgentSessionKey> {
        let provider = provider_for_agent(self.agent)?;
        let session_id = self.session_context.session_id.as_deref()?.trim();
        (!session_id.is_empty()).then(|| CLIAgentSessionKey {
            provider,
            session_id: session_id.to_owned(),
        })
    }

    fn reset_prompt_history(&mut self) {
        self.prompt_history = AgentPromptHistory::default();
        self.prompt_history_load_state = PromptHistoryLoadState::NotRequested;
        self.prompt_history_generation = self.prompt_history_generation.wrapping_add(1);
    }

    /// Applies a durable provider identity discovered outside the PTY event stream.
    /// Existing listener and status state stay attached to the pane; only history keyed to the
    /// previous identity is invalidated.
    fn seed_session_identity(&mut self, agent: CLIAgent, session_id: String) -> bool {
        if self.agent != agent {
            return false;
        }
        if self.session_context.session_id.as_deref() != Some(session_id.as_str()) {
            self.reset_prompt_history();
            self.session_context.session_id = Some(session_id);
        }
        true
    }

    /// Enforces that listener events belong to the outer session. Identity changes are accepted
    /// only by explicit listener registration or restore seeding, never from a nested PTY event.
    fn prepare_for_event_identity(&mut self, event: &CLIAgentEvent) -> bool {
        if event.agent != self.agent {
            return false;
        }
        if let (Some(current), Some(incoming)) = (
            self.session_context.session_id.as_deref(),
            event.session_id.as_deref(),
        ) {
            if current != incoming {
                return false;
            }
        }
        true
    }

    pub fn is_remote(&self) -> bool {
        self.remote_host.is_some()
    }

    /// Whether the session surfaces trustworthy fine-grained status
    /// (in-progress / blocked / success). True only after receiving a rich OSC
    /// 777 notification. Codex's OSC 9 fallback emits only opaque `Stop`
    /// notifications and never sets `received_rich_notification`, so it does
    /// not qualify. Synthetic listener registration also does not qualify until
    /// an actual rich notification arrives.
    pub fn supports_rich_status(&self) -> bool {
        self.received_rich_notification
    }

    /// Clears state populated by `PermissionRequest`. Called whenever the
    /// session leaves the permission flow (the user replied, a new prompt
    /// is submitted, or the session ends successfully) so the permission
    /// summary doesn't leak into later UI surfaces — most visibly the tab
    /// title, which can fall back to `summary` when `query` is unset.
    fn clear_permission_scoped_state(&mut self) {
        self.session_context.summary = None;
        self.session_context.tool_name = None;
        self.session_context.tool_input_preview = None;
    }

    /// Applies an event to this session, updating context and status.
    /// Returns the new status if it changed, or `None` if the event was irrelevant.
    fn apply_event(&mut self, event: &CLIAgentEvent) -> Option<CLIAgentSessionStatus> {
        self.session_context.cwd = event.cwd.clone().or(self.session_context.cwd.take());
        self.session_context.project = event
            .project
            .clone()
            .or(self.session_context.project.take());
        self.session_context.session_id = event
            .session_id
            .clone()
            .or(self.session_context.session_id.take());
        self.session_context.transcript_path = event
            .payload
            .transcript_path
            .clone()
            .or(self.session_context.transcript_path.take());

        let new_status = match &event.event {
            CLIAgentEventType::PromptSubmit => {
                self.session_context.query = event.payload.query.clone();
                self.session_context.response = None;
                self.clear_permission_scoped_state();
                CLIAgentSessionStatus::InProgress
            }
            CLIAgentEventType::ToolComplete => {
                if !matches!(self.status, CLIAgentSessionStatus::Blocked { .. }) {
                    return None;
                }
                CLIAgentSessionStatus::InProgress
            }
            CLIAgentEventType::Stop => {
                self.session_context.query = event.payload.query.clone();
                self.session_context.response = event.payload.response.clone();
                self.clear_permission_scoped_state();
                CLIAgentSessionStatus::Success
            }
            CLIAgentEventType::PermissionRequest => {
                self.session_context.summary = event.payload.summary.clone();
                self.session_context.tool_name = event.payload.tool_name.clone();
                self.session_context.tool_input_preview = event.payload.tool_input_preview.clone();
                CLIAgentSessionStatus::Blocked {
                    message: event.payload.summary.clone(),
                }
            }
            CLIAgentEventType::QuestionAsked => CLIAgentSessionStatus::Blocked {
                message: event
                    .payload
                    .summary
                    .clone()
                    .or_else(|| Some("Waiting for your answer".to_owned())),
            },
            CLIAgentEventType::PermissionReplied => {
                if !matches!(self.status, CLIAgentSessionStatus::Blocked { .. }) {
                    return None;
                }
                self.clear_permission_scoped_state();
                CLIAgentSessionStatus::InProgress
            }
            // IdlePrompt means the agent is sitting at its prompt waiting for input.
            // This should not affect status — otherwise it would override Success after a Stop event.
            CLIAgentEventType::IdlePrompt => return None,
            CLIAgentEventType::SessionStart => {
                self.plugin_version = event.payload.plugin_version.clone();
                return None;
            }
            CLIAgentEventType::Unknown(_) => return None,
        };

        self.status = new_status.clone();
        Some(new_status)
    }
}

fn provider_for_agent(agent: CLIAgent) -> Option<AgentResumeProvider> {
    match agent {
        CLIAgent::Claude => Some(AgentResumeProvider::Claude),
        CLIAgent::Codex => Some(AgentResumeProvider::Codex),
        _ => None,
    }
}

fn agent_for_provider(provider: AgentResumeProvider) -> CLIAgent {
    match provider {
        AgentResumeProvider::Claude => CLIAgent::Claude,
        AgentResumeProvider::Codex => CLIAgent::Codex,
    }
}

/// Merge a durable prefix with prompts observed while it was loading. Occurrence counts, rather
/// than a set, preserve intentional repeated messages while suppressing the persisted/live copy of
/// the same turn.
fn merge_loaded_and_live_history(
    mut loaded: AgentPromptHistory,
    live: AgentPromptHistory,
) -> AgentPromptHistory {
    let mut durable_occurrences: HashMap<String, usize> = HashMap::new();
    for prompt in &loaded.prompts {
        *durable_occurrences.entry(prompt.text.clone()).or_default() += 1;
    }

    let mut live_occurrences: HashMap<String, usize> = HashMap::new();
    for prompt in live.prompts {
        let occurrence = live_occurrences.entry(prompt.text.clone()).or_default();
        *occurrence += 1;
        if *occurrence > durable_occurrences.get(&prompt.text).copied().unwrap_or(0) {
            loaded.prompts.push(prompt);
        }
    }
    loaded.is_partial |= live.is_partial;
    loaded
}

fn prompt_from_trusted_event(event: &CLIAgentEvent) -> Option<AgentPrompt> {
    if event.source != CLIAgentEventSource::RichPlugin
        || event.event != CLIAgentEventType::PromptSubmit
    {
        return None;
    }
    let text = event.payload.query.as_ref()?;
    if text.trim().is_empty() {
        return None;
    }
    Some(AgentPrompt {
        timestamp: None,
        text: text.clone(),
    })
}

/// Events emitted by `CLIAgentSessionsModel` for subscribers (e.g., `AgentNotificationsModel`).
#[allow(dead_code)] // `agent` fields on Started/InputSessionChanged/Ended are used for logging and future subscribers.
#[derive(Debug, Clone)]
pub enum CLIAgentSessionsModelEvent {
    Started {
        terminal_view_id: EntityId,
        agent: CLIAgent,
    },
    StatusChanged {
        terminal_view_id: EntityId,
        agent: CLIAgent,
        status: CLIAgentSessionStatus,
        session_context: Box<CLIAgentSessionContext>,
    },
    InputSessionChanged {
        terminal_view_id: EntityId,
        agent: CLIAgent,
        /// The input state BEFORE this change. When transitioning from
        /// `Open` → `Closed`, contains the saved input config to restore.
        previous_input_state: CLIAgentInputState,
        /// The input state AFTER this change.
        new_input_state: CLIAgentInputState,
    },
    Ended {
        terminal_view_id: EntityId,
        agent: CLIAgent,
    },
    /// The agent session has been updated. Subscribers may use this as a trigger for best-effort
    /// saving of state derived from the agent's session.
    SessionUpdated {
        terminal_view_id: EntityId,
        agent: CLIAgent,
    },
}

impl CLIAgentSessionsModelEvent {
    pub fn terminal_view_id(&self) -> EntityId {
        match self {
            CLIAgentSessionsModelEvent::Started {
                terminal_view_id, ..
            }
            | CLIAgentSessionsModelEvent::StatusChanged {
                terminal_view_id, ..
            }
            | CLIAgentSessionsModelEvent::InputSessionChanged {
                terminal_view_id, ..
            }
            | CLIAgentSessionsModelEvent::Ended {
                terminal_view_id, ..
            }
            | CLIAgentSessionsModelEvent::SessionUpdated {
                terminal_view_id, ..
            } => *terminal_view_id,
        }
    }
}

/// Singleton model that tracks pane-scoped CLI agent state and plugin-enriched session context.
pub struct CLIAgentSessionsModel {
    sessions: HashMap<EntityId, CLIAgentSession>,
    /// Tracks (agent, remote_host) pairs where an auto plugin operation (install or update) has failed.
    /// Shared across all views so failure in one tab is reflected everywhere.
    plugin_auto_failures: HashSet<(CLIAgent, Option<String>)>,
}

impl Entity for CLIAgentSessionsModel {
    type Event = CLIAgentSessionsModelEvent;
}

impl SingletonEntity for CLIAgentSessionsModel {}

impl CLIAgentSessionsModel {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            plugin_auto_failures: HashSet::new(),
        }
    }

    pub fn session(&self, terminal_view_id: EntityId) -> Option<&CLIAgentSession> {
        self.sessions.get(&terminal_view_id)
    }

    /// Seeds a restored pane from its persisted provider resume command before any plugin event.
    /// The provider session, rather than the new view ID, remains the durable history identity.
    pub fn seed_resumed_session(
        &mut self,
        terminal_view_id: EntityId,
        provider: AgentResumeProvider,
        session_id: String,
        ctx: &mut ModelContext<Self>,
    ) {
        if !session_context_enabled() || session_id.trim().is_empty() {
            return;
        }

        let session_id = session_id.trim().to_owned();
        let agent = agent_for_provider(provider);
        let upgraded_existing = self
            .sessions
            .get_mut(&terminal_view_id)
            .is_some_and(|session| session.seed_session_identity(agent, session_id.clone()));
        if !upgraded_existing {
            self.set_session(
                terminal_view_id,
                CLIAgentSession {
                    agent,
                    status: CLIAgentSessionStatus::InProgress,
                    session_context: CLIAgentSessionContext {
                        session_id: Some(session_id),
                        ..Default::default()
                    },
                    input_state: CLIAgentInputState::Closed,
                    should_auto_toggle_input: false,
                    listener: None,
                    plugin_version: None,
                    remote_host: None,
                    draft_text: None,
                    custom_command_prefix: None,
                    received_rich_notification: false,
                    prompt_history: AgentPromptHistory::default(),
                    prompt_history_load_state: PromptHistoryLoadState::NotRequested,
                    prompt_history_generation: 0,
                },
                ctx,
            );
        }
        self.start_prompt_history_load(terminal_view_id, ctx);
    }

    fn start_prompt_history_load(
        &mut self,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
        if !session_context_enabled() {
            return;
        }

        let Some(session) = self.sessions.get_mut(&terminal_view_id) else {
            return;
        };
        let Some(key) = session.session_key() else {
            return;
        };
        if matches!(
            &session.prompt_history_load_state,
            PromptHistoryLoadState::Loading { key: loading, .. } if loading == &key
        ) || matches!(
            session.prompt_history_load_state,
            PromptHistoryLoadState::Ready
        ) {
            return;
        }

        session.prompt_history_generation = session.prompt_history_generation.wrapping_add(1);
        let generation = session.prompt_history_generation;
        session.prompt_history_load_state = PromptHistoryLoadState::Loading {
            key: key.clone(),
            generation,
        };
        let transcript_path = session
            .session_context
            .transcript_path
            .as_deref()
            .map(PathBuf::from);
        let provider = key.provider;
        let session_id = key.session_id.clone();

        ctx.emit(CLIAgentSessionsModelEvent::SessionUpdated {
            terminal_view_id,
            agent: session.agent,
        });
        ctx.spawn(
            async move {
                unblock(move || {
                    read_prompt_history(provider, &session_id, transcript_path.as_deref())
                })
                .await
            },
            move |model, loaded, ctx| {
                model.finish_prompt_history_load(terminal_view_id, key, generation, loaded, ctx);
            },
        );
    }

    fn finish_prompt_history_load(
        &mut self,
        terminal_view_id: EntityId,
        key: CLIAgentSessionKey,
        generation: u64,
        loaded: AgentPromptHistory,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(session) = self.sessions.get_mut(&terminal_view_id) else {
            return;
        };
        let is_current = session
            .prompt_history_load_state
            .matches_loading(&key, generation)
            && session.session_key().as_ref() == Some(&key);
        if !is_current {
            return;
        }

        session.prompt_history =
            merge_loaded_and_live_history(loaded, std::mem::take(&mut session.prompt_history));
        session.prompt_history_load_state =
            if !session.prompt_history.prompts.is_empty() || session.prompt_history.is_partial {
                PromptHistoryLoadState::Ready
            } else {
                PromptHistoryLoadState::Unavailable
            };
        ctx.emit(CLIAgentSessionsModelEvent::SessionUpdated {
            terminal_view_id,
            agent: session.agent,
        });
    }

    /// Returns `true` if the rich input editor is currently open for this terminal.
    pub fn is_input_open(&self, terminal_view_id: EntityId) -> bool {
        self.sessions
            .get(&terminal_view_id)
            .is_some_and(|s| matches!(s.input_state, CLIAgentInputState::Open { .. }))
    }

    /// Registers a plugin-backed listener on the session for this terminal.
    ///
    /// If a session for the same agent already exists (e.g. created earlier by
    /// command detection), it is upgraded with the listener and plugin context.
    /// Otherwise a new session is created.
    ///
    /// The optional `cwd` / `project` / `session_id` fields supply initial
    /// context when available (e.g. from a `SessionStart` event). Passing
    /// `None` for all three is fine — happens when the plugin is installed
    /// mid-session and there is no start event to extract context from.
    #[allow(clippy::too_many_arguments)]
    pub fn register_listener(
        &mut self,
        terminal_view_id: EntityId,
        agent: CLIAgent,
        cwd: Option<String>,
        project: Option<String>,
        session_id: Option<String>,
        plugin_version: Option<String>,
        remote_host: Option<String>,
        should_auto_toggle_input: bool,
        listener: ModelHandle<CLIAgentSessionListener>,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(session) = self
            .sessions
            .get_mut(&terminal_view_id)
            .filter(|s| s.agent == agent)
        {
            // Upgrade existing session with plugin context.
            session.status = CLIAgentSessionStatus::InProgress;
            session.listener = Some(listener);
            session.plugin_version = plugin_version;
            session.remote_host = remote_host;
            session.should_auto_toggle_input = should_auto_toggle_input;
            session.session_context.cwd = cwd.or(session.session_context.cwd.take());
            session.session_context.project = project.or(session.session_context.project.take());
            if session_id.is_some() && session_id != session.session_context.session_id {
                session.reset_prompt_history();
            }
            session.session_context.session_id =
                session_id.or(session.session_context.session_id.take());
            self.start_prompt_history_load(terminal_view_id, ctx);
            return;
        }

        self.set_session(
            terminal_view_id,
            CLIAgentSession {
                agent,
                status: CLIAgentSessionStatus::InProgress,
                session_context: CLIAgentSessionContext {
                    cwd,
                    project,
                    session_id,
                    ..Default::default()
                },
                input_state: CLIAgentInputState::Closed,
                should_auto_toggle_input,
                listener: Some(listener),
                plugin_version,
                remote_host,
                draft_text: None,
                custom_command_prefix: None,
                received_rich_notification: false,
                prompt_history: AgentPromptHistory::default(),
                prompt_history_load_state: PromptHistoryLoadState::NotRequested,
                prompt_history_generation: 0,
            },
            ctx,
        );
        self.start_prompt_history_load(terminal_view_id, ctx);
    }

    pub fn remove_session(&mut self, terminal_view_id: EntityId, ctx: &mut ModelContext<Self>) {
        if let Some(session) = self.sessions.remove(&terminal_view_id) {
            ctx.emit(CLIAgentSessionsModelEvent::Ended {
                terminal_view_id,
                agent: session.agent,
            });
        }
    }

    /// Updates the session's status and context from a parsed CLI agent event.
    /// Rich plugin events latch `received_rich_notification` so rich-status
    /// surfaces stay consistent even if the first event was not SessionStart.
    pub fn update_from_event(
        &mut self,
        terminal_view_id: EntityId,
        event: &CLIAgentEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(session) = self.sessions.get_mut(&terminal_view_id) else {
            return;
        };

        // The listener is owned by the outer agent process. Ignore every mismatched nested-agent
        // event; explicit listener registration or restore seeding owns identity switches.
        if !session.prepare_for_event_identity(event) {
            return;
        }

        if event.source == CLIAgentEventSource::RichPlugin {
            session.received_rich_notification = true;
        }

        let event_type = &event.event;
        let live_prompt = session_context_enabled()
            .then(|| prompt_from_trusted_event(event))
            .flatten();
        if let Some(new_status) = session.apply_event(event) {
            let agent = session.agent;
            ctx.emit(CLIAgentSessionsModelEvent::StatusChanged {
                terminal_view_id,
                agent,
                status: new_status,
                session_context: Box::new(session.session_context.clone()),
            });
        }

        if let Some(prompt) = live_prompt {
            session.prompt_history.prompts.push(prompt);
        }

        let should_load = matches!(
            event_type,
            CLIAgentEventType::SessionStart | CLIAgentEventType::PromptSubmit
        );

        if matches!(
            event_type,
            CLIAgentEventType::SessionStart
                | CLIAgentEventType::PromptSubmit
                | CLIAgentEventType::ToolComplete
        ) {
            ctx.emit(CLIAgentSessionsModelEvent::SessionUpdated {
                terminal_view_id,
                agent: session.agent,
            });
        }
        if should_load {
            self.start_prompt_history_load(terminal_view_id, ctx);
        }
    }

    pub fn open_input(
        &mut self,
        terminal_view_id: EntityId,
        entrypoint: CLIAgentInputEntrypoint,
        previous_input_config: InputConfig,
        previous_was_lock_set_with_empty_buffer: bool,
        should_auto_toggle_input: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(session) = self.sessions.get_mut(&terminal_view_id) else {
            return;
        };

        let previous_input_state = session.input_state;
        session.input_state = CLIAgentInputState::Open {
            entrypoint,
            previous_input_config,
            previous_was_lock_set_with_empty_buffer,
        };
        session.should_auto_toggle_input = should_auto_toggle_input;

        ctx.emit(CLIAgentSessionsModelEvent::InputSessionChanged {
            terminal_view_id,
            agent: session.agent,
            previous_input_state,
            new_input_state: session.input_state,
        });
    }

    pub fn close_input(
        &mut self,
        terminal_view_id: EntityId,
        should_auto_toggle_input: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(session) = self.sessions.get_mut(&terminal_view_id) else {
            return;
        };
        if session.input_state == CLIAgentInputState::Closed {
            return;
        }

        let previous_input_state = session.input_state;
        session.input_state = CLIAgentInputState::Closed;
        session.should_auto_toggle_input = should_auto_toggle_input;
        ctx.emit(CLIAgentSessionsModelEvent::InputSessionChanged {
            terminal_view_id,
            agent: session.agent,
            previous_input_state,
            new_input_state: CLIAgentInputState::Closed,
        });
    }

    pub fn set_session(
        &mut self,
        terminal_view_id: EntityId,
        session: CLIAgentSession,
        ctx: &mut ModelContext<Self>,
    ) {
        let agent = session.agent;
        // Close any open rich input before replacing, so subscribers can
        // restore input config before the session ends.
        self.close_input(terminal_view_id, false, ctx);
        if let Some(old) = self.sessions.insert(terminal_view_id, session) {
            ctx.emit(CLIAgentSessionsModelEvent::Ended {
                terminal_view_id,
                agent: old.agent,
            });
        }

        ctx.emit(CLIAgentSessionsModelEvent::Started {
            terminal_view_id,
            agent,
        });
    }

    /// Records that an auto plugin operation (install or update) failed for the given agent/host.
    /// `remote_host` is `None` for local sessions, `Some("user@hostname")` for remote.
    #[cfg(not(target_family = "wasm"))]
    pub fn record_plugin_auto_failure(&mut self, agent: CLIAgent, remote_host: Option<String>) {
        self.plugin_auto_failures.insert((agent, remote_host));
    }

    /// Saves draft text from the rich input composer for the given terminal.
    /// Stores `None` for empty or whitespace-only text.
    pub fn set_draft(&mut self, terminal_view_id: EntityId, text: String) {
        if let Some(session) = self.sessions.get_mut(&terminal_view_id) {
            session.draft_text = if text.trim().is_empty() {
                None
            } else {
                Some(text)
            };
        }
    }

    /// Clears any saved draft text for the given terminal.
    pub fn clear_draft(&mut self, terminal_view_id: EntityId) {
        if let Some(session) = self.sessions.get_mut(&terminal_view_id) {
            session.draft_text = None;
        }
    }

    /// Returns and clears the draft text for the given terminal, if any.
    pub fn take_draft(&mut self, terminal_view_id: EntityId) -> Option<String> {
        self.sessions
            .get_mut(&terminal_view_id)
            .and_then(|s| s.draft_text.take())
    }

    /// Whether an auto plugin operation has previously failed for this agent on this host.
    pub fn has_plugin_auto_failed(&self, agent: CLIAgent, remote_host: &Option<String>) -> bool {
        self.plugin_auto_failures
            .contains(&(agent, remote_host.clone()))
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
