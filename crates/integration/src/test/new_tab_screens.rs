use std::collections::HashMap;
use std::time::Duration;

use settings::Setting as _;
use warp::integration_testing::cli_agent::{
    new_tab_view_is, shell_folder_contains, NewTabView, NEW_TAB_CHAT_TOGGLE_POSITION_ID,
    NEW_TAB_TERMINAL_TOGGLE_POSITION_ID,
};
use warp::integration_testing::terminal::util::ExpectedExitStatus;
use warp::integration_testing::terminal::{
    clear_blocklist_to_remove_bootstrapped_blocks, execute_command_for_single_terminal_in_tab,
    wait_until_bootstrapped_single_pane_for_tab,
};
use warp::settings::AutoCreateWorktreesForNewTabs;
use warpui_core::integration::{AssertionCallback, TestStep};

use crate::Builder;

fn shot(name: &str) -> TestStep {
    TestStep::new(name).with_take_screenshot(format!("{name}.png"))
}

fn wait_for(name: &str, assertion: AssertionCallback, timeout: Duration) -> TestStep {
    TestStep::new(name)
        .add_named_assertion(name, assertion)
        .set_timeout(timeout)
}

fn keys(name: &str, keystrokes: &[&str]) -> TestStep {
    TestStep::new(name)
        .with_keystrokes(keystrokes)
        .set_post_step_pause(Duration::from_millis(500))
}

fn click(name: &str, position_id: &'static str) -> TestStep {
    TestStep::new(name)
        .with_click_on_saved_position(position_id)
        .set_post_step_pause(Duration::from_millis(500))
}

/// A new tab in a project: it opens on its terminal with the Terminal/Chat toggle above while a
/// worktree of the project is made in the background, and the shell moves into it. The toggle
/// switches to the chat view and Escape comes back; a command typed in the terminal runs in the
/// worktree and puts the tab to use. Closed, the tab's worktree goes to the next new tab, ignored
/// files and all, and that tab opens on the view the toggle last chose. No agent is started. Uses
/// a repository made in the test's own home folder, so its worktrees land there too.
///
/// ```sh
/// WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS=1 \
///   cargo run -p integration --bin integration -- test_new_tab_start_screen
/// ```
pub fn test_new_tab_start_screen() -> Builder {
    let quick = Duration::from_secs(10);
    let worktree_ready = Duration::from_secs(30);
    Builder::new()
        .with_real_display()
        .with_user_defaults(HashMap::from([
            (
                AutoCreateWorktreesForNewTabs::storage_key().to_string(),
                true.to_string(),
            ),
            // Closing a tab ends its shell then, rather than keeping the tab to reopen, which
            // leaves its worktree in use.
            ("UndoCloseEnabled".to_owned(), false.to_string()),
        ]))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "mkdir -p ~/project && cd ~/project && git init -q -b main \
             && printf 'node_modules/\\n' > .gitignore && git add .gitignore \
             && git -c user.name=Test -c user.email=test@example.com commit -q -m first"
                .to_owned(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(keys("new tab", &["cmd-t"]).set_post_step_pause(Duration::from_millis(200)))
        .with_step(wait_for(
            "the terminal shows with the toggle at once",
            new_tab_view_is(1, Some(NewTabView::Terminal)),
            quick,
        ))
        .with_step(shot("new-tab-1-terminal-preparing"))
        .with_step(wait_for(
            "the shell moves into a new worktree",
            shell_folder_contains(1, "/worktrees/project/"),
            worktree_ready,
        ))
        .with_step(wait_for(
            "the toggle still shows",
            new_tab_view_is(1, Some(NewTabView::Terminal)),
            quick,
        ))
        .with_step(shot("new-tab-2-terminal-ready"))
        .with_step(click("switch to chat", NEW_TAB_CHAT_TOGGLE_POSITION_ID))
        .with_step(wait_for(
            "the chat view shows",
            new_tab_view_is(1, Some(NewTabView::Chat)),
            quick,
        ))
        .with_step(shot("new-tab-3-chat"))
        .with_step(keys("back to the terminal", &["escape"]))
        .with_step(wait_for(
            "the terminal shows again",
            new_tab_view_is(1, Some(NewTabView::Terminal)),
            quick,
        ))
        // An ignored file, which stays with the worktree when the next tab takes it.
        .with_step(
            TestStep::new("run a command in the terminal")
                .with_input_string(
                    "mkdir -p node_modules && echo kept > node_modules/kept.txt",
                    Some(&["enter"]),
                )
                .set_post_step_pause(Duration::from_secs(1)),
        )
        .with_step(wait_for(
            "the tab is in use, so the toggle goes",
            new_tab_view_is(1, None),
            quick,
        ))
        .with_step(shot("new-tab-4-command-ran"))
        .with_step(execute_command_for_single_terminal_in_tab(
            1,
            "test -f node_modules/kept.txt".to_owned(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(keys("close the tab", &["cmd-w"]).set_post_step_pause(Duration::from_secs(3)))
        .with_step(keys("another new tab", &["cmd-t"]))
        .with_step(wait_for(
            "it opens on chat, the view the toggle chose",
            new_tab_view_is(1, Some(NewTabView::Chat)),
            quick,
        ))
        .with_step(wait_for(
            "the next tab gets a worktree",
            shell_folder_contains(1, "/worktrees/project/"),
            worktree_ready,
        ))
        .with_step(shot("new-tab-5-reused-chat"))
        .with_step(click(
            "switch to the terminal",
            NEW_TAB_TERMINAL_TOGGLE_POSITION_ID,
        ))
        .with_step(wait_for(
            "the terminal shows",
            new_tab_view_is(1, Some(NewTabView::Terminal)),
            quick,
        ))
        .with_step(shot("new-tab-6-terminal-again"))
        .with_step(execute_command_for_single_terminal_in_tab(
            1,
            "test -f node_modules/kept.txt".to_owned(),
            ExpectedExitStatus::Success,
            (),
        ))
}
