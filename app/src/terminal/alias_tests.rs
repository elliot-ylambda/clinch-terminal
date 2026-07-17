use super::*;

#[test]
fn test_is_expandable_alias_when_expandable() {
    // Alias is not in the alias value
    let expandable = is_expandable_alias("gco", "git checkout");
    assert!(expandable);

    // Alias is in the alias value but not in a command position
    let expandable = is_expandable_alias("gco", "git checkout gco");
    assert!(expandable);
}

#[test]
fn test_is_expandable_alias_when_unexpandable() {
    let expandable = is_expandable_alias("ls", "ls -G");
    assert!(!expandable);

    let expandable = is_expandable_alias("ls", "ls");
    assert!(!expandable);
}

#[test]
fn test_is_expandable_alias_when_alias_value_is_empty() {
    let expandable = is_expandable_alias("ls", "");
    assert!(!expandable);
}
