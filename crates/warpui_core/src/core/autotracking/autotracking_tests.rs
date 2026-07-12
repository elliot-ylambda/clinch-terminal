use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use super::*;
use crate::elements::*;
use crate::platform::WindowStyle;
use crate::{App, AppContext, Element, Entity, ModelHandle, TypedActionView, View, ViewHandle};

#[derive(Default)]
struct Model {
    first: Tracked<usize>,
    second: Tracked<bool>,
    third: Tracked<isize>,
}

impl Entity for Model {
    type Event = ();
}

struct TestView {
    model: ModelHandle<Model>,
    field: Tracked<usize>,
    other_field: Tracked<bool>,
    counter: Arc<AtomicUsize>,
}

impl TestView {
    fn new(model: ModelHandle<Model>, counter: Arc<AtomicUsize>) -> Self {
        TestView {
            model,
            field: Tracked::new(0),
            other_field: Tracked::new(false),
            counter,
        }
    }
}

impl Entity for TestView {
    type Event = ();
}

impl View for TestView {
    fn ui_name() -> &'static str {
        "TestView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        // While rendering, we explicitly read / depend on the fields `first` and `second` of
        // model, as well as `field` of the View itself.

        // We explicitly do _not_ depend on `Model::third` nor `other_field` on the View.
        let model = self.model.as_ref(app);
        let _first = *model.first;
        let _second = *model.second;
        let _field = *self.field;

        // Increment the render counter so that we can track how often a View is rendered
        self.counter.fetch_add(1, Ordering::Relaxed);

        Empty::new().finish()
    }
}

impl TypedActionView for TestView {
    type Action = ();
}

#[test]
fn test_update_view_dependency_rerenders() {
    App::test((), |mut app| async move {
        let model_handle = app.add_model(|_| Model::default());

        let render_counter = Arc::new(AtomicUsize::new(0));
        let (_, view_handle) = app.add_window(WindowStyle::NotStealFocus, |_| {
            TestView::new(model_handle.clone(), render_counter.clone())
        });

        // Force the window to be rendered the first time
        view_handle.update(&mut app, |_, _| {});
        assert_eq!(render_counter.load(Ordering::Relaxed), 1);

        // Update an internal View dependency and confirm that it causes a rerender of the View
        view_handle.update(&mut app, |view, _| {
            *view.field += 1;
        });
        assert_eq!(render_counter.load(Ordering::Relaxed), 2);
    });
}

#[test]
fn test_update_view_non_dependency_no_rerender() {
    App::test((), |mut app| async move {
        let model_handle = app.add_model(|_| Model::default());

        let render_counter = Arc::new(AtomicUsize::new(0));
        let (_, view_handle) = app.add_window(WindowStyle::NotStealFocus, |_| {
            TestView::new(model_handle.clone(), render_counter.clone())
        });

        // Force the window to be rendered the first time
        view_handle.update(&mut app, |_, _| {});
        assert_eq!(render_counter.load(Ordering::Relaxed), 1);

        // Update an internal View field that is not a dependency and confirm that it does not
        // cause a rerender of the View
        view_handle.update(&mut app, |view, _| {
            *view.other_field = true;
        });
        assert_eq!(render_counter.load(Ordering::Relaxed), 1);
    });
}

#[test]
fn test_update_model_dependency_rerenders() {
    App::test((), |mut app| async move {
        let model_handle = app.add_model(|_| Model::default());

        let render_counter = Arc::new(AtomicUsize::new(0));
        let (_, view_handle) = app.add_window(WindowStyle::NotStealFocus, |_| {
            TestView::new(model_handle.clone(), render_counter.clone())
        });

        // Force the window to be rendered the first time
        view_handle.update(&mut app, |_, _| {});
        assert_eq!(render_counter.load(Ordering::Relaxed), 1);

        // Update a Model dependency and confirm that it causes a rerender of the View
        model_handle.update(&mut app, |model, _| {
            *model.first += 1;
        });
        assert_eq!(render_counter.load(Ordering::Relaxed), 2);

        // Update another Model dependency and confirm that it causes a rerender
        model_handle.update(&mut app, |model, _| {
            *model.second = true;
        });
        assert_eq!(render_counter.load(Ordering::Relaxed), 3);
    });
}

#[test]
fn test_update_model_non_dependency_no_rerender() {
    App::test((), |mut app| async move {
        let model_handle = app.add_model(|_| Model::default());

        let render_counter = Arc::new(AtomicUsize::new(0));
        let (_, view_handle) = app.add_window(WindowStyle::NotStealFocus, |_| {
            TestView::new(model_handle.clone(), render_counter.clone())
        });

        // Force the window to be rendered the first time
        view_handle.update(&mut app, |_, _| {});
        assert_eq!(render_counter.load(Ordering::Relaxed), 1);

        // Update a Model field that is not a dependency and confirm that it does not
        // cause a rerender of the View
        model_handle.update(&mut app, |model, _| {
            *model.third -= 1000;
        });
        assert_eq!(render_counter.load(Ordering::Relaxed), 1);
    });
}

#[test]
fn test_updates_multiple_sources() {
    App::test((), |mut app| async move {
        let model_handle = app.add_model(|_| Model::default());

        let render_counter = Arc::new(AtomicUsize::new(0));
        let (_, view_handle) = app.add_window(WindowStyle::NotStealFocus, |_| {
            TestView::new(model_handle.clone(), render_counter.clone())
        });

        // Force the window to be rendered the first time
        view_handle.update(&mut app, |_, _| {});
        assert_eq!(render_counter.load(Ordering::Relaxed), 1);

        // Update several Model dependencies and confirm it causes a single rerender
        model_handle.update(&mut app, |model, _| {
            *model.first += 1;
            *model.second = true;
        });
        assert_eq!(render_counter.load(Ordering::Relaxed), 2);

        // Update a View dependency and confirm it causes another rerender
        view_handle.update(&mut app, |view, _| {
            *view.field += 100;
        });
        assert_eq!(render_counter.load(Ordering::Relaxed), 3);

        // Update a field that is not a dependency and confirm that it doesn't cause a rerender
        view_handle.update(&mut app, |view, _| {
            *view.other_field = true;
        });
        assert_eq!(render_counter.load(Ordering::Relaxed), 3);

        // Update a non-dependency on the Model and confirm that there is no rerender
        model_handle.update(&mut app, |model, _| {
            *model.third -= 100;
        });
        assert_eq!(render_counter.load(Ordering::Relaxed), 3);

        // Update the view _and_ the model in the same call and confirm that it causes a single
        // rerender
        view_handle.update(&mut app, |view, ctx| {
            *view.field += 20;
            view.model.update(ctx, |model, _| {
                *model.first += 23;
            });
        });
        assert_eq!(render_counter.load(Ordering::Relaxed), 4);
    });
}

#[test]
fn test_model_updates_multiple_views() {
    struct OtherView {
        model: ModelHandle<Model>,
        counter: Arc<AtomicUsize>,
    }

    impl OtherView {
        fn new(model: ModelHandle<Model>, counter: Arc<AtomicUsize>) -> Self {
            OtherView { model, counter }
        }
    }

    impl Entity for OtherView {
        type Event = ();
    }

    impl View for OtherView {
        fn ui_name() -> &'static str {
            "OtherView"
        }

        fn render(&self, app: &AppContext) -> Box<dyn Element> {
            let model = self.model.as_ref(app);
            // This view depends on `second` and `third` in the model, so updates to those should
            // cause it to rerender (overlapping `second` with the main test view)
            let _second = *model.second;
            let _third = *model.third;

            self.counter.fetch_add(1, Ordering::Relaxed);

            Empty::new().finish()
        }
    }

    /// Mounts both tracked views so their dependency-driven re-renders are
    /// observable: re-renders of views that are not on screen are deferred
    /// until they are mounted again.
    struct PairRoot {
        first: ViewHandle<TestView>,
        second: ViewHandle<OtherView>,
    }

    impl Entity for PairRoot {
        type Event = ();
    }

    impl View for PairRoot {
        fn ui_name() -> &'static str {
            "PairRoot"
        }

        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            Flex::row()
                .with_children([
                    ChildView::new(&self.first).finish(),
                    ChildView::new(&self.second).finish(),
                ])
                .finish()
        }
    }

    impl TypedActionView for PairRoot {
        type Action = ();
    }

    App::test((), |mut app| async move {
        let model_handle = app.add_model(|_| Model::default());

        let first_render_counter = Arc::new(AtomicUsize::new(0));
        let second_render_counter = Arc::new(AtomicUsize::new(0));
        let (_, root) = {
            let model_handle = model_handle.clone();
            let first_render_counter = first_render_counter.clone();
            let second_render_counter = second_render_counter.clone();
            app.add_window(WindowStyle::NotStealFocus, |ctx| {
                let first =
                    ctx.add_view(|_| TestView::new(model_handle.clone(), first_render_counter));
                let second =
                    ctx.add_view(|_| OtherView::new(model_handle.clone(), second_render_counter));
                PairRoot { first, second }
            })
        };

        // Force the window to be rendered the first time
        root.update(&mut app, |_, _| {});
        assert_eq!(first_render_counter.load(Ordering::Relaxed), 1);
        assert_eq!(second_render_counter.load(Ordering::Relaxed), 1);

        // Update a field on the model that _only_ the first view depends on and confirm that is
        // the only view rerendered
        model_handle.update(&mut app, |model, _| {
            *model.first += 100;
        });
        assert_eq!(first_render_counter.load(Ordering::Relaxed), 2);
        assert_eq!(second_render_counter.load(Ordering::Relaxed), 1);

        // Update a field on the model that _both_ views depend on and confirm both are rerendered
        model_handle.update(&mut app, |model, _| {
            *model.second = true;
        });
        assert_eq!(first_render_counter.load(Ordering::Relaxed), 3);
        assert_eq!(second_render_counter.load(Ordering::Relaxed), 2);

        // Update a field on the model that _only_ the second view depends on and confirm that is
        // the only view rerendered
        model_handle.update(&mut app, |model, _| {
            *model.third -= 55;
        });
        assert_eq!(first_render_counter.load(Ordering::Relaxed), 3);
        assert_eq!(second_render_counter.load(Ordering::Relaxed), 3);
    });
}

#[test]
fn test_tracked_updates_to_unmounted_view_stop_driving_frames() {
    struct ToggleRoot {
        first: ViewHandle<TestView>,
        second: ViewHandle<TrackedThirdReader>,
        show_second: bool,
    }

    impl Entity for ToggleRoot {
        type Event = ();
    }

    impl View for ToggleRoot {
        fn ui_name() -> &'static str {
            "ToggleRoot"
        }

        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            let mut children = vec![ChildView::new(&self.first).finish()];
            if self.show_second {
                children.push(ChildView::new(&self.second).finish());
            }
            Flex::row().with_children(children).finish()
        }
    }

    impl TypedActionView for ToggleRoot {
        type Action = ();
    }

    /// Reads only `Model::third`, so tracked writes to `third` affect no other view.
    struct TrackedThirdReader {
        model: ModelHandle<Model>,
        counter: Arc<AtomicUsize>,
    }

    impl Entity for TrackedThirdReader {
        type Event = ();
    }

    impl View for TrackedThirdReader {
        fn ui_name() -> &'static str {
            "TrackedThirdReader"
        }

        fn render(&self, app: &AppContext) -> Box<dyn Element> {
            let _third = *self.model.as_ref(app).third;
            self.counter.fetch_add(1, Ordering::Relaxed);
            Empty::new().finish()
        }
    }

    impl TypedActionView for TrackedThirdReader {
        type Action = ();
    }

    App::test((), |mut app| async move {
        let model_handle = app.add_model(|_| Model::default());
        let first_counter = Arc::new(AtomicUsize::new(0));
        let second_counter = Arc::new(AtomicUsize::new(0));

        let (window_id, root) = {
            let model_handle = model_handle.clone();
            let first_counter = first_counter.clone();
            let second_counter = second_counter.clone();
            app.add_window(WindowStyle::NotStealFocus, |ctx| {
                let first = ctx.add_view(|_| TestView::new(model_handle.clone(), first_counter));
                let second = ctx.add_view(|_| TrackedThirdReader {
                    model: model_handle.clone(),
                    counter: second_counter,
                });
                ToggleRoot {
                    first,
                    second,
                    show_second: true,
                }
            })
        };

        root.update(&mut app, |_, _| {});
        assert_eq!(second_counter.load(Ordering::Relaxed), 1);

        // Unmount the tracked reader.
        root.update(&mut app, |root, ctx| {
            root.show_second = false;
            ctx.notify();
        });

        // The first tracked write may still drive one frame: the reader's
        // dependency registrations are only cleared when its re-render is
        // deferred. After that, further writes must be inert — otherwise a
        // hidden view reading tracked state repaints an identical scene on
        // every change, indefinitely.
        model_handle.update(&mut app, |model, _| {
            *model.third += 1;
        });
        let frames_after_first_write = app.update(|ctx| {
            ctx.presenter(window_id)
                .expect("window should have a presenter")
                .borrow()
                .frame_count()
        });
        model_handle.update(&mut app, |model, _| {
            *model.third += 1;
        });
        let frames_after_second_write = app.update(|ctx| {
            ctx.presenter(window_id)
                .expect("window should have a presenter")
                .borrow()
                .frame_count()
        });
        assert_eq!(
            frames_after_second_write, frames_after_first_write,
            "tracked writes read only by an unmounted view must stop scheduling frames"
        );
        assert_eq!(
            second_counter.load(Ordering::Relaxed),
            1,
            "the unmounted reader must not have been re-rendered"
        );

        // Remounting re-renders the reader (registering fresh dependencies), so
        // tracked writes drive its re-render again.
        root.update(&mut app, |root, ctx| {
            root.show_second = true;
            ctx.notify();
        });
        assert_eq!(second_counter.load(Ordering::Relaxed), 2);
        model_handle.update(&mut app, |model, _| {
            *model.third += 1;
        });
        assert_eq!(
            second_counter.load(Ordering::Relaxed),
            3,
            "a remounted reader must re-register its tracked dependencies"
        );
    });
}
