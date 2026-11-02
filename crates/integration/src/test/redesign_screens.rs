use warp::integration_testing::terminal::{
    clear_blocklist_to_remove_bootstrapped_blocks, execute_echo_str,
    wait_until_bootstrapped_single_pane_for_tab,
};
use warpui_core::integration::TestStep;

use crate::Builder;

fn shot(name: &str) -> TestStep {
    TestStep::new(name).with_take_screenshot(format!("{name}.png"))
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
    Builder::new()
        .with_real_display()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(execute_echo_str(0, "hello from the redesign"))
        .with_step(shot("main"))
        .with_step(press("open palette", &["cmd-p"]))
        .with_step(shot("palette"))
        .with_step(press("close palette", &["escape"]))
        .with_step(press("open find", &["cmd-f"]))
        .with_step(shot("find"))
        .with_step(press("close find", &["escape"]))
        .with_step(press("open settings", &["cmd-,"]))
        .with_step(shot("settings"))
}
