use std::fs;

use super::*;

// Builds a temp dir with a fake `.claude/commands` tree and returns its path.
fn write(root: &std::path::Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

#[test]
fn claude_commands_are_named_and_namespaced() {
    let tmp = tempfile::tempdir().unwrap();
    let cmd_dir = tmp.path().join(".claude/commands");
    write(
        &cmd_dir,
        "review.md",
        "---\ndescription: Review the diff\n---\nReview it.",
    );
    write(&cmd_dir, "frontend/component.md", "Make a component.");
    write(&cmd_dir, "notes.txt", "ignored, not markdown");

    let mut got = scan_claude_commands_dir(&cmd_dir, CommandScope::Project);
    got.sort_by(|a, b| a.name.cmp(&b.name));

    assert_eq!(got.len(), 2);
    assert_eq!(got[0].name, "/frontend:component");
    assert_eq!(got[0].invocation, "/frontend:component");
    assert_eq!(got[0].description, None);
    assert_eq!(got[1].name, "/review");
    assert_eq!(got[1].description.as_deref(), Some("Review the diff"));
    assert_eq!(got[1].scope, CommandScope::Project);
    assert_eq!(got[1].provider, CommandProvider::Claude);
}

#[test]
fn missing_dir_yields_empty() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(scan_claude_commands_dir(&tmp.path().join("nope"), CommandScope::Home).is_empty());
}

#[test]
fn description_absent_without_front_matter() {
    let desc = front_matter_description("Just a body, no front matter.");
    assert_eq!(desc, None);
    let desc = front_matter_description("---\ndescription: Hi\nmodel: opus\n---\nbody");
    assert_eq!(desc.as_deref(), Some("Hi"));
}
