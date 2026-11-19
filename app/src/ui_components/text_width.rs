use warpui::elements::DEFAULT_UI_LINE_HEIGHT_RATIO;
use warpui::fonts::Properties;
use warpui::platform::LineStyle;
use warpui::text_layout::{ClipConfig, StyleAndFont, TextStyle, DEFAULT_TOP_BOTTOM_RATIO};
use warpui::AppContext;

use crate::appearance::Appearance;

/// The width of `text` laid out on one line in the UI font, for sizing an element to its label
/// before layout.
pub(crate) fn ui_text_width(
    text: &str,
    font_size: f32,
    properties: Properties,
    appearance: &Appearance,
    app: &AppContext,
) -> f32 {
    if text.is_empty() {
        return 0.;
    }
    app.font_cache()
        .text_layout_system()
        .layout_line(
            text,
            LineStyle {
                font_size,
                line_height_ratio: DEFAULT_UI_LINE_HEIGHT_RATIO,
                baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
                fixed_width_tab_size: None,
            },
            &[(
                0..text.chars().count(),
                StyleAndFont::new(appearance.ui_font_family(), properties, TextStyle::new()),
            )],
            f32::MAX,
            ClipConfig::default(),
        )
        .width
}
