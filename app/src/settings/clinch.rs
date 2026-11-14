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
    remove_unused_worktrees: RemoveUnusedWorktrees {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "clinch.projects.remove_unused_worktrees",
        description: "Remove Clinch's worktrees that nothing has used for a week and that hold no changes, untracked files or commits missing from main. The three most recently used are kept for new tabs.",
    },
    new_tab_chat_view: NewTabChatView {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "clinch.new_tab.chat_view",
        description: "Open new tabs in the chat view, a box that starts Claude Code or Codex with your first message, instead of the terminal. The toggle at the top of a new tab switches between them.",
    },
    new_tab_start_with: NewTabStartWith {
        type: String,
        default: "claude".to_string(),
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "clinch.new_tab.start_with",
        description: "Which agent a new tab's chat view starts: \"claude\" or \"codex\".",
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
    // Gates Clinch's automatic hourly check against its signed GitHub release feed. On by
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
        description: "Check GitHub at most once an hour for a signed Clinch update. When off, Clinch makes no automatic network requests; you can still check on demand from Clinch → Check for Updates…",
    },
    // Decides whether a session cut off mid-turn by a restart is told to carry on. Clinch always
    // reopens the conversation; this is only about the turn in progress.
    continue_sessions_after_restart: ClinchContinueSessionsAfterRestart {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "clinch.sessions.continue_after_restart",
        description: "After Clinch restarts or updates, tell Claude Code, Codex and Devin sessions that were mid-turn to continue where they left off.",
    },
    // An idle Claude Code tab holds about 700 MB, and a Codex tab keeps its helper processes too,
    // so dozens of forgotten tabs can fill memory. A tab asleep has no agent process and wakes on
    // its saved conversation. Zero turns automatic sleep off; the tab menu still sleeps by hand.
    sleep_idle_agent_tabs_after_hours: ClinchSleepIdleAgentTabsAfterHours {
        type: u32,
        default: 2,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "clinch.sessions.sleep_idle_after_hours",
        description: "Put Claude Code and Codex tabs to sleep after this many hours without use, to free memory. A tab that is working, waiting on you, or on screen never sleeps. 0 turns this off.",
    },
    agent_conversation_finder_agent: AgentConversationFinderAgent {
        type: String,
        default: "all".to_string(),
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "clinch.agent_conversation_finder.agent",
        description: "The default agent filter for the agent conversation finder.",
    },
    // Closing a project's coordinator tab turns this off, so a closed coordinator stays closed
    // across launches until the user opens one again from the tab menu.
    coordinator_enabled: CoordinatorSessionEnabled {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "clinch.coordinator.enabled",
        description: "Pin a coordinator Claude Code, Codex or Devin session first in each project's tab list. Closing a coordinator turns this off.",
    },
    coordinator_agent: CoordinatorSessionAgent {
        type: String,
        default: "claude".to_string(),
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "clinch.coordinator.agent",
        description: "The agent that new coordinator sessions start: \"claude\", \"codex\" or \"devin\".",
    },
    // Auto-coordinate spends turns of the user's own Claude or Codex plan, so a coordinator acts
    // on its own only once the user switches it on. Each project's switch overrides this default.
    coordinator_auto_default_on: CoordinatorAutoDefaultOn {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "clinch.coordinator.auto.default_on",
        description: "Let coordinators act on their own in projects where you have not switched auto-coordinate on or off. Each wakeup uses a turn of your Claude or Codex plan.",
    },
    coordinator_auto_watch: CoordinatorAutoWatch {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "clinch.coordinator.auto.watch",
        description: "Auto-coordinate: report sessions that finish, get stuck, or need you.",
    },
    coordinator_auto_name_sessions: CoordinatorAutoNameSessions {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "clinch.coordinator.auto.name_sessions",
        description: "Auto-coordinate: give new sessions a short name after their first turn. Tabs you named are never renamed.",
    },
    coordinator_auto_spot_conflicts: CoordinatorAutoSpotConflicts {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "clinch.coordinator.auto.spot_conflicts",
        description: "Auto-coordinate: warn when two sessions change the same files or branch.",
    },
    coordinator_auto_line_up_shipping: CoordinatorAutoLineUpShipping {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "clinch.coordinator.auto.line_up_shipping",
        description: "Auto-coordinate: line up merges and releases one at a time, asking before each one.",
    },
    coordinator_auto_clean_up: CoordinatorAutoCleanUp {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "clinch.coordinator.auto.clean_up",
        description: "Auto-coordinate: offer to close sessions whose work has merged, asking first.",
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
        "devin" => AgentFilter::Devin,
        "all" => AgentFilter::All,
        _ => AgentFilter::default(),
    }
}

pub fn agent_conversation_finder_agent_value(agent: AgentFilter) -> &'static str {
    match agent {
        AgentFilter::All => "all",
        AgentFilter::Claude => "claude",
        AgentFilter::Codex => "codex",
        AgentFilter::Devin => "devin",
    }
}

#[cfg(test)]
#[path = "clinch_tests.rs"]
mod tests;
