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
/// Even anchored on word boundaries, a prompt this short ("hi", "ok", "go") occurs too often in a
/// transcript's ordinary prose for the top hit to mean anything. Reporting it as unlocatable beats
/// scrolling somewhere wrong, which the user has no way to recognize as wrong.
const MIN_PATTERN_CHARS: usize = 4;

/// Why a prompt could not be turned into a search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsearchablePrompt {
    /// No non-whitespace content at all.
    Empty,
    /// Real content, but too short to identify a unique location.
    TooShort,
}

/// Builds the regex used to find `prompt_text` in the grid.
pub fn agent_prompt_search_pattern(prompt_text: &str) -> Result<String, UnsearchablePrompt> {
    let first_line = prompt_text
        .lines()
        .find(|line| !line.trim().is_empty())
        .ok_or(UnsearchablePrompt::Empty)?;

    let mut tokens = Vec::new();
    let mut chars_used = 0;
    for token in first_line.split_whitespace() {
        if tokens.len() >= MAX_PATTERN_TOKENS || chars_used >= MAX_PATTERN_CHARS {
            break;
        }
        chars_used += token.chars().count();
        tokens.push(token);
    }

    let Some((first, last)) = tokens.first().zip(tokens.last()) else {
        return Err(UnsearchablePrompt::Empty);
    };
    let significant_chars: usize = tokens.iter().map(|token| token.chars().count()).sum();
    if significant_chars < MIN_PATTERN_CHARS {
        return Err(UnsearchablePrompt::TooShort);
    }

    // `\s*`, not `\s+`. The grid search walks a wrap-transparent grapheme cursor, so a soft wrap
    // contributes *no* separator character between the last cell of one row and the first of the
    // next — `\s+` would fail on any prompt long enough to wrap, which is most of them. `\s*` also
    // absorbs the indentation agent TUIs add to their own hard-wrapped continuation lines, and the
    // padding at the end of a short row.
    let body = tokens.iter().map(|token| escape(token)).join("\\s*");

    // Word boundaries stop a short prompt from matching inside a longer word ("hi" in "this"). Only
    // meaningful next to a word character: `\b` after the `.` of "Longer story." would demand a
    // following word character and never match.
    let starts_on_word = first.chars().next().is_some_and(char::is_alphanumeric);
    let ends_on_word = last.chars().last().is_some_and(char::is_alphanumeric);
    Ok(format!(
        "{}{body}{}",
        if starts_on_word { "\\b" } else { "" },
        if ends_on_word { "\\b" } else { "" },
    ))
}

/// Where a prompt was found in the blocklist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptLocation {
    pub block_index: BlockIndex,
    /// Range within the block's *output* grid.
    pub range: Match,
}

/// Finds the most recent rendering of `prompt_text` in `block_list`.
///
/// Searches blocks newest-first and, within a block, takes the bottom-most match. After a restart
/// the same prompt can be painted twice — once in the restored static block and again if the
/// resumed agent replays its history — and the newest copy is the live conversation, which is the
/// one the user means.
///
/// Only output grids are searched. A block's prompt-and-command grid holds the shell command that
/// launched the agent (`claude`, `cx`), never conversation text.
pub fn locate_agent_prompt(
    block_list: &BlockList,
    prompt_text: &str,
) -> Result<PromptLocation, PromptLookupFailure> {
    let pattern = agent_prompt_search_pattern(prompt_text)
        .map_err(PromptLookupFailure::Unsearchable)?;
    let dfas = RegexDFAs::new_with_config(
        &pattern,
        FindConfig {
            is_regex_enabled: true,
            // Prompts render verbatim, so matching case costs nothing and rules out false hits.
            is_case_sensitive: true,
        },
    )
    .map_err(|_| PromptLookupFailure::NotPainted)?;

    let agent_view_state = block_list.agent_view_state();
    (0..block_list.blocks().len())
        .rev()
        .find_map(|index| {
            let block_index = BlockIndex::from(index);
            let block = block_list
                .block_at(block_index)
                .filter(|block| !block.is_empty(agent_view_state))?;
            // `find` walks the grid from the end backwards, so matches arrive bottom-most first.
            // Taking the first one lands on the newest painting of this prompt within the block.
            let range = block.find_output_grid_matches(&dfas).into_iter().next()?;
            Some(PromptLocation { block_index, range })
        })
        .ok_or(PromptLookupFailure::NotPainted)
}

/// Why a jump could not be performed. Distinct cases because they need to be reported differently:
/// telling a user that a message visibly on screen "isn't in the scrollback" is simply false.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptLookupFailure {
    /// The prompt could not be turned into a usable search.
    Unsearchable(UnsearchablePrompt),
    /// A real search ran and found nothing painted in this pane.
    NotPainted,
}

#[cfg(test)]
#[path = "cli_agent_prompt_locator_tests.rs"]
mod tests;

