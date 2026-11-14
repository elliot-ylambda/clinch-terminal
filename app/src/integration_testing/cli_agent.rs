//! Integration-test helpers for Claude Code and Codex panes.
use warpui::integration::{AssertionCallback, TestStep};
use warpui::{App, SingletonEntity, ViewHandle, WindowId};

pub use crate::ai::blocklist::agent_view::agent_input_footer::COORDINATOR_AGENT_PICKER_POSITION_ID;
use crate::integration_testing::view_getters::{pane_group_view, single_terminal_view_for_tab};
pub use crate::terminal::cli_agent_sessions::background_work::BackgroundWork;
use crate::terminal::cli_agent_sessions::{
    CLIAgentInputState, CLIAgentSession, CLIAgentSessionContext, CLIAgentSessionStatus,
    CLIAgentSessionsModel,
};
pub use crate::terminal::view::COORDINATOR_HARNESS_PICKER_POSITION_ID;
#[cfg(feature = "local_fs")]
pub use crate::terminal::view::{
    NewTabView, NEW_TAB_CHAT_TOGGLE_POSITION_ID, NEW_TAB_TERMINAL_TOGGLE_POSITION_ID,
};
use crate::terminal::{CLIAgent, TerminalView};

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

/// The terminal in `tab_index` whose session is active: the tab's only one, or the focused one in
/// a split tab.
fn active_terminal_view(
    app: &App,
    window_id: WindowId,
    tab_index: usize,
) -> ViewHandle<TerminalView> {
    pane_group_view(app, window_id, tab_index).read(app, |pane_group, ctx| {
        pane_group
            .active_session_view(ctx)
            .expect("the tab has an active terminal")
    })
}

/// Asserts that the agent screen in `tab_index`'s active pane contains `needle`: the alt screen in
/// full-screen mode, otherwise the running block's output.
pub fn cli_agent_screen_contains(tab_index: usize, needle: &'static str) -> AssertionCallback {
    cli_agent_screen_shows(tab_index, needle, true)
}

/// Asserts that the agent screen in `tab_index`'s active pane does not contain `needle`.
pub fn cli_agent_screen_lacks(tab_index: usize, needle: &'static str) -> AssertionCallback {
    cli_agent_screen_shows(tab_index, needle, false)
}

fn cli_agent_screen_shows(
    tab_index: usize,
    needle: &'static str,
    shows: bool,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = active_terminal_view(app, window_id, tab_index);
        terminal_view.read(app, |view, _| {
            let screen = agent_screen(view);
            warpui::async_assert!(
                screen.contains(needle) == shows,
                "The agent screen should {}contain {needle:?}, but shows:\n{screen}",
                if shows { "" } else { "not " }
            )
        })
    })
}

/// Asserts that the Coordinator in `tab_index` shows `needle`. A failure also says why
/// auto-coordinate is holding its message, if it is.
pub fn coordinator_screen_contains(tab_index: usize, needle: &'static str) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let summary = crate::integration_testing::view_getters::workspace_view(app, window_id)
            .read(app, |workspace, ctx| workspace.auto_coordinate_summary(ctx));
        let terminal_view = active_terminal_view(app, window_id, tab_index);
        terminal_view.read(app, |view, ctx| {
            let screen = agent_screen(view);
            let (_, unavailable) = view.local_control_agent_readiness(ctx);
            warpui::async_assert!(
                screen.contains(needle),
                "The coordinator should show {needle:?}. Auto-coordinate: {summary} Readiness: \
                 {unavailable:?}. It shows:\n{screen}"
            )
        })
    })
}

/// The agent's screen: the alt screen in full-screen mode, otherwise the running block's output.
fn agent_screen(view: &TerminalView) -> String {
    let model = view.model.lock();
    if model.is_alt_screen_active() {
        model.alt_screen().output_to_string()
    } else {
        model.block_list().active_block().output_to_string()
    }
}

/// Asserts whether Clinch's input box is open on the agent in `tab_index`'s active pane and, when
/// `text` is given, what it holds.
pub fn one_box_input_is(
    tab_index: usize,
    open: bool,
    text: Option<&'static str>,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = active_terminal_view(app, window_id, tab_index);
        let (is_open, input, screen) = terminal_view.read(app, |view, ctx| {
            (
                CLIAgentSessionsModel::as_ref(ctx).is_input_open(view.view_id()),
                view.input().clone(),
                agent_screen(view),
            )
        });
        let buffer = input.read(app, |input, ctx| input.buffer_text(ctx));
        warpui::async_assert!(
            is_open == open && text.is_none_or(|text| buffer == text),
            "The box should be {} holding {text:?}, but is {} holding {buffer:?}. The agent \
             screen shows:\n{screen}",
            if open { "open" } else { "closed" },
            if is_open { "open" } else { "closed" },
        )
    })
}

/// Asserts the ghost text the box in `tab_index` shows after its text: the agent's predicted next
/// prompt, or `None` for none.
pub fn one_box_ghost_text_is(
    tab_index: usize,
    expected: Option<&'static str>,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = active_terminal_view(app, window_id, tab_index);
        let input = terminal_view.read(app, |view, _| view.input().clone());
        let ghost = input.read(app, |input, ctx| {
            input
                .editor()
                .as_ref(ctx)
                .current_autosuggestion_text()
                .map(str::to_owned)
        });
        warpui::async_assert_eq!(expected, ghost.as_deref())
    })
}

/// Asserts that the box in `tab_index` is open and its text starts with `prefix`.
pub fn one_box_input_starts_with(tab_index: usize, prefix: &'static str) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = active_terminal_view(app, window_id, tab_index);
        let (is_open, input) = terminal_view.read(app, |view, ctx| {
            (
                CLIAgentSessionsModel::as_ref(ctx).is_input_open(view.view_id()),
                view.input().clone(),
            )
        });
        let buffer = input.read(app, |input, ctx| input.buffer_text(ctx));
        warpui::async_assert!(
            is_open && buffer.starts_with(prefix),
            "The box should be open holding text starting {prefix:?}, but is {} holding {buffer:?}",
            if is_open { "open" } else { "closed" },
        )
    })
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

/// Asserts what a fresh tab shows in `tab_index`'s active pane: its terminal or its chat view,
/// with the toggle above, or `None` once the tab is in use and the toggle is gone.
#[cfg(feature = "local_fs")]
pub fn new_tab_view_is(tab_index: usize, expected: Option<NewTabView>) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = active_terminal_view(app, window_id, tab_index);
        let actual = terminal_view.read(app, |view, _| view.new_tab_view());
        warpui::async_assert_eq!(expected, actual)
    })
}

/// Asserts the agent a Coordinator footer's picker in `tab_index`'s active pane shows, such as
/// "Codex", or `None` when its footer has no picker.
pub fn coordinator_agent_picker_is(
    tab_index: usize,
    expected: Option<&'static str>,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = active_terminal_view(app, window_id, tab_index);
        let actual = terminal_view.read(app, |view, ctx| {
            view.input()
                .as_ref(ctx)
                .agent_input_footer()
                .as_ref(ctx)
                .coordinator_picker_agent(ctx)
        });
        warpui::async_assert_eq!(expected, actual)
    })
}

/// Asserts that the shell in `tab_index`'s active pane works in a folder whose path contains
/// `needle`.
pub fn shell_folder_contains(tab_index: usize, needle: &'static str) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = active_terminal_view(app, window_id, tab_index);
        let folder = terminal_view.read(app, |view, ctx| {
            view.canonical_session_pwd_if_local(ctx)
                .map(|path| path.as_path().display().to_string())
        });
        warpui::async_assert!(
            folder
                .as_deref()
                .is_some_and(|folder| folder.contains(needle)),
            "The shell should work in a folder containing {needle:?}, but works in {folder:?}"
        )
    })
}
