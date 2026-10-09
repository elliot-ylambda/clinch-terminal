//! One status glyph for Claude Code and Codex tab titles.
//!
//! Both agents prefix their terminal title with their own status glyph, in different styles:
//!
//! | State   | Claude Code               | Codex                             |
//! |---------|---------------------------|-----------------------------------|
//! | Idle    | `✳ Fix the build`         | `Fix the build \| repo`           |
//! | Working | `◐ Fix the build` (spins) | `⠏ Fix the build \| repo` (spins) |
//!
//! Tab chrome shows either that title or Clinch's own prompt-based title, so every Claude Code and
//! Codex title is given Claude Code's glyph. Neither agent changes its title while waiting on the
//! user (Claude keeps `✳`), so Clinch's own session status supplies a `!` for that state.
use crate::terminal::CLIAgent;

/// Claude Code's working spinner frames, in its rotation order.
const WORKING_FRAMES: [char; 4] = ['◐', '◓', '◑', '◒'];
const IDLE_GLYPH: char = '✳';
const NEEDS_INPUT_GLYPH: char = '!';
/// Codex suffixes its title with ` | <project>`; the project is already in the tab chrome.
const CODEX_PROJECT_SEPARATOR: &str = " | ";

/// Clinch's view of the session, which outranks or backs up the agent's own title glyph.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct AgentTitleStatus {
    /// Blocked on a question or approval.
    pub(crate) needs_input: bool,
    /// Mid-turn according to Clinch's session events.
    pub(crate) is_working: bool,
}

/// A Claude Code or Codex tab title in Claude Code's glyph style, or `None` for other agents.
///
/// `display_text` is the text Clinch chose for the tab (such as the session's prompt); when it is
/// `None` the agent's own terminal title text is used. The glyph comes from `status` when the
/// agent needs input, otherwise from the spinner in `raw_terminal_title`, falling back to
/// `status.is_working` for a title that carries no glyph.
pub(crate) fn standardize_agent_title(
    agent: CLIAgent,
    raw_terminal_title: &str,
    display_text: Option<&str>,
    status: AgentTitleStatus,
) -> Option<String> {
    if !matches!(agent, CLIAgent::Claude | CLIAgent::Codex) {
        return None;
    }

    let (leading_glyph, title_text) = split_agent_title(agent, raw_terminal_title);
    let glyph = if status.needs_input {
        NEEDS_INPUT_GLYPH
    } else if let Some(frame) = leading_glyph.and_then(working_frame) {
        frame
    } else if status.is_working {
        WORKING_FRAMES[0]
    } else {
        IDLE_GLYPH
    };

    let text = match display_text.map(str::trim) {
        // Prompt-derived text is user content, not a provider terminal title. Preserve its
        // punctuation and pipes; only the raw terminal title has a status/project wrapper.
        Some(text) if !text.is_empty() => text,
        _ => title_text,
    };
    let text = if text.is_empty() {
        agent.display_name()
    } else {
        text
    };
    Some(format!("{glyph} {text}"))
}

/// Removes provider chrome before a raw title participates in Clinch's title selection.
/// Prompt-derived titles must not pass through this cleanup.
pub(crate) fn agent_title_text(agent: CLIAgent, raw_terminal_title: &str) -> &str {
    if matches!(agent, CLIAgent::Claude | CLIAgent::Codex) {
        split_agent_title(agent, raw_terminal_title).1
    } else {
        raw_terminal_title
    }
}

/// Splits an agent title into its first status glyph and the remaining text.
fn split_agent_title(agent: CLIAgent, title: &str) -> (Option<char>, &str) {
    let mut text = title.trim();
    if agent == CLIAgent::Codex {
        if let Some((task, _project)) = text.rsplit_once(CODEX_PROJECT_SEPARATOR) {
            text = task.trim();
        }
    }
    // Codex can repeat its spinner (`⠼ ⠼`) before a task name exists, so strip every leading
    // glyph and keep the first one to tell working from idle.
    let mut leading_glyph = None;
    while let Some(glyph) = text.chars().next().filter(|c| is_status_glyph(*c)) {
        leading_glyph.get_or_insert(glyph);
        text = text[glyph.len_utf8()..].trim_start();
    }
    (leading_glyph, text)
}

fn is_status_glyph(c: char) -> bool {
    c == IDLE_GLYPH || c == NEEDS_INPUT_GLYPH || WORKING_FRAMES.contains(&c) || is_braille(c)
}

fn is_braille(c: char) -> bool {
    ('\u{2800}'..='\u{28FF}').contains(&c)
}

/// The Claude Code spinner frame for an agent's working glyph, or `None` when the glyph means
/// idle. Codex's braille frames map onto Claude's four so its title keeps spinning.
fn working_frame(glyph: char) -> Option<char> {
    if WORKING_FRAMES.contains(&glyph) {
        Some(glyph)
    } else if is_braille(glyph) {
        Some(WORKING_FRAMES[glyph as usize % WORKING_FRAMES.len()])
    } else {
        None
    }
}

#[cfg(test)]
#[path = "title_glyph_tests.rs"]
mod tests;
