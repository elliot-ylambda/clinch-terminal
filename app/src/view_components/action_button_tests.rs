use super::ButtonSize;

// The large CLI-footer button must be visibly bigger than the standard
// agent-input button in the size dimensions that don't require a live
// Appearance/AppContext (those are exercised via manual/visual testing).
#[test]
fn agent_input_button_large_is_bigger() {
    assert_eq!(
        ButtonSize::AgentInputButtonLarge.button_horizontal_padding(),
        9.0,
        "large variant should use 9px horizontal padding"
    );
    assert!(
        ButtonSize::AgentInputButtonLarge.button_horizontal_padding()
            > ButtonSize::AgentInputButton.button_horizontal_padding(),
        "large horizontal padding must exceed the standard AgentInputButton (4.0)"
    );
    assert!(
        ButtonSize::AgentInputButtonLarge.keystroke_left_spacing()
            >= ButtonSize::AgentInputButton.keystroke_left_spacing()
    );
}
