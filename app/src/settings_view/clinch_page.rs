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
use crate::imessage::{
    IMessageConnectionStatus, IMessageCoordinator, IMessagePermission, IMessageSetupRequirements,
    IMessageTestStatus,
};
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
    IMessageNotificationsDefault,
    #[cfg(target_os = "macos")]
    IMessageTest,
    #[cfg(target_os = "macos")]
    IMessageRefresh,
    #[cfg(target_os = "macos")]
    IMessageRequestAutomation,
    #[cfg(target_os = "macos")]
    IMessageChangeNumber,
    #[cfg(target_os = "macos")]
    OpenMessages,
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
        if matches!(
            IMessageCoordinator::as_ref(ctx).status(),
            IMessageConnectionStatus::Paused(_) | IMessageConnectionStatus::Error
        ) {
            IMessageCoordinator::handle(ctx).update(ctx, |coordinator, ctx| {
                coordinator.refresh_health(ctx);
            });
        }

        #[cfg(target_os = "macos")]
        let imessage_recipient_editor = ctx.add_typed_action_view(|ctx| {
            let mut input = SubmittableTextInput::new(ctx)
                .validate_on_submit(|recipient| {
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
                })
                .with_submit_label("Connect");
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
                ctx.open_url(
                    "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles",
                );
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
            ClinchSettingsPageAction::IMessageNotificationsDefault => {
                let enabled = IMessageCoordinator::as_ref(ctx)
                    .configuration()
                    .notifications_enabled_by_default;
                IMessageCoordinator::handle(ctx).update(ctx, |coordinator, ctx| {
                    coordinator.set_notifications_enabled_by_default(!enabled, ctx);
                });
            }
            #[cfg(target_os = "macos")]
            ClinchSettingsPageAction::IMessageTest => {
                IMessageCoordinator::handle(ctx).update(ctx, |coordinator, ctx| {
                    coordinator.send_test_message(ctx);
                });
            }
            #[cfg(target_os = "macos")]
            ClinchSettingsPageAction::IMessageRefresh => {
                IMessageCoordinator::handle(ctx).update(ctx, |coordinator, ctx| {
                    coordinator.refresh_health(ctx);
                });
            }
            #[cfg(target_os = "macos")]
            ClinchSettingsPageAction::IMessageRequestAutomation => {
                IMessageCoordinator::handle(ctx).update(ctx, |coordinator, ctx| {
                    coordinator.request_automation_access(ctx);
                });
                ctx.open_url(
                    "x-apple.systempreferences:com.apple.preference.security?Privacy_Automation",
                );
            }
            #[cfg(target_os = "macos")]
            ClinchSettingsPageAction::IMessageChangeNumber => {
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
                                "Session capture disabled".to_owned(),
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
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for SessionCaptureWidget {
    type View = ClinchSettingsPageView;

    fn search_terms(&self) -> &str {
        "clinch claude codex session capture restore resume hooks integration local conversations \
         enable disable remove"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        _app: &AppContext,
    ) -> Box<dyn Element> {
        let switch = appearance
            .ui_builder()
            .switch(self.switch_state.clone())
            .check(view.session_capture_enabled)
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
            switch,
            Some(
                "Enabled by default so Clinch can reconnect restored panes to their Claude Code or \
                 Codex conversations and list recent conversations for reopening. It adds clearly \
                 marked hooks to ~/.claude/settings.json and ~/.codex/config.toml, installs helper \
                 executables in ~/.warp/agent-resume-bin/, stores local pane/session metadata and \
                 prompt mirrors in ~/.warp/agent-resume/, and records the setting plus a non-secret \
                 receipt in Clinch's Application Support directory. Turning it off removes the \
                 hooks and helpers, remembers the setting, and keeps captured metadata. \
                 Notification plugins are separate."
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IMessageSetupStage {
    EnterNumber,
    Connecting,
    ReadyToTest,
    SendingTest,
    FullDiskAccess,
    Automation,
    MessagesSignIn,
    AwaitingReply,
    ReplyMismatch,
    Connected,
    Error,
}

#[cfg(target_os = "macos")]
fn imessage_setup_stage(
    setup_complete: bool,
    recipient_is_empty: bool,
    status: &IMessageConnectionStatus,
) -> IMessageSetupStage {
    if recipient_is_empty {
        return IMessageSetupStage::EnterNumber;
    }
    match status {
        IMessageConnectionStatus::Paused(IMessagePermission::FullDiskAccess) => {
            IMessageSetupStage::FullDiskAccess
        }
        IMessageConnectionStatus::Paused(IMessagePermission::Automation) => {
            IMessageSetupStage::Automation
        }
        IMessageConnectionStatus::Paused(IMessagePermission::MessagesSignIn) => {
            IMessageSetupStage::MessagesSignIn
        }
        IMessageConnectionStatus::AwaitingCalibrationReply => IMessageSetupStage::AwaitingReply,
        IMessageConnectionStatus::CalibrationReplyMismatch => IMessageSetupStage::ReplyMismatch,
        IMessageConnectionStatus::ReadyToTest => IMessageSetupStage::ReadyToTest,
        IMessageConnectionStatus::SendingSetupMessage => IMessageSetupStage::SendingTest,
        IMessageConnectionStatus::Connected | IMessageConnectionStatus::Disabled
            if setup_complete =>
        {
            IMessageSetupStage::Connected
        }
        IMessageConnectionStatus::Error => IMessageSetupStage::Error,
        IMessageConnectionStatus::SetupRequired
        | IMessageConnectionStatus::Connecting
        | IMessageConnectionStatus::Connected
        | IMessageConnectionStatus::Disabled => IMessageSetupStage::Connecting,
    }
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct IMessageTestPresentation {
    label: &'static str,
    description: &'static str,
    enabled: bool,
}

#[cfg(target_os = "macos")]
fn imessage_test_presentation(
    setup_complete: bool,
    connection_enabled: bool,
    status: &IMessageConnectionStatus,
    requirements: IMessageSetupRequirements,
    test_status: IMessageTestStatus,
) -> IMessageTestPresentation {
    let sending = matches!(status, IMessageConnectionStatus::SendingSetupMessage)
        || matches!(test_status, IMessageTestStatus::Sending);
    let enabled = !sending
        && if setup_complete {
            connection_enabled && matches!(status, IMessageConnectionStatus::Connected)
        } else {
            requirements.all_ready()
        };
    let label = if sending {
        "Sending…"
    } else if matches!(test_status, IMessageTestStatus::Failed) {
        "Try again"
    } else if setup_complete && matches!(test_status, IMessageTestStatus::Sent) {
        "Test again"
    } else if !setup_complete
        && (matches!(status, IMessageConnectionStatus::AwaitingCalibrationReply)
            || matches!(test_status, IMessageTestStatus::Sent))
    {
        "Send again"
    } else {
        "Test iMessage"
    };
    let description = if sending {
        "Sending a test message through Messages…"
    } else if !setup_complete
        && matches!(status, IMessageConnectionStatus::AwaitingCalibrationReply)
    {
        "Setup message sent and confirmed. Reply with its one-time code from your phone."
    } else if !setup_complete
        && matches!(status, IMessageConnectionStatus::CalibrationReplyMismatch)
    {
        "Clinch received your message, but it did not match the one-time code. Reply with the code exactly as shown."
    } else if matches!(test_status, IMessageTestStatus::Failed) {
        "The test message was not confirmed. Check the setup checklist, then try again."
    } else if !requirements.all_ready() {
        "Complete the setup checklist below before sending a test message."
    } else if !setup_complete {
        "Everything is ready. Test iMessage sends the setup message and one-time reply code."
    } else if !connection_enabled {
        "Turn on the iMessage connection below to send a test message."
    } else if matches!(test_status, IMessageTestStatus::Sent) {
        "Test message sent. Check Messages on your phone."
    } else {
        "Test sends a short message without changing any agent notification settings."
    };
    IMessageTestPresentation {
        label,
        description,
        enabled,
    }
}

#[cfg(target_os = "macos")]
fn imessage_requirement_label(name: &str, status: Option<bool>) -> String {
    match status {
        Some(true) => format!("✓ {name}"),
        Some(false) => format!("○ {name}"),
        None => format!("… Checking {name}"),
    }
}

#[cfg(target_os = "macos")]
#[derive(Default)]
struct IMessageWidget {
    enabled_switch_state: SwitchStateHandle,
    notifications_default_switch_state: SwitchStateHandle,
    test_button_state: MouseStateHandle,
    check_again_button_state: MouseStateHandle,
    phone_requirement_button_state: MouseStateHandle,
    change_number_button_state: MouseStateHandle,
    messages_button_state: MouseStateHandle,
    automation_button_state: MouseStateHandle,
    full_disk_access_button_state: MouseStateHandle,
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

        let stage = imessage_setup_stage(
            configuration.setup_complete,
            configuration.recipient.trim().is_empty(),
            status,
        );
        let phone_chip = || {
            super::render_model_chips(
                [configuration.recipient.clone()],
                appearance,
                appearance.theme().active_ui_text_color(),
            )
        };
        let change_number_button = || {
            appearance
                .ui_builder()
                .button(
                    ButtonVariant::Secondary,
                    self.change_number_button_state.clone(),
                )
                .with_text_label("Change number".to_owned())
                .build()
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(ClinchSettingsPageAction::IMessageChangeNumber);
                })
                .finish()
        };

        let requirements = coordinator.setup_requirements();
        let test_presentation = imessage_test_presentation(
            configuration.setup_complete,
            configuration.enabled,
            status,
            requirements,
            coordinator.test_status(),
        );
        let setup = if matches!(stage, IMessageSetupStage::EnterNumber) {
            render_body_item::<ClinchSettingsPageAction>(
                "Set up your phone".to_owned(),
                None,
                LocalOnlyIconState::Hidden,
                ToggleState::Enabled,
                appearance,
                ConstrainedBox::new(ChildView::new(&view.imessage_recipient_editor).finish())
                    .with_width(360.)
                    .finish(),
                Some(
                    "Enter the number associated with your iPhone, then select Connect or press \
                     Return. No iPhone app or Clinch server is involved."
                        .to_owned(),
                ),
            )
        } else {
            let setup_label = match stage {
                IMessageSetupStage::EnterNumber => unreachable!(),
                IMessageSetupStage::Connecting => "Checking iMessage setup",
                IMessageSetupStage::ReadyToTest => "Ready to test iMessage",
                IMessageSetupStage::SendingTest => "Sending setup message",
                IMessageSetupStage::FullDiskAccess => "Full Disk Access required",
                IMessageSetupStage::Automation => "Messages access required",
                IMessageSetupStage::MessagesSignIn => "Sign in to iMessage",
                IMessageSetupStage::AwaitingReply => "Finish connecting on your phone",
                IMessageSetupStage::ReplyMismatch => "Reply did not match the setup code",
                IMessageSetupStage::Connected => "Connected to this number",
                IMessageSetupStage::Error => "iMessage needs attention",
            };
            let mut test_button = appearance
                .ui_builder()
                .button(ButtonVariant::Secondary, self.test_button_state.clone())
                .with_text_label(test_presentation.label.to_owned())
                .build();
            if !test_presentation.enabled {
                test_button = test_button.disable();
            }
            let test_button = test_button
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(ClinchSettingsPageAction::IMessageTest);
                })
                .finish();
            render_body_item::<ClinchSettingsPageAction>(
                setup_label.to_owned(),
                None,
                LocalOnlyIconState::Hidden,
                ToggleState::Enabled,
                appearance,
                Flex::row()
                    .with_spacing(8.)
                    .with_child(phone_chip())
                    .with_child(test_button)
                    .with_child(change_number_button())
                    .finish(),
                Some(test_presentation.description.to_owned()),
            )
        };

        let checklist = if configuration.recipient.trim().is_empty() {
            None
        } else {
            let requirement_button =
                |name: &str,
                 requirement_status: Option<bool>,
                 state: MouseStateHandle,
                 action: ClinchSettingsPageAction| {
                    let mut button = appearance
                        .ui_builder()
                        .button(ButtonVariant::Secondary, state)
                        .with_text_label(imessage_requirement_label(name, requirement_status))
                        .build();
                    if requirement_status == Some(true) {
                        button = button.disable();
                    }
                    button
                        .on_click(move |ctx, _, _| {
                            ctx.dispatch_typed_action(action.clone());
                        })
                        .finish()
                };
            let mut check_again = appearance
                .ui_builder()
                .button(
                    ButtonVariant::Secondary,
                    self.check_again_button_state.clone(),
                )
                .with_text_label("Check again".to_owned())
                .build();
            if matches!(coordinator.test_status(), IMessageTestStatus::Sending) {
                check_again = check_again.disable();
            }
            let check_again = check_again
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(ClinchSettingsPageAction::IMessageRefresh);
                })
                .finish();
            Some(render_body_item::<ClinchSettingsPageAction>(
                "Setup checklist".to_owned(),
                None,
                LocalOnlyIconState::Hidden,
                ToggleState::Enabled,
                appearance,
                Flex::column()
                    .with_spacing(6.)
                    .with_child(requirement_button(
                        "Phone number",
                        Some(true),
                        self.phone_requirement_button_state.clone(),
                        ClinchSettingsPageAction::IMessageChangeNumber,
                    ))
                    .with_child(requirement_button(
                        "Full Disk Access",
                        requirements.full_disk_access,
                        self.full_disk_access_button_state.clone(),
                        ClinchSettingsPageAction::OpenFullDiskAccessSettings,
                    ))
                    .with_child(requirement_button(
                        "Allow Clinch to control Messages",
                        requirements.automation,
                        self.automation_button_state.clone(),
                        ClinchSettingsPageAction::IMessageRequestAutomation,
                    ))
                    .with_child(requirement_button(
                        "Signed in to iMessage",
                        requirements.imessage_available,
                        self.messages_button_state.clone(),
                        ClinchSettingsPageAction::OpenMessages,
                    ))
                    .with_child(check_again)
                    .finish(),
                Some(
                    "Select any incomplete item to open the place where it can be completed. \
                     Completed requirements stay checked."
                        .to_owned(),
                ),
            ))
        };

        let enabled_switch = appearance
            .ui_builder()
            .switch(self.enabled_switch_state.clone())
            .check(configuration.enabled)
            .build()
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(ClinchSettingsPageAction::IMessageEnabled);
            })
            .finish();
        let enabled = render_body_item::<ClinchSettingsPageAction>(
            "iMessage connection".into(),
            None,
            LocalOnlyIconState::Hidden,
            ToggleState::Enabled,
            appearance,
            enabled_switch,
            Some(
                "Turning this off pauses completion messages and phone replies without forgetting \
                 the connected number or individual footer choices."
                    .into(),
            ),
        );

        let notifications_default = render_body_item::<ClinchSettingsPageAction>(
            "Get notified by default".into(),
            None,
            LocalOnlyIconState::Hidden,
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .switch(self.notifications_default_switch_state.clone())
                .check(configuration.notifications_enabled_by_default)
                .build()
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(
                        ClinchSettingsPageAction::IMessageNotificationsDefault,
                    );
                })
                .finish(),
            Some(
                "On by default. Sessions without an individual footer choice follow this setting. \
                 A Get notified: Yes or No choice for a session is preserved across changes and \
                 restarts."
                    .into(),
            ),
        );

        let mut column = Flex::column().with_child(setup);
        if let Some(checklist) = checklist {
            column.add_child(checklist);
        }
        if configuration.setup_complete {
            column.add_child(enabled);
            column.add_child(notifications_default);
        }
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
