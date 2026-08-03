//! Locates a CLI agent's past prompt in this pane's rendered scrollback.
//!
//! The message-history dropdown is populated from the agent's transcript file on disk
//! (see [`crate::agent_resume::read_prompt_history`]), which has no positional link to the
//! terminal grid. To scroll to a prompt we have to find where it was actually painted.
//!
//! Searching the grid is the only approach that works in every case. Recording a grid position
//! when the prompt is submitted looks tempting, but it cannot survive session restore: restored
//! blocks are rebuilt by replaying `SerializedBlock::stylized_output` into a fresh grid, so rows
//! renumber, and no submit event ever fires for replayed text. Conversely, assuming restored text
//! is present is wrong too — only *completed* blocks are persisted, truncated to
//! `MAX_SERIALIZED_STYLIZED_OUTPUT_LINES`. Searching what is currently on screen is correct under
//! both.

use itertools::Itertools;
use regex::escape;

use crate::terminal::model::blocks::BlockList;
use crate::terminal::model::find::{FindConfig, Match, RegexDFAs};
use crate::terminal::model::terminal_model::BlockIndex;

/// Tokens taken from the prompt's first line to build the search pattern.
///
/// Enough to be distinctive without reaching so far into the prompt that a mid-sentence rewrap or
/// an agent-inserted ellipsis breaks the match.
const MAX_PATTERN_TOKENS: usize = 8;

/// Cap on the pattern's source text, in characters.
const MAX_PATTERN_CHARS: usize = 64;

/// Shortest prompt prefix worth searching for.
///
/// Below this a prompt ("ok", "yes", "continue") matches so much of a typical agent transcript
/// that the top hit would be noise, so we report it as unlocatable instead.
const MIN_PATTERN_CHARS: usize = 8;

/// Builds the regex used to find `prompt_text` in the grid, or `None` if the prompt is too short
/// to identify a unique location.
pub fn agent_prompt_search_pattern(prompt_text: &str) -> Option<String> {
    let first_line = prompt_text.lines().find(|line| !line.trim().is_empty())?;

    let mut tokens = Vec::new();
    let mut chars_used = 0;
    for token in first_line.split_whitespace() {
        if tokens.len() >= MAX_PATTERN_TOKENS || chars_used >= MAX_PATTERN_CHARS {
            break;
        }
        chars_used += token.chars().count();
        tokens.push(token);
    }

    let significant_chars: usize = tokens.iter().map(|token| token.chars().count()).sum();
    if tokens.is_empty() || significant_chars < MIN_PATTERN_CHARS {
        return None;
    }

    // `\s*`, not `\s+`. The grid search walks a wrap-transparent grapheme cursor, so a soft wrap
    // contributes *no* separator character between the last cell of one row and the first of the
    // next — `\s+` would fail on any prompt long enough to wrap, which is most of them. `\s*` also
    // absorbs the indentation agent TUIs add to their own hard-wrapped continuation lines, and the
    // padding at the end of a short row. It can over-match ("the\s*quick" hitting "thequick"),
    // which is harmless for locating a known prompt.
    Some(tokens.iter().map(|token| escape(token)).join("\\s*"))
}

/// Where a prompt was found in the blocklist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptLocation {
    pub block_index: BlockIndex,
    /// Range within the block's *output* grid.
    pub range: Match,
}

/// Finds the most recent rendering of `prompt_text` in `block_list`, or `None` if it isn't on
/// screen anywhere.
///
/// Searches blocks newest-first and, within a block, takes the last match. After a restart the
/// same prompt can be painted twice — once in the restored static block and again if the resumed
/// agent replays its history — and the newest copy is the live conversation, which is the one the
/// user means.
///
/// Only output grids are searched. A block's prompt-and-command grid holds the shell command that
/// launched the agent (`claude`, `cx`), never conversation text.
pub fn locate_agent_prompt(block_list: &BlockList, prompt_text: &str) -> Option<PromptLocation> {
    let pattern = agent_prompt_search_pattern(prompt_text)?;
    let dfas = RegexDFAs::new_with_config(
        &pattern,
        FindConfig {
            is_regex_enabled: true,
            // Prompts render verbatim, so matching case costs nothing and rules out false hits.
            is_case_sensitive: true,
        },
    )
    .ok()?;

    let agent_view_state = block_list.agent_view_state();
    (0..block_list.blocks().len()).rev().find_map(|index| {
        let block_index = BlockIndex::from(index);
        let block = block_list
            .block_at(block_index)
            .filter(|block| !block.is_empty(agent_view_state))?;
        let range = block.find_output_grid_matches(&dfas).pop()?;
        Some(PromptLocation { block_index, range })
    })
}

#[cfg(test)]
#[path = "cli_agent_prompt_locator_tests.rs"]
mod tests;

