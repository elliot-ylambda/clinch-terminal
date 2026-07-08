use super::derive_label;

#[test]
fn derive_label_trims_and_falls_back() {
    assert_eq!(derive_label("/review the code"), "/review the code");
    assert_eq!(derive_label(""), "Custom");
    assert_eq!(derive_label(&"x".repeat(40)), "x".repeat(24));
}

#[test]
fn derive_label_uses_first_non_empty_line() {
    assert_eq!(
        derive_label("\n\n  /deploy staging  \nsecond line"),
        "/deploy staging"
    );
    assert_eq!(derive_label("   \n\t\n"), "Custom");
}
