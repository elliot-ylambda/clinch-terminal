use settings::Setting as _;

use super::{
    agent_conversation_finder_agent_value, agent_conversation_finder_scope_value,
    parse_agent_conversation_finder_agent, parse_agent_conversation_finder_scope,
    AgentConversationFinderAgent, AgentConversationFinderScope, AutoCreateWorktreesForNewTabs,
};
use crate::search::command_palette::agent_conversations::{AgentFilter, ScopeFilter};

#[test]
fn auto_create_worktrees_for_new_tabs_defaults_to_on() {
    assert!(AutoCreateWorktreesForNewTabs::default_value());
}

#[test]
fn agent_conversation_finder_settings_round_trip_and_legacy_values_fall_back() {
    assert_eq!(
        AgentConversationFinderScope::default_value(),
        "this_project"
    );
    assert_eq!(AgentConversationFinderAgent::default_value(), "all");

    for scope in [
        ScopeFilter::ThisProject,
        ScopeFilter::ProjectWorktrees,
        ScopeFilter::All,
        ScopeFilter::Bookmarked,
    ] {
        assert_eq!(
            parse_agent_conversation_finder_scope(agent_conversation_finder_scope_value(scope)),
            scope
        );
    }
    for agent in [AgentFilter::All, AgentFilter::Claude, AgentFilter::Codex] {
        assert_eq!(
            parse_agent_conversation_finder_agent(agent_conversation_finder_agent_value(agent)),
            agent
        );
    }

    assert_eq!(
        parse_agent_conversation_finder_scope("legacy_scope"),
        ScopeFilter::default()
    );
    assert_eq!(
        parse_agent_conversation_finder_agent("legacy_agent"),
        AgentFilter::default()
    );
}
