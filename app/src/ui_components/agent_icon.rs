//! Source-facing helpers that centralize the derivation of the agent-icon shape
//! ([`IconWithStatusVariant`]) from the underlying state models. The invariant the
//! helpers enforce: any single logical agent run renders as the same brand color, glyph,
//! and ambient-vs-local treatment regardless of which surface is rendering it (vertical
//! tabs, pane header, conversation list, notifications mailbox).
//!
//! Each helper is a thin adapter over one data source. Surfaces call the helper for
//! whichever source they hold and feed the resulting variant into
//! [`render_icon_with_status`]. The pure inner functions in this module are exercised
//! directly by the cross-surface consistency tests in `agent_icon_tests.rs`.
use warp_cli::agent::Harness;
use warpui::{AppContext, EntityId, SingletonEntity};

use crate::ai::agent::conversation::ConversationStatus;
use crate::ai::agent_conversations_model::{
    AgentConversationEntry, AgentConversationProvenance, AgentConversationsModel,
    AgentRunDisplayStatus,
};
use crate::ai::agent_management::active_focused_terminal_id;
use crate::terminal::cli_agent_sessions::{CLIAgentSession, CLIAgentSessionsModel};
use crate::terminal::session_settings::SessionSettings;
use crate::terminal::view::TerminalView;
use crate::terminal::CLIAgent;
use crate::ui_components::icon_with_status::IconWithStatusVariant;

/// Returns the agent-icon variant for a live [`TerminalView`], or `None` when the terminal is
/// not an agent surface (plain terminal / shell / empty conversation).
///
/// Resolution order:
/// 1. A [`CLIAgentSessionsModel`] session with a known agent wins. Plugin-backed sessions
///    surface rich status; command-detected sessions don't.
/// 2. A task-backed run uses task status and harness so the terminal chrome and the
///    matching conversation list card stay in lockstep.
/// 3. Live ambient pre-dispatch or a selected local conversation falls through to the
///    no-task waterfall.
/// 4. Everything else returns `None` so the caller renders a plain-terminal indicator.
pub(crate) fn terminal_view_agent_icon_variant(
    terminal_view: &TerminalView,
    app: &AppContext,
) -> Option<IconWithStatusVariant> {
    let cli_agent_session = CLIAgentSessionsModel::as_ref(app).session(terminal_view.id());

    // Resolve the ambient task id from [`TerminalView::ambient_agent_task_id_for_details_panel`],
    // falling back to the selected conversation's server metadata for restored cloud transcripts.
    let ambient_task_id = terminal_view
        .ambient_agent_task_id_for_details_panel(app)
        .or_else(|| {
            terminal_view
                .selected_conversation_server_metadata(app)
                .and_then(|m| m.ambient_agent_task_id)
        });
    let task_data = ambient_task_id
        .and_then(|task_id| AgentConversationsModel::as_ref(app).get_task_data(&task_id));

    // Local orchestration children are dispatched as server tasks (so they carry an ambient
    // task id) but execute on the user's machine, so they must not get the cloud treatment.
    let is_local_child = terminal_view.selected_conversation_is_local_child(app);

    // Defer to the card helper when we have task data and no CLI session takes precedence.
    if cli_agent_session.is_none() {
        if let Some(task) = task_data.as_ref() {
            let status = AgentRunDisplayStatus::from_task(task, app).to_conversation_status();
            let harness = task
                .agent_config_snapshot
                .as_ref()
                .and_then(|config| config.harness.as_ref())
                .map(|harness| harness.harness_type)
                .unwrap_or(Harness::Oz);
            return Some(agent_icon_variant_for_run(harness, status, !is_local_child));
        }
    }

    let is_ambient = terminal_view.is_ambient_agent_session(app)
        || (ambient_task_id.is_some() && !is_local_child);
    let inputs = TerminalIconInputs {
        is_ambient,
        cli_session: cli_agent_session.map(|session| CLISessionInputs {
            agent: session.agent,
            has_listener: session.listener.is_some(),
            status: session.status.to_conversation_status(),
            supports_rich_status: session.supports_rich_status(),
        }),
        selected_third_party_cli_agent: terminal_view
            .ambient_agent_view_model()
            .and_then(|model| model.as_ref(app).selected_third_party_cli_agent()),
        selected_conversation_status: terminal_view.selected_conversation_status_for_display(app),
        has_selected_conversation: terminal_view
            .selected_conversation_display_title(app)
            .is_some(),
    };
    let variant = agent_icon_variant_from_terminal_inputs(&inputs)?;
    Some(apply_awaiting_user_treatment(
        variant,
        terminal_view.id(),
        active_focused_terminal_id(app),
    ))
}

/// Presentational treatment for "the agent's turn ended and it's now waiting on you."
///
/// When a live CLI-agent session has finished a turn (`Success`) on a terminal the user is not
/// currently viewing, render the yellow attention glyph (via `ConversationStatus::Blocked`, which
/// already maps to `yellow_stop_icon`) instead of the ✓ "done" glyph, so a background tab visibly
/// signals "your turn". Presentation only — the session's real `CLIAgentSessionStatus`, the
/// desktop-notification path, and the mailbox are untouched. Pure and focus-parameterized so it is
/// unit-testable without an `AppContext`.
fn apply_awaiting_user_treatment(
    variant: IconWithStatusVariant,
    terminal_view_id: EntityId,
    focused_terminal_id: Option<EntityId>,
) -> IconWithStatusVariant {
    match variant {
        IconWithStatusVariant::CLIAgent {
            agent,
            status: Some(ConversationStatus::Success),
            is_ambient,
        } if focused_terminal_id != Some(terminal_view_id) => IconWithStatusVariant::CLIAgent {
            agent,
            status: Some(ConversationStatus::Blocked {
                blocked_action: String::new(),
            }),
            is_ambient,
        },
        other => other,
    }
}

/// Like [`terminal_view_agent_icon_variant`], but suppresses the CLI-agent (Claude/Codex)
/// variant when the `show_agent_status_on_tabs` setting is off, so the agent status badge
/// is hidden on tab/pane surfaces. Non-CLI variants (Oz/ambient task runs) are unaffected.
pub(crate) fn terminal_view_agent_icon_variant_respecting_tab_setting(
    terminal_view: &TerminalView,
    app: &AppContext,
) -> Option<IconWithStatusVariant> {
    let variant = terminal_view_agent_icon_variant(terminal_view, app)?;
    if matches!(variant, IconWithStatusVariant::CLIAgent { .. })
        && !SessionSettings::as_ref(app)
            .notifications
            .show_agent_status_on_tabs
    {
        return None;
    }
    Some(variant)
}

/// Returns the CLI-agent status that tab chrome can safely display.
///
/// Rich plugin sessions expose their full lifecycle. Command-detected sessions and Codex's
/// native OSC 9 fallback can also reliably expose their initial `InProgress` state because the
/// session is created only after the command is observed running, but their completed states are
/// withheld because those fallback notifications do not describe the full lifecycle.
pub(crate) fn cli_agent_session_status_for_display(
    session: &CLIAgentSession,
) -> Option<ConversationStatus> {
    cli_session_status_for_display(
        &session.status.to_conversation_status(),
        session.listener.is_some(),
        session.supports_rich_status(),
    )
}

pub(crate) fn agent_conversation_entry_icon_variant(
    entry: &AgentConversationEntry,
) -> IconWithStatusVariant {
    let status = entry.display.status.to_conversation_status();
    let is_ambient = matches!(entry.provenance, AgentConversationProvenance::AmbientRun)
        || entry.backing.has_ambient_run
        || entry.identity.ambient_agent_task_id.is_some();
    agent_icon_variant_for_run(
        entry.display.harness.unwrap_or(Harness::Oz),
        status,
        is_ambient,
    )
}

/// Primitive inputs to the terminal-view waterfall, gathered once from the live
/// [`TerminalView`] / [`AppContext`].
struct TerminalIconInputs {
    is_ambient: bool,
    cli_session: Option<CLISessionInputs>,
    /// Third-party CLI agent for a live ambient run before task data is available (e.g.
    /// Claude pre-dispatch). `None` otherwise; task-derived harnesses are handled upstream.
    selected_third_party_cli_agent: Option<CLIAgent>,
    /// The conversation status that the terminal view would surface in its status-icon slot.
    selected_conversation_status: Option<ConversationStatus>,
    /// Whether the terminal view currently has a selected conversation (ambient or local).
    has_selected_conversation: bool,
}

/// CLI-session-derived inputs for the terminal waterfall.
struct CLISessionInputs {
    agent: CLIAgent,
    /// Whether the session is backed by a plugin listener. Plugin-backed sessions report
    /// rich status; command-detected sessions only know that an agent is running.
    has_listener: bool,
    status: ConversationStatus,
    /// Whether the agent's session handler exposes rich status (plugin-backed handlers report
    /// rich status; Codex's OSC 9 handler does not). A non-rich session can still reliably
    /// report its initial in-progress state because it is created only after the CLI command is
    /// observed running; only its completed states are withheld from the icon.
    supports_rich_status: bool,
}

/// Pure waterfall from primitive inputs to an [`IconWithStatusVariant`]. Mirrors the
/// resolution order documented on [`terminal_view_agent_icon_variant`].
fn agent_icon_variant_from_terminal_inputs(
    inputs: &TerminalIconInputs,
) -> Option<IconWithStatusVariant> {
    // 1. CLI session with a known (non-Unknown) agent wins. Rich handlers surface every status.
    //    Command-detected sessions and Codex's OSC 9 fallback still surface InProgress: that
    //    state is trustworthy because the session is created only after its command is observed
    //    running. Their completed states remain hidden because the fallback notifications are
    //    not rich enough to distinguish the full lifecycle.
    if let Some(session) = inputs
        .cli_session
        .as_ref()
        .filter(|s| !matches!(s.agent, CLIAgent::Unknown))
    {
        let status = cli_session_status_for_display(
            &session.status,
            session.has_listener,
            session.supports_rich_status,
        );
        return Some(IconWithStatusVariant::CLIAgent {
            agent: session.agent,
            status,
            is_ambient: inputs.is_ambient,
        });
    }

    // 2. Live ambient run with a third-party harness selected, before task data is
    //    available (e.g. Claude pre-dispatch). `Unknown` is filtered so an unrecognized
    //    harness doesn't render as an unbranded gray circle.
    if inputs.is_ambient {
        if let Some(agent) = inputs
            .selected_third_party_cli_agent
            .filter(|agent| !matches!(agent, CLIAgent::Unknown))
        {
            return Some(IconWithStatusVariant::CLIAgent {
                agent,
                status: inputs.selected_conversation_status.clone(),
                is_ambient: true,
            });
        }
    }

    // 3. Selected conversation OR ambient (Oz) terminal: Oz agent variant.
    if inputs.has_selected_conversation || inputs.is_ambient {
        return Some(IconWithStatusVariant::OzAgent {
            status: inputs.selected_conversation_status.clone(),
            is_ambient: inputs.is_ambient,
        });
    }

    None
}

fn cli_session_status_for_display(
    status: &ConversationStatus,
    has_listener: bool,
    supports_rich_status: bool,
) -> Option<ConversationStatus> {
    ((has_listener && supports_rich_status) || matches!(status, ConversationStatus::InProgress))
        .then(|| status.clone())
}

/// Pure run-card logic: maps a [`Harness`], status, and ambient flag into an
/// [`IconWithStatusVariant`]. Falls back to the Oz variant for [`Harness::Oz`] and
/// [`Harness::Unknown`], the latter so a future-server harness this client doesn't
/// recognize doesn't render an unbranded gray circle.
pub(crate) fn agent_icon_variant_for_run(
    harness: Harness,
    status: ConversationStatus,
    is_ambient: bool,
) -> IconWithStatusVariant {
    let cli_agent =
        CLIAgent::from_harness(harness).filter(|agent| !matches!(agent, CLIAgent::Unknown));
    match cli_agent {
        Some(agent) => IconWithStatusVariant::CLIAgent {
            agent,
            status: Some(status),
            is_ambient,
        },
        None => IconWithStatusVariant::OzAgent {
            status: Some(status),
            is_ambient,
        },
    }
}

#[cfg(test)]
#[path = "agent_icon_tests.rs"]
mod tests;
