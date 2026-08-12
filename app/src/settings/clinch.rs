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
    // Gates Clinch's automatic weekly check against its signed GitHub release feed. On by
    // default so security fixes reach users who never open Settings. Turning it off leaves the
    // app making no automatic network requests at all; Clinch → Check for Updates… still works,
    // and CLINCH_NO_UPDATE_CHECK overrides this setting for headless or managed machines.
    //
    // `SyncToCloud::Never`: Clinch ships with no backend, and this is a per-machine choice.
    automatic_update_check: ClinchAutomaticUpdateCheck {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::MAC,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "clinch.updates.automatic_check",
        description: "Check GitHub once a week for a signed Clinch update. When off, Clinch makes no automatic network requests; you can still check on demand from Clinch → Check for Updates…",
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
        "project_worktrees" => ScopeFilter::ProjectWorktrees,
        "bookmarked" => ScopeFilter::Bookmarked,
        "this_project" => ScopeFilter::ThisProject,
        _ => ScopeFilter::default(),
    }
}

pub fn agent_conversation_finder_scope_value(scope: ScopeFilter) -> &'static str {
    match scope {
        ScopeFilter::ThisProject => "this_project",
        ScopeFilter::ProjectWorktrees => "project_worktrees",
        ScopeFilter::All => "all",
        ScopeFilter::Bookmarked => "bookmarked",
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
