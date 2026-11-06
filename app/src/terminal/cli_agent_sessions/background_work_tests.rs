use super::*;

const RULE: &str = "────────────────────────────────────────";

/// Claude Code 2.1.319's bottom rows, captured from a live pane, with `pills` in the mode row.
fn claude_screen(pills: &str) -> Vec<String> {
    vec![
        "✻ Baked for 3m 51s · done 10:47 AM · 1 shell still running".to_owned(),
        String::new(),
        RULE.to_owned(),
        "❯ ".to_owned(),
        RULE.to_owned(),
        "  v2.1.319 | Opus 5.5 high 14% | ~/projects/clinch | main".to_owned(),
        format!("  ⏵⏵ bypass permissions on{pills} · ← 4 agents"),
    ]
}

/// Codex 0.167's bottom rows, captured from a live pane, with `status` above the composer.
fn codex_screen(status: &str) -> Vec<String> {
    vec![
        "• Started the background terminal.".to_owned(),
        String::new(),
        status.to_owned(),
        String::new(),
        String::new(),
        "› Ask Codex to do anything".to_owned(),
        String::new(),
        "  GPT-6-Astra xhigh · ~/projects/clinch · Ready · 0.167.0 · 258K window".to_owned(),
        "  ← for agents · ? for shortcuts".to_owned(),
    ]
}

fn work(shells: u32, monitors: u32, tasks: u32, subagents: u32) -> BackgroundWork {
    BackgroundWork {
        shells,
        monitors,
        tasks,
        subagents,
    }
}

#[test]
fn claude_counts_shells_and_monitors_in_singular_and_plural() {
    let cases = [
        ("", work(0, 0, 0, 0)),
        (" · 1 shell", work(1, 0, 0, 0)),
        (" · 3 shells", work(3, 0, 0, 0)),
        (" · 1 monitor", work(0, 1, 0, 0)),
        (" · 1 shell, 1 monitor", work(1, 1, 0, 0)),
        (" · 2 shells, 4 monitors", work(2, 4, 0, 0)),
        (" · 2 Artifact comment monitors", work(0, 2, 0, 0)),
    ];
    for (pills, expected) in cases {
        assert_eq!(
            parse_status_rows(CLIAgent::Claude, &claude_screen(pills)),
            Some(expected),
            "pills: {pills:?}"
        );
    }
}

#[test]
fn claude_totals_mixed_tasks_and_counts_background_subagents() {
    assert_eq!(
        parse_status_rows(CLIAgent::Claude, &claude_screen(" · 3 background tasks")),
        Some(work(0, 0, 3, 0))
    );
    assert_eq!(
        parse_status_rows(CLIAgent::Claude, &claude_screen(" · 1 local agent")),
        Some(work(0, 0, 0, 1))
    );
    assert_eq!(
        parse_status_rows(CLIAgent::Claude, &claude_screen(" · 2 MCP tasks")),
        Some(work(0, 0, 2, 0))
    );
}

#[test]
fn claude_ignores_agent_view_transcript_and_truncated_pills() {
    // `← 4 agents` is in every claude_screen; the transcript's `1 shell still running` sits above
    // the composer. Neither counts.
    assert_eq!(
        parse_status_rows(CLIAgent::Claude, &claude_screen("")),
        Some(BackgroundWork::default())
    );
    // A narrow pane truncates the row; the cut segment is dropped rather than guessed.
    assert_eq!(
        parse_status_rows(CLIAgent::Claude, &claude_screen(" · 1 shell, 1 mon…")),
        Some(work(1, 0, 0, 0))
    );
}

#[test]
fn claude_reads_a_multiline_draft_and_a_labelled_upper_rule() {
    let rows = [
        "──── fix-the-build ─────────────────────",
        "❯ first line",
        "  second line",
        RULE,
        "  ⏵⏵ accept edits on · 2 shells",
    ];
    assert_eq!(
        parse_status_rows(CLIAgent::Claude, &rows),
        Some(work(2, 0, 0, 0))
    );
}

#[test]
fn claude_without_its_composer_is_unknown() {
    // A permission dialog replaces the composer; its border has no prompt row under it.
    let dialog = [
        RULE,
        " Bash command",
        "   rm -rf build",
        " Do you want to proceed?",
        " ❯ 1. Yes",
        "   2. No",
    ];
    assert_eq!(parse_status_rows(CLIAgent::Claude, &dialog), None);
    assert_eq!(parse_status_rows::<&str>(CLIAgent::Claude, &[]), None);
}

#[test]
fn codex_counts_background_terminals_while_working_and_idle() {
    assert_eq!(
        parse_status_rows(
            CLIAgent::Codex,
            &codex_screen(
                "• Working (22s • esc to interrupt) · 1 background terminal running · /ps to view · /stop to close"
            )
        ),
        Some(work(1, 0, 0, 0))
    );
    assert_eq!(
        parse_status_rows(
            CLIAgent::Codex,
            &codex_screen("  3 background terminals running · /ps to view · /stop to close")
        ),
        Some(work(3, 0, 0, 0))
    );
    assert_eq!(
        parse_status_rows(CLIAgent::Codex, &codex_screen("")),
        Some(BackgroundWork::default())
    );
}

#[test]
fn codex_without_its_composer_near_the_bottom_is_unknown() {
    // A transcript user message also starts with `›`; with no composer at the bottom it must not
    // be mistaken for one.
    let mut rows = vec![
        "  1 background terminal running · /ps to view".to_owned(),
        "› fix the build".to_owned(),
    ];
    rows.extend((0..8).map(|line| format!("  approval option {line}")));
    assert_eq!(parse_status_rows(CLIAgent::Codex, &rows), None);
}

#[test]
fn other_agents_have_no_status_line() {
    assert_eq!(
        parse_status_rows(CLIAgent::Gemini, &claude_screen(" · 1 shell")),
        None
    );
}
