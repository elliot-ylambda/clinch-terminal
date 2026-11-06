//! Integration-test helpers for Claude Code and Codex panes.
use warpui::integration::{AssertionCallback, TestStep};
use warpui::SingletonEntity;

use crate::integration_testing::view_getters::single_terminal_view_for_tab;
pub use crate::terminal::cli_agent_sessions::background_work::BackgroundWork;
use crate::terminal::cli_agent_sessions::{
    CLIAgentInputState, CLIAgentSession, CLIAgentSessionContext, CLIAgentSessionStatus,
    CLIAgentSessionsModel,
};
use crate::terminal::CLIAgent;

/// A freshly detected `agent` session with no provider events yet.
pub(crate) fn detected_cli_agent_session(agent: CLIAgent) -> CLIAgentSession {
    CLIAgentSession {
        agent,
        status: CLIAgentSessionStatus::InProgress,
        session_context: CLIAgentSessionContext::default(),
        input_state: CLIAgentInputState::Closed,
        should_auto_toggle_input: false,
        listener: None,
        remote_host: None,
        plugin_version: None,
        draft_text: None,
        custom_command_prefix: None,
        received_rich_notification: false,
        has_observed_turn_activity: false,
        turn_interrupted_by_user: false,
        prompt_history: Default::default(),
        prompt_history_load_state: Default::default(),
        prompt_history_generation: 0,
    }
}

/// Marks the terminal in `tab_index` as running `agent`, as command detection would. The agent's
/// command is usually still running, so this step skips the default idle-terminal assertions.
pub fn attach_cli_agent_session(tab_index: usize, agent: CLIAgent) -> TestStep {
    TestStep::new(&format!("Attach a {agent:?} session")).with_action(
        move |app, window_id, _step_data| {
            let terminal_view = single_terminal_view_for_tab(app, window_id, tab_index);
            terminal_view.update(app, |view, ctx| {
                let view_id = view.view_id();
                CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                    sessions.set_session(view_id, detected_cli_agent_session(agent), ctx);
                });
            });
        },
    )
}

/// Asserts the background work shown for the agent session in `tab_index`.
pub fn cli_agent_background_work_is(
    tab_index: usize,
    expected: BackgroundWork,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = single_terminal_view_for_tab(app, window_id, tab_index);
        terminal_view.read(app, |view, ctx| {
            let actual = CLIAgentSessionsModel::as_ref(ctx)
                .session(view.view_id())
                .map(CLIAgentSession::background_work);
            warpui::async_assert_eq!(Some(expected), actual)
        })
    })
}
