use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use pathfinder_geometry::vector::vec2f;

use super::*;
use crate::elements::{ConstrainedBox, DispatchEventResult, EventHandler, Rect, ZIndex};
use crate::platform::WindowStyle;
use crate::{
    App, AppContext, Entity, Event, Presenter, TypedActionView, ViewContext, WindowInvalidation,
};

#[derive(Clone, Copy, Hash, PartialEq, Eq, Debug)]
enum ElementIdentifier {
    BottomContainer,
}

#[derive(Default)]
struct View {
    // Maps identifier to number of mouse down events
    mouse_downs: HashMap<ElementIdentifier, usize>,
}

fn init(app: &mut AppContext) {
    app.add_action("container_test:mouse_down", View::mouse_down);
}

impl View {
    fn mouse_down(&mut self, identifier: &ElementIdentifier, _: &mut ViewContext<Self>) -> bool {
        log::info!("Recording mouse_down on element {identifier:?}");
        let entry = self.mouse_downs.entry(*identifier).or_insert(0);
        *entry += 1;
        true
    }
}

impl Entity for View {
    type Event = ();
}

impl crate::core::View for View {
    fn ui_name() -> &'static str {
        "container_test_view"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        Container::new(
            EventHandler::new(
                ConstrainedBox::new(Rect::new().finish())
                    .with_height(100.)
                    .with_width(100.)
                    .finish(),
            )
            .on_left_mouse_down(|evt, _, _| {
                evt.dispatch_action(
                    "container_test:mouse_down",
                    ElementIdentifier::BottomContainer,
                );
                DispatchEventResult::StopPropagation
            })
            .finish(),
        )
        .with_foreground_overlay(Fill::Solid(ColorU::white()))
        .finish()
    }
}

impl TypedActionView for View {
    type Action = ();
}

#[test]
fn test_container_element_overlay_click_handling() {
    App::test((), |mut app| async move {
        let app = &mut app;
        app.update(init);
        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| View::default());

        let mut presenter = Presenter::new(window_id);

        let mut updated = HashSet::new();
        updated.insert(app.root_view_id(window_id).unwrap());
        let invalidation = WindowInvalidation {
            updated,
            ..Default::default()
        };

        app.update(move |ctx| {
            presenter.invalidate(invalidation, ctx);
            let scene = presenter.build_scene(vec2f(100., 100.), 1., None, ctx);
            assert_eq!(scene.z_index(), ZIndex::new(0));
            assert_eq!(scene.layer_count(), 2);
            let presenter = Rc::new(RefCell::new(presenter));

            ctx.simulate_window_event(
                Event::LeftMouseDown {
                    position: vec2f(50., 50.),
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                window_id,
                presenter,
            );
        });

        view.read(app, |view, _| {
            assert_eq!(
                1,
                *view
                    .mouse_downs
                    .get(&ElementIdentifier::BottomContainer)
                    .unwrap()
            );
        });
    });
}

#[test]
fn snapping_gives_both_edges_of_a_frame_the_same_pixel_alignment() {
    // A flex row dividing leftover space, or text measured to a fraction of a
    // point, leaves a container's right edge mid-pixel while its left edge sits
    // on a boundary. Unsnapped, the border on that edge antialiases to partial
    // coverage and reads as a thinner line.
    let scale_factor = 2.;
    let rect = RectF::new(vec2f(10., 5.), vec2f(159.6, 28.4));
    let snapped = snap_to_device_pixels(rect, scale_factor);

    for edge in [
        snapped.min_x(),
        snapped.min_y(),
        snapped.max_x(),
        snapped.max_y(),
    ] {
        let device_pixels = edge * scale_factor;
        assert_eq!(
            device_pixels,
            device_pixels.round(),
            "edge {edge} does not land on a device pixel"
        );
    }
    // Snapping moves an edge by less than half a device pixel, so the frame
    // stays where the layout put it.
    assert!((snapped.max_x() - rect.max_x()).abs() <= 0.5 / scale_factor);
    assert!((snapped.max_y() - rect.max_y()).abs() <= 0.5 / scale_factor);
}

#[test]
fn snapping_keeps_a_sliver_thinner_than_a_device_pixel_visible() {
    let scale_factor = 2.;
    // A 0.2pt divider would otherwise round away to nothing and vanish.
    let sliver = RectF::new(vec2f(10., 5.), vec2f(0.2, 40.));
    let snapped = snap_to_device_pixels(sliver, scale_factor);

    assert!(
        snapped.width() > 0.,
        "a visible sliver must not snap away to zero width"
    );
    assert_eq!(snapped.width(), 1. / scale_factor);
}

#[test]
fn snapping_is_a_no_op_for_an_unknown_scale_factor() {
    let rect = RectF::new(vec2f(10.3, 5.7), vec2f(159.6, 28.4));
    assert_eq!(snap_to_device_pixels(rect, 0.), rect);
    assert_eq!(snap_to_device_pixels(rect, f32::NAN), rect);
}
