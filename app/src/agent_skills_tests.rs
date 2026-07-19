use super::*;

fn bundled_skill_contents() -> String {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../resources/bundled/agent-skills/clinch-toolbelt/SKILL.md"
    );
    std::fs::read_to_string(path).expect("bundled clinch-toolbelt SKILL.md must exist")
}

#[test]
fn bundled_skill_carries_a_managed_marker() {
    let contents = bundled_skill_contents();
    assert!(
        contents.contains("<!-- managed-by: Clinch; version: 1.0.0 -->"),
        "the bundled skill must carry the Clinch managed marker"
    );
}

/// The skill tells agents to materialize the shipped defaults before switching a
/// footer from `default` to `custom`. If the defaults change in code but not in
/// the skill, agents would delete buttons from user footers. Keep them in sync.
#[test]
fn bundled_skill_lists_every_default_custom_insert() {
    use crate::ai::blocklist::agent_view::toolbar_item::AgentToolbarItemKind;

    let contents = bundled_skill_contents();
    let defaults = AgentToolbarItemKind::cli_default_left()
        .into_iter()
        .chain(AgentToolbarItemKind::terminal_default_left());
    for item in defaults {
        if let AgentToolbarItemKind::CustomInsert { label, text } = item {
            assert!(
                contents.contains(&format!("label = \"{label}\"")),
                "SKILL.md is missing default button label {label:?}"
            );
            assert!(
                contents.contains(&format!("text = \"{text}\"")),
                "SKILL.md is missing default button text {text:?}"
            );
        }
    }
    // Spot-check the non-custom CLI defaults agents must materialize as well.
    for name in [
        "\"fork_session\"",
        "\"compact\"",
        "\"continue_prompt\"",
        "\"looks_good_prompt\"",
        "\"transfer_agent\"",
        "\"voice_input\"",
    ] {
        assert!(
            contents.contains(name),
            "SKILL.md is missing default item {name}"
        );
    }
}
