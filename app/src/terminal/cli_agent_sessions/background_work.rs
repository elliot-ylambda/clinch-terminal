//! Background work a Claude Code or Codex session reports on its own status line.
//!
//! Hooks report when a background shell or monitor starts, but nothing reports when one exits or
//! is killed, so the status line the agent draws under its composer is the only live count:
//!
//! | Agent       | Where                                   | Text                                            |
//! |-------------|-----------------------------------------|-------------------------------------------------|
//! | Claude Code | rows below the composer's lower rule    | `⏵⏵ bypass permissions on · 1 shell, 1 monitor` |
//! | Codex       | a few rows above the `›` composer       | `1 background terminal running · /ps to view`   |
//!
//! Claude itemizes only when every background task is a shell or monitor. A mixed set reads
//! `3 background tasks`, and background subagents alone read `2 local agents`. Its `← 4 agents`
//! segment counts every session in Claude's machine-wide agent view, not this session, so the
//! leading arrow keeps it from parsing as a count.
//!
//! Subagents also arrive through `SubagentStart`/`SubagentStop` hooks; see
//! [`super::CLIAgentSession::background_work`].
use crate::terminal::CLIAgent;

/// How many rows at the bottom of the visible screen hold the composer and its status line.
pub(crate) const STATUS_SCAN_ROWS: usize = 16;

/// Separator between status line segments, in both agents.
const SEGMENT_SEPARATOR: &str = " · ";
/// The prompt glyphs Claude Code draws at the start of its composer (normal and bash mode).
const CLAUDE_PROMPT_GLYPHS: [char; 2] = ['❯', '!'];
/// Codex's composer prompt; `»` is its older glyph.
const CODEX_PROMPT_GLYPHS: [char; 2] = ['›', '»'];
/// Codex's composer sits right above its footer, never higher than this many rows from the bottom.
const CODEX_MAX_ROWS_BELOW_PROMPT: usize = 6;
/// Codex's background-terminal summary sits within this many rows above the composer.
const CODEX_MAX_STATUS_ROWS_ABOVE_PROMPT: usize = 4;

/// Background work counted for one session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BackgroundWork {
    /// Background shells (Claude) or background terminals (Codex).
    pub shells: u32,
    /// Claude Code monitors.
    pub monitors: u32,
    /// Background tasks Claude reports only as a total, such as a mix of shells and subagents.
    pub tasks: u32,
    /// Running subagents.
    pub subagents: u32,
}

impl BackgroundWork {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    fn add_segment(&mut self, segment: &str) {
        // Claude joins shells and monitors in one segment: `1 shell, 2 monitors`.
        for part in segment.split(", ") {
            let Some((count, noun)) = count_and_noun(part) else {
                continue;
            };
            let field = match noun {
                "shell" | "shells" => &mut self.shells,
                "background terminal running" | "background terminals running" => &mut self.shells,
                // Includes Claude's `Artifact comment monitors`.
                _ if noun.ends_with("monitor") || noun.ends_with("monitors") => &mut self.monitors,
                "background task"
                | "background tasks"
                | "MCP task"
                | "MCP tasks"
                | "background dynamic workflow"
                | "background dynamic workflows" => &mut self.tasks,
                "local agent" | "local agents" => &mut self.subagents,
                _ => continue,
            };
            *field = field.saturating_add(count);
        }
    }
}

/// Reads background work from the bottom rows of an agent's screen, top to bottom.
///
/// Returns `None` when the composer is not on screen, such as while a dialog replaces it, so the
/// caller keeps the last known counts instead of dropping them to zero.
pub(crate) fn parse_status_rows<S: AsRef<str>>(
    agent: CLIAgent,
    rows: &[S],
) -> Option<BackgroundWork> {
    match agent {
        CLIAgent::Claude => claude_status_rows(rows),
        CLIAgent::Codex => codex_status_rows(rows),
        _ => None,
    }
    .map(|status_rows| {
        let mut work = BackgroundWork::default();
        for row in status_rows {
            for segment in row.as_ref().split(SEGMENT_SEPARATOR) {
                work.add_segment(segment);
            }
        }
        work
    })
}

/// The rows under Claude Code's composer: the optional custom status line and the mode/task row.
fn claude_status_rows<S: AsRef<str>>(rows: &[S]) -> Option<&[S]> {
    let lower_rule = rows.iter().rposition(|row| is_full_rule(row.as_ref()))?;
    // The composer's upper rule can carry a label, and a draft can span several rows; its first
    // row starts with the prompt glyph. A dialog's border has no prompt row under it.
    let upper_rule = rows[..lower_rule]
        .iter()
        .rposition(|row| row.as_ref().trim_start().starts_with('─'))?;
    let prompt_row = rows[upper_rule + 1..lower_rule].first()?.as_ref();
    if !prompt_row.trim_start().starts_with(CLAUDE_PROMPT_GLYPHS) {
        return None;
    }
    Some(&rows[lower_rule + 1..])
}

/// The rows just above Codex's composer, where it summarizes running background terminals.
fn codex_status_rows<S: AsRef<str>>(rows: &[S]) -> Option<&[S]> {
    let prompt_row = rows
        .iter()
        .rposition(|row| row.as_ref().starts_with(CODEX_PROMPT_GLYPHS))?;
    if rows.len() - prompt_row > CODEX_MAX_ROWS_BELOW_PROMPT {
        return None;
    }
    Some(&rows[prompt_row.saturating_sub(CODEX_MAX_STATUS_ROWS_ABOVE_PROMPT)..prompt_row])
}

fn is_full_rule(row: &str) -> bool {
    let row = row.trim();
    row.chars().count() >= 8 && row.chars().all(|c| c == '─')
}

/// Splits `3 shells` into `(3, "shells")`. Leading glyphs such as `• ` or `← ` make it not a count.
fn count_and_noun(text: &str) -> Option<(u32, &str)> {
    let (count, noun) = text.trim().split_once(' ')?;
    Some((count.parse().ok()?, noun.trim()))
}

#[cfg(test)]
#[path = "background_work_tests.rs"]
mod tests;
