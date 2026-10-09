use super::{agent_title_text, standardize_agent_title, AgentTitleStatus};
use crate::terminal::CLIAgent;

// Raw titles below were captured from Claude Code 2.1.293 and Codex 0.161.0.

const IDLE: AgentTitleStatus = AgentTitleStatus {
    needs_input: false,
    is_working: false,
};
const WORKING: AgentTitleStatus = AgentTitleStatus {
    needs_input: false,
    is_working: true,
};
const NEEDS_INPUT: AgentTitleStatus = AgentTitleStatus {
    needs_input: true,
    is_working: false,
};

fn title(agent: CLIAgent, raw: &str, text: Option<&str>, status: AgentTitleStatus) -> String {
    standardize_agent_title(agent, raw, text, status).unwrap()
}

fn is_working_frame(title: &str) -> bool {
    ["◐", "◓", "◑", "◒"]
        .iter()
        .any(|frame| title.starts_with(frame))
}

#[test]
fn claude_titles_keep_their_own_glyphs() {
    assert_eq!(
        title(CLIAgent::Claude, "✳ Claude Code", None, IDLE),
        "✳ Claude Code"
    );
    assert_eq!(
        title(CLIAgent::Claude, "◑ Slow counting with sleep", None, IDLE),
        "◑ Slow counting with sleep"
    );
}

#[test]
fn codex_titles_take_claude_style() {
    // Idle: no glyph, project suffix.
    assert_eq!(
        title(
            CLIAgent::Codex,
            "Run three sequential sleeps | cx-test",
            None,
            IDLE
        ),
        "✳ Run three sequential sleeps"
    );
    // Working: a braille spinner becomes one of Claude's spinner frames.
    let working = title(
        CLIAgent::Codex,
        "⠏ Run three sequential sleeps | cx-test",
        None,
        IDLE,
    );
    assert!(is_working_frame(&working), "{working}");
    assert!(
        working.ends_with(" Run three sequential sleeps"),
        "{working}"
    );
    // Before a task name exists Codex repeats its spinner.
    let starting = title(CLIAgent::Codex, "⠼ ⠼ | cx-test", None, IDLE);
    assert!(is_working_frame(&starting), "{starting}");
    assert!(starting.ends_with(" Codex"), "{starting}");
}

#[test]
fn codex_spinner_frames_keep_spinning() {
    let frames = ["⠋", "⠙", "⠹", "⠸"].map(|frame| {
        title(CLIAgent::Codex, &format!("{frame} Task"), None, IDLE)
            .chars()
            .next()
    });
    let distinct: std::collections::HashSet<_> = frames.iter().collect();
    assert!(distinct.len() > 1, "{frames:?}");
}

#[test]
fn prompt_titles_get_the_same_glyph_as_the_agent_title() {
    assert_eq!(
        title(
            CLIAgent::Codex,
            "Fix it | repo",
            Some("Fix the flaky test"),
            IDLE
        ),
        "✳ Fix the flaky test"
    );
    let working = title(
        CLIAgent::Claude,
        "◓ Fixing",
        Some("Fix the flaky test"),
        IDLE,
    );
    assert_eq!(working, "◓ Fix the flaky test");
    // A title without a glyph falls back to Clinch's working status.
    let working = title(
        CLIAgent::Codex,
        "cx-test",
        Some("Fix the flaky test"),
        WORKING,
    );
    assert!(is_working_frame(&working), "{working}");
}

#[test]
fn selected_provider_titles_have_only_one_status_glyph() {
    for (agent, raw) in [
        (CLIAgent::Claude, "◓ Fix cat data.csv | sort -u"),
        (CLIAgent::Codex, "⠏ Fix cat data.csv | sort -u | repo"),
    ] {
        let selected = agent_title_text(agent, raw);
        assert_eq!(selected, "Fix cat data.csv | sort -u");
        assert_eq!(
            title(agent, raw, Some(selected), NEEDS_INPUT),
            "! Fix cat data.csv | sort -u"
        );
    }
}

#[test]
fn prompt_titles_preserve_pipes_and_leading_punctuation() {
    for agent in [CLIAgent::Claude, CLIAgent::Codex] {
        assert_eq!(
            title(
                agent,
                "Fix pipeline | repo",
                Some("Fix cat data.csv | sort -u"),
                IDLE,
            ),
            "✳ Fix cat data.csv | sort -u"
        );
        assert_eq!(
            title(agent, "Task | repo", Some("!important CSS rule"), IDLE),
            "✳ !important CSS rule"
        );
    }
}

#[test]
fn raw_codex_title_only_removes_the_final_project_suffix() {
    assert_eq!(
        title(
            CLIAgent::Codex,
            "⠏ Fix cat data.csv | sort -u | repo",
            None,
            NEEDS_INPUT,
        ),
        "! Fix cat data.csv | sort -u"
    );
}

#[test]
fn waiting_on_the_user_shows_an_exclamation_for_both_agents() {
    assert_eq!(
        title(CLIAgent::Claude, "✳ Pick a color", None, NEEDS_INPUT),
        "! Pick a color"
    );
    assert_eq!(
        title(CLIAgent::Codex, "⠏ Pick a color | repo", None, NEEDS_INPUT),
        "! Pick a color"
    );
}

#[test]
fn other_agents_are_left_alone() {
    assert_eq!(
        standardize_agent_title(CLIAgent::Unknown, "⠏ something", None, IDLE),
        None
    );
}
