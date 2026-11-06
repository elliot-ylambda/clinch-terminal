use warpui::{App, SingletonEntity};

use super::*;
use crate::agent_resume::AgentPromptHistory;
use crate::terminal::cli_agent_sessions::background_work::BackgroundWork;
use crate::terminal::cli_agent_sessions::{
    CLIAgentInputState, CLIAgentSession, CLIAgentSessionContext, CLIAgentSessionStatus,
    PromptHistoryLoadState,
};
use crate::terminal::{SizeInfo, SizeUpdate, SizeUpdateReason};
use crate::test_util::add_window_with_terminal;
use crate::test_util::terminal::initialize_app_for_terminal_view;

fn session(agent: CLIAgent) -> CLIAgentSession {
    CLIAgentSession {
        agent,
        status: CLIAgentSessionStatus::Success,
        session_context: CLIAgentSessionContext::default(),
        input_state: CLIAgentInputState::Closed,
        should_auto_toggle_input: false,
        listener: None,
        remote_host: None,
        plugin_version: None,
        draft_text: None,
        custom_command_prefix: None,
        received_rich_notification: true,
        has_observed_turn_activity: true,
        turn_interrupted_by_user: false,
        prompt_history: AgentPromptHistory::default(),
        prompt_history_load_state: PromptHistoryLoadState::Ready,
        prompt_history_generation: 1,
    }
}

/// Clears a 24x120 alt screen and draws Claude Code's composer with `mode_row` under it.
fn claude_frame(mode_row: &str) -> String {
    let rule = "─".repeat(120);
    format!(
        "\x1b[0m\x1b[2J\x1b[10;1H✻ Baked for 3m · 1 shell still running\x1b[18;1H{rule}\x1b[19;1H\x1b[1m❯\x1b[0m \x1b[20;1H{rule}\x1b[21;3H\x1b[2mv2.1.319 | Opus 5.5 | ~/clinch\x1b[22;3H{mode_row}\x1b[0m\x1b[19;3H"
    )
}

/// Gives the pane a 24x120 alt screen, wide enough that no status line wraps.
fn enter_wide_alt_screen(view: &TerminalView) {
    let mut model = view.model.lock();
    let last_size = *model.block_list().size();
    let new_size = SizeInfo::new_without_font_metrics(24, 120);
    model.resize(SizeUpdate {
        update_reason: SizeUpdateReason::Refresh,
        last_size,
        new_size,
        new_gap_height: None,
        natural_rows: new_size.rows(),
        natural_cols: new_size.columns(),
    });
    model.enter_alt_screen(true);
}

/// Draws `frame` into the pane, scans it, and returns the session's counts.
fn draw_and_scan(
    view: &mut TerminalView,
    frame: &str,
    ctx: &mut ViewContext<TerminalView>,
) -> Option<BackgroundWork> {
    view.model.lock().process_bytes(frame);
    view.scan_cli_agent_background_work(ctx);
    CLIAgentSessionsModel::as_ref(ctx)
        .session(view.view_id)
        .map(CLIAgentSession::background_work)
}

#[test]
fn scans_live_counts_from_the_rendered_status_line() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            enter_wide_alt_screen(view);
            let view_id = view.view_id;
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.set_session(view_id, session(CLIAgent::Claude), ctx);
            });

            let shells_and_monitor = BackgroundWork {
                shells: 2,
                monitors: 1,
                ..Default::default()
            };
            assert_eq!(
                draw_and_scan(
                    view,
                    &claude_frame("⏵⏵ bypass permissions on · 2 shells, 1 monitor · ← 4 agents"),
                    ctx
                ),
                Some(shells_and_monitor)
            );

            // A permission dialog replaces the composer: the last counts stay.
            assert_eq!(
                draw_and_scan(
                    view,
                    "\x1b[0m\x1b[2J\x1b[16;1H────────────────\x1b[18;2HDo you want to proceed?\x1b[19;2H❯ 1. Yes",
                    ctx
                ),
                Some(shells_and_monitor)
            );

            // The shells and the monitor finish: the pills disappear and so do the counts.
            assert_eq!(
                draw_and_scan(
                    view,
                    &claude_frame("⏵⏵ bypass permissions on · ← 4 agents"),
                    ctx
                ),
                Some(BackgroundWork::default())
            );

            // Codex draws its summary a few rows above its composer.
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.set_session(view_id, session(CLIAgent::Codex), ctx);
            });
            assert_eq!(
                draw_and_scan(
                    view,
                    "\x1b[0m\x1b[2J\x1b[17;3H3 background terminals running · /ps to view · /stop to close\x1b[20;1H\x1b[1m›\x1b[22m Ask Codex to do anything\x1b[22;3Hgpt-6 xhigh · Ready\x1b[23;3H← for agents · ? for shortcuts\x1b[20;3H",
                    ctx
                ),
                Some(BackgroundWork {
                    shells: 3,
                    ..Default::default()
                })
            );
        });
    })
}

#[test]
fn bottom_visible_rows_keeps_wide_characters_once() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, _| {
            enter_wide_alt_screen(view);
            let mut model = view.model.lock();
            model.process_bytes("\x1b[0m\x1b[2J\x1b[1;1H日本 · 1 shell");
            let grid = model.alt_screen().grid_handler();
            let rows = bottom_visible_rows(grid, grid.visible_rows());
            assert_eq!(rows.first().map(String::as_str), Some("日本 · 1 shell"));
            assert!(rows[1..].iter().all(String::is_empty));
        });
    })
}
