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

/// Shortest prompt that can be searched for on its own.
///
/// Below this ("hi", "ok", "go") the text occurs too often in a transcript's ordinary prose for a
/// standalone top hit to mean anything. Such a prompt is still locatable when the surrounding
/// history pins down *where* to look — see [`locate_agent_prompt_in_history`] — so this bound only
/// governs an unconstrained search.
const MIN_STANDALONE_PATTERN_CHARS: usize = 4;

/// Why a prompt could not be turned into a search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsearchablePrompt {
    /// No non-whitespace content at all.
    Empty,
    /// Real content, but too short to identify a location without a surrounding anchor.
    TooShortToSearchAlone,
}

/// Whether `prompt_text` is distinctive enough to locate without help from its neighbours.
fn is_distinctive(prompt_text: &str) -> bool {
    pattern_tokens(prompt_text)
        .map(|tokens| {
            tokens.iter().map(|token| token.chars().count()).sum::<usize>()
                >= MIN_STANDALONE_PATTERN_CHARS
        })
        .unwrap_or(false)
}

/// The leading tokens of the prompt's first non-empty line, bounded in count and length.
fn pattern_tokens(prompt_text: &str) -> Option<Vec<&str>> {
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
    (!tokens.is_empty()).then_some(tokens)
}

/// Builds the regex used to find `prompt_text` in the grid, regardless of how short it is.
///
/// Callers that search without a positional anchor must reject non-[`is_distinctive`] prompts
/// themselves; this builds a pattern for anything with content.
fn unbounded_search_pattern(prompt_text: &str) -> Result<String, UnsearchablePrompt> {
    let tokens = pattern_tokens(prompt_text).ok_or(UnsearchablePrompt::Empty)?;
    let (first, last) = (tokens[0], tokens[tokens.len() - 1]);

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

/// Builds the regex used to find `prompt_text` on its own, with no positional anchor.
pub fn agent_prompt_search_pattern(prompt_text: &str) -> Result<String, UnsearchablePrompt> {
    let pattern = unbounded_search_pattern(prompt_text)?;
    if !is_distinctive(prompt_text) {
        return Err(UnsearchablePrompt::TooShortToSearchAlone);
    }
    Ok(pattern)
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
    locate_agent_prompt_in_history(block_list, &[prompt_text], 0)
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

/// A match's position in the blocklist, ordered the way the conversation reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct DocumentPosition {
    block: usize,
    row: usize,
    col: usize,
}

impl DocumentPosition {
    fn of(block_index: BlockIndex, range: &Match) -> Self {
        Self {
            block: block_index.0,
            row: range.start().row,
            col: range.start().col,
        }
    }
}

/// Every match for `pattern`, ordered as the conversation reads: oldest first.
fn matches_in_reading_order(
    block_list: &BlockList,
    pattern: &str,
) -> Option<Vec<(BlockIndex, Match)>> {
    let dfas = RegexDFAs::new_with_config(
        pattern,
        FindConfig {
            is_regex_enabled: true,
            // Prompts render verbatim, so matching case costs nothing and rules out false hits.
            is_case_sensitive: true,
        },
    )
    .ok()?;

    let agent_view_state = block_list.agent_view_state();
    let mut found = Vec::new();
    for index in 0..block_list.blocks().len() {
        let block_index = BlockIndex::from(index);
        let Some(block) = block_list
            .block_at(block_index)
            .filter(|block| !block.is_empty(agent_view_state))
        else {
            continue;
        };
        let mut ranges = block.find_output_grid_matches(&dfas);
        // `find` walks the grid from the end backwards, so it yields bottom-most first.
        ranges.reverse();
        found.extend(ranges.into_iter().map(|range| (block_index, range)));
    }
    Some(found)
}

/// Locates prompt `index` of `prompt_texts`, using its neighbours to pin down where to look.
///
/// A prompt's own text is not always distinctive — "hi" or "ok" occurs throughout a transcript's
/// ordinary prose, and searching for it alone would land somewhere arbitrary. But its *position* in
/// the conversation is known exactly: it falls after the previous prompt and before the next one.
/// Resolving the nearest distinctive neighbours first turns an ambiguous search into a bounded one,
/// which is what makes short messages jumpable at all.
///
/// Falls back to an unanchored search when no neighbour resolves, so a lone distinctive prompt
/// still works.
pub fn locate_agent_prompt_in_history(
    block_list: &BlockList,
    prompt_texts: &[&str],
    index: usize,
) -> Result<PromptLocation, PromptLookupFailure> {
    let target = prompt_texts.get(index).copied().unwrap_or_default();
    let pattern =
        unbounded_search_pattern(target).map_err(PromptLookupFailure::Unsearchable)?;
    let candidates =
        matches_in_reading_order(block_list, &pattern).ok_or(PromptLookupFailure::NotPainted)?;
    if candidates.is_empty() {
        return Err(PromptLookupFailure::NotPainted);
    }

    // The nearest neighbour on each side that can be found on its own bounds the search window.
    let lower = prompt_texts[..index]
        .iter()
        .enumerate()
        .rev()
        .find(|(_, text)| is_distinctive(text))
        .and_then(|(_, text)| resolve_distinctive(block_list, text))
        .map(|(block_index, range)| DocumentPosition::of(block_index, &range));
    let upper = prompt_texts
        .get(index + 1..)
        .unwrap_or_default()
        .iter()
        .find(|text| is_distinctive(text))
        .and_then(|text| resolve_distinctive(block_list, text))
        .map(|(block_index, range)| DocumentPosition::of(block_index, &range));

    let in_window = |candidate: &(BlockIndex, Match)| {
        let position = DocumentPosition::of(candidate.0, &candidate.1);
        lower.is_none_or(|bound| position > bound) && upper.is_none_or(|bound| position < bound)
    };

    // Only when a neighbour actually resolved. With no bound every match "fits", which would turn
    // the search below into an unanchored first-match guess — exactly what this path exists to
    // avoid.
    if lower.is_some() || upper.is_some() {
        // Within the window the prompt precedes the agent's reply to it, so the earliest match is
        // the prompt itself rather than a later echo of the same words.
        if let Some((block_index, range)) = candidates.iter().find(|c| in_window(c)) {
            return Ok(PromptLocation {
                block_index: *block_index,
                range: range.clone(),
            });
        }
    }

    // No window, or nothing inside it. Only a prompt distinctive on its own can be trusted here;
    // for anything shorter an unbounded guess is worse than admitting we cannot place it.
    if !is_distinctive(target) {
        return Err(PromptLookupFailure::Unsearchable(
            UnsearchablePrompt::TooShortToSearchAlone,
        ));
    }
    // Newest painting wins: after a restart the same prompt appears in both the restored block and
    // the resumed agent's replay, and the live copy is the one the user means.
    let (block_index, range) = candidates.last().expect("checked non-empty above");
    Ok(PromptLocation {
        block_index: *block_index,
        range: range.clone(),
    })
}

/// Resolves a prompt that is distinctive on its own, to its newest painting.
fn resolve_distinctive(block_list: &BlockList, text: &str) -> Option<(BlockIndex, Match)> {
    let pattern = unbounded_search_pattern(text).ok()?;
    matches_in_reading_order(block_list, &pattern)?.pop()
}

#[cfg(test)]
#[path = "cli_agent_prompt_locator_tests.rs"]
mod tests;

