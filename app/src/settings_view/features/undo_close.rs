use std::cell::RefCell;
use std::collections::HashMap;

use settings::{Setting, ToggleableSetting};
use warpui::elements::{Flex, MouseStateHandle, ParentElement};
use warpui::ui_components::components::UiComponent;
use warpui::ui_components::switch::SwitchStateHandle;
use warpui::{AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext};

use crate::appearance::Appearance;
use crate::report_if_error;
use crate::settings_view::settings_page::{render_body_item, LocalOnlyIconState, ToggleState};
use crate::undo_close::settings::UndoCloseEnabled;
use crate::undo_close::{UndoCloseSettings, MAX_RETAINED_CLOSED_ITEMS};

#[derive(Debug, Clone, Copy)]
pub enum Action {
    ToggleUndoCloseEnabled,
}

/// A view containing settings relating to the undo close feature.
pub struct UndoCloseView {
    /// State for the enable/disable toggle switch.
    switch_state: SwitchStateHandle,
    /// State for the local only icon tooltip.
    local_only_icon_states: RefCell<HashMap<String, MouseStateHandle>>,
}

impl UndoCloseView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        ctx.subscribe_to_model(&UndoCloseSettings::handle(ctx), |_, _, _, ctx| {
            ctx.notify();
        });

        Self {
            switch_state: Default::default(),
            local_only_icon_states: Default::default(),
        }
    }
}

impl Entity for UndoCloseView {
    type Event = ();
}

impl View for UndoCloseView {
    fn ui_name() -> &'static str {
        "UndoCloseView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let ui_builder = appearance.ui_builder();

        let settings = UndoCloseSettings::as_ref(app);
        let enabled = *settings.enabled;

        Flex::column()
            .with_child(render_body_item::<Action>(
                "Enable reopening of closed sessions".into(),
                None,
                LocalOnlyIconState::for_setting(
                    UndoCloseEnabled::storage_key(),
                    UndoCloseEnabled::sync_to_cloud(),
                    &mut self.local_only_icon_states.borrow_mut(),
                    app,
                ),
                ToggleState::Enabled,
                appearance,
                ui_builder
                    .switch(self.switch_state.clone())
                    .check(enabled)
                    .build()
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(Action::ToggleUndoCloseEnabled);
                    })
                    .finish(),
                Some(
                    format!(
                        "Keep up to {MAX_RETAINED_CLOSED_ITEMS} recently closed sessions for the lifetime of the app, with no time limit."
                    ),
                ),
            ))
            .finish()
    }
}

impl TypedActionView for UndoCloseView {
    type Action = Action;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut warpui::ViewContext<Self>) {
        let Action::ToggleUndoCloseEnabled = action;
        UndoCloseSettings::handle(ctx).update(ctx, |settings, ctx| {
            report_if_error!(settings.enabled.toggle_and_save_value(ctx));
        })
    }
}
