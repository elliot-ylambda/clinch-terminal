//! End-to-end check that a CLI agent tab's background-work counts follow what the agent draws.
//!
//! A real shell draws Claude Code's composer and status line (captured from Claude Code 2.1.319)
//! into the alt screen, so the counts travel the production path: PTY bytes, the pane's wakeup,
//! the throttled scan and the session model.

use warp::integration_testing::cli_agent::{
    attach_cli_agent_session, cli_agent_background_work_is, BackgroundWork,
};
use warp::integration_testing::terminal::{
    execute_long_running_command, wait_until_bootstrapped_single_pane_for_tab,
};
use warp::terminal::CLIAgent;
use warpui_core::integration::TestStep;

use super::new_builder;
use crate::Builder;

/// Draws Claude's composer at the bottom of the alt screen with two shells and a monitor running,
/// then three seconds later redraws the status line once they finished. Octal escapes keep the
/// typed command ASCII: `─` rule, `❯` prompt and `·` separator.
const CLAUDE_FRAMES: &str = concat!(
    "sleep 2; R=$(printf '\\342\\224\\200%.0s' 1 2 3 4 5 6 7 8 9 10 11 12); ",
    "printf '\\033[?1049h\\033[2J\\033[999;1H\\033[3A%s\\r\\n\\342\\235\\257 \\r\\n%s\\r\\n",
    "  bypass permissions on \\302\\267 2 shells, 1 monitor' \"$R\" \"$R\"; ",
    "sleep 3; printf '\\r\\033[2K  bypass permissions on'; sleep 60"
);

pub fn test_cli_agent_background_work_follows_the_status_line() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_long_running_command(0, CLAUDE_FRAMES.to_owned()))
        .with_step(attach_cli_agent_session(0, CLIAgent::Claude))
        .with_step(background_work_step(
            "Shells and the monitor appear",
            BackgroundWork {
                shells: 2,
                monitors: 1,
                ..Default::default()
            },
        ))
        .with_step(background_work_step(
            "They clear once the status line drops them",
            BackgroundWork::default(),
        ))
}

/// Waits for the counts while the frames command keeps running.
fn background_work_step(name: &str, expected: BackgroundWork) -> TestStep {
    TestStep::new(name)
        .set_timeout(std::time::Duration::from_secs(10))
        .add_named_assertion(
            format!("background work is {expected:?}"),
            cli_agent_background_work_is(0, expected),
        )
}
