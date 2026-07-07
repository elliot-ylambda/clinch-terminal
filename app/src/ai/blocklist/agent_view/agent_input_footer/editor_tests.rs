use super::*;

#[test]
fn next_selection_with_custom_button_appends_and_materializes_default() {
    let next = next_selection_with_custom_button(
        CLIAgentToolbarChipSelection::Default,
        "Ship".into(),
        "/deploy".into(),
    );
    let CLIAgentToolbarChipSelection::Custom { left, .. } = next else {
        panic!("expected Custom");
    };
    // Default left items are materialized, then the new button is appended last.
    assert_eq!(
        left.last(),
        Some(&AgentToolbarItemKind::CustomInsert {
            label: "Ship".into(),
            text: "/deploy".into()
        })
    );
    assert!(left.contains(&AgentToolbarItemKind::ForkSession)); // materialized default
}
