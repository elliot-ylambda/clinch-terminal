use std::cell::RefCell;
use std::collections::HashMap;

use ::settings::{Setting, ToggleableSetting};
use warpui::elements::{ChildView, ConstrainedBox, Element, Flex, MouseStateHandle, ParentElement};
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::UiComponent;
use warpui::ui_components::switch::SwitchStateHandle;
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle};

use super::settings_page::{
    render_body_item, Category, LocalOnlyIconState, MatchData, PageType, SettingsPageMeta,
    SettingsPageViewHandle, SettingsWidget,
};
use super::{SettingsSection, ToggleState};
use crate::appearance::Appearance;
#[cfg(target_os = "macos")]
use crate::imessage::IMessageCoordinator;
use crate::report_if_error;
use crate::settings::{
    AutoCreateWorktreesForNewTabs, CliAgentUsageSettings, ClinchSettings, ShowCliAgentPlanLimits,
};
use crate::terminal::session_settings::{NotificationsSettings, SessionSettings};
#[cfg(target_os = "macos")]
use crate::view_components::{SubmittableTextInput, SubmittableTextInputEvent};

#[derive(Clone, Debug, PartialEq)]
pub enum ClinchSettingsPageAction {
    #[cfg(target_os = "macos")]
    IMessageEnabled,
    #[cfg(target_os = "macos")]
    IMessageRefresh,
    #[cfg(target_os = "macos")]
    IMessageDisconnect,
    #[cfg(target_os = "macos")]
    OpenMessages,
    #[cfg(target_os = "macos")]
    OpenAutomationSettings,
    #[cfg(target_os = "macos")]
    OpenFullDiskAccessSettings,
    SessionCapture,
    AgentStatusOnTabs,
    AutoCreateWorktreesForNewTabs,
    CliAgentPlanLimits,
}

pub struct ClinchSettingsPageView {
    page: PageType<Self>,
    local_only_icon_tooltip_states: RefCell<HashMap<String, MouseStateHandle>>,
    session_capture_enabled: bool,
    #[cfg(target_os = "macos")]
    imessage_recipient_editor: ViewHandle<SubmittableTextInput>,
}

impl ClinchSettingsPageView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        ctx.subscribe_to_model(&SessionSettings::handle(ctx), |_, _, _, ctx| ctx.notify());
        ctx.subscribe_to_model(&CliAgentUsageSettings::handle(ctx), |_, _, _, ctx| {
            ctx.notify()
        });
        ctx.subscribe_to_model(&ClinchSettings::handle(ctx), |_, _, _, ctx| ctx.notify());

        #[cfg(target_os = "macos")]
        ctx.subscribe_to_model(&IMessageCoordinator::handle(ctx), |_, _, _, ctx| {
            ctx.notify()
        });

        #[cfg(target_os = "macos")]
        let imessage_recipient_editor = ctx.add_typed_action_view(|ctx| {
            let mut input = SubmittableTextInput::new(ctx).validate_on_submit(|recipient| {
                let digits = recipient
                    .chars()
                    .filter(|character| character.is_ascii_digit())
                    .count();
                (7..=15).contains(&digits)
                    && recipient.chars().count() <= 32
                    && recipient.chars().all(|character| {
                        character.is_ascii_digit()
                            || matches!(character, '+' | '-' | '(' | ')' | ' ' | '.')
                    })
            });
            input.set_placeholder_text("iPhone number, for example +1 415 555 1212", ctx);
            let existing = ClinchSettings::as_ref(ctx).imessage().recipient.clone();
            if !existing.is_empty() {
                input.editor().update(ctx, |editor, ctx| {
                    editor.set_buffer_text(&existing, ctx);
                });
            }
            input
        });
        #[cfg(target_os = "macos")]
        ctx.subscribe_to_view(&imessage_recipient_editor, |_, _, event, ctx| match event {
            SubmittableTextInputEvent::Submit(recipient) => {
                IMessageCoordinator::handle(ctx).update(ctx, |coordinator, ctx| {
                    coordinator.begin_setup(recipient.clone(), ctx);
                });
            }
            SubmittableTextInputEvent::Escape => {}
        });

        let agent_widgets: Vec<Box<dyn SettingsWidget<View = Self>>> = vec![
            Box::new(SessionCaptureWidget::default()),
            Box::new(AgentStatusBadgesWidget::default()),
        ];
        let mut project_widgets: Vec<Box<dyn SettingsWidget<View = Self>>> = vec![];
        if ClinchSettings::as_ref(ctx)
            .auto_create_worktrees_for_new_tabs
            .is_supported_on_current_platform()
        {
            project_widgets.push(Box::new(AutoCreateWorktreesWidget::default()));
        }
        let mut usage_widgets: Vec<Box<dyn SettingsWidget<View = Self>>> = vec![];
        if CliAgentUsageSettings::as_ref(ctx)
            .show_plan_limits
            .is_supported_on_current_platform()
        {
            usage_widgets.push(Box::new(CliAgentPlanLimitsWidget::default()));
        }

        let mut categories = vec![];
        #[cfg(target_os = "macos")]
        categories.push(Category::new(
            "iMessage",
            vec![Box::new(IMessageWidget::default())],
        ));
        if !project_widgets.is_empty() {
            categories.push(Category::new("Projects", project_widgets));
        }
        categories.push(Category::new("Agents", agent_widgets));
        if !usage_widgets.is_empty() {
            categories.push(Category::new("Usage", usage_widgets));
        }

        Self {
            page: PageType::new_categorized(categories, Some("Clinch Settings")),
            local_only_icon_tooltip_states: RefCell::new(HashMap::new()),
            #[cfg(target_os = "macos")]
            session_capture_enabled: crate::agent_resume::capture_layer_enabled(),
            #[cfg(not(target_os = "macos"))]
            session_capture_enabled: false,
            #[cfg(target_os = "macos")]
            imessage_recipient_editor,
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
            #[cfg(target_os = "macos")]
            ClinchSettingsPageAction::IMessageEnabled => {
                let enabled = IMessageCoordinator::as_ref(ctx).configuration().enabled;
                IMessageCoordinator::handle(ctx).update(ctx, |coordinator, ctx| {
                    coordinator.set_enabled(!enabled, ctx);
                });
            }
            #[cfg(target_os = "macos")]
            ClinchSettingsPageAction::IMessageRefresh => {
                IMessageCoordinator::handle(ctx).update(ctx, |coordinator, ctx| {
                    coordinator.refresh_health(ctx);
                });
            }
            #[cfg(target_os = "macos")]
            ClinchSettingsPageAction::IMessageDisconnect => {
                IMessageCoordinator::handle(ctx).update(ctx, |coordinator, ctx| {
                    coordinator.disconnect(ctx);
                });
                self.imessage_recipient_editor.update(ctx, |input, ctx| {
                    input
                        .editor()
                        .update(ctx, |editor, ctx| editor.clear_buffer(ctx));
                });
            }
            #[cfg(target_os = "macos")]
            ClinchSettingsPageAction::OpenMessages => {
                ctx.open_url("imessage:");
            }
            #[cfg(target_os = "macos")]
            ClinchSettingsPageAction::OpenAutomationSettings => {
                ctx.open_url(
                    "x-apple.systempreferences:com.apple.preference.security?Privacy_Automation",
                );
            }
            #[cfg(target_os = "macos")]
            ClinchSettingsPageAction::OpenFullDiskAccessSettings => {
                ctx.open_url(
                    "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles",
                );
            }
            ClinchSettingsPageAction::SessionCapture => {
                #[cfg(target_os = "macos")]
                {
                    let next = !self.session_capture_enabled;
                    let result = crate::agent_resume::set_capture_layer_enabled(next);
                    self.session_capture_enabled = crate::agent_resume::capture_layer_enabled();
                    let window_id = ctx.window_id();
                    crate::ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                        let toast = match result {
                            Ok(()) if next => crate::view_components::DismissibleToast::success(
                                "Session capture enabled".to_owned(),
                            ),
                            Ok(()) => crate::view_components::DismissibleToast::success(
                                "Session capture integration removed".to_owned(),
                            ),
                            Err(error) => {
                                log::error!("could not change Clinch session capture: {error}");
                                crate::view_components::DismissibleToast::error(format!(
                                    "Could not change session capture: {error}"
                                ))
                            }
                        };
                        toast_stack.add_ephemeral_toast(toast, window_id, ctx);
                    });
                    ctx.notify();
                }
            }
            ClinchSettingsPageAction::AgentStatusOnTabs => {
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
            ClinchSettingsPageAction::AutoCreateWorktreesForNewTabs => {
                ClinchSettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings
                        .auto_create_worktrees_for_new_tabs
                        .toggle_and_save_value(ctx));
                });
                ctx.notify();
            }
            ClinchSettingsPageAction::CliAgentPlanLimits => {
                CliAgentUsageSettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.show_plan_limits.toggle_and_save_value(ctx));
                });
                ctx.notify();
            }
        }
    }
}

#[derive(Default)]
struct SessionCaptureWidget {
    button_state: MouseStateHandle,
}

impl SettingsWidget for SessionCaptureWidget {
    type View = ClinchSettingsPageView;

    fn search_terms(&self) -> &str {
        "clinch claude codex session capture hooks integration local conversations enable remove"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        _app: &AppContext,
    ) -> Box<dyn Element> {
        let label = if view.session_capture_enabled {
            "Remove integration"
        } else {
            "Enable session capture"
        };
        let button = appearance
            .ui_builder()
            .button(ButtonVariant::Secondary, self.button_state.clone())
            .with_text_label(label.to_owned())
            .build()
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(ClinchSettingsPageAction::SessionCapture);
            })
            .finish();

        render_body_item::<ClinchSettingsPageAction>(
            "Claude Code and Codex session capture".into(),
            None,
            LocalOnlyIconState::Hidden,
            ToggleState::Enabled,
            appearance,
            button,
            Some(
                "Optional. Enabling adds clearly marked hooks to ~/.claude/settings.json and \
                 ~/.codex/config.toml, installs helpers in ~/.warp/agent-resume-bin/, stores local \
                 session metadata in ~/.warp/agent-resume/, and records consent under Clinch's \
                 Application Support directory. Removing it keeps captured metadata unless you \
                 purge it separately. No notification plugin is installed."
                    .into(),
            ),
        )
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

#[cfg(target_os = "macos")]
#[derive(Default)]
struct IMessageWidget {
    enabled_switch_state: SwitchStateHandle,
    refresh_button_state: MouseStateHandle,
    messages_button_state: MouseStateHandle,
    automation_button_state: MouseStateHandle,
    full_disk_access_button_state: MouseStateHandle,
    disconnect_button_state: MouseStateHandle,
}

#[cfg(target_os = "macos")]
impl SettingsWidget for IMessageWidget {
    type View = ClinchSettingsPageView;

    fn search_terms(&self) -> &str {
        "clinch imessage iphone phone text message me notifications replies automation full disk access"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let coordinator = IMessageCoordinator::as_ref(app);
        let configuration = coordinator.configuration();
        let status = coordinator.status();

        let refresh_button = appearance
            .ui_builder()
            .button(ButtonVariant::Secondary, self.refresh_button_state.clone())
            .with_text_label("Refresh".to_owned())
            .build()
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(ClinchSettingsPageAction::IMessageRefresh);
            })
            .finish();

        let setup = render_body_item::<ClinchSettingsPageAction>(
            status.label().to_owned(),
            None,
            LocalOnlyIconState::Hidden,
            ToggleState::Enabled,
            appearance,
            refresh_button,
            Some(
                "Uses the Messages account on this Mac to text your iPhone. No iPhone app or \
                 Clinch server is involved. Clinch cannot distinguish an iPhone reply from a \
                 reply sent through another device signed in to the same Apple Account."
                    .into(),
            ),
        );

        let recipient = render_body_item::<ClinchSettingsPageAction>(
            if configuration.setup_complete {
                "Connected iPhone number"
            } else {
                "Set up your iPhone number"
            }
            .into(),
            None,
            LocalOnlyIconState::Hidden,
            ToggleState::Enabled,
            appearance,
            ConstrainedBox::new(ChildView::new(&view.imessage_recipient_editor).finish())
                .with_width(320.)
                .finish(),
            Some(if configuration.setup_complete {
                "Edit the number and press Return or the arrow to start setup over. Clinch sends \
                 a new calibration iMessage and waits for its one-time reply. Starting over \
                 clears retained and queued phone replies."
                    .into()
            } else {
                "Press Return or the arrow to send a calibration iMessage, then reply with the \
                 one-time code from your iPhone. Standard Messages delivery may apply."
                    .into()
            }),
        );

        let enabled_switch = {
            let switch = appearance
                .ui_builder()
                .switch(self.enabled_switch_state.clone())
                .check(configuration.enabled);
            if configuration.setup_complete {
                switch
                    .build()
                    .on_click(|ctx, _, _| {
                        ctx.dispatch_typed_action(ClinchSettingsPageAction::IMessageEnabled);
                    })
                    .finish()
            } else {
                switch.disable().build().finish()
            }
        };
        let enabled = render_body_item::<ClinchSettingsPageAction>(
            "Message me when agents finish".into(),
            None,
            LocalOnlyIconState::Hidden,
            if configuration.setup_complete {
                ToggleState::Enabled
            } else {
                ToggleState::Disabled
            },
            appearance,
            enabled_switch,
            Some(
                "Each successful turn sends its full final response as plain-text parts. This is \
                 enabled by default for current and future durable Codex and Claude Code \
                 sessions, and each session can be opted out from its footer. Replies queue \
                 while an agent is working and never answer permission prompts remotely."
                    .into(),
            ),
        );

        let messages_button = appearance
            .ui_builder()
            .button(ButtonVariant::Secondary, self.messages_button_state.clone())
            .with_text_label("Open Messages".to_owned())
            .build()
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(ClinchSettingsPageAction::OpenMessages);
            })
            .finish();
        let automation_button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Secondary,
                self.automation_button_state.clone(),
            )
            .with_text_label("Messages automation".to_owned())
            .build()
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(ClinchSettingsPageAction::OpenAutomationSettings);
            })
            .finish();
        let full_disk_access_button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Secondary,
                self.full_disk_access_button_state.clone(),
            )
            .with_text_label("Full Disk Access".to_owned())
            .build()
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(ClinchSettingsPageAction::OpenFullDiskAccessSettings);
            })
            .finish();
        let permissions = render_body_item::<ClinchSettingsPageAction>(
            "macOS permissions".into(),
            None,
            LocalOnlyIconState::Hidden,
            ToggleState::Enabled,
            appearance,
            Flex::row()
                .with_spacing(8.)
                .with_child(messages_button)
                .with_child(automation_button)
                .with_child(full_disk_access_button)
                .finish(),
            Some(
                "Automation lets Clinch ask Messages to send. Full Disk Access lets the bundled \
                 local helper watch the Messages database for replies. After granting access, \
                 quit and reopen Clinch, then click Refresh."
                    .into(),
            ),
        );

        let disconnect_button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Secondary,
                self.disconnect_button_state.clone(),
            )
            .with_text_label("Disconnect and clear routing data".to_owned())
            .build()
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(ClinchSettingsPageAction::IMessageDisconnect);
            })
            .finish();
        let disconnect = render_body_item::<ClinchSettingsPageAction>(
            "Disconnect iMessage".into(),
            None,
            LocalOnlyIconState::Hidden,
            if configuration.setup_complete || !configuration.recipient.is_empty() {
                ToggleState::Enabled
            } else {
                ToggleState::Disabled
            },
            appearance,
            disconnect_button,
            Some(
                "Stops the local bridge and deletes Clinch's phone configuration, route codes, \
                 queued replies, and processed-message IDs. It does not delete Messages history."
                    .into(),
            ),
        );

        let mut column = Flex::column()
            .with_child(setup)
            .with_child(recipient)
            .with_child(enabled)
            .with_child(permissions);
        column.add_child(disconnect);
        column.finish()
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
                    ctx.dispatch_typed_action(ClinchSettingsPageAction::AgentStatusOnTabs);
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
struct AutoCreateWorktreesWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for AutoCreateWorktreesWidget {
    type View = ClinchSettingsPageView;

    fn search_terms(&self) -> &str {
        "clinch projects git worktree worktrees new tabs main isolated branches automatic"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let enabled = *ClinchSettings::as_ref(app).auto_create_worktrees_for_new_tabs;
        render_body_item::<ClinchSettingsPageAction>(
            "Create new tabs in Git worktrees".into(),
            None,
            LocalOnlyIconState::for_setting(
                AutoCreateWorktreesForNewTabs::storage_key(),
                AutoCreateWorktreesForNewTabs::sync_to_cloud(),
                &mut view.local_only_icon_tooltip_states.borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .switch(self.switch_state.clone())
                .check(enabled)
                .build()
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(
                        ClinchSettingsPageAction::AutoCreateWorktreesForNewTabs,
                    );
                })
                .finish(),
            Some(
                "When the active project is a local Git repository with a main branch, create \
                 ordinary new terminal and Agent tabs in isolated linked worktrees."
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
                    ctx.dispatch_typed_action(ClinchSettingsPageAction::CliAgentPlanLimits);
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
