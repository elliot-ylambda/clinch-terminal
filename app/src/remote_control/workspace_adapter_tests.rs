use super::*;

#[test]
fn activity_aggregation_keeps_attention_and_work_visible() {
    assert_eq!(
        merge_activity(ProjectActivity::Working, ProjectActivity::NeedsAttention),
        ProjectActivity::NeedsAttention
    );
    assert_eq!(
        merge_activity(ProjectActivity::Done, ProjectActivity::RunningCommand),
        ProjectActivity::RunningCommand
    );
}

#[test]
fn local_directory_validation_rejects_relative_and_file_paths() {
    assert!(canonical_local_directory("relative/path").is_err());
    let directory = tempfile::tempdir().unwrap();
    assert_eq!(
        canonical_local_directory(directory.path().to_str().unwrap()).unwrap(),
        std::fs::canonicalize(directory.path()).unwrap()
    );
    let file = directory.path().join("file.txt");
    std::fs::write(&file, b"x").unwrap();
    assert!(canonical_local_directory(file.to_str().unwrap()).is_err());
}

#[test]
fn mobile_titles_are_nonempty_collapsed_and_bounded() {
    assert_eq!(
        nonempty_title("  hello   world  ".to_owned(), "fallback"),
        "hello world"
    );
    assert_eq!(nonempty_title("   ".to_owned(), "fallback"), "fallback");
    assert_eq!(
        nonempty_title("x".repeat(500), "fallback").chars().count(),
        120
    );
}
