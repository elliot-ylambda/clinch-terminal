use std::path::PathBuf;

use super::*;
use crate::terminal::view::start_screen_project_name;

#[test]
fn the_start_command_starts_the_agent_with_the_text_or_runs_it() {
    let claude = StartChoice::Agent(AgentResumeProvider::Claude);
    let zsh = Some(ShellType::Zsh);
    assert_eq!(start_command(claude, "  ", zsh).as_deref(), Some("claude"));
    assert_eq!(
        start_command(claude, "fix the parser's tests\n", zsh).as_deref(),
        Some("claude 'fix the parser'\\''s tests'")
    );
    assert_eq!(
        start_command(claude, "-v means verbose?", zsh).as_deref(),
        Some("claude -- '-v means verbose?'")
    );
    assert_eq!(
        start_command(StartChoice::Terminal, " npm test ", zsh).as_deref(),
        Some("npm test")
    );
    assert_eq!(start_command(StartChoice::Terminal, "", zsh), None);
}

#[test]
fn text_is_quoted_for_the_tab_s_own_shell() {
    let text = "it's a\\ test\nline two";
    assert_eq!(
        quote_for(Some(ShellType::Bash), text),
        "'it'\\''s a\\ test\nline two'"
    );
    assert_eq!(
        quote_for(Some(ShellType::Fish), text),
        "'it\\'s a\\\\ test\nline two'"
    );
    assert_eq!(
        quote_for(Some(ShellType::PowerShell), "it's \u{2019}quoted\u{2019}"),
        "'it''s \u{2019}\u{2019}quoted\u{2019}\u{2019}'"
    );
}

#[test]
fn the_command_line_moves_into_the_worktree_first() {
    let path = std::path::Path::new("/x/worktrees/my project/mesa-quail");
    let start = (
        StartChoice::Agent(AgentResumeProvider::Codex),
        "go".to_owned(),
    );
    let codex = AgentResumeProvider::Codex.launch_command_prefix(true);
    assert_eq!(
        shell_command_line(Some(path), Some(&start), Some(ShellType::Zsh)),
        Some(format!(
            "cd '/x/worktrees/my project/mesa-quail' && {codex} go"
        ))
    );
    assert_eq!(
        shell_command_line(Some(path), None, Some(ShellType::Zsh)).as_deref(),
        Some("cd '/x/worktrees/my project/mesa-quail'")
    );
    assert_eq!(shell_command_line(None, None, None), None);
}

#[test]
fn the_saved_choice_round_trips_and_defaults_to_claude() {
    for choice in NEW_TAB_CHOICES {
        assert_eq!(
            configured_choice(choice_setting(*choice).expect("an agent")),
            *choice
        );
    }
    // The terminal is the toggle's other side now, so an old "terminal" choice starts Claude.
    for setting in ["something else", "terminal"] {
        assert_eq!(
            configured_choice(setting),
            StartChoice::Agent(AgentResumeProvider::Claude)
        );
    }
    assert_eq!(choice_setting(StartChoice::Terminal), None);
}

#[test]
fn the_worktree_label_names_a_reused_folder_and_its_new_branch() {
    let fresh = PreparedWorktree {
        path: PathBuf::from("/x/worktrees/magister/mesa-quail"),
        branch: "mesa-quail".to_owned(),
        base: "origin/main".to_owned(),
        reused: false,
    };
    assert_eq!(
        worktree_label(&fresh),
        "Worktree mesa-quail · from origin/main"
    );
    let reused = PreparedWorktree {
        branch: "dune-wren".to_owned(),
        reused: true,
        ..fresh
    };
    assert_eq!(
        worktree_label(&reused),
        "Worktree mesa-quail on dune-wren · from origin/main"
    );
}

#[test]
fn the_project_is_the_folder_name_except_at_home() {
    assert_eq!(
        start_screen_project_name(std::path::Path::new("/x/magistermarketing")).as_deref(),
        Some("magistermarketing")
    );
    if let Some(home) = dirs::home_dir() {
        assert_eq!(start_screen_project_name(&home), None);
    }
}
