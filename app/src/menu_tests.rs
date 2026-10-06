use warp_core::ui::appearance::Appearance;
use warpui::platform::WindowStyle;
use warpui::{App, TypedActionView};

use super::{Menu, MenuAction, MenuItem, MenuItemFields, SelectAction, SubMenu};

#[test]
fn scrollable_menu_tooltip_escapes_clip_and_preserves_row_hover() {
    use std::cell::RefCell;
    use std::collections::HashSet;
    use std::rc::Rc;

    use pathfinder_geometry::vector::vec2f;
    use warpui::{Event, Presenter, WindowInvalidation};

    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let fields = MenuItemFields::<()>::new("Abbreviated message…")
            .with_tooltip("The complete message continues here. ".repeat(40))
            .with_tooltip_max_width(260.)
            .with_tooltip_position(super::MenuTooltipPosition::Above);
        let mouse = fields.mouse_state.clone();
        let (window_id, menu) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut menu = Menu::<()>::new()
                .with_width(220.)
                .with_menu_variant(super::MenuVariant::scrollable());
            menu.set_height(80.);
            menu.set_items(vec![fields.into_item()], ctx);
            menu
        });
        let presenter = Rc::new(RefCell::new(Presenter::new(window_id)));
        app.update(|ctx| {
            for _ in 0..3 {
                presenter.borrow_mut().invalidate(
                    WindowInvalidation {
                        updated: HashSet::from([menu.id()]),
                        ..Default::default()
                    },
                    ctx,
                );
                presenter
                    .borrow_mut()
                    .build_scene(vec2f(800., 800.), 1., None, ctx);
                let event = Event::MouseMoved {
                    position: vec2f(25., 18.),
                    cmd: false,
                    shift: false,
                    is_synthetic: false,
                };
                ctx.simulate_window_event(event.clone(), window_id, presenter.clone());
                ctx.set_last_mouse_move_event(window_id, event);
            }
            assert!(
                mouse.lock().unwrap().is_hovered(),
                "tooltip must not steal row hover"
            );
            let presenter = presenter.borrow();
            let scene = presenter.scene().unwrap();
            // The test font backend returns empty text frames, so verify the
            // popup's painted container here. The history tests independently
            // assert that its payload retains every character of the message.
            let tooltip = scene
                .layers()
                .find(|layer| layer.click_through && !layer.rects.is_empty())
                .expect("tooltip must paint in a click-through overlay");
            assert!(
                tooltip.clip_bounds.is_none(),
                "menu scrolling must not clip the popup"
            );
        });
    });
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum TestAction {
    Root,
    ChildOne,
    ChildTwo,
}

fn test_submenu_items() -> Vec<MenuItem<TestAction>> {
    vec![
        MenuItem::Submenu {
            fields: MenuItemFields::new_submenu("submenu"),
            menu: SubMenu::new(vec![
                MenuItemFields::new("child one")
                    .with_on_select_action(TestAction::ChildOne)
                    .into_item(),
                MenuItemFields::new("child two")
                    .with_on_select_action(TestAction::ChildTwo)
                    .into_item(),
            ]),
        },
        MenuItemFields::new("root")
            .with_on_select_action(TestAction::Root)
            .into_item(),
    ]
}

#[test]
fn test_menu_item_selectable() {
    assert!(MenuItemFields::<()>::new("normal").into_item().selectable());
    assert!(!MenuItemFields::<()>::new("disabled")
        .with_disabled(true)
        .into_item()
        .selectable());
    assert!(!MenuItem::<()>::Separator.selectable());
}

#[test]
fn test_next_and_previous_indexes() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        let items = vec![
            MenuItemFields::<()>::new("item1")
                .with_disabled(true)
                .into_item(),
            MenuItemFields::<()>::new("item2").into_item(),
            MenuItemFields::<()>::new("item3")
                .with_disabled(true)
                .into_item(),
            MenuItemFields::<()>::new("item4").into_item(),
            MenuItemFields::<()>::new("item5").into_item(),
        ];

        let (_, menu) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut menu = Menu::<()>::new();
            menu.set_items(items, ctx);
            menu
        });

        menu.update(&mut app, |menu, _ctx| {
            assert!(menu.selected_item().is_none());

            menu.menu
                .select_internal(SelectAction::Index { row: 1, item: 0 });
            assert!(menu.selected_item().is_some());
            assert_eq!(
                menu.selected_item().unwrap().fields().unwrap().label(),
                "item2"
            );

            // Make sure we skip the disabled menu items
            menu.menu.select_internal(SelectAction::Next);
            assert!(menu.selected_item().is_some());
            assert_eq!(
                menu.selected_item().unwrap().fields().unwrap().label(),
                "item4"
            );

            menu.menu.select_internal(SelectAction::Next);
            assert!(menu.selected_item().is_some());
            assert_eq!(
                menu.selected_item().unwrap().fields().unwrap().label(),
                "item5"
            );

            // Make sure we go around
            menu.menu.select_internal(SelectAction::Next);
            assert!(menu.selected_item().is_some());
            assert_eq!(
                menu.selected_item().unwrap().fields().unwrap().label(),
                "item2"
            );

            // Makre sure we go around with Prev action too
            menu.menu.select_internal(SelectAction::Previous);
            assert!(menu.selected_item().is_some());
            assert_eq!(
                menu.selected_item().unwrap().fields().unwrap().label(),
                "item5"
            );

            menu.menu.select_internal(SelectAction::Previous);
            assert!(menu.selected_item().is_some());
            assert_eq!(
                menu.selected_item().unwrap().fields().unwrap().label(),
                "item4"
            );

            // Makre sure we skip the disabled ones for previous as well
            menu.menu.select_internal(SelectAction::Previous);
            assert!(menu.selected_item().is_some());
            assert_eq!(
                menu.selected_item().unwrap().fields().unwrap().label(),
                "item2"
            );
        });
    })
}

#[test]
fn test_right_opens_selected_submenu_and_selects_first_child() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        let (_, menu) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut menu = Menu::<TestAction>::new();
            menu.set_items(test_submenu_items(), ctx);
            menu
        });

        menu.update(&mut app, |menu, ctx| {
            menu.set_selected_by_index(0, ctx);
            menu.handle_action(&MenuAction::OpenSubmenu, ctx);

            assert_eq!(menu.selected_index(), Some(0));
            let submenu = menu.menu.selected_submenu().unwrap();
            assert_eq!(submenu.selected_index(), Some(0));
            assert_eq!(
                submenu.selected_item().unwrap().fields().unwrap().label(),
                "child one"
            );
        });
    })
}

#[test]
fn test_up_and_down_navigate_the_active_submenu() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        let (_, menu) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut menu = Menu::<TestAction>::new();
            menu.set_items(test_submenu_items(), ctx);
            menu
        });

        menu.update(&mut app, |menu, ctx| {
            menu.set_selected_by_index(0, ctx);
            menu.handle_action(&MenuAction::OpenSubmenu, ctx);
            menu.handle_action(&MenuAction::Select(SelectAction::Next), ctx);

            let submenu = menu.menu.selected_submenu().unwrap();
            assert_eq!(submenu.selected_index(), Some(1));
            assert_eq!(
                submenu.selected_item().unwrap().fields().unwrap().label(),
                "child two"
            );

            menu.handle_action(&MenuAction::Select(SelectAction::Previous), ctx);

            let submenu = menu.menu.selected_submenu().unwrap();
            assert_eq!(submenu.selected_index(), Some(0));
            assert_eq!(
                submenu.selected_item().unwrap().fields().unwrap().label(),
                "child one"
            );
        });
    })
}

#[test]
fn test_enter_uses_the_active_submenu_selection() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        let (_, menu) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut menu = Menu::<TestAction>::new();
            menu.set_items(test_submenu_items(), ctx);
            menu
        });

        let mut selected_action = None;
        menu.update(&mut app, |menu, ctx| {
            menu.set_selected_by_index(0, ctx);
            menu.handle_action(&MenuAction::OpenSubmenu, ctx);
            menu.handle_action(&MenuAction::Select(SelectAction::Next), ctx);

            selected_action = menu.menu.selected_action_for_enter(ctx);
        });

        assert_eq!(selected_action, Some(TestAction::ChildTwo));
    })
}

#[test]
fn test_right_is_a_noop_for_leaf_items() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        let (_, menu) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut menu = Menu::<TestAction>::new();
            menu.set_items(test_submenu_items(), ctx);
            menu
        });

        menu.update(&mut app, |menu, ctx| {
            menu.set_selected_by_index(1, ctx);
            menu.handle_action(&MenuAction::OpenSubmenu, ctx);

            assert_eq!(menu.selected_index(), Some(1));
            assert!(menu.menu.selected_submenu().is_none());
            assert_eq!(
                menu.selected_item().unwrap().fields().unwrap().label(),
                "root"
            );
        });
    })
}
