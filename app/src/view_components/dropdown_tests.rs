use warp_core::ui::appearance::Appearance;
use warpui::platform::WindowStyle;
use warpui::App;

use super::Dropdown;

#[test]
fn menu_header_override_populates_an_empty_selection() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (_, dropdown) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut dropdown = Dropdown::<()>::new(ctx);
            dropdown.set_menu_header_text_override(|_| "First restored message".to_owned());
            dropdown
        });

        dropdown.read(&app, |dropdown, ctx| {
            assert_eq!(
                dropdown.top_bar_text_and_font_family(ctx).0,
                "First restored message"
            );
        });
    });
}
