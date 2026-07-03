use cli_agent_usage::format::friendly_model_name;
use cli_agent_usage::UsageSnapshot;

use crate::terminal::CLIAgent;

/// The tab chip label for a CLI-agent session, or `None` when there's nothing to
/// show: unknown agent, no session id, no indexed model, or a noise model id.
pub fn cli_agent_model_label(
    snapshot: &UsageSnapshot,
    agent: CLIAgent,
    session_id: Option<&str>,
) -> Option<String> {
    if matches!(agent, CLIAgent::Unknown) {
        return None;
    }
    let raw = snapshot.model_for_session(session_id?)?;
    let friendly = friendly_model_name(raw);
    if friendly.is_empty() {
        None
    } else {
        Some(friendly)
    }
}

#[cfg(test)]
#[path = "tab_model_label_tests.rs"]
mod tests;
