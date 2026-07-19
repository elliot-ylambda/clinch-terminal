use settings::macros::define_settings_group;
use settings::{SupportedPlatforms, SyncToCloud};

use crate::search::command_palette::agent_conversations::{AgentFilter, ScopeFilter};

define_settings_group!(ClinchSettings, settings: [
    auto_create_worktrees_for_new_tabs: AutoCreateWorktreesForNewTabs {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "clinch.projects.auto_create_worktrees_for_new_tabs",
        description: "Create ordinary new terminal and Agent tabs in isolated Git worktrees based on main when the active project is a local Git repository.",
    },
    agent_conversation_finder_scope: AgentConversationFinderScope {
        type: String,
        default: "this_project".to_string(),
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "clinch.agent_conversation_finder.scope",
        description: "The default project scope for the agent conversation finder.",
    },
    agent_conversation_finder_agent: AgentConversationFinderAgent {
        type: String,
        default: "all".to_string(),
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "clinch.agent_conversation_finder.agent",
        description: "The default agent filter for the agent conversation finder.",
    }
]);

pub fn parse_agent_conversation_finder_scope(value: &str) -> ScopeFilter {
    match value {
        "all" => ScopeFilter::All,
        "this_project" => ScopeFilter::ThisProject,
        _ => ScopeFilter::default(),
    }
}

pub fn agent_conversation_finder_scope_value(scope: ScopeFilter) -> &'static str {
    match scope {
        ScopeFilter::ThisProject => "this_project",
        ScopeFilter::All => "all",
    }
}

pub fn parse_agent_conversation_finder_agent(value: &str) -> AgentFilter {
    match value {
        "claude" => AgentFilter::Claude,
        "codex" => AgentFilter::Codex,
        "all" => AgentFilter::All,
        _ => AgentFilter::default(),
    }
}

pub fn agent_conversation_finder_agent_value(agent: AgentFilter) -> &'static str {
    match agent {
        AgentFilter::All => "all",
        AgentFilter::Claude => "claude",
        AgentFilter::Codex => "codex",
    }
}

#[cfg(test)]
#[path = "clinch_tests.rs"]
mod tests;
