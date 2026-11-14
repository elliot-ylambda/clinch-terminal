use std::collections::HashMap;
use std::time::Duration;

use settings::Setting as _;
use warp::integration_testing::cli_agent::{
    cli_agent_screen_contains, cli_agent_screen_lacks, coordinator_agent_picker_is,
    coordinator_screen_contains, one_box_ghost_text_is, one_box_input_is,
    one_box_input_starts_with, COORDINATOR_AGENT_PICKER_POSITION_ID,
    COORDINATOR_HARNESS_PICKER_POSITION_ID,
};
use warp::integration_testing::terminal::util::ExpectedExitStatus;
use warp::integration_testing::terminal::{
    clear_blocklist_to_remove_bootstrapped_blocks, execute_command_for_single_terminal_in_tab,
    wait_until_bootstrapped_single_pane_for_tab,
};
use warp::integration_testing::view_getters::workspace_view;
use warp::settings::{
    AutoCreateWorktreesForNewTabs, CoordinatorAutoDefaultOn, CoordinatorSessionAgent,
    OneBoxAgentInput,
};
use warp::workspace::view::auto_coordinate::AUTO_COORDINATE_PILL_POSITION_ID;
use warp::workspace::WorkspaceAction;
use warpui_core::integration::{AssertionCallback, TestStep};

use crate::Builder;

/// A stand-in for Claude Code's screen: its header, its composer between two rules, the status line
/// and mode row under it. From 12 to 19 seconds in it shows a permission dialog instead of the
/// composer, and it redraws on every resize. `STANDIN_INLINE=1` draws on the normal screen
/// instead of the alt screen. `STANDIN_SUGGESTION` is a predicted next prompt to show dim in the
/// empty composer, in place of the fixed example.
const STAND_IN_CLAUDE: &str = r#"#!/bin/sh
[ -n "$STANDIN_INLINE" ] || printf '\033[?1049h'
header() {
  printf '\033[2J\033[1;3H\033[1mClaude Code\033[0m v2.1.319 (stand-in)'
  printf '\033[2;3HOpus 5.5 with xhigh effort · ~\033[4;3H\033[2mUsing Opus 5.5 · /model\033[0m'
}
composer() {
  printf '\033[%d;1H%s' $((rows-4)) "$rule"
  printf '\033[%d;1H❯ \033[2m%s\033[0m' $((rows-3)) "${STANDIN_SUGGESTION:-Try \"how does <filepath> work?\"}"
  printf '\033[%d;1H%s' $((rows-2)) "$rule"
  printf '\033[%d;3Hv2.1.319 | Opus 5.5 xhigh 0%% | ~' $((rows-1))
  printf '\033[%d;3H⏵⏵ auto mode on (shift+tab to cycle) · 2 local agents' "$rows"
  printf '\033[%d;3H' $((rows-3))
}
dialog() {
  printf '\033[%d;1H%s' $((rows-6)) "$rule"
  printf '\033[%d;2HBash command: ls -la' $((rows-5))
  printf '\033[%d;2HDo you want to proceed?' $((rows-4))
  printf '\033[%d;2H❯ 1. Yes' $((rows-3))
  printf '\033[%d;4H2. No' $((rows-2))
  printf '\033[%d;2H\033[2mEsc to cancel\033[0m' $((rows-1))
  printf '\033[%d;4H' $((rows-3))
}
# Like a real TUI, redraw whenever the size changes; the dialog shows from 12s to 19s.
start=$(date +%s)
drawn=
while :; do
  set -- $(stty size)
  rows=$1
  cols=$2
  elapsed=$(( $(date +%s) - start ))
  if [ "$elapsed" -ge 12 ] && [ "$elapsed" -lt 19 ]; then phase=dialog; else phase=composer; fi
  if [ "$phase $rows $cols" != "$drawn" ]; then
    rule=$(awk -v n="$cols" 'BEGIN { for (i = 0; i < n; i++) printf "─" }')
    header
    "$phase"
    drawn="$phase $rows $cols"
  fi
  sleep 0.5
done
"#;

fn shot(name: &str) -> TestStep {
    TestStep::new(name).with_take_screenshot(format!("{name}.png"))
}

fn wait(name: &str, duration: Duration) -> TestStep {
    TestStep::new(name).set_post_step_pause(duration)
}

/// Writes the stand-in as `~/bin/claude` and puts `~/bin` first on every shell's PATH, including
/// the Coordinator's, so no real agent or account is involved.
fn install_stand_in() -> TestStep {
    install_stand_in_as("claude", STAND_IN_CLAUDE)
}

/// Writes `script` as `~/bin/<name>` and puts `~/bin` first on every shell's PATH.
fn install_stand_in_as(name: &str, script: &str) -> TestStep {
    let hex: String = script.bytes().map(|b| format!("{b:02x}")).collect();
    execute_command_for_single_terminal_in_tab(
        0,
        format!(
            "mkdir -p ~/bin && printf %s {hex} | xxd -r -p > ~/bin/{name} && chmod +x ~/bin/{name} \
             && echo 'export PATH=~/bin:$PATH' | tee -a ~/.zshrc >> ~/.bashrc \
             && export PATH=~/bin:$PATH"
        ),
        ExpectedExitStatus::Success,
        (),
    )
}

/// The stand-in with Codex's name in its header, for a Coordinator that switches to Codex.
fn stand_in_codex() -> String {
    STAND_IN_CLAUDE.replace(
        r"\033[1mClaude Code\033[0m v2.1.319 (stand-in)",
        r"\033[1mOpenAI Codex\033[0m (stand-in)",
    )
}

/// Opens this project's Coordinator, as its tab menu does.
fn open_coordinator() -> TestStep {
    TestStep::new("open the coordinator")
        .with_action(|app, window_id, _| {
            let workspace = workspace_view(app, window_id);
            app.update(|ctx| {
                ctx.dispatch_typed_action_for_view(
                    window_id,
                    workspace.id(),
                    &WorkspaceAction::OpenCoordinatorSession,
                );
            });
        })
        .set_post_step_pause(Duration::from_millis(800))
}

/// Switching the Coordinator's agent from its footer, with the box off as it is by default, on
/// stand-ins for Claude Code and Codex: the footer leads with a picker naming the agent, its menu
/// says switching starts a new conversation, picking Codex restarts the Coordinator with Codex,
/// and closing the menu without picking changes nothing.
///
/// ```sh
/// WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS=1 \
///   cargo run -p integration --bin integration -- test_coordinator_agent_picker
/// ```
pub fn test_coordinator_agent_picker() -> Builder {
    let start = Duration::from_secs(30);
    let quick = Duration::from_secs(10);
    Builder::new()
        .with_real_display()
        .with_user_defaults(HashMap::from([(
            AutoCreateWorktreesForNewTabs::storage_key().to_string(),
            false.to_string(),
        )]))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(install_stand_in())
        .with_step(install_stand_in_as("codex", &stand_in_codex()))
        .with_step(open_coordinator())
        .with_step(wait_for(
            "the footer names Claude Code",
            coordinator_agent_picker_is(0, Some("Claude Code")),
            start,
        ))
        .with_step(shot("agent-picker-1-claude"))
        .with_step(
            TestStep::new("open the agent picker")
                .with_click_on_saved_position(COORDINATOR_AGENT_PICKER_POSITION_ID)
                .set_post_step_pause(Duration::from_millis(500)),
        )
        .with_step(shot("agent-picker-2-menu"))
        .with_step(keys("move to Codex", &["down"]))
        .with_step(shot("agent-picker-2b-codex-selected"))
        .with_step(keys("pick it", &["enter"]).set_post_step_pause(Duration::from_secs(1)))
        .with_step(wait_for(
            "the coordinator runs Codex",
            coordinator_agent_picker_is(0, Some("Codex")),
            start,
        ))
        .with_step(wait_for(
            "the stand-in Codex is on screen",
            cli_agent_screen_contains(0, "OpenAI Codex"),
            quick,
        ))
        .with_step(shot("agent-picker-3-codex"))
        .with_step(
            TestStep::new("open it again")
                .with_click_on_saved_position(COORDINATOR_AGENT_PICKER_POSITION_ID)
                .set_post_step_pause(Duration::from_millis(500)),
        )
        .with_step(keys("close it without picking", &["escape"]))
        .with_step(wait_for(
            "still Codex",
            coordinator_agent_picker_is(0, Some("Codex")),
            quick,
        ))
        .with_step(shot("agent-picker-4-closed"))
}

fn wait_for(name: &str, assertion: AssertionCallback, timeout: Duration) -> TestStep {
    TestStep::new(name)
        .add_named_assertion(name, assertion)
        .set_timeout(timeout)
}

fn keys(name: &str, keystrokes: &[&str]) -> TestStep {
    TestStep::new(name)
        .with_keystrokes(keystrokes)
        .set_post_step_pause(Duration::from_millis(300))
}

fn type_text(name: &str, text: &str) -> TestStep {
    TestStep::new(name)
        .with_input_string(text, None)
        .set_post_step_pause(Duration::from_millis(300))
}

/// Runs the installed `claude` and `codex` with the real home directory, so they are signed in,
/// while the shell and Clinch's own state stay in the test's sandbox home. Codex runs without its
/// shared background server, whose hooks would report to the real Clinch. Their hooks find Node
/// in Homebrew's directory. New shells start in `project`, where the Coordinator runs too.
fn use_real_agents(real_home: &str, project: &str) -> TestStep {
    // The sandbox's .zshrc may not end in a newline.
    let rc = format!(
        "\nagent_path='{real_home}/.local/bin:/opt/homebrew/bin'\n\
         claude() {{ HOME='{real_home}' PATH=\"$agent_path:$PATH\" command '{real_home}/.local/bin/claude' \"$@\"; }}\n\
         codex() {{ HOME='{real_home}' PATH=\"$agent_path:$PATH\" command '{real_home}/.local/bin/codex' --no-daemon \"$@\"; }}\n\
         [ \"$PWD\" = \"$HOME\" ] && cd '{project}'\n"
    );
    let hex: String = rc.bytes().map(|b| format!("{b:02x}")).collect();
    execute_command_for_single_terminal_in_tab(
        0,
        format!("printf %s {hex} | xxd -r -p >> ~/.zshrc && . ~/.zshrc && cd '{project}'"),
        ExpectedExitStatus::Success,
        (),
    )
}

/// Native screenshots of one-box input on the real Claude Code and Codex installed and signed in
/// on this machine: each agent's own `/` menu, a rewound message moving into the box, a double
/// Escape clearing the box, and the Coordinator's start screen switching harness with its draft.
/// Sends three one-line prompts.
///
/// ```sh
/// CLINCH_ONE_BOX_REAL_HOME=$HOME CLINCH_ONE_BOX_PROJECT=<a folder both agents trust> \
/// WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS=1 \
///   cargo run -p integration --bin integration -- test_one_box_real_agents
/// ```
pub fn test_one_box_real_agents() -> Builder {
    /// The description of `/effort` in Claude Code's command menu.
    const CLAUDE_EFFORT_ENTRY: &str = "Set effort level";
    /// The description of `/diff` in Codex's command menu.
    const CODEX_DIFF_ENTRY: &str = "show git diff";
    let real_home = std::env::var("CLINCH_ONE_BOX_REAL_HOME")
        .expect("set CLINCH_ONE_BOX_REAL_HOME to the home directory the agents are signed in from");
    let project = std::env::var("CLINCH_ONE_BOX_PROJECT")
        .expect("set CLINCH_ONE_BOX_PROJECT to a folder both agents already trust");
    const OK: &str = "Reply with only the word ok";
    let start = Duration::from_secs(45);
    let turn = Duration::from_secs(90);
    let quick = Duration::from_secs(10);

    Builder::new()
        .with_real_display()
        .with_user_defaults(HashMap::from([
            // The box is off by default for now.
            (
                OneBoxAgentInput::storage_key().to_string(),
                true.to_string(),
            ),
            (
                AutoCreateWorktreesForNewTabs::storage_key().to_string(),
                false.to_string(),
            ),
        ]))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(use_real_agents(&real_home, &project))
        // Claude Code.
        .with_step(type_text("start Claude", "claude --model haiku").with_keystrokes(&["enter"]))
        .with_step(wait_for(
            "Claude ready",
            one_box_input_is(0, true, Some("")),
            start,
        ))
        .with_step(shot("claude-1-ready"))
        // A slash command stays in the box, and Claude's own menu shows above it.
        .with_step(type_text("type a command", "/eff"))
        .with_step(wait_for(
            "Claude's menu lists /effort",
            cli_agent_screen_contains(0, CLAUDE_EFFORT_ENTRY),
            quick,
        ))
        .with_step(wait_for(
            "the box keeps the command",
            one_box_input_is(0, true, Some("/eff")),
            quick,
        ))
        .with_step(shot("claude-2-slash-menu"))
        .with_step(keys("complete it", &["tab"]))
        .with_step(wait_for(
            "the completed command shows in the box",
            one_box_input_is(0, true, Some("/effort ")),
            quick,
        ))
        .with_step(shot("claude-2b-tab-completed"))
        .with_step(keys("delete it", &["escape"]))
        .with_step(wait_for(
            "box empty",
            one_box_input_is(0, true, Some("")),
            quick,
        ))
        .with_step(wait_for(
            "Claude's menu closed",
            cli_agent_screen_lacks(0, CLAUDE_EFFORT_ENTRY),
            quick,
        ))
        .with_step(shot("claude-3-slash-deleted"))
        .with_step(type_text("type a command", "/mo"))
        .with_step(keys("delete a letter", &["backspace"]))
        .with_step(type_text("finish it", "odel"))
        .with_step(wait_for(
            "the box holds /model",
            one_box_input_is(0, true, Some("/model")),
            quick,
        ))
        .with_step(keys("run /model", &["enter"]).set_post_step_pause(Duration::from_secs(2)))
        .with_step(wait_for(
            "box stays aside for the picker",
            one_box_input_is(0, false, None),
            quick,
        ))
        .with_step(shot("claude-3b-model-picker"))
        .with_step(keys("close the picker", &["escape"]))
        .with_step(wait_for(
            "box back after the picker",
            one_box_input_is(0, true, Some("")),
            quick,
        ))
        .with_step(type_text("send", OK).with_keystrokes(&["enter"]))
        .with_step(wait_for(
            "Claude replied",
            cli_agent_screen_contains(0, "· done"),
            turn,
        ))
        .with_step(wait_for(
            "box back after the turn",
            one_box_input_is(0, true, Some("")),
            quick,
        ))
        .with_step(shot("claude-4-replied"))
        // Integration builds don't read session transcripts, so Clinch knows none of the messages
        // and Up goes to the agent's own history, which shows in its composer.
        .with_step(keys("up arrow", &["up"]))
        .with_step(wait_for(
            "Claude's history",
            cli_agent_screen_contains(0, "History"),
            quick,
        ))
        .with_step(wait_for(
            "box steps aside for history",
            one_box_input_is(0, false, None),
            quick,
        ))
        .with_step(shot("claude-4b-up-arrow"))
        .with_step(keys("down arrow", &["down"]))
        .with_step(wait_for(
            "box back after history",
            one_box_input_is(0, true, Some("")),
            quick,
        ))
        .with_step(
            keys("cycle the mode", &["shift-tab"]).set_post_step_pause(Duration::from_millis(800)),
        )
        .with_step(shot("claude-4c-shift-tab"))
        .with_step(type_text("first line", "line one").with_keystrokes(&["shift-enter"]))
        .with_step(type_text("second line", "line two"))
        .with_step(wait_for(
            "two lines in the box",
            one_box_input_is(0, true, Some("line one\nline two")),
            quick,
        ))
        .with_step(shot("claude-4d-two-lines"))
        .with_step(keys("double escape", &["escape", "escape"]))
        .with_step(wait_for(
            "two lines cleared",
            one_box_input_is(0, true, Some("")),
            quick,
        ))
        .with_step(
            TestStep::new("let Claude's double Escape window pass")
                .set_post_step_pause(Duration::from_secs(1)),
        )
        .with_step(keys("open rewind", &["escape", "escape"]))
        .with_step(wait_for(
            "rewind picker",
            cli_agent_screen_contains(0, "Rewind"),
            quick,
        ))
        .with_step(wait_for(
            "box steps aside for rewind",
            one_box_input_is(0, false, None),
            quick,
        ))
        .with_step(shot("claude-5-rewind-picker"))
        .with_step(keys("pick the message", &["up", "enter"]))
        .with_step(wait_for(
            "restore options",
            cli_agent_screen_contains(0, "Restore conversation"),
            quick,
        ))
        .with_step(keys("restore", &["enter"]))
        .with_step(wait_for(
            "restored text moves into the box",
            one_box_input_is(0, true, Some(OK)),
            quick,
        ))
        .with_step(shot("claude-6-restored-into-box"))
        .with_step(keys("double escape", &["escape", "escape"]))
        .with_step(wait_for(
            "double escape clears the box",
            one_box_input_is(0, true, Some("")),
            quick,
        ))
        .with_step(shot("claude-7-double-escape-cleared"))
        .with_step(
            type_text("exit Claude", "/exit")
                .with_keystrokes(&["enter"])
                .set_post_step_pause(Duration::from_secs(3)),
        )
        .with_step(shot("claude-8-exited"))
        // Codex.
        .with_step(
            type_text("start Codex", "codex -c model_reasoning_effort=low")
                .with_keystrokes(&["enter"]),
        )
        .with_step(wait_for(
            "Codex ready",
            one_box_input_is(0, true, Some("")),
            start,
        ))
        .with_step(shot("codex-1-ready"))
        // Codex's welcome text lists /model, so /diff, which only its menu shows.
        .with_step(type_text("type a command", "/dif"))
        .with_step(wait_for(
            "Codex's menu lists /diff",
            cli_agent_screen_contains(0, CODEX_DIFF_ENTRY),
            quick,
        ))
        .with_step(wait_for(
            "the box keeps the command",
            one_box_input_is(0, true, Some("/dif")),
            quick,
        ))
        .with_step(shot("codex-2-slash-menu"))
        .with_step(keys("complete it", &["tab"]))
        .with_step(wait_for(
            "the completed command shows in the box",
            // Codex completes without a trailing space.
            one_box_input_is(0, true, Some("/diff")),
            quick,
        ))
        .with_step(shot("codex-2b-tab-completed"))
        .with_step(keys("delete it", &["escape"]))
        .with_step(wait_for(
            "box empty",
            one_box_input_is(0, true, Some("")),
            quick,
        ))
        .with_step(wait_for(
            "Codex's menu closed",
            cli_agent_screen_lacks(0, CODEX_DIFF_ENTRY),
            quick,
        ))
        .with_step(shot("codex-2c-slash-deleted"))
        .with_step(type_text("type a command", "/model"))
        .with_step(shot("codex-2d-typed-model"))
        .with_step(keys("run /model", &["enter"]).set_post_step_pause(Duration::from_secs(2)))
        .with_step(wait_for(
            "box steps aside for Codex's picker",
            one_box_input_is(0, false, None),
            quick,
        ))
        .with_step(shot("codex-2e-model-picker"))
        .with_step(keys("close the picker", &["escape"]))
        .with_step(wait_for(
            "box back after the picker",
            one_box_input_is(0, true, Some("")),
            quick,
        ))
        .with_step(type_text("send", OK).with_keystrokes(&["enter"]))
        .with_step(wait_for(
            "Codex replied",
            cli_agent_screen_contains(0, "Worked for"),
            turn,
        ))
        .with_step(wait_for(
            "box back after the turn",
            one_box_input_is(0, true, Some("")),
            quick,
        ))
        .with_step(shot("codex-3-replied"))
        // Codex's history fills its composer, and that text moves into the box.
        .with_step(keys("up arrow", &["up"]))
        .with_step(wait_for(
            "Codex's recalled message moves into the box",
            one_box_input_is(0, true, Some(OK)),
            quick,
        ))
        .with_step(shot("codex-3b-up-arrow"))
        .with_step(keys("double escape", &["escape", "escape"]))
        .with_step(wait_for(
            "box cleared",
            one_box_input_is(0, true, Some("")),
            quick,
        ))
        .with_step(
            TestStep::new("let the double Escape window pass")
                .set_post_step_pause(Duration::from_secs(1)),
        )
        .with_step(keys("browse the transcript", &["escape", "escape"]))
        .with_step(wait_for(
            "transcript",
            cli_agent_screen_contains(0, "Browsing transcript"),
            quick,
        ))
        .with_step(wait_for(
            "box steps aside",
            one_box_input_is(0, false, None),
            quick,
        ))
        .with_step(shot("codex-4-browsing"))
        .with_step(keys("rewind", &["enter"]))
        .with_step(wait_for(
            "rewound text moves into the box",
            one_box_input_is(0, true, Some(OK)),
            quick,
        ))
        .with_step(shot("codex-5-rewound-into-box"))
        .with_step(keys("double escape", &["escape", "escape"]))
        .with_step(wait_for(
            "box cleared",
            one_box_input_is(0, true, Some("")),
            quick,
        ))
        .with_step(
            type_text("quit Codex", "/quit")
                .with_keystrokes(&["enter"])
                .set_post_step_pause(Duration::from_secs(3)),
        )
        // The Coordinator, which is pinned first.
        .with_step(open_coordinator())
        .with_step(shot("coordinator-1-starting"))
        .with_step(wait_for(
            "coordinator ready",
            one_box_input_is(0, true, Some("")),
            start,
        ))
        .with_step(shot("coordinator-2-start-screen"))
        // A command makes way for the agent's menu; deleting it brings the start screen back.
        .with_step(type_text("type a command", "/eff"))
        .with_step(wait_for(
            "Claude's menu over the start screen's place",
            cli_agent_screen_contains(0, CLAUDE_EFFORT_ENTRY),
            quick,
        ))
        .with_step(wait_for(
            "the box keeps the command",
            one_box_input_is(0, true, Some("/eff")),
            quick,
        ))
        .with_step(shot("coordinator-2b-slash-menu"))
        .with_step(keys("delete it", &["escape"]))
        .with_step(wait_for(
            "box empty",
            one_box_input_is(0, true, Some("")),
            quick,
        ))
        .with_step(shot("coordinator-2c-start-screen-again"))
        .with_step(type_text("type a draft", "hello from the start screen"))
        .with_step(
            TestStep::new("open the harness picker")
                .with_click_on_saved_position(COORDINATOR_HARNESS_PICKER_POSITION_ID)
                .set_post_step_pause(Duration::from_millis(500)),
        )
        .with_step(shot("coordinator-3-picker"))
        .with_step(
            keys("pick Codex", &["down", "enter"]).set_post_step_pause(Duration::from_secs(1)),
        )
        .with_step(shot("coordinator-4-switching"))
        .with_step(wait_for(
            "Codex coordinator keeps the draft",
            one_box_input_is(0, true, Some("hello from the start screen")),
            start,
        ))
        .with_step(shot("coordinator-5-codex-start-screen"))
        .with_step(keys("select the draft", &["cmd-a"]))
        .with_step(type_text("send", OK).with_keystrokes(&["enter"]))
        .with_step(wait_for(
            "coordinator replied",
            cli_agent_screen_contains(0, "Worked for"),
            turn,
        ))
        .with_step(shot("coordinator-6-after-first-message"))
}

/// Switching auto-coordinate on for a new Coordinator with the box off, as it is by default, on
/// the signed-in Codex: its first message is the one that starts the loop, not "back on", and it
/// goes out once Codex has started even though the switch was flipped while it was starting.
/// Spends one turn, which checks every session in the test's empty project.
///
/// ```sh
/// CLINCH_ONE_BOX_REAL_HOME=$HOME CLINCH_ONE_BOX_PROJECT=<a folder Codex trusts> \
/// WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS=1 \
///   cargo run -p integration --bin integration -- test_coordinator_auto_on_kickoff
/// ```
pub fn test_coordinator_auto_on_kickoff() -> Builder {
    const KICKOFF: &str = "Start auto-coordinating this project";
    let real_home = std::env::var("CLINCH_ONE_BOX_REAL_HOME")
        .expect("set CLINCH_ONE_BOX_REAL_HOME to the home directory the agents are signed in from");
    let project = std::env::var("CLINCH_ONE_BOX_PROJECT")
        .expect("set CLINCH_ONE_BOX_PROJECT to a folder Codex already trusts");

    Builder::new()
        .with_real_display()
        .with_user_defaults(HashMap::from([
            (
                AutoCreateWorktreesForNewTabs::storage_key().to_string(),
                false.to_string(),
            ),
            (
                CoordinatorAutoDefaultOn::storage_key().to_string(),
                false.to_string(),
            ),
            (
                CoordinatorSessionAgent::storage_key().to_string(),
                serde_json::to_string("codex").expect("a string serializes"),
            ),
        ]))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(use_real_agents(&real_home, &project))
        .with_step(open_coordinator().set_post_step_pause(Duration::from_millis(300)))
        // Flipped while Codex is still starting, as a user does right after opening it.
        .with_step(set_auto_coordinate("auto-coordinate on", true))
        .with_step(shot("kickoff-1-auto-on-while-starting"))
        .with_step(wait_for(
            "the coordinator gets the message that starts the loop",
            coordinator_screen_contains(0, KICKOFF),
            Duration::from_secs(60),
        ))
        .with_step(wait_for(
            "not the one for picking up after a pause",
            cli_agent_screen_lacks(0, "Auto-coordinate is back on"),
            Duration::from_secs(5),
        ))
        .with_step(shot("kickoff-2-sent"))
}

/// Switches the Coordinator's auto-coordinate on or off, as its row's switch does.
fn set_auto_coordinate(name: &str, on: bool) -> TestStep {
    TestStep::new(name)
        .with_action(move |app, window_id, _| {
            let workspace = workspace_view(app, window_id);
            app.update(|ctx| {
                ctx.dispatch_typed_action_for_view(
                    window_id,
                    workspace.id(),
                    &WorkspaceAction::SetAutoCoordinate(on),
                );
            });
        })
        .set_post_step_pause(Duration::from_millis(800))
}

/// Auto-coordinate's loop on a real Codex Coordinator. With auto-coordinate on, the start screen
/// offers the first message that starts the loop, and switching it off and on before anything is
/// sent only withdraws and offers it again. Once the conversation has started, switching off
/// types "Pause all loops" into the Coordinator, which stays open, and switching on asks it to
/// pick up again. Sends one one-line prompt; the pause and the resume each spend a turn.
///
/// ```sh
/// CLINCH_ONE_BOX_REAL_HOME=$HOME CLINCH_ONE_BOX_PROJECT=<a folder both agents trust> \
/// WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS=1 \
///   cargo run -p integration --bin integration -- test_coordinator_auto_pause
/// ```
pub fn test_coordinator_auto_pause() -> Builder {
    const KICKOFF: &str = "Start auto-coordinating this project";
    const OK: &str = "Reply with only the word ok";
    let real_home = std::env::var("CLINCH_ONE_BOX_REAL_HOME")
        .expect("set CLINCH_ONE_BOX_REAL_HOME to the home directory the agents are signed in from");
    let project = std::env::var("CLINCH_ONE_BOX_PROJECT")
        .expect("set CLINCH_ONE_BOX_PROJECT to a folder both agents already trust");
    let start = Duration::from_secs(45);
    let turn = Duration::from_secs(150);
    let quick = Duration::from_secs(10);

    Builder::new()
        .with_real_display()
        .with_user_defaults(HashMap::from([
            // The box is off by default for now.
            (
                OneBoxAgentInput::storage_key().to_string(),
                true.to_string(),
            ),
            (
                AutoCreateWorktreesForNewTabs::storage_key().to_string(),
                false.to_string(),
            ),
            (
                CoordinatorAutoDefaultOn::storage_key().to_string(),
                true.to_string(),
            ),
            (
                CoordinatorSessionAgent::storage_key().to_string(),
                serde_json::to_string("codex").expect("a string serializes"),
            ),
        ]))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(use_real_agents(&real_home, &project))
        .with_step(open_coordinator())
        .with_step(wait_for(
            "the start screen offers the first message",
            one_box_input_starts_with(0, KICKOFF),
            start,
        ))
        .with_step(shot("auto-1-kickoff-offered"))
        .with_step(set_auto_coordinate(
            "auto-coordinate off before starting",
            false,
        ))
        .with_step(wait_for(
            "the offer is withdrawn",
            one_box_input_is(0, true, Some("")),
            quick,
        ))
        .with_step(wait_for(
            "nothing was sent",
            cli_agent_screen_lacks(0, "Pause all loops"),
            quick,
        ))
        .with_step(shot("auto-2-offer-withdrawn"))
        .with_step(set_auto_coordinate("auto-coordinate back on", true))
        .with_step(wait_for(
            "the offer is back",
            one_box_input_starts_with(0, KICKOFF),
            quick,
        ))
        // A cheap first message in place of the kickoff, which would check every session.
        .with_step(keys("select the offer", &["cmd-a"]))
        .with_step(type_text("send", OK).with_keystrokes(&["enter"]))
        .with_step(wait_for(
            "the coordinator replied",
            cli_agent_screen_contains(0, "Worked for"),
            turn,
        ))
        .with_step(shot("auto-3-conversation-started"))
        .with_step(set_auto_coordinate("pause auto-coordinate", false))
        .with_step(wait_for(
            "the coordinator is told to pause all loops",
            cli_agent_screen_contains(0, "Pause all loops"),
            turn,
        ))
        .with_step(wait("let it answer", Duration::from_secs(20)))
        .with_step(shot("auto-4-paused"))
        .with_step(
            TestStep::new("open auto-coordinate settings")
                .with_click_on_saved_position(AUTO_COORDINATE_PILL_POSITION_ID)
                .set_post_step_pause(Duration::from_millis(800)),
        )
        .with_step(shot("auto-5-paused-settings"))
        .with_step(
            TestStep::new("close auto-coordinate settings")
                .with_click_on_saved_position(AUTO_COORDINATE_PILL_POSITION_ID)
                .set_post_step_pause(Duration::from_millis(500)),
        )
        .with_step(set_auto_coordinate("resume auto-coordinate", true))
        .with_step(wait_for(
            "the coordinator is told to pick up again",
            cli_agent_screen_contains(0, "Auto-coordinate is back on"),
            turn,
        ))
        .with_step(shot("auto-6-resumed"))
}

/// One-box slash commands on the real Claude Code and Codex in the cases
/// [`test_one_box_real_agents`] leaves out:
/// - Claude Code's classic renderer, which draws its menu under the composer
/// - Enter on part of a command, and a command typed with its argument
/// - a command typed while the agent works, then covered by a permission dialog
/// - Codex's Tab while it works, which completes rather than queues
/// - a narrow split pane, where a long command and the agent's hint wrap
/// - closing and reopening the box with a command in it
///
/// Sends two prompts that run a short `sleep`, and has Claude Code create two empty files in the
/// project folder, since it asks before running commands that write.
///
/// ```sh
/// CLINCH_ONE_BOX_REAL_HOME=$HOME CLINCH_ONE_BOX_PROJECT=<a folder both agents trust> \
/// WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS=1 \
///   cargo run -p integration --bin integration -- test_one_box_command_edges
/// ```
pub fn test_one_box_command_edges() -> Builder {
    const CLAUDE_EFFORT_ENTRY: &str = "Set effort level";
    const CLAUDE_RENAME_ENTRY: &str = "Rename the current conversation";
    const CLAUDE_MODEL_ENTRY: &str = "Set the AI model";
    const CLAUDE_PERMISSION: &str = "Do you want to proceed?";
    const CLAUDE_RENAMED: &str = "Session renamed to";
    const CODEX_DIFF_ENTRY: &str = "show git diff";
    const LONG_RENAME: &str = "/rename a-long-conversation-name-that-wraps-in-this-narrow-pane";
    let real_home = std::env::var("CLINCH_ONE_BOX_REAL_HOME")
        .expect("set CLINCH_ONE_BOX_REAL_HOME to the home directory the agents are signed in from");
    let project = std::env::var("CLINCH_ONE_BOX_PROJECT")
        .expect("set CLINCH_ONE_BOX_PROJECT to a folder both agents already trust");
    let start = Duration::from_secs(45);
    let turn = Duration::from_secs(90);
    let quick = Duration::from_secs(10);

    let builder = Builder::new()
        .with_real_display()
        .with_user_defaults(HashMap::from([
            // The box is off by default for now.
            (OneBoxAgentInput::storage_key().to_string(), true.to_string()),(
            AutoCreateWorktreesForNewTabs::storage_key().to_string(),
            false.to_string(),
        )]))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(use_real_agents(&real_home, &project))
        // Claude Code's classic renderer, asking before it runs commands.
        .with_step(
            type_text(
                "start classic Claude",
                "CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN=1 claude --model haiku --permission-mode manual",
            )
            .with_keystrokes(&["enter"]),
        )
        .with_step(wait_for(
            "Claude ready",
            one_box_input_is(0, true, Some("")),
            start,
        ))
        .with_step(shot("classic-1-ready"))
        .with_step(type_text("type a command", "/eff"))
        .with_step(wait_for(
            "Claude's menu lists /effort under the composer",
            cli_agent_screen_contains(0, CLAUDE_EFFORT_ENTRY),
            quick,
        ))
        .with_step(wait_for(
            "the box keeps the command",
            one_box_input_is(0, true, Some("/eff")),
            quick,
        ))
        .with_step(shot("classic-2-menu-below"))
        .with_step(keys("complete it", &["tab"]))
        .with_step(wait_for(
            "the completed command shows in the box",
            one_box_input_is(0, true, Some("/effort ")),
            quick,
        ))
        .with_step(shot("classic-3-tab-completed"))
        .with_step(keys("delete it", &["escape"]))
        .with_step(wait_for(
            "box empty",
            one_box_input_is(0, true, Some("")),
            quick,
        ))
        .with_step(wait_for(
            "Claude's menu closed",
            cli_agent_screen_lacks(0, CLAUDE_EFFORT_ENTRY),
            quick,
        ))
        // Enter on part of a command runs the one the menu picked, here without its optional
        // argument, which a conversation with no messages can't make up.
        .with_step(type_text("type part of a command", "/renam"))
        .with_step(wait_for(
            "Claude's menu lists /rename",
            cli_agent_screen_contains(0, CLAUDE_RENAME_ENTRY),
            quick,
        ))
        .with_step(keys("run it", &["enter"]))
        .with_step(wait_for(
            "box empty after Enter ran it",
            one_box_input_is(0, true, Some("")),
            quick,
        ))
        .with_step(wait_for(
            "Claude ran /rename",
            cli_agent_screen_contains(0, "Usage: /rename"),
            quick,
        ))
        .with_step(shot("classic-4-enter-ran"))
        .with_step(type_text("type a command and its argument", "/rename one-box-edges"))
        .with_step(wait_for(
            "the box holds the whole command",
            one_box_input_is(0, true, Some("/rename one-box-edges")),
            quick,
        ))
        .with_step(keys("run it", &["enter"]))
        .with_step(wait_for(
            "Claude ran it with the argument",
            cli_agent_screen_contains(0, "renamed to: one-box-edges"),
            quick,
        ))
        .with_step(wait_for(
            "box empty after running it",
            one_box_input_is(0, true, Some("")),
            quick,
        ))
        .with_step(shot("classic-5-renamed"))
        // A command typed while Claude works, then covered by its permission dialog.
        .with_step(
            type_text(
                "send",
                "Run sleep 8 && touch one-box-a.txt with the Bash tool. When it finishes, run \
                 touch one-box-b.txt with the Bash tool. Then reply with only the word ok",
            )
            .with_keystrokes(&["enter"]),
        )
        .with_step(wait_for(
            "the box steps aside for the first permission dialog",
            one_box_input_is(0, false, None),
            turn,
        ))
        .with_step(wait_for(
            "first permission dialog",
            cli_agent_screen_contains(0, CLAUDE_PERMISSION),
            quick,
        ))
        .with_step(shot("classic-6-first-dialog"))
        .with_step(keys("allow the first command", &["enter"]))
        .with_step(wait_for(
            "box back while the command runs",
            one_box_input_is(0, true, Some("")),
            quick,
        ))
        .with_step(type_text("type a command while Claude works", "/eff"))
        .with_step(wait_for(
            "Claude's menu while it works",
            cli_agent_screen_contains(0, CLAUDE_EFFORT_ENTRY),
            quick,
        ))
        .with_step(wait_for(
            "the box keeps the command while Claude works",
            one_box_input_is(0, true, Some("/eff")),
            quick,
        ))
        .with_step(shot("classic-7-busy-menu"))
        .with_step(wait_for(
            "the box steps aside for the second dialog",
            one_box_input_is(0, false, None),
            turn,
        ))
        .with_step(shot("classic-8-dialog-over-command"))
        .with_step(keys("allow the second command", &["enter"]))
        .with_step(wait_for(
            "the command comes back into the box",
            one_box_input_is(0, true, Some("/eff")),
            turn,
        ))
        .with_step(wait_for(
            "Claude's menu is back",
            cli_agent_screen_contains(0, CLAUDE_EFFORT_ENTRY),
            quick,
        ))
        .with_step(shot("classic-9-command-back"))
        .with_step(keys("delete it", &["escape"]))
        .with_step(wait_for(
            "box empty",
            one_box_input_is(0, true, Some("")),
            quick,
        ))
        .with_step(wait("let the turn finish", Duration::from_secs(5)))
        .with_step(shot("classic-10-turn-done"))
        .with_step(
            type_text("exit Claude", "/exit")
                .with_keystrokes(&["enter"])
                .set_post_step_pause(Duration::from_secs(3)),
        )
        // Codex, working.
        .with_step(
            type_text("start Codex", "codex -c model_reasoning_effort=low -a never")
                .with_keystrokes(&["enter"]),
        )
        .with_step(wait_for(
            "Codex ready",
            one_box_input_is(0, true, Some("")),
            start,
        ))
        .with_step(
            type_text(
                "send",
                "Run sleep 12 in the shell, then reply with only the word ok",
            )
            .with_keystrokes(&["enter"]),
        )
        .with_step(wait_for(
            "Codex works",
            cli_agent_screen_contains(0, "sleep 12"),
            turn,
        ))
        .with_step(wait_for(
            "box open while Codex works",
            one_box_input_is(0, true, Some("")),
            quick,
        ))
        .with_step(type_text("type a command while Codex works", "/dif"))
        .with_step(wait_for(
            "Codex's menu while it works",
            cli_agent_screen_contains(0, CODEX_DIFF_ENTRY),
            quick,
        ))
        .with_step(shot("codex-1-busy-menu"))
        .with_step(keys("Tab while Codex works", &["tab"]))
        .with_step(wait_for(
            "Tab completed the command instead of queueing it",
            one_box_input_is(0, true, Some("/diff")),
            quick,
        ))
        .with_step(shot("codex-2-busy-tab"))
        .with_step(keys("delete it", &["escape"]))
        .with_step(wait_for(
            "box empty",
            one_box_input_is(0, true, Some("")),
            quick,
        ))
        .with_step(wait_for(
            "Escape left the turn running, and it finished",
            cli_agent_screen_contains(0, "Worked for"),
            turn,
        ))
        .with_step(shot("codex-3-turn-done"))
        // Enter on part of a command runs the command Codex's menu picked.
        .with_step(type_text("type part of a command", "/dif"))
        .with_step(wait_for(
            "Codex's menu lists /diff",
            cli_agent_screen_contains(0, CODEX_DIFF_ENTRY),
            quick,
        ))
        .with_step(keys("run it", &["enter"]))
        .with_step(wait_for(
            "Codex ran /diff",
            cli_agent_screen_contains(0, "not inside a git repository"),
            quick,
        ))
        .with_step(wait_for(
            "box empty after running it",
            one_box_input_is(0, true, Some("")),
            quick,
        ))
        .with_step(shot("codex-4-ran-diff"))
        .with_step(
            type_text("quit Codex", "/quit")
                .with_keystrokes(&["enter"])
                .set_post_step_pause(Duration::from_secs(3)),
        );
    // Claude Code's fullscreen renderer in the newest of three side-by-side panes, which has focus.
    builder
        .with_step(keys("split right", &["cmd-d"]).set_post_step_pause(Duration::from_secs(3)))
        .with_step(keys("split right", &["cmd-d"]).set_post_step_pause(Duration::from_secs(3)))
        .with_step(type_text("start Claude", "claude --model haiku").with_keystrokes(&["enter"]))
        .with_step(wait_for(
            "Claude ready in the narrow pane",
            one_box_input_is(0, true, Some("")),
            start,
        ))
        .with_step(shot("narrow-1-ready"))
        .with_step(type_text("type a command", "/eff"))
        .with_step(wait_for(
            "Claude's menu in the narrow pane",
            cli_agent_screen_contains(0, CLAUDE_EFFORT_ENTRY),
            quick,
        ))
        .with_step(keys("complete it", &["tab"]))
        .with_step(wait_for(
            "the completed command, without the wrapped hint",
            one_box_input_is(0, true, Some("/effort ")),
            quick,
        ))
        .with_step(shot("narrow-2-hint"))
        .with_step(keys("delete it", &["escape"]))
        .with_step(wait_for(
            "box empty",
            one_box_input_is(0, true, Some("")),
            quick,
        ))
        .with_step(type_text("type a long command", LONG_RENAME))
        .with_step(wait_for(
            "the box holds the long command",
            one_box_input_is(0, true, Some(LONG_RENAME)),
            quick,
        ))
        .with_step(shot("narrow-3-long-command"))
        .with_step(keys("run it", &["enter"]))
        .with_step(wait_for(
            "Claude ran the long command",
            cli_agent_screen_contains(0, CLAUDE_RENAMED),
            quick,
        ))
        .with_step(wait_for(
            "box empty after running it",
            one_box_input_is(0, true, Some("")),
            quick,
        ))
        // Closing the box takes its command out of Claude's composer; reopening puts it back.
        .with_step(type_text("type a command", "/mo"))
        .with_step(wait_for(
            "Claude's menu lists /model",
            cli_agent_screen_contains(0, CLAUDE_MODEL_ENTRY),
            quick,
        ))
        .with_step(keys("close the box", &["ctrl-g"]))
        .with_step(wait_for(
            "box closed",
            one_box_input_is(0, false, None),
            quick,
        ))
        .with_step(wait_for(
            "Claude's composer is empty again",
            cli_agent_screen_lacks(0, CLAUDE_MODEL_ENTRY),
            quick,
        ))
        .with_step(shot("narrow-4-box-closed"))
        .with_step(keys("reopen the box", &["ctrl-g"]))
        .with_step(wait_for(
            "the box reopens with the command",
            one_box_input_is(0, true, Some("/mo")),
            quick,
        ))
        .with_step(wait_for(
            "Claude's menu is back",
            cli_agent_screen_contains(0, CLAUDE_MODEL_ENTRY),
            quick,
        ))
        .with_step(shot("narrow-5-box-reopened"))
        .with_step(keys("delete it", &["escape"]))
        .with_step(wait_for(
            "box empty",
            one_box_input_is(0, true, Some("")),
            quick,
        ))
        .with_step(
            type_text("exit Claude", "/exit")
                .with_keystrokes(&["enter"])
                .set_post_step_pause(Duration::from_secs(3)),
        )
}

/// Native screenshots of one-box agent input, Clinch's box drawn in place of Claude Code's own
/// composer: idle, typing, stepping aside for a permission dialog and coming back, inline, and in
/// the Coordinator.
///
/// ```sh
/// WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS=1 \
///   cargo run -p integration --bin integration -- test_one_box_screens
/// ```
/// Claude Code's predicted next prompt, which it draws dim in its empty composer, shows as ghost
/// text in Clinch's box. Tab or → accepts it, as in Claude's own composer, and emptying the box
/// brings it back. Uses the stand-in, so no agent or account is involved.
///
/// ```sh
/// WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS=1 \
///   cargo run -p integration --bin integration -- test_one_box_agent_suggestion
/// ```
pub fn test_one_box_agent_suggestion() -> Builder {
    const SUGGESTION: &str = "run the failing tests again";
    let quick = Duration::from_secs(10);
    Builder::new()
        .with_real_display()
        .with_user_defaults(HashMap::from([
            // The box is off by default for now.
            (
                OneBoxAgentInput::storage_key().to_string(),
                true.to_string(),
            ),
            (
                AutoCreateWorktreesForNewTabs::storage_key().to_string(),
                false.to_string(),
            ),
        ]))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(install_stand_in())
        .with_step(
            TestStep::new("start the stand-in with a suggestion")
                .with_input_string(
                    &format!("STANDIN_SUGGESTION='{SUGGESTION}' claude"),
                    Some(&["enter"]),
                )
                .set_post_step_pause(Duration::from_secs(3)),
        )
        .with_step(wait_for(
            "the box is open and empty",
            one_box_input_is(0, true, Some("")),
            quick,
        ))
        .with_step(wait_for(
            "the suggestion shows as ghost text",
            one_box_ghost_text_is(0, Some(SUGGESTION)),
            quick,
        ))
        .with_step(shot("suggestion-1-ghost-text"))
        .with_step(keys("accept it with Tab", &["tab"]))
        .with_step(wait_for(
            "Tab filled the box",
            one_box_input_is(0, true, Some(SUGGESTION)),
            quick,
        ))
        .with_step(shot("suggestion-2-accepted"))
        .with_step(keys("clear the box", &["cmd-a", "backspace"]))
        .with_step(wait_for(
            "the box is empty",
            one_box_input_is(0, true, Some("")),
            quick,
        ))
        .with_step(wait_for(
            "the suggestion comes back",
            one_box_ghost_text_is(0, Some(SUGGESTION)),
            quick,
        ))
        .with_step(keys("accept it with the right arrow", &["right"]))
        .with_step(wait_for(
            "the right arrow filled the box",
            one_box_input_is(0, true, Some(SUGGESTION)),
            quick,
        ))
        .with_step(shot("suggestion-3-accepted-with-right-arrow"))
}

pub fn test_one_box_screens() -> Builder {
    Builder::new()
        .with_real_display()
        .with_user_defaults(HashMap::from([
            // The box is off by default for now.
            (
                OneBoxAgentInput::storage_key().to_string(),
                true.to_string(),
            ),
            (
                AutoCreateWorktreesForNewTabs::storage_key().to_string(),
                false.to_string(),
            ),
        ]))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(install_stand_in())
        .with_step(
            TestStep::new("start the fullscreen stand-in")
                .with_input_string("claude", Some(&["enter"]))
                .set_post_step_pause(Duration::from_secs(5)),
        )
        .with_step(shot("one-box-fullscreen"))
        .with_step(wait_for(
            "Claude's fixed example is not offered as a suggestion",
            one_box_ghost_text_is(0, None),
            Duration::from_secs(10),
        ))
        .with_step(
            TestStep::new("type into Clinch's box")
                .with_input_string("write the tests for the parser", None)
                .set_post_step_pause(Duration::from_millis(600)),
        )
        .with_step(shot("one-box-typing"))
        // The stand-in shows its dialog 12 seconds after starting.
        .with_step(wait("wait for the dialog", Duration::from_secs(8)))
        .with_step(shot("one-box-dialog"))
        .with_step(wait(
            "wait for the composer to return",
            Duration::from_secs(7),
        ))
        .with_step(shot("one-box-back"))
        .with_step(
            TestStep::new("new tab")
                .with_keystrokes(&["cmd-t"])
                .set_post_step_pause(Duration::from_secs(2)),
        )
        .with_step(
            TestStep::new("start the inline stand-in")
                .with_input_string("STANDIN_INLINE=1 claude", Some(&["enter"]))
                .set_post_step_pause(Duration::from_secs(5)),
        )
        .with_step(shot("one-box-inline"))
        .with_step(open_coordinator().set_post_step_pause(Duration::from_secs(6)))
        .with_step(shot("one-box-coordinator"))
}
