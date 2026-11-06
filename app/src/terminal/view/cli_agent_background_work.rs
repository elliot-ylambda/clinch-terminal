//! TerminalView glue that keeps a Claude Code or Codex pane's background-work counts live.
//!
//! The agent redraws its status line whenever a background shell, monitor or terminal starts or
//! ends, and every redraw reaches this pane as a PTY wakeup, including in tabs that are not on
//! screen. A wakeup schedules one scan [`SCAN_DELAY`] later; wakeups in between join it, so a busy
//! TUI is read at most a few times a second and the last redraw is never missed. The parsing
//! lives in `cli_agent_sessions::background_work`.
use std::time::Duration;

use warpui::r#async::Timer;
use warpui::{SingletonEntity, ViewContext};

use super::use_agent_footer::cli_agent_native_grid;
use super::TerminalView;
use crate::terminal::cli_agent_sessions::background_work::{parse_status_rows, STATUS_SCAN_ROWS};
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
use crate::terminal::model::cell::Flags;
use crate::terminal::model::grid::grid_handler::GridHandler;
use crate::terminal::model::grid::Dimensions as _;
use crate::terminal::CLIAgent;

const SCAN_DELAY: Duration = Duration::from_millis(250);

impl TerminalView {
    pub(super) fn schedule_cli_agent_background_scan(&mut self, ctx: &mut ViewContext<Self>) {
        if self.cli_agent_background_scan_pending || self.native_status_line_agent(ctx).is_none() {
            return;
        }
        self.cli_agent_background_scan_pending = true;
        ctx.spawn(Timer::after(SCAN_DELAY), |me, _, ctx| {
            me.cli_agent_background_scan_pending = false;
            me.scan_cli_agent_background_work(ctx);
        });
    }

    fn scan_cli_agent_background_work(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(agent) = self.native_status_line_agent(ctx) else {
            return;
        };
        let rows = {
            let model = self.model.lock();
            bottom_visible_rows(cli_agent_native_grid(&model), STATUS_SCAN_ROWS)
        };
        let work = parse_status_rows(agent, &rows);
        let view_id = self.view_id;
        CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
            sessions.update_screen_background_work(view_id, work, ctx);
        });
    }

    /// The pane's agent when it draws a status line Clinch can read.
    fn native_status_line_agent(&self, ctx: &ViewContext<Self>) -> Option<CLIAgent> {
        CLIAgentSessionsModel::as_ref(ctx)
            .session(self.view_id)
            .map(|session| session.agent)
            .filter(|agent| matches!(agent, CLIAgent::Claude | CLIAgent::Codex))
    }
}

/// The last `max_rows` visible rows as plain text, top to bottom, without trailing blanks.
pub(super) fn bottom_visible_rows(grid: &GridHandler, max_rows: usize) -> Vec<String> {
    let first_visible_row = grid.history_size();
    let past_last_visible_row = first_visible_row
        .saturating_add(grid.visible_rows())
        .min(grid.total_rows());
    let first_row = past_last_visible_row
        .saturating_sub(max_rows)
        .max(first_visible_row);
    (first_row..past_last_visible_row)
        .map(|row_idx| {
            let Some(row) = grid.row(row_idx) else {
                return String::new();
            };
            let text: String = (0..row.len())
                .map(|col| &row[col])
                .filter(|cell| !cell.flags.contains(Flags::WIDE_CHAR_SPACER))
                .map(|cell| if cell.c == '\0' { ' ' } else { cell.c })
                .collect();
            text.trim_end().to_owned()
        })
        .collect()
}

#[cfg(test)]
#[path = "cli_agent_background_work_tests.rs"]
mod tests;
