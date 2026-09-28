use super::*;

#[test]
fn section_names_are_trimmed_and_cannot_be_empty() {
    assert_eq!(validated_name("  Backend  ".to_owned()).unwrap(), "Backend");
    assert_eq!(
        validated_name("   ".to_owned()).unwrap_err().code,
        ErrorCode::InvalidParams
    );
}

#[test]
fn default_color_clears_the_section_tint() {
    assert_eq!(
        parse_color("default".to_owned()).unwrap(),
        SelectedSectionColor::Unset
    );
    assert_eq!(
        parse_color("magenta".to_owned()).unwrap(),
        SelectedSectionColor::Color(SectionColor::Magenta)
    );
    assert_eq!(
        parse_color("clinch-green".to_owned()).unwrap(),
        SelectedSectionColor::Color(SectionColor::ClinchGreen)
    );
}
