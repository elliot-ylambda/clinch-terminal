/// Returns whether the alias can be expanded by Warp given its value.
///
/// We don't expand on any alias that starts with itself, as it leads to
/// cases where the alias is expanded twice: once as the user types in the
/// editor and again by the shell when the command is entered.
// TODO: CORE-240 Don't expand if any command in the alias value is equal
// to the alias itself.
pub fn is_expandable_alias(alias: &str, alias_value: &str) -> bool {
    if let Some(command_token) = alias_value.split_whitespace().next() {
        return alias != command_token;
    }
    // If the alias value is empty, we don't expand.
    false
}

#[cfg(test)]
#[path = "alias_tests.rs"]
pub mod tests;
