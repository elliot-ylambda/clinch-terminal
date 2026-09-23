//! Read-only search across bounded terminal text in selected tabs.
#[cfg(test)]
#[path = "tab_grep_tests.rs"]
mod tests;

use std::collections::BTreeSet;

use ::local_control::protocol::{
    TabGrepMatch, TabGrepParams, TabGrepResult, TargetSelector, WindowTarget,
};
use ::local_control::{Action, ActionKind, ControlError, ErrorCode};
use regex::{Regex, RegexBuilder};
use warpui::ModelContext;

use crate::local_control::handlers::metadata::select_pane_entries;
use crate::local_control::resolver::reject_target_families;
use crate::local_control::LocalControlBridge;

const MAX_PATTERN_BYTES: usize = 4 * 1024;
const MAX_MATCHES: u32 = 1_000;
const MAX_MATCH_TEXT_BYTES: usize = 4 * 1024;
const MAX_SEARCHABLE_BYTES_PER_PANE: usize = 128 * 1024;
const MAX_TOTAL_SEARCHABLE_BYTES: usize = 8 * 1024 * 1024;

pub(crate) fn handle(
    action: &Action,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    reject_target_families(
        ActionKind::TabGrep,
        target.session.is_some(),
        "session selectors",
    )?;
    let params = action.params_as::<TabGrepParams>()?;
    let matcher = compile_matcher(&params)?;

    // A selector-free grep is deliberately scoped to the active project window. Exact tab or
    // pane IDs can still address an inactive window without first activating it.
    let mut target = target.clone();
    if target.window.is_none() && target.tab.is_none() && target.pane.is_none() {
        target.window = Some(WindowTarget::Active);
    }
    let panes = select_pane_entries(&target, ActionKind::TabGrep, ctx)?;
    let searched_tabs = panes
        .iter()
        .map(|entry| (entry.window_id.to_string(), entry.tab_id.clone()))
        .collect::<BTreeSet<_>>()
        .len() as u32;

    let mut searched_panes = 0u32;
    let mut skipped_non_terminal_panes = 0u32;
    let mut content_truncated = false;
    let mut matches_truncated = false;
    let mut matches = Vec::new();
    let mut searched_bytes = 0usize;

    'panes: for entry in panes {
        let (tab_title, terminal) = entry.pane_group.read(ctx, |pane_group, ctx| {
            (
                pane_group.display_title(ctx),
                pane_group.terminal_view_from_pane_id(entry.pane_id, ctx),
            )
        });
        let Some(terminal) = terminal else {
            skipped_non_terminal_panes += 1;
            continue;
        };
        let remaining_bytes = MAX_TOTAL_SEARCHABLE_BYTES.saturating_sub(searched_bytes);
        if remaining_bytes == 0 {
            content_truncated = true;
            break;
        }
        searched_panes += 1;
        let max_bytes = remaining_bytes.min(MAX_SEARCHABLE_BYTES_PER_PANE);
        let (text, pane_content_truncated) = terminal.read(ctx, |terminal, _| {
            terminal.local_control_searchable_text(max_bytes)
        });
        searched_bytes += text.len();
        content_truncated |= pane_content_truncated;

        for (line_index, line) in text.lines().enumerate() {
            let Some(found) = matcher.find(line) else {
                continue;
            };
            if matches.len() >= params.max_matches as usize {
                matches_truncated = true;
                break 'panes;
            }
            let (text, text_truncated) = bounded_match_text(line, found.start(), found.end());
            matches.push(TabGrepMatch {
                window_id: entry.window_id.to_string(),
                window_index: entry.window_index as u32,
                tab_id: entry.tab_id.clone(),
                tab_index: entry.tab_index as u32,
                tab_title: tab_title.clone(),
                pane_id: entry.pane_id.to_string(),
                pane_index: entry.index as u32,
                line_number: u32::try_from(line_index + 1).unwrap_or(u32::MAX),
                text_truncated,
                text,
            });
        }
    }

    let result = TabGrepResult {
        action: ActionKind::TabGrep,
        pattern: params.pattern,
        searched_tabs,
        searched_panes,
        skipped_non_terminal_panes,
        match_count: matches.len() as u32,
        content_truncated,
        matches_truncated,
        matches,
    };
    serde_json::to_value(result).map_err(|error| {
        ControlError::with_details(
            ErrorCode::Internal,
            "failed to serialize tab.grep response",
            error.to_string(),
        )
    })
}

fn bounded_match_text(line: &str, match_start: usize, match_end: usize) -> (String, bool) {
    if line.len() <= MAX_MATCH_TEXT_BYTES {
        return (line.to_owned(), false);
    }

    let match_len = match_end.saturating_sub(match_start);
    let surrounding_bytes =
        MAX_MATCH_TEXT_BYTES.saturating_sub(match_len.min(MAX_MATCH_TEXT_BYTES));
    let mut start = match_start.saturating_sub(surrounding_bytes / 2);
    start = start.min(line.len() - MAX_MATCH_TEXT_BYTES);
    while !line.is_char_boundary(start) {
        start += 1;
    }
    let mut end = (start + MAX_MATCH_TEXT_BYTES).min(line.len());
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    if match_end <= line.len()
        && match_end > end
        && match_end.saturating_sub(start) <= MAX_MATCH_TEXT_BYTES
    {
        end = match_end;
    }

    let mut text = String::with_capacity(end - start + 6);
    if start > 0 {
        text.push('…');
    }
    text.push_str(&line[start..end]);
    if end < line.len() {
        text.push('…');
    }
    (text, true)
}

fn compile_matcher(params: &TabGrepParams) -> Result<Regex, ControlError> {
    if params.pattern.is_empty() {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            "tab.grep requires a non-empty pattern",
        ));
    }
    if params.pattern.len() > MAX_PATTERN_BYTES {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            format!("tab.grep patterns cannot exceed {MAX_PATTERN_BYTES} bytes"),
        ));
    }
    if !(1..=MAX_MATCHES).contains(&params.max_matches) {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            format!("tab.grep max_matches must be between 1 and {MAX_MATCHES}"),
        ));
    }

    let pattern = if params.fixed_strings {
        regex::escape(&params.pattern)
    } else {
        params.pattern.clone()
    };
    RegexBuilder::new(&pattern)
        .case_insensitive(params.ignore_case)
        .build()
        .map_err(|error| {
            ControlError::with_details(
                ErrorCode::InvalidParams,
                "tab.grep received an invalid regular expression",
                error.to_string(),
            )
        })
}
