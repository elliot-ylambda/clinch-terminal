//! Contracts for app-wide CLI-agent discovery and direct coordination.
use serde::{Deserialize, Serialize};

/// Filters use returned opaque IDs. Projects/sections are ORed within each family,
/// and the two families intersect. Empty filters inspect the whole app instance.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentScope {
    #[serde(default)]
    pub projects: Vec<String>,
    #[serde(default)]
    pub sections: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentTargetParams {
    pub agent_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentReadParams {
    pub agent_id: String,
    #[serde(default)]
    pub after: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Start at the most recent bounded portion of the transcript.
    #[serde(default)]
    pub tail: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<AgentRole>,
    /// Include only records with nonempty message text, excluding tool-only records.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub messages_only: bool,
}

pub fn default_limit() -> usize {
    100
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSendParams {
    pub agent_id: String,
    pub text: String,
    /// Caller must acknowledge the current conversation/input revision.
    pub expected_revision: String,
    /// Logical message UUID, retained durably for seven days.
    pub request_id: String,
    #[serde(default)]
    pub queue: bool,
    #[serde(default = "default_expiry")]
    pub expires_in: u32,
    /// A stable UUID for the coordinating conversation; required for queued sends.
    #[serde(default)]
    pub sender_id: Option<String>,
}

pub fn default_expiry() -> u32 {
    1800
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentMessageParams {
    pub request_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentMessageListParams {
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub before: Option<i64>,
    #[serde(default = "default_message_limit")]
    pub limit: u32,
}

pub fn default_message_limit() -> u32 {
    50
}

pub const MAX_PROMPT_BYTES: usize = 64 * 1024;
pub const MAX_READ_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaneReadParams {
    pub pane_id: String,
    pub max_bytes: usize,
}

#[cfg(test)]
#[path = "agents_tests.rs"]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    User,
    Assistant,
    Tool,
}
impl AgentRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        }
    }
}
impl std::str::FromStr for AgentRole {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "user" => Ok(Self::User),
            "assistant" => Ok(Self::Assistant),
            "tool" => Ok(Self::Tool),
            _ => Err("role must be user, assistant, or tool".into()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentProvider {
    Claude,
    Codex,
}
impl AgentProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }
}
impl std::str::FromStr for AgentProvider {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "claude" => Ok(Self::Claude),
            "codex" => Ok(Self::Codex),
            _ => Err("provider must be claude or codex".into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentLaunchParams {
    pub provider: AgentProvider,
    pub project_id: String,
    #[serde(default)]
    pub section_id: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub background: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentInterruptParams {
    pub agent_id: String,
    pub expected_revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentInboxParams {
    pub reader_id: String,
    #[serde(default)]
    pub scope: AgentScope,
    #[serde(default = "default_inbox_limit")]
    pub limit: usize,
}
pub fn default_inbox_limit() -> usize {
    3
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentInboxAckParams {
    pub reader_id: String,
    pub batch_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentEventsParams {
    #[serde(default)]
    pub after: Option<String>,
    #[serde(default)]
    pub scope: AgentScope,
    #[serde(default = "default_limit")]
    pub limit: usize,
}
