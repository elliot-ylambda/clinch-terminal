use std::cell::RefCell;
use std::collections::HashMap;

use ::settings::{Setting, ToggleableSetting};
use warpui::elements::{Element, MouseStateHandle};
use warpui::ui_components::components::UiComponent;
use warpui::ui_components::switch::SwitchStateHandle;
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle};

use super::settings_page::{
    render_body_item, Category, LocalOnlyIconState, MatchData, PageType, SettingsPageMeta,
    SettingsPageViewHandle, SettingsWidget,
};
use super::{SettingsSection, ToggleState};
use crate::appearance::Appearance;
use crate::report_if_error;
use crate::settings::{CliAgentUsageSettings, ShowCliAgentPlanLimits};
use crate::terminal::session_settings::{NotificationsSettings, SessionSettings};

#[derive(Clone, Debug, PartialEq)]
pub enum ClinchSettingsPageAction {
    ToggleAgentStatusOnTabs,
    ToggleCliAgentPlanLimits,
}

pub struct ClinchSettingsPageView {
    page: PageType<Self>,
    local_only_icon_tooltip_states: RefCell<HashMap<String, MouseStateHandle>>,
}

impl ClinchSettingsPageView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        ctx.subscribe_to_model(&SessionSettings::handle(ctx), |_, _, _, ctx| ctx.notify());
        ctx.subscribe_to_model(&CliAgentUsageSettings::handle(ctx), |_, _, _, ctx| {
            ctx.notify()
        });

        let agent_widgets: Vec<Box<dyn SettingsWidget<View = Self>>> =
            vec![Box::new(AgentStatusBadgesWidget::default())];
        let mut usage_widgets: Vec<Box<dyn SettingsWidget<View = Self>>> = vec![];
        if CliAgentUsageSettings::as_ref(ctx)
            .show_plan_limits
            .is_supported_on_current_platform()
        {
            usage_widgets.push(Box::new(CliAgentPlanLimitsWidget::default()));
        }

        let mut categories = vec![Category::new("Agents", agent_widgets)];
        if !usage_widgets.is_empty() {
            categories.push(Category::new("Usage", usage_widgets));
        }

        Self {
            page: PageType::new_categorized(categories, Some("Clinch Settings")),
            local_only_icon_tooltip_states: RefCell::new(HashMap::new()),
        }
    }
}

impl Entity for ClinchSettingsPageView {
    type Event = ();
}

impl TypedActionView for ClinchSettingsPageView {
    type Action = ClinchSettingsPageAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ClinchSettingsPageAction::ToggleAgentStatusOnTabs => {
                let current = SessionSettings::as_ref(ctx).notifications.value().clone();
                let next = NotificationsSettings {
                    show_agent_status_on_tabs: !current.show_agent_status_on_tabs,
                    ..current
                };
                SessionSettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.notifications.set_value(next, ctx));
                });
                ctx.notify();
            }
            ClinchSettingsPageAction::ToggleCliAgentPlanLimits => {
                CliAgentUsageSettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.show_plan_limits.toggle_and_save_value(ctx));
                });
                ctx.notify();
            }
        }
    }
}

impl View for ClinchSettingsPageView {
    fn ui_name() -> &'static str {
        "ClinchSettingsPage"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        self.page.render(self, app)
    }
}

impl SettingsPageMeta for ClinchSettingsPageView {
    fn section() -> SettingsSection {
        SettingsSection::Clinch
    }

    fn should_render(&self, _ctx: &AppContext) -> bool {
        true
    }

    fn update_filter(&mut self, query: &str, ctx: &mut ViewContext<Self>) -> MatchData {
        self.page.update_filter(query, ctx)
    }

    fn scroll_to_widget(&mut self, widget_id: &'static str) {
        self.page.scroll_to_widget(widget_id)
    }

    fn clear_highlighted_widget(&mut self) {
        self.page.clear_highlighted_widget();
    }
}

impl From<ViewHandle<ClinchSettingsPageView>> for SettingsPageViewHandle {
    fn from(view_handle: ViewHandle<ClinchSettingsPageView>) -> Self {
        SettingsPageViewHandle::Clinch(view_handle)
    }
}

#[derive(Default)]
struct AgentStatusBadgesWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for AgentStatusBadgesWidget {
    type View = ClinchSettingsPageView;

    fn search_terms(&self) -> &str {
        "clinch claude codex cli agent status badges tabs attention"
    }

    fn render(
        &self,
        _view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let enabled = SessionSettings::as_ref(app)
            .notifications
            .show_agent_status_on_tabs;
        render_body_item::<ClinchSettingsPageAction>(
            "Show agent status badges on tabs".into(),
            None,
            LocalOnlyIconState::Hidden,
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .switch(self.switch_state.clone())
                .check(enabled)
                .build()
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(ClinchSettingsPageAction::ToggleAgentStatusOnTabs);
                })
                .finish(),
            Some(
                "Show Claude Code and Codex status on terminal tabs when an agent is working, \
                 finished, or needs attention."
                    .into(),
            ),
        )
    }
}

#[derive(Default)]
struct CliAgentPlanLimitsWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for CliAgentPlanLimitsWidget {
    type View = ClinchSettingsPageView;

    fn search_terms(&self) -> &str {
        "clinch claude code live plan limits usage keychain anthropic rate limit gauges"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let show_plan_limits = *CliAgentUsageSettings::as_ref(app).show_plan_limits;
        render_body_item::<ClinchSettingsPageAction>(
            "Show Claude Code live plan limits".into(),
            None,
            LocalOnlyIconState::for_setting(
                ShowCliAgentPlanLimits::storage_key(),
                ShowCliAgentPlanLimits::sync_to_cloud(),
                &mut view.local_only_icon_tooltip_states.borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .switch(self.switch_state.clone())
                .check(show_plan_limits)
                .build()
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(ClinchSettingsPageAction::ToggleCliAgentPlanLimits);
                })
                .finish(),
            Some(
                "Show live rate-limit gauges in the tab-bar usage widget. When enabled, Clinch \
                 reads Claude Code's OAuth token from the macOS Keychain and queries Anthropic's \
                 usage endpoint. This is off by default."
                    .into(),
            ),
        )
    }
}

#[cfg(test)]
#[path = "clinch_page_tests.rs"]
mod tests;
