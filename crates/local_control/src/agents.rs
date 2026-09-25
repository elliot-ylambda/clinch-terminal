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
