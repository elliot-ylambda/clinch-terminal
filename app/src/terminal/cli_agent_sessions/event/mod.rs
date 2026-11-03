mod v1;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::terminal::CLIAgent;

#[cfg_attr(not(feature = "local_tty"), allow(dead_code))]
type EventParser = fn(&str) -> Option<CLIAgentEvent>;

/// Sentinel title that identifies structured CLI agent events sent via OSC 777.
/// The `"agent"` field in the JSON body distinguishes which agent sent it.
pub const CLI_AGENT_NOTIFICATION_SENTINEL: &str = "warp://cli-agent";

/// The event type encoded in the `"event"` field of the JSON body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CLIAgentEventType {
    SessionStart,
    PromptSubmit,
    ToolComplete,
    SubagentStart,
    SubagentStop,
    Stop,
    StopFailure,
    PermissionRequest,
    PermissionReplied,
    QuestionAsked,
    IdlePrompt,
    Unknown(String),
}

/// How a CLI agent event reached Warp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CLIAgentEventSource {
    /// Structured OSC 777 notification from a rich plugin.
    RichPlugin,
    /// Native Codex OSC 9 fallback notification.
    CodexOsc9Fallback,
}

/// Provider-reported reason that a turn stopped. This is deliberately
/// narrower than the session status: a normal successful Stop must never be
/// mistaken for a usage-limit stop merely because another pane exhausted the
/// same account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CLIAgentStopReason {
    UsageLimit {
        /// The reset the provider stated in its own limit message (Claude
        /// Code: "resets 6:50pm (America/Los_Angeles)"). Lets auto-continue
        /// schedule without plan-usage data.
        stated_reset: Option<DateTime<Utc>>,
    },
}

impl CLIAgentStopReason {
    /// A usage-limit stop whose reset is parsed from the provider's message.
    pub(crate) fn usage_limit(
        agent: CLIAgent,
        texts: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> Self {
        let now = Utc::now();
        let stated_reset = match agent {
            CLIAgent::Claude => texts
                .into_iter()
                .filter_map(|text| {
                    cli_agent_usage::limit_message::claude_limit_reset(text.as_ref(), now)
                })
                .max(),
            _ => None,
        };
        Self::UsageLimit { stated_reset }
    }
}

/// Event-specific fields that vary by event type.
#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct CLIAgentEventPayload {
    pub query: Option<String>,
    pub response: Option<String>,
    pub transcript_path: Option<String>,
    pub summary: Option<String>,
    pub tool_name: Option<String>,
    pub tool_input_preview: Option<String>,
    /// Stable identity supplied by Claude Code's SubagentStart/SubagentStop hooks.
    pub subagent_id: Option<String>,
    pub plugin_version: Option<String>,
    pub stop_reason: Option<CLIAgentStopReason>,
}

/// Best-effort classifier for legacy Claude notifications that cannot carry
/// the structured `stop_reason` field. Require an explicit limit assertion;
/// generic error text is not causal enough for an automatic PTY write.
///
/// Codex is deliberately absent: Codex runs no hook when a turn ends at its
/// usage limit (the rollout's `task_complete` error is the only record, read
/// by `cli_agent_usage::codex`), so no Codex notification ever carries this text.
pub(crate) fn infer_stop_reason(
    agent: CLIAgent,
    texts: impl IntoIterator<Item = impl AsRef<str>>,
) -> Option<CLIAgentStopReason> {
    if agent != CLIAgent::Claude {
        return None;
    }
    let texts: Vec<String> = texts
        .into_iter()
        .map(|text| text.as_ref().to_owned())
        .collect();
    let usage_limit = texts.iter().any(|text| {
        let text = text.to_ascii_lowercase();
        (text.contains("you've hit your") && text.contains("limit"))
            || text.contains("usage limit reached")
            || text.contains("rate limit reached")
    });
    usage_limit.then(|| CLIAgentStopReason::usage_limit(agent, &texts))
}

/// A parsed event from a CLI agent plugin.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct CLIAgentEvent {
    pub v: u32,
    pub agent: CLIAgent,
    pub event: CLIAgentEventType,
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    pub project: Option<String>,
    pub payload: CLIAgentEventPayload,
    pub source: CLIAgentEventSource,
}

/// Version-specific parsers, indexed by (version - 1).
/// Adding a new version means appending a parser here,
/// which automatically bumps `current_protocol_version()`.
#[cfg_attr(not(feature = "local_tty"), allow(dead_code))]
const VERSIONED_PARSERS: &[EventParser] = &[v1::parse];

/// The current CLI agent protocol version this build of Warp supports.
/// Exported as the `WARP_CLI_AGENT_PROTOCOL_VERSION` env var on the PTY
/// so plugins can negotiate a compatible payload format.
#[cfg_attr(not(feature = "local_tty"), allow(dead_code))]
pub const fn current_protocol_version() -> u32 {
    VERSIONED_PARSERS.len() as u32
}

/// Attempts to parse an OSC 777 `PluggableNotification` into a typed `CLIAgentEvent`.
/// Dispatches to the correct version-specific parser based on the `"v"` field. Returns `None`
/// if the title doesn't match the sentinel, the body isn't valid JSON, or the version is unsupported.
pub fn parse_event(title: Option<&str>, body: &str) -> Option<CLIAgentEvent> {
    if title? != CLI_AGENT_NOTIFICATION_SENTINEL {
        return None;
    }

    let version_probe: VersionProbe = serde_json::from_str(body).ok()?;
    let version = version_probe.v.unwrap_or(1);

    let index = (version as usize).checked_sub(1)?;
    match VERSIONED_PARSERS.get(index) {
        Some(parser) => parser(body),
        None => {
            log::error!(
                "Received CLI agent event with unsupported schema version \
                 {version}. The CLI agent plugin or Warp may need to be updated."
            );
            None
        }
    }
}

#[derive(Deserialize)]
struct VersionProbe {
    v: Option<u32>,
}
