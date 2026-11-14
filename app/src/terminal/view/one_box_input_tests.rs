use super::shortened_hint;

#[test]
fn the_hint_gets_a_third_of_a_narrow_pane_and_none_of_a_tiny_one() {
    let hint = "[low|medium|high|xhigh|max|auto|ultracode [on|off]]";
    assert_eq!(shortened_hint(hint, 200).as_deref(), Some(hint));
    assert_eq!(shortened_hint(hint, 38).as_deref(), Some("[low|medium…"));
    assert_eq!(shortened_hint(hint, 20), None);
    assert_eq!(shortened_hint("[name]", 30).as_deref(), Some("[name]"));
}
