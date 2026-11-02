use std::collections::HashMap;

use settings::Setting as _;
use warp::integration_testing::terminal::util::ExpectedExitStatus;
use warp::integration_testing::terminal::{
    clear_blocklist_to_remove_bootstrapped_blocks, execute_command_for_single_terminal_in_tab,
    execute_echo_str, wait_until_bootstrapped_single_pane_for_tab,
};
use warp::integration_testing::view_getters::workspace_view;
use warp::settings::{AutoCreateWorktreesForNewTabs, Theme};
use warp::themes::theme::ThemeKind;
use warp::workspace::WorkspaceAction;
use warpui_core::integration::TestStep;

use crate::Builder;

fn shot(name: &str) -> TestStep {
    TestStep::new(name).with_take_screenshot(format!("{name}.png"))
}

/// Dispatches a workspace action on the active window, then waits a moment for it to render.
fn workspace_action(name: &str, action: WorkspaceAction) -> TestStep {
    TestStep::new(name)
        .with_action(move |app, window_id, _| {
            let workspace = workspace_view(app, window_id);
            app.update(|ctx| {
                ctx.dispatch_typed_action_for_view(window_id, workspace.id(), &action);
            });
        })
        .set_post_step_pause(std::time::Duration::from_millis(600))
}

fn press(name: &str, keystrokes: &[&str]) -> TestStep {
    TestStep::new(name)
        .with_keystrokes(keystrokes)
        .set_post_step_pause(std::time::Duration::from_millis(400))
}

/// Saves native screenshots of the redesigned surfaces for side-by-side comparison with
/// `web/design-prototype`. Runs in an isolated window and profile:
///
/// ```sh
/// WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS=1 \
///   cargo run -p integration --bin integration -- test_redesign_screens
/// ```
pub fn test_redesign_screens() -> Builder {
    redesign_screens(HashMap::new())
}

/// The same screens in the light theme.
pub fn test_redesign_screens_paper() -> Builder {
    redesign_screens(HashMap::from([(
        Theme::storage_key().to_string(),
        serde_json::to_string(&ThemeKind::ClinchPaper).expect("theme kind serializes"),
    )]))
}

fn redesign_screens(mut user_defaults: HashMap<String, String>) -> Builder {
    // New tabs would otherwise add real worktrees to the repo the test runs inside.
    user_defaults.insert(
        AutoCreateWorktreesForNewTabs::storage_key().to_string(),
        false.to_string(),
    );
    Builder::new()
        .with_real_display()
        .with_user_defaults(user_defaults)
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(execute_echo_str(0, "hello from the redesign"))
        .with_step(press("second tab", &["cmd-t"]))
        .with_step(press("third tab", &["cmd-t"]))
        .with_step(press("back to first tab", &["cmd-1"]))
        .with_step(workspace_action(
            "new section",
            WorkspaceAction::CreateNewTabGroup,
        ))
        .with_step(shot("main"))
        .with_step(press("open palette", &["cmd-p"]))
        .with_step(shot("palette"))
        .with_step(press("close palette", &["escape"]))
        .with_step(press("open find", &["cmd-f"]))
        .with_step(shot("find"))
        .with_step(press("close find", &["escape"]))
        .with_step(press("split right", &["cmd-d"]))
        .with_step(shot("split"))
        .with_step(press("open settings", &["cmd-,"]))
        .with_step(shot("settings"))
}

/// The agent surfaces, against a stand-in `claude` that only waits, so no real agent or account is
/// involved: the row glyph, footer and More menu all key off the command name.
pub fn test_redesign_agent_screens() -> Builder {
    Builder::new()
        .with_real_display()
        .with_user_defaults(HashMap::from([(
            AutoCreateWorktreesForNewTabs::storage_key().to_string(),
            false.to_string(),
        )]))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "mkdir -p ~/bin; printf '#!/bin/sh\\nsleep 60\\n' > ~/bin/claude; chmod +x ~/bin/claude; export PATH=~/bin:$PATH"
                .to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            TestStep::new("start the stand-in agent")
                .with_input_string("claude", Some(&["enter"]))
                .set_post_step_pause(std::time::Duration::from_secs(3)),
        )
        .with_step(workspace_action("open coordinator", WorkspaceAction::OpenCoordinatorSession))
        .with_step(
            TestStep::new("let the coordinator start")
                .set_post_step_pause(std::time::Duration::from_secs(3)),
        )
        .with_step(shot("agent"))
        .with_step(
            TestStep::new("open More")
                .with_click_on_saved_position("agent_footer:more")
                .set_post_step_pause(std::time::Duration::from_millis(400)),
        )
        .with_step(shot("agent-more"))
}
