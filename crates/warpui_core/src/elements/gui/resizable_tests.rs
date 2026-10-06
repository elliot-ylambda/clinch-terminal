use super::*;
use crate::elements::{ConstrainedBox, Empty};
use crate::platform::WindowStyle;
use crate::{App, Entity, TypedActionView, View};

struct TestView {
    resize_state: ResizableStateHandle,
    parent_height: f32,
    parent_bounds: bool,
}

impl Entity for TestView {
    type Event = ();
}

impl TypedActionView for TestView {
    type Action = ();
}

impl View for TestView {
    fn ui_name() -> &'static str {
        "Resizable::tests::TestView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        let callback: BoundsCallback = Box::new(|size| (50_f32.min(size.y()), size.y()));
        let resizable = Resizable::new(self.resize_state.clone(), Empty::new().finish())
            .with_dragbar_side(DragBarSide::Top);
        let resizable = if self.parent_bounds {
            resizable.with_parent_bounds_callback(callback)
        } else {
            resizable.with_bounds_callback(callback)
        };
        ConstrainedBox::new(resizable.finish())
            .with_height(self.parent_height)
            .finish()
    }
}

#[test]
fn parent_bounds_clamp_saved_height_and_allow_immediate_drag_back() {
    App::test((), |mut app| async move {
        let state = resizable_state_handle(300.);
        let (_, view) = app.add_window(WindowStyle::NotStealFocus, |_| TestView {
            resize_state: state.clone(),
            parent_height: 200.,
            parent_bounds: true,
        });
        view.update(&mut app, |_, ctx| ctx.notify());
        assert_eq!(state.lock().unwrap().size(), 200.);

        // Reducing the available sidebar space must also clamp the remembered drag size.
        view.update(&mut app, |view, ctx| {
            view.parent_height = 100.;
            ctx.notify();
        });
        let mut state = state.lock().unwrap();
        assert_eq!(state.size(), 100.);
        state.begin_resizing(vec2f(0., 200.));
        state.check_for_resize(vec2f(0., 220.), Some(vec2f(0., 200.)), DragBarSide::Top);
        assert_eq!(state.size(), 80.);
    });
}

#[test]
fn window_bounds_remain_independent_of_parent_height() {
    App::test((), |mut app| async move {
        let state = resizable_state_handle(75.);
        let (_, view) = app.add_window(WindowStyle::NotStealFocus, |_| TestView {
            resize_state: state.clone(),
            parent_height: 100.,
            parent_bounds: false,
        });
        view.update(&mut app, |_, ctx| ctx.notify());
        assert_eq!(state.lock().unwrap().size(), 75.);

        view.update(&mut app, |view, ctx| {
            view.parent_height = 50.;
            ctx.notify();
        });
        assert_eq!(state.lock().unwrap().size(), 75.);
    });
}
