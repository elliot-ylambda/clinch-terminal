use warpui::App;

use super::*;
use crate::terminal::TerminalModel;

#[test]
fn builds_a_whitespace_tolerant_pattern() {
    let pattern = agent_prompt_search_pattern("please review the failing test").unwrap();
    assert_eq!(pattern, "\\bplease\\s*review\\s*the\\s*failing\\s*test\\b");
}

#[test]
fn anchors_on_word_boundaries_only_where_a_word_character_sits() {
    // Trailing `\b` after the `.` would demand a following word character and never match.
    let pattern = agent_prompt_search_pattern("Longer story.").unwrap();
    assert_eq!(pattern, "\\bLonger\\s*story\\.");

    let pattern = agent_prompt_search_pattern("...and then what").unwrap();
    assert!(pattern.starts_with("\\.\\.\\."));
    assert!(pattern.ends_with("what\\b"));
}

#[test]
fn word_boundaries_keep_a_short_prompt_out_of_longer_words() {
    let pattern = agent_prompt_search_pattern("fine").unwrap();
    let regex = regex::Regex::new(&pattern).unwrap();
    assert!(regex.is_match("that's fine by me"));
    assert!(!regex.is_match("we should refine the parser"));
}

#[test]
fn matches_across_a_wrap_that_inserts_no_separator() {
    let pattern = agent_prompt_search_pattern("please review the failing test").unwrap();
    let regex = regex::Regex::new(&pattern).unwrap();
    // Soft wrap: the grid yields the two rows' cells back to back with nothing between them.
    assert!(regex.is_match("please review thefailing test"));
    // Hard wrap with the continuation indent an agent TUI adds.
    assert!(regex.is_match("please review the\n    failing test"));
}

#[test]
fn escapes_regex_metacharacters_in_the_prompt() {
    let pattern = agent_prompt_search_pattern("what does foo(bar) do here").unwrap();
    let regex = regex::Regex::new(&pattern).unwrap();
    assert!(regex.is_match("what does foo(bar) do here"));
    assert!(!regex.is_match("what does fooXbarY do here"));
}

#[test]
fn distinguishes_too_short_from_empty() {
    // These must not be reported as "not in scrollback" — they are on screen, just unfindable.
    assert_eq!(
        agent_prompt_search_pattern("hi"),
        Err(UnsearchablePrompt::TooShort)
    );
    assert_eq!(
        agent_prompt_search_pattern("ok"),
        Err(UnsearchablePrompt::TooShort)
    );
    assert_eq!(
        agent_prompt_search_pattern("  \n  "),
        Err(UnsearchablePrompt::Empty)
    );
    assert_eq!(agent_prompt_search_pattern(""), Err(UnsearchablePrompt::Empty));
}

#[test]
fn uses_only_the_first_non_empty_line() {
    // A leading blank line is common when a prompt is pasted in.
    let pattern = agent_prompt_search_pattern("\n\nfix the flaky test\nand then push").unwrap();
    assert_eq!(pattern, "\\bfix\\s*the\\s*flaky\\s*test\\b");
}

#[test]
fn bounds_the_pattern_for_long_prompts() {
    let prompt = "alpha bravo charlie delta echo foxtrot golf hotel india juliett kilo";
    let pattern = agent_prompt_search_pattern(prompt).unwrap();
    assert_eq!(pattern.split("\\s*").count(), MAX_PATTERN_TOKENS);
    assert!(!pattern.contains("india"));
}

#[test]
fn stops_at_the_character_cap_before_the_token_cap() {
    // Ten characters per token: the seventh crosses `MAX_PATTERN_CHARS` (64), the eighth is
    // never reached even though `MAX_PATTERN_TOKENS` would allow it.
    let prompt = "aaaaaaaaaa bbbbbbbbbb cccccccccc dddddddddd \
                  eeeeeeeeee ffffffffff gggggggggg hhhhhhhhhh";
    let pattern = agent_prompt_search_pattern(prompt).unwrap();

    assert_eq!(pattern.split("\\s*").count(), 7);
    // The cap is checked before each token, so the one that crosses it still completes —
    // truncating mid-word would build a pattern matching text the user never typed.
    assert!(pattern.ends_with("gggggggggg\\b"));
    assert!(!pattern.contains("hhhhhhhhhh"));
}

#[test]
fn locates_a_prompt_in_the_agent_block_output() {
    App::test((), |mut _app| async move {
        let mut model = TerminalModel::mock(None, None);
        model.simulate_block("ls", "file.txt\r\n");
        model.simulate_block(
            "claude",
            "> please review the failing test\r\nI'll take a look.\r\n",
        );

        let location =
            locate_agent_prompt(model.block_list(), "please review the failing test").unwrap();

        assert_eq!(location.block_index, 2.into());
        assert_eq!(location.range.start().row, 0);
    });
}

#[test]
fn reports_none_when_the_prompt_was_never_painted() {
    App::test((), |mut _app| async move {
        let mut model = TerminalModel::mock(None, None);
        model.simulate_block("claude", "> a different question entirely\r\n");

        // The case a resumed or bridged session hits: the transcript lists the prompt, but this
        // pane never rendered it.
        assert_eq!(
            locate_agent_prompt(model.block_list(), "please review the failing test"),
            Err(PromptLookupFailure::NotPainted)
        );
    });
}

#[test]
fn prefers_the_newest_painting_when_a_prompt_appears_twice() {
    App::test((), |mut _app| async move {
        let mut model = TerminalModel::mock(None, None);
        // What a restart looks like: the restored block replays the old conversation, then the
        // resumed agent paints it again in a fresh block.
        model.simulate_block("claude", "> please review the failing test\r\n");
        model.simulate_block(
            "claude --resume abc",
            "> please review the failing test\r\nresuming.\r\n",
        );

        let location =
            locate_agent_prompt(model.block_list(), "please review the failing test").unwrap();

        // The live conversation, not the restored copy.
        assert_eq!(location.block_index, 2.into());
    });
}

#[test]
fn ignores_the_command_that_launched_the_agent() {
    App::test((), |mut _app| async move {
        let mut model = TerminalModel::mock(None, None);
        // The prompt text appears only in the block's command, never its output. Matching it would
        // scroll to the shell command rather than to a message.
        model.simulate_block("echo please review the failing test", "");

        assert_eq!(
            locate_agent_prompt(model.block_list(), "please review the failing test"),
            Err(PromptLookupFailure::NotPainted)
        );
    });
}

/// Reproduces a real Codex session: prompts "hi", "Tell me a short story.", "Longer story.".
/// Clicking "hi" reported "isn't in this pane's scrollback" while it sat plainly on screen. The
/// refusal is right — two characters cannot be located — but it has to say so accurately.
#[test]
fn short_first_prompt_reports_too_short_not_missing_from_scrollback() {
    App::test((), |mut _app| async move {
        let mut model = TerminalModel::mock(None, None);
        model.simulate_long_running_block(
            "cx",
            "› hi\r\nHello! How can I help?\r\n\
             › Tell me a short story.\r\nOnce upon a time...\r\n\
             › Longer story.\r\nA longer tale follows.\r\n",
        );

        assert_eq!(
            locate_agent_prompt(model.block_list(), "hi"),
            Err(PromptLookupFailure::Unsearchable(UnsearchablePrompt::TooShort))
        );
        // The other two are locatable, so the feature works for them in this same session.
        assert!(locate_agent_prompt(model.block_list(), "Tell me a short story.").is_ok());
        assert!(locate_agent_prompt(model.block_list(), "Longer story.").is_ok());
    });
}

/// The agent block is still executing while the user clicks — output must be searchable before the
/// command finishes.
#[test]
fn locates_a_prompt_while_the_agent_block_is_still_running() {
    App::test((), |mut _app| async move {
        let mut model = TerminalModel::mock(None, None);
        model.simulate_long_running_block("cx", "› Tell me a short story.\r\nOnce upon a time...\r\n");

        assert!(locate_agent_prompt(model.block_list(), "Tell me a short story.").is_ok());
    });
}

#[test]
fn skips_prompts_too_short_to_locate_rather_than_matching_noise() {
    App::test((), |mut _app| async move {
        let mut model = TerminalModel::mock(None, None);
        model.simulate_block("claude", "> ok\r\nok, done.\r\n");

        // "ok" occurs all over a real transcript; reporting it as unlocatable beats scrolling
        // somewhere arbitrary.
        assert_eq!(
            locate_agent_prompt(model.block_list(), "ok"),
            Err(PromptLookupFailure::Unsearchable(UnsearchablePrompt::TooShort))
        );
    });
}
