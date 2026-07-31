use std::cell::RefCell;
use std::collections::HashMap;

use ::settings::{Setting, ToggleableSetting};
#[cfg(feature = "local_fs")]
use clinch_companion_protocol::{DeviceId, PairingClaimId};
#[cfg(feature = "local_fs")]
use warp_core::channel::ChannelState;
use warpui::color::ColorU;
use warpui::elements::{
    Align, Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Element, Empty,
    Expanded, Flex, MainAxisSize, MouseStateHandle, ParentElement, Radius, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::ui_components::switch::SwitchStateHandle;
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle};

use super::settings_page::{
    render_body_item, render_settings_info_banner, Category, LocalOnlyIconState, MatchData,
    PageType, SettingsPageMeta, SettingsPageViewHandle, SettingsWidget, CONTENT_FONT_SIZE,
};
use super::{SettingsSection, ToggleState};
use crate::appearance::Appearance;
use crate::drive::sharing::qr_code::{qr_matrix_for_url, QrMatrix, QUIET_ZONE_MODULES};
#[cfg(feature = "local_fs")]
use crate::remote_control::{RemoteControlService, RemoteControlStatus};
use crate::report_if_error;
use crate::settings::{
    AutoCreateWorktreesForNewTabs, CliAgentUsageSettings, ClinchAutomaticUpdateCheck,
    ClinchSettings, ShowCliAgentPlanLimits,
};
use crate::terminal::session_settings::{NotificationsSettings, SessionSettings};
use crate::ui_components::icons::Icon;

const TAILSCALE_MAC_DOWNLOAD_URL: &str = "https://tailscale.com/download/mac";
const TAILSCALE_IOS_DOWNLOAD_URL: &str = "https://tailscale.com/download/ios";
const CLINCH_REMOTE_CONTROL_GUIDE_URL: &str = "https://clinch.sh/remote-control";

#[derive(Clone, Debug, PartialEq)]
pub enum ClinchSettingsPageAction {
    SessionCapture,
    AgentStatusOnTabs,
    AutoCreateWorktreesForNewTabs,
    CliAgentPlanLimits,
    AutomaticUpdateCheck,
    #[cfg(feature = "local_fs")]
    RemoteControlToggle,
    #[cfg(feature = "local_fs")]
    RemoteControlRetry,
    #[cfg(feature = "local_fs")]
    RemoteControlPair,
    #[cfg(feature = "local_fs")]
    RemoteControlCancelPairing,
    #[cfg(feature = "local_fs")]
    RemoteControlApprove(PairingClaimId),
    #[cfg(feature = "local_fs")]
    RemoteControlReject(PairingClaimId),
    #[cfg(feature = "local_fs")]
    RemoteControlRevoke(DeviceId),
    #[cfg(feature = "local_fs")]
    RemoteControlRevokeAll,
    CopyText(String),
    OpenUrl(String),
}

pub struct ClinchSettingsPageView {
    page: PageType<Self>,
    local_only_icon_tooltip_states: RefCell<HashMap<String, MouseStateHandle>>,
    session_capture_enabled: bool,
}

impl ClinchSettingsPageView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        ctx.subscribe_to_model(&SessionSettings::handle(ctx), |_, _, _, ctx| ctx.notify());
        ctx.subscribe_to_model(&CliAgentUsageSettings::handle(ctx), |_, _, _, ctx| {
            ctx.notify()
        });
        ctx.subscribe_to_model(&ClinchSettings::handle(ctx), |_, _, _, ctx| ctx.notify());
        #[cfg(feature = "local_fs")]
        if !ChannelState::has_backend() {
            ctx.subscribe_to_model(&RemoteControlService::handle(ctx), |_, _, _, ctx| {
                ctx.notify()
            });
        }

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

        let mut updates_widgets: Vec<Box<dyn SettingsWidget<View = Self>>> = vec![];
        if ClinchSettings::as_ref(ctx)
            .automatic_update_check
            .is_supported_on_current_platform()
        {
            updates_widgets.push(Box::new(AutomaticUpdateCheckWidget::default()));
        }

        let mut categories = vec![];
        #[cfg(feature = "local_fs")]
        if !ChannelState::has_backend() {
            categories.push(
                Category::new(
                    "Remote Control (Preview)",
                    vec![Box::new(RemoteControlSetupWidget::default())],
                )
                .with_subtitle(
                    "Securely connect your phone through your own Tailscale network — no Clinch \
                     account or hosted Clinch relay.",
                ),
            );
        }
        if !project_widgets.is_empty() {
            categories.push(Category::new("Projects", project_widgets));
        }
        categories.push(Category::new("Agents", agent_widgets));
        if !usage_widgets.is_empty() {
            categories.push(Category::new("Usage", usage_widgets));
        }
        if !updates_widgets.is_empty() {
            categories.push(Category::new("Updates", updates_widgets));
        }

        Self {
            page: PageType::new_categorized(categories, Some("Clinch Settings")),
            local_only_icon_tooltip_states: RefCell::new(HashMap::new()),
            #[cfg(target_os = "macos")]
            session_capture_enabled: crate::agent_resume::capture_layer_enabled(),
            #[cfg(not(target_os = "macos"))]
            session_capture_enabled: false,
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
            ClinchSettingsPageAction::AutomaticUpdateCheck => {
                ClinchSettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.automatic_update_check.toggle_and_save_value(ctx));
                });
                ctx.notify();
            }
            #[cfg(feature = "local_fs")]
            ClinchSettingsPageAction::RemoteControlToggle => {
                RemoteControlService::handle(ctx).update(ctx, |service, ctx| {
                    service.set_enabled(!service.view_state().enabled, ctx);
                });
            }
            #[cfg(feature = "local_fs")]
            ClinchSettingsPageAction::RemoteControlRetry => {
                RemoteControlService::handle(ctx).update(ctx, |service, ctx| service.retry(ctx));
            }
            #[cfg(feature = "local_fs")]
            ClinchSettingsPageAction::RemoteControlPair => {
                RemoteControlService::handle(ctx).update(ctx, |service, ctx| {
                    if let Err(error) = service.create_pairing_invitation(ctx) {
                        log::warn!("could not create Remote Control invitation: {error}");
                    }
                });
            }
            #[cfg(feature = "local_fs")]
            ClinchSettingsPageAction::RemoteControlCancelPairing => {
                RemoteControlService::handle(ctx)
                    .update(ctx, |service, ctx| service.cancel_pairing_invitation(ctx));
            }
            #[cfg(feature = "local_fs")]
            ClinchSettingsPageAction::RemoteControlApprove(claim_id) => {
                let claim_id = *claim_id;
                RemoteControlService::handle(ctx).update(ctx, |service, ctx| {
                    if let Err(error) = service.approve_pairing(claim_id, ctx) {
                        log::warn!("could not approve Remote Control phone: {error}");
                    }
                });
            }
            #[cfg(feature = "local_fs")]
            ClinchSettingsPageAction::RemoteControlReject(claim_id) => {
                let claim_id = *claim_id;
                RemoteControlService::handle(ctx).update(ctx, |service, ctx| {
                    if let Err(error) = service.reject_pairing(claim_id, ctx) {
                        log::warn!("could not reject Remote Control phone: {error}");
                    }
                });
            }
            #[cfg(feature = "local_fs")]
            ClinchSettingsPageAction::RemoteControlRevoke(device_id) => {
                let device_id = *device_id;
                RemoteControlService::handle(ctx).update(ctx, |service, ctx| {
                    if let Err(error) = service.revoke_device(device_id, ctx) {
                        log::warn!("could not revoke Remote Control phone: {error}");
                    }
                });
            }
            #[cfg(feature = "local_fs")]
            ClinchSettingsPageAction::RemoteControlRevokeAll => {
                RemoteControlService::handle(ctx).update(ctx, |service, ctx| {
                    let disable = service.view_state().enabled;
                    if let Err(error) = service.revoke_all_devices(ctx) {
                        log::warn!("could not revoke all Remote Control phones: {error}");
                    } else if disable {
                        service.set_enabled(false, ctx);
                    }
                });
            }
            ClinchSettingsPageAction::CopyText(text) => ctx.dispatch_typed_action(
                &crate::workspace::WorkspaceAction::CopyTextToClipboard(text.clone()),
            ),
            ClinchSettingsPageAction::OpenUrl(url) => ctx.open_url(url),
        }
    }
}

pub(crate) fn remote_control_setup_widget_id() -> &'static str {
    RemoteControlSetupWidget::static_widget_id()
}

#[derive(Default)]
struct RemoteControlSetupWidget {
    mac_download_mouse_state: MouseStateHandle,
    ios_download_mouse_state: MouseStateHandle,
    guide_mouse_state: MouseStateHandle,
    pair_mouse_state: MouseStateHandle,
    retry_mouse_state: MouseStateHandle,
    revoke_all_mouse_state: MouseStateHandle,
    copy_link_mouse_state: MouseStateHandle,
    cancel_pairing_mouse_state: MouseStateHandle,
    enable_switch_state: SwitchStateHandle,
    dynamic_mouse_states: RefCell<HashMap<String, MouseStateHandle>>,
}

impl RemoteControlSetupWidget {
    fn render_action_button(
        label: impl Into<String>,
        url: impl Into<String>,
        mouse_state: MouseStateHandle,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let label = label.into();
        let url = url.into();
        appearance
            .ui_builder()
            .button(ButtonVariant::Secondary, mouse_state)
            .with_text_label(label)
            .with_style(UiComponentStyles {
                font_size: Some(CONTENT_FONT_SIZE),
                padding: Some(Coords::default().top(6.).bottom(6.).left(12.).right(12.)),
                ..Default::default()
            })
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(ClinchSettingsPageAction::OpenUrl(url.clone()));
            })
            .finish()
    }

    fn render_typed_action_button(
        label: impl Into<String>,
        action: ClinchSettingsPageAction,
        mouse_state: MouseStateHandle,
        appearance: &Appearance,
        disabled: bool,
    ) -> Box<dyn Element> {
        let button = appearance
            .ui_builder()
            .button(ButtonVariant::Secondary, mouse_state)
            .with_text_label(label.into())
            .with_style(UiComponentStyles {
                font_size: Some(CONTENT_FONT_SIZE),
                padding: Some(Coords::default().top(6.).bottom(6.).left(12.).right(12.)),
                ..Default::default()
            });
        let button = if disabled { button.disabled() } else { button }.build();
        if disabled {
            button.finish()
        } else {
            button
                .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
                .finish()
        }
    }

    fn dynamic_mouse_state(&self, key: String) -> MouseStateHandle {
        self.dynamic_mouse_states
            .borrow_mut()
            .entry(key)
            .or_default()
            .clone()
    }

    fn render_step(
        number: usize,
        title: &'static str,
        description: &'static str,
        action: Option<Box<dyn Element>>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let number = ConstrainedBox::new(
            Container::new(
                Align::new(
                    Text::new_inline(
                        number.to_string(),
                        appearance.ui_font_family(),
                        CONTENT_FONT_SIZE,
                    )
                    .with_style(Properties::default().weight(Weight::Bold))
                    .with_color(theme.main_text_color(theme.accent()).into())
                    .finish(),
                )
                .finish(),
            )
            .with_background(theme.accent())
            .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
            .finish(),
        )
        .with_width(24.)
        .with_height(24.)
        .finish();

        let copy = Expanded::new(
            1.,
            Flex::column()
                .with_child(
                    Text::new_inline(
                        title.to_owned(),
                        appearance.ui_font_family(),
                        CONTENT_FONT_SIZE,
                    )
                    .with_style(Properties::default().weight(Weight::Semibold))
                    .with_color(theme.active_ui_text_color().into())
                    .finish(),
                )
                .with_child(
                    Container::new(
                        Text::new(
                            description.to_owned(),
                            appearance.ui_font_family(),
                            CONTENT_FONT_SIZE,
                        )
                        .with_color(theme.nonactive_ui_text_color().into())
                        .finish(),
                    )
                    .with_margin_top(4.)
                    .finish(),
                )
                .finish(),
        )
        .finish();

        let mut row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(number)
            .with_child(
                Container::new(copy)
                    .with_margin_left(12.)
                    .with_margin_right(12.)
                    .finish(),
            );
        if let Some(action) = action {
            row.add_child(action);
        }

        Container::new(row.finish())
            .with_border(Border::all(1.).with_border_fill(theme.outline()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
            .with_uniform_padding(12.)
            .with_margin_bottom(8.)
            .finish()
    }

    fn render_qr_matrix(matrix: &QrMatrix) -> Box<dyn Element> {
        const QR_SIZE: f32 = 208.;
        let modules_with_quiet_zone = matrix.width().saturating_add(QUIET_ZONE_MODULES * 2);
        let module_size = QR_SIZE / modules_with_quiet_zone as f32;
        let mut column = Flex::column().with_main_axis_size(MainAxisSize::Max);
        for y in 0..modules_with_quiet_zone {
            let mut row = Flex::row().with_main_axis_size(MainAxisSize::Max);
            for x in 0..modules_with_quiet_zone {
                let matrix_x = x.saturating_sub(QUIET_ZONE_MODULES);
                let matrix_y = y.saturating_sub(QUIET_ZONE_MODULES);
                let dark = x >= QUIET_ZONE_MODULES
                    && y >= QUIET_ZONE_MODULES
                    && matrix_x < matrix.width()
                    && matrix_y < matrix.width()
                    && matrix.is_dark(matrix_x, matrix_y);
                row.add_child(
                    ConstrainedBox::new(
                        Container::new(Empty::new().finish())
                            .with_background(if dark {
                                ColorU::black()
                            } else {
                                ColorU::white()
                            })
                            .finish(),
                    )
                    .with_width(module_size)
                    .with_height(module_size)
                    .finish(),
                );
            }
            column.add_child(row.finish());
        }
        ConstrainedBox::new(column.finish())
            .with_width(QR_SIZE)
            .with_height(QR_SIZE)
            .finish()
    }

    fn render_privacy_note(appearance: &Appearance) -> Box<dyn Element> {
        Container::new(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    ConstrainedBox::new(
                        Icon::Lock
                            .to_warpui_icon(appearance.theme().nonactive_ui_text_color())
                            .finish(),
                    )
                    .with_width(16.)
                    .with_height(16.)
                    .finish(),
                )
                .with_child(
                    Container::new(
                        Text::new(
                            "No Clinch account, relay, or Remote Control analytics are involved. \
                             The gateway listens only on this Mac and Tailscale privately proxies \
                             it inside your tailnet. Tailscale requires its own account and is \
                             governed by its own plan and privacy terms.",
                            appearance.ui_font_family(),
                            CONTENT_FONT_SIZE,
                        )
                        .with_color(appearance.theme().nonactive_ui_text_color().into())
                        .finish(),
                    )
                    .with_margin_left(8.)
                    .finish(),
                )
                .finish(),
        )
        .with_margin_top(4.)
        .finish()
    }

    #[cfg(feature = "local_fs")]
    fn render_native(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let state = RemoteControlService::as_ref(app).view_state().clone();
        let (status_title, status_description) = match &state.status {
            RemoteControlStatus::Disabled => (
                "Remote Control is off".to_owned(),
                "Nothing is listening and no terminal data is reachable from your phone."
                    .to_owned(),
            ),
            RemoteControlStatus::Starting => (
                "Starting private access…".to_owned(),
                "Clinch is checking Tailscale and creating a loopback-only companion.".to_owned(),
            ),
            RemoteControlStatus::TailscaleNotInstalled => (
                "Install Tailscale to continue".to_owned(),
                "Clinch could not find a supported Tailscale installation on this Mac.".to_owned(),
            ),
            RemoteControlStatus::TailscaleStopped => (
                "Start Tailscale to continue".to_owned(),
                "Tailscale is installed on this Mac, but its network connection is stopped."
                    .to_owned(),
            ),
            RemoteControlStatus::TailscaleSignInRequired { .. } => (
                "Sign in to Tailscale".to_owned(),
                "Connect this Mac to the same tailnet you will use on your phone.".to_owned(),
            ),
            RemoteControlStatus::TailscaleConsentRequired { .. } => (
                "Approve private HTTPS".to_owned(),
                "Tailscale needs one-time permission to issue the private tailnet certificate."
                    .to_owned(),
            ),
            RemoteControlStatus::Ready { remote_url, .. } => (
                "Ready to pair".to_owned(),
                format!("Private mobile address: {remote_url}"),
            ),
            RemoteControlStatus::Error { message, .. } => {
                ("Remote Control needs attention".to_owned(), message.clone())
            }
        };
        let mut content = Flex::column().with_child(
            Container::new(render_settings_info_banner(
                &status_title,
                Some(&status_description),
                appearance,
            ))
            .with_margin_bottom(12.)
            .finish(),
        );

        content.add_child(Self::render_step(
            1,
            "Install Tailscale on this Mac",
            "Install Tailscale, approve the macOS VPN permission, and sign in. Clinch never opens \
             a public port or enables Funnel.",
            Some(Self::render_action_button(
                "Get Tailscale for Mac",
                TAILSCALE_MAC_DOWNLOAD_URL,
                self.mac_download_mouse_state.clone(),
                appearance,
            )),
            appearance,
        ));
        content.add_child(Self::render_step(
            2,
            "Connect your iPhone or iPad",
            "Install Tailscale on the phone and sign in to the same tailnet. The same setup works \
             on shared Wi-Fi or over 5G while this Mac is awake and Clinch is running.",
            Some(Self::render_action_button(
                "Get Tailscale for iOS",
                TAILSCALE_IOS_DOWNLOAD_URL,
                self.ios_download_mouse_state.clone(),
                appearance,
            )),
            appearance,
        ));
        let enable_switch = appearance
            .ui_builder()
            .switch(self.enable_switch_state.clone())
            .check(state.enabled)
            .build()
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(ClinchSettingsPageAction::RemoteControlToggle)
            })
            .finish();
        content.add_child(Self::render_step(
            3,
            "Enable Remote Control",
            "This local-only choice starts Clinch's private companion and gives it one isolated \
             Tailscale Serve path. Turning it off ends live sessions and removes only that path; \
             paired phones stay authorized for the next enable unless you revoke them below.",
            Some(enable_switch),
            appearance,
        ));

        let status_action = match &state.status {
            RemoteControlStatus::TailscaleSignInRequired {
                action_url: Some(url),
            }
            | RemoteControlStatus::TailscaleConsentRequired {
                action_url: Some(url),
            } => Some(Self::render_action_button(
                "Continue in Tailscale",
                url.clone(),
                self.retry_mouse_state.clone(),
                appearance,
            )),
            RemoteControlStatus::Ready { remote_url, .. } => Some(Self::render_action_button(
                "Open mobile app",
                remote_url.clone(),
                self.retry_mouse_state.clone(),
                appearance,
            )),
            RemoteControlStatus::Error {
                retryable: true, ..
            }
            | RemoteControlStatus::TailscaleStopped
            | RemoteControlStatus::TailscaleSignInRequired { action_url: None }
            | RemoteControlStatus::TailscaleConsentRequired { action_url: None } => {
                Some(Self::render_typed_action_button(
                    "Try again",
                    ClinchSettingsPageAction::RemoteControlRetry,
                    self.retry_mouse_state.clone(),
                    appearance,
                    false,
                ))
            }
            _ => None,
        };
        if let Some(action) = status_action {
            content.add_child(Container::new(action).with_margin_bottom(8.).finish());
        }
        if matches!(
            &state.status,
            RemoteControlStatus::TailscaleNotInstalled
                | RemoteControlStatus::TailscaleStopped
                | RemoteControlStatus::TailscaleSignInRequired {
                    action_url: Some(_)
                }
                | RemoteControlStatus::TailscaleConsentRequired {
                    action_url: Some(_)
                }
        ) {
            content.add_child(
                Container::new(Self::render_typed_action_button(
                    "I've finished — check again",
                    ClinchSettingsPageAction::RemoteControlRetry,
                    self.guide_mouse_state.clone(),
                    appearance,
                    false,
                ))
                .with_margin_bottom(8.)
                .finish(),
            );
        }

        content.add_child(Self::render_step(
            4,
            "Pair this phone",
            "Generate a five-minute, single-use QR code. The phone creates a non-exportable key; \
             nothing is authorized until you approve its name and fingerprint below. Safari \
             works immediately, and Add to Home Screen is optional.",
            Some(Self::render_typed_action_button(
                if state.active_invitation.is_some() {
                    "Refresh QR code"
                } else {
                    "Pair phone"
                },
                ClinchSettingsPageAction::RemoteControlPair,
                self.pair_mouse_state.clone(),
                appearance,
                !state.status.is_ready(),
            )),
            appearance,
        ));

        if let Some(invitation) = &state.active_invitation {
            if let Ok(matrix) = qr_matrix_for_url(&invitation.pairing_url) {
                content.add_child(
                    Container::new(
                        Flex::column()
                            .with_cross_axis_alignment(CrossAxisAlignment::Center)
                            .with_child(
                                Container::new(Self::render_qr_matrix(&matrix))
                                    .with_background(ColorU::white())
                                    .with_uniform_padding(10.)
                                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                                    .finish(),
                            )
                            .with_child(
                                Container::new(
                                    Text::new(
                                        format!(
                                            "Scan with your phone. Expires at {}.",
                                            invitation.expires_at.format("%H:%M:%S UTC")
                                        ),
                                        appearance.ui_font_family(),
                                        CONTENT_FONT_SIZE,
                                    )
                                    .with_color(appearance.theme().nonactive_ui_text_color().into())
                                    .finish(),
                                )
                                .with_margin_top(8.)
                                .finish(),
                            )
                            .finish(),
                    )
                    .with_margin_bottom(12.)
                    .finish(),
                );
            }
            content.add_child(
                Container::new(
                    Flex::row()
                        .with_child(Self::render_typed_action_button(
                            "Copy pairing link",
                            ClinchSettingsPageAction::CopyText(invitation.pairing_url.clone()),
                            self.copy_link_mouse_state.clone(),
                            appearance,
                            false,
                        ))
                        .with_child(
                            Container::new(Self::render_typed_action_button(
                                "Cancel QR code",
                                ClinchSettingsPageAction::RemoteControlCancelPairing,
                                self.cancel_pairing_mouse_state.clone(),
                                appearance,
                                false,
                            ))
                            .with_margin_left(6.)
                            .finish(),
                        )
                        .finish(),
                )
                .with_margin_bottom(12.)
                .finish(),
            );
        }

        if !state.pending_claims.is_empty() {
            content.add_child(
                Container::new(
                    Text::new_inline(
                        "Waiting for your approval",
                        appearance.ui_font_family(),
                        CONTENT_FONT_SIZE,
                    )
                    .with_style(Properties::default().weight(Weight::Semibold))
                    .with_color(appearance.theme().active_ui_text_color().into())
                    .finish(),
                )
                .with_margin_bottom(8.)
                .finish(),
            );
        }
        for claim in &state.pending_claims {
            let fingerprint: String = claim.public_key_fingerprint.chars().take(20).collect();
            let approve = Self::render_typed_action_button(
                "Approve",
                ClinchSettingsPageAction::RemoteControlApprove(claim.id),
                self.dynamic_mouse_state(format!("approve-{}", claim.id)),
                appearance,
                false,
            );
            let reject = Self::render_typed_action_button(
                "Reject",
                ClinchSettingsPageAction::RemoteControlReject(claim.id),
                self.dynamic_mouse_state(format!("reject-{}", claim.id)),
                appearance,
                false,
            );
            content.add_child(
                Container::new(
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(
                            Expanded::new(
                                1.,
                                Text::new(
                                    format!(
                                        "{} · {:?}\nKey {}…",
                                        claim.device_name, claim.platform, fingerprint
                                    ),
                                    appearance.ui_font_family(),
                                    CONTENT_FONT_SIZE,
                                )
                                .with_color(appearance.theme().active_ui_text_color().into())
                                .finish(),
                            )
                            .finish(),
                        )
                        .with_child(approve)
                        .with_child(Container::new(reject).with_margin_left(6.).finish())
                        .finish(),
                )
                .with_border(Border::all(1.).with_border_fill(appearance.theme().outline()))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
                .with_uniform_padding(10.)
                .with_margin_bottom(8.)
                .finish(),
            );
        }

        if !state.paired_devices.is_empty() {
            content.add_child(
                Container::new(
                    Text::new_inline(
                        "Paired phones",
                        appearance.ui_font_family(),
                        CONTENT_FONT_SIZE,
                    )
                    .with_style(Properties::default().weight(Weight::Semibold))
                    .with_color(appearance.theme().active_ui_text_color().into())
                    .finish(),
                )
                .with_margin_top(4.)
                .with_margin_bottom(8.)
                .finish(),
            );
        }
        for device in &state.paired_devices {
            let revoke = Self::render_typed_action_button(
                "Revoke",
                ClinchSettingsPageAction::RemoteControlRevoke(device.id),
                self.dynamic_mouse_state(format!("revoke-{}", device.id)),
                appearance,
                false,
            );
            let connection = if device.connected {
                "Connected"
            } else {
                "Not connected"
            };
            let last_seen = device
                .last_seen_at
                .map(|timestamp| timestamp.format("%Y-%m-%d %H:%M UTC").to_string())
                .unwrap_or_else(|| "Not connected yet".to_owned());
            let capabilities = device
                .capabilities
                .iter()
                .map(|capability| format!("{capability:?}"))
                .collect::<Vec<_>>()
                .join(", ");
            content.add_child(
                Container::new(
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(
                            Expanded::new(
                                1.,
                                Text::new(
                                    format!(
                                        "{} · {:?} · {connection}\nLast seen: {last_seen} · {capabilities}",
                                        device.name, device.platform
                                    ),
                                    appearance.ui_font_family(),
                                    CONTENT_FONT_SIZE,
                                )
                                .with_color(appearance.theme().active_ui_text_color().into())
                                .finish(),
                            )
                            .finish(),
                        )
                        .with_child(revoke)
                        .finish(),
                )
                .with_border(Border::all(1.).with_border_fill(appearance.theme().outline()))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
                .with_uniform_padding(10.)
                .with_margin_bottom(8.)
                .finish(),
            );
        }

        if !state.paired_devices.is_empty() {
            content.add_child(
                Container::new(Self::render_typed_action_button(
                    if state.enabled {
                        "Turn off & revoke all phones"
                    } else {
                        "Revoke all paired phones"
                    },
                    ClinchSettingsPageAction::RemoteControlRevokeAll,
                    self.revoke_all_mouse_state.clone(),
                    appearance,
                    false,
                ))
                .with_margin_bottom(8.)
                .finish(),
            );
        }

        content.add_child(Self::render_privacy_note(appearance));
        content.add_child(
            Container::new(Self::render_action_button(
                "Setup and security guide",
                CLINCH_REMOTE_CONTROL_GUIDE_URL,
                self.guide_mouse_state.clone(),
                appearance,
            ))
            .with_margin_top(8.)
            .finish(),
        );
        content.finish()
    }
}

impl SettingsWidget for RemoteControlSetupWidget {
    type View = ClinchSettingsPageView;

    fn search_terms(&self) -> &str {
        "clinch remote control mobile phone iphone ipad tailscale tailnet qr pairing private \
         network 5g wifi no account telemetry web app pwa"
    }

    fn render(
        &self,
        _view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        #[cfg(feature = "local_fs")]
        {
            self.render_native(appearance, app)
        }
        #[cfg(not(feature = "local_fs"))]
        {
            let _ = app;
            Flex::column()
                .with_child(render_settings_info_banner(
                    "Remote Control requires the native Clinch app",
                    Some("Install Clinch on macOS to host the private companion."),
                    appearance,
                ))
                .with_child(Self::render_privacy_note(appearance))
                .finish()
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
struct AutomaticUpdateCheckWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for AutomaticUpdateCheckWidget {
    type View = ClinchSettingsPageView;

    fn search_terms(&self) -> &str {
        "clinch update updates automatic check weekly github network privacy offline telemetry \
         disable"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let enabled = *ClinchSettings::as_ref(app).automatic_update_check;
        render_body_item::<ClinchSettingsPageAction>(
            "Check for updates automatically".into(),
            None,
            LocalOnlyIconState::for_setting(
                ClinchAutomaticUpdateCheck::storage_key(),
                ClinchAutomaticUpdateCheck::sync_to_cloud(),
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
                    ctx.dispatch_typed_action(ClinchSettingsPageAction::AutomaticUpdateCheck);
                })
                .finish(),
            Some(
                "Ask GitHub once a week whether a signed Clinch update exists. Nothing is \
                 downloaded until you approve it, and no identifier or usage data is sent. Turn \
                 this off and Clinch makes no automatic network requests at all — you can still \
                 check on demand from Clinch → Check for Updates…. Setting \
                 CLINCH_NO_UPDATE_CHECK=1 in the environment also turns it off."
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
