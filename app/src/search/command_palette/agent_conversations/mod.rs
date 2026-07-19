//! "Reopen agent conversation" picker: recent CLI-agent (Claude/Codex) conversations
//! from the agent-resume journal, reopened in a new tab via their resume command.

mod data_source;
mod search_item;

pub use data_source::{AgentFilter, DataSource, FolderEntry, ScopeFilter};

use crate::terminal::cli_agent::CLIAgent;

fn cli_agent_for_resume_provider(provider: &str) -> Option<CLIAgent> {
    match provider {
        "claude" => Some(CLIAgent::Claude),
        "codex" => Some(CLIAgent::Codex),
        _ => None,
    }
}

fn provider_display_name(provider: &str) -> &str {
    cli_agent_for_resume_provider(provider)
        .map(|agent| agent.display_name())
        .unwrap_or(provider)
}
