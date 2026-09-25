use std::cell::RefCell;
use std::collections::HashMap;

use ::settings::{Setting, ToggleableSetting};
#[cfg(feature = "local_fs")]
use chrono::{DateTime, Local, Utc};
#[cfg(feature = "local_fs")]
use clinch_companion_protocol::{DeviceId, DevicePlatform, PairingClaimId};
#[cfg(feature = "local_fs")]
use warp_core::channel::ChannelState;
#[cfg(feature = "local_fs")]
use warpui::clipboard::ClipboardContent;
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
    render_body_item, render_sub_header_with_description, Category, LocalOnlyIconState, MatchData,
    PageType, SettingsPageMeta, SettingsPageViewHandle, SettingsWidget, CONTENT_FONT_SIZE,
};
use super::{SettingsSection, ToggleState};
use crate::appearance::Appearance;
use crate::drive::sharing::qr_code::{qr_matrix_for_url, QrMatrix, QUIET_ZONE_MODULES};
#[cfg(feature = "local_fs")]
use crate::remote_control::{RemoteControlService, RemoteControlStatus, RemoteControlViewState};
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

/// The short code both the phone and this Mac display for a pending pairing: the first eight
/// hex digits of the phone key's SHA-256 fingerprint. The web app derives it identically.
#[cfg(feature = "local_fs")]
fn pairing_code(fingerprint: &str) -> String {
    let code = fingerprint
        .chars()
        .filter(char::is_ascii_hexdigit)
        .take(8)
        .collect::<String>()
        .to_ascii_uppercase();
    match code.len() {
        8 => format!("{}-{}", &code[..4], &code[4..]),
        _ => code,
    }
}

#[cfg(feature = "local_fs")]
fn countdown(expires_at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let seconds = (expires_at - now).num_seconds().max(0);
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

#[cfg(feature = "local_fs")]
fn device_activity_label(
    connected: bool,
    last_seen_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> String {
    if connected {
        return "Connected now".to_owned();
    }
    let Some(last_seen_at) = last_seen_at else {
        return "Not connected yet".to_owned();
    };
    let minutes = (now - last_seen_at).num_minutes();
    match minutes {
        ..=0 => "Last seen just now".to_owned(),
        1..=59 => format!("Last seen {minutes} min ago"),
        60..=1439 => format!("Last seen {} h ago", minutes / 60),
        _ => format!(
            "Last seen {}",
            last_seen_at.with_timezone(&Local).format("%b %-d")
        ),
    }
}

#[cfg(feature = "local_fs")]
fn platform_label(platform: &DevicePlatform) -> &'static str {
    match platform {
        DevicePlatform::Ios => "iPhone",
        DevicePlatform::Ipados => "iPad",
        DevicePlatform::Macos => "Mac",
        DevicePlatform::Android => "Android",
        DevicePlatform::Other => "Other device",
    }
}

/// Removing a phone is destructive (it must re-pair), so it takes a second click.
#[cfg(feature = "local_fs")]
#[derive(Clone, Copy, Debug, PartialEq)]
enum ArmedRemoval {
    Device(DeviceId),
    All,
}

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
    #[cfg(feature = "local_fs")]
    RemoteControlCopyLink(String),
    #[cfg(feature = "local_fs")]
    RemoteControlDismissError,
    OpenUrl(String),
}

pub struct ClinchSettingsPageView {
    page: PageType<Self>,
    local_only_icon_tooltip_states: RefCell<HashMap<String, MouseStateHandle>>,
    session_capture_enabled: bool,
    #[cfg(feature = "local_fs")]
    armed_removal: Option<ArmedRemoval>,
    #[cfg(feature = "local_fs")]
    link_copied: bool,
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
            ctx.observe(&RemoteControlService::handle(ctx), |_, _, ctx| ctx.notify());
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

        let mut categories = Vec::new();
        #[cfg(feature = "local_fs")]
        if !ChannelState::has_backend() {
            categories.push(Category::new(
                "",
                vec![Box::new(RemoteControlSetupWidget::default())],
            ));
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
            #[cfg(feature = "local_fs")]
            armed_removal: None,
            #[cfg(feature = "local_fs")]
            link_copied: false,
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
                RemoteControlService::handle(ctx)
                    .update(ctx, |service, ctx| service.start_pairing(ctx));
            }
            #[cfg(feature = "local_fs")]
            ClinchSettingsPageAction::RemoteControlCancelPairing => {
                RemoteControlService::handle(ctx)
                    .update(ctx, |service, ctx| service.cancel_pairing(ctx));
            }
            #[cfg(feature = "local_fs")]
            ClinchSettingsPageAction::RemoteControlApprove(claim_id) => {
                let claim_id = *claim_id;
                RemoteControlService::handle(ctx)
                    .update(ctx, |service, ctx| service.approve_pairing(claim_id, ctx));
            }
            #[cfg(feature = "local_fs")]
            ClinchSettingsPageAction::RemoteControlReject(claim_id) => {
                let claim_id = *claim_id;
                RemoteControlService::handle(ctx)
                    .update(ctx, |service, ctx| service.reject_pairing(claim_id, ctx));
            }
            #[cfg(feature = "local_fs")]
            ClinchSettingsPageAction::RemoteControlRevoke(device_id) => {
                let target = ArmedRemoval::Device(*device_id);
                if self.armed_removal == Some(target) {
                    self.armed_removal = None;
                    let device_id = *device_id;
                    RemoteControlService::handle(ctx)
                        .update(ctx, |service, ctx| service.revoke_device(device_id, ctx));
                } else {
                    self.armed_removal = Some(target);
                }
                ctx.notify();
            }
            #[cfg(feature = "local_fs")]
            ClinchSettingsPageAction::RemoteControlRevokeAll => {
                if self.armed_removal == Some(ArmedRemoval::All) {
                    self.armed_removal = None;
                    RemoteControlService::handle(ctx)
                        .update(ctx, |service, ctx| service.revoke_all_devices(ctx));
                } else {
                    self.armed_removal = Some(ArmedRemoval::All);
                }
                ctx.notify();
            }
            #[cfg(feature = "local_fs")]
            ClinchSettingsPageAction::RemoteControlCopyLink(url) => {
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(url.clone()));
                self.link_copied = true;
                ctx.notify();
            }
            #[cfg(feature = "local_fs")]
            ClinchSettingsPageAction::RemoteControlDismissError => {
                RemoteControlService::handle(ctx)
                    .update(ctx, |service, ctx| service.dismiss_pairing_error(ctx));
            }
            ClinchSettingsPageAction::OpenUrl(url) => ctx.open_url(url),
        }
    }
}

#[cfg(test)]
pub(crate) fn remote_control_setup_widget_id() -> &'static str {
    RemoteControlSetupWidget::static_widget_id()
}

#[derive(Default)]
struct RemoteControlSetupWidget {
    guide_mouse_state: MouseStateHandle,
    pair_mouse_state: MouseStateHandle,
    status_action_mouse_state: MouseStateHandle,
    copy_link_mouse_state: MouseStateHandle,
    phone_download_mouse_state: MouseStateHandle,
    revoke_all_mouse_state: MouseStateHandle,
    dismiss_error_mouse_state: MouseStateHandle,
    enable_switch_state: SwitchStateHandle,
    dynamic_mouse_states: RefCell<HashMap<String, MouseStateHandle>>,
}

/// What the Tailscale checklist step says and offers for a given service status.
#[cfg(feature = "local_fs")]
struct TailscaleStepCopy {
    done: bool,
    description: String,
    action: Option<(&'static str, ClinchSettingsPageAction)>,
    busy: bool,
}

#[cfg(feature = "local_fs")]
fn tailscale_step_copy(status: &RemoteControlStatus) -> TailscaleStepCopy {
    let waiting = |description: &str, action| TailscaleStepCopy {
        done: false,
        description: format!("{description} Clinch checks again automatically."),
        action,
        busy: false,
    };
    match status {
        RemoteControlStatus::Disabled => TailscaleStepCopy {
            done: false,
            description: "Clinch checks Tailscale once Remote Control is on.".to_owned(),
            action: None,
            busy: false,
        },
        RemoteControlStatus::Starting => TailscaleStepCopy {
            done: false,
            description: "Checking Tailscale…".to_owned(),
            action: None,
            busy: true,
        },
        RemoteControlStatus::TailscaleNotInstalled => waiting(
            "Install Tailscale on this Mac and sign in.",
            Some((
                "Get Tailscale for Mac",
                ClinchSettingsPageAction::OpenUrl(TAILSCALE_MAC_DOWNLOAD_URL.to_owned()),
            )),
        ),
        RemoteControlStatus::TailscaleStopped => waiting(
            "Tailscale is installed but disconnected. Open Tailscale and connect.",
            None,
        ),
        RemoteControlStatus::TailscaleSignInRequired { action_url } => waiting(
            "Sign this Mac in to Tailscale.",
            action_url.clone().map(|url| {
                (
                    "Sign in to Tailscale",
                    ClinchSettingsPageAction::OpenUrl(url),
                )
            }),
        ),
        RemoteControlStatus::TailscaleConsentRequired { action_url } => waiting(
            "Tailscale needs one-time permission to issue this Mac's private HTTPS certificate.",
            action_url
                .clone()
                .map(|url| ("Allow in Tailscale", ClinchSettingsPageAction::OpenUrl(url))),
        ),
        RemoteControlStatus::Ready { remote_url, .. } => TailscaleStepCopy {
            done: true,
            description: format!("Private address: {remote_url}"),
            action: None,
            busy: false,
        },
        RemoteControlStatus::Error { message, retryable } => TailscaleStepCopy {
            done: false,
            description: message.clone(),
            action: retryable
                .then_some(("Try again", ClinchSettingsPageAction::RemoteControlRetry)),
            busy: false,
        },
    }
}

impl RemoteControlSetupWidget {
    fn button(
        label: impl Into<String>,
        action: ClinchSettingsPageAction,
        mouse_state: MouseStateHandle,
        appearance: &Appearance,
        variant: ButtonVariant,
        disabled: bool,
    ) -> Box<dyn Element> {
        let button = appearance
            .ui_builder()
            .button(variant, mouse_state)
            .with_text_label(label.into())
            .with_style(UiComponentStyles {
                font_size: Some(CONTENT_FONT_SIZE),
                padding: Some(Coords::default().top(6.).bottom(6.).left(12.).right(12.)),
                ..Default::default()
            });
        if disabled {
            return button.disabled().build().finish();
        }
        button
            .build()
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
            .finish()
    }

    fn dynamic_mouse_state(&self, key: String) -> MouseStateHandle {
        self.dynamic_mouse_states
            .borrow_mut()
            .entry(key)
            .or_default()
            .clone()
    }

    fn muted_text(text: impl Into<String>, appearance: &Appearance) -> Box<dyn Element> {
        Text::new(text.into(), appearance.ui_font_family(), CONTENT_FONT_SIZE)
            .with_color(appearance.theme().nonactive_ui_text_color().into())
            .finish()
    }

    fn strong_text(
        text: impl Into<String>,
        size: f32,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        Text::new_inline(text.into(), appearance.ui_font_family(), size)
            .with_style(Properties::default().weight(Weight::Semibold))
            .with_color(appearance.theme().active_ui_text_color().into())
            .finish()
    }

    /// One checklist row: a numbered badge that turns into a check mark once the step is done.
    #[cfg(feature = "local_fs")]
    fn render_step(
        number: usize,
        done: bool,
        busy: bool,
        title: &str,
        description: String,
        actions: Vec<Box<dyn Element>>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let badge_content: Box<dyn Element> = if busy {
            ConstrainedBox::new(
                Icon::Loading
                    .to_warpui_icon(theme.active_ui_text_color())
                    .finish(),
            )
            .with_width(14.)
            .with_height(14.)
            .finish()
        } else if done {
            ConstrainedBox::new(
                Icon::Check
                    .to_warpui_icon(theme.main_text_color(theme.accent()))
                    .finish(),
            )
            .with_width(14.)
            .with_height(14.)
            .finish()
        } else {
            Text::new_inline(
                number.to_string(),
                appearance.ui_font_family(),
                CONTENT_FONT_SIZE,
            )
            .with_style(Properties::default().weight(Weight::Bold))
            .with_color(theme.main_text_color(theme.surface_2()).into())
            .finish()
        };
        let badge = ConstrainedBox::new(
            Container::new(Align::new(badge_content).finish())
                .with_background(if done {
                    theme.accent()
                } else {
                    theme.surface_2()
                })
                .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
                .finish(),
        )
        .with_width(24.)
        .with_height(24.)
        .finish();

        let copy = Expanded::new(
            1.,
            Flex::column()
                .with_child(Self::strong_text(title, CONTENT_FONT_SIZE, appearance))
                .with_child(
                    Container::new(Self::muted_text(description, appearance))
                        .with_margin_top(4.)
                        .finish(),
                )
                .finish(),
        )
        .finish();

        let mut row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(badge)
            .with_child(
                Container::new(copy)
                    .with_margin_left(12.)
                    .with_margin_right(12.)
                    .finish(),
            );
        for (index, action) in actions.into_iter().enumerate() {
            row.add_child(
                Container::new(action)
                    .with_margin_left(if index == 0 { 0. } else { 8. })
                    .finish(),
            );
        }
        Container::new(row.finish())
            .with_uniform_padding(12.)
            .with_margin_bottom(4.)
            .finish()
    }

    fn render_section_header(appearance: &Appearance) -> Box<dyn Element> {
        Container::new(render_sub_header_with_description(
            appearance,
            "Remote Control (Beta)",
            "Use Clinch from your phone over your own private Tailscale network — no Clinch \
             account or hosted relay.",
        ))
        .with_border(Border::bottom(1.).with_border_fill(appearance.theme().outline()))
        .with_margin_top(16.)
        .with_margin_bottom(12.)
        .finish()
    }

    fn render_card(
        content: Box<dyn Element>,
        emphasized: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        Container::new(content)
            .with_background(if emphasized {
                theme.surface_2()
            } else {
                theme.surface_1()
            })
            .with_border(if emphasized {
                Border::all(2.).with_border_fill(theme.accent())
            } else {
                Border::all(1.).with_border_fill(theme.outline())
            })
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(9.)))
            .with_uniform_padding(16.)
            .with_margin_top(4.)
            .with_margin_bottom(12.)
            .finish()
    }

    #[cfg(feature = "local_fs")]
    fn render_pairing_error(&self, message: &str, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    ConstrainedBox::new(
                        Icon::AlertCircle
                            .to_warpui_icon(theme.ui_error_color().into())
                            .finish(),
                    )
                    .with_width(16.)
                    .with_height(16.)
                    .finish(),
                )
                .with_child(
                    Expanded::new(
                        1.,
                        Container::new(
                            Text::new(
                                message.to_owned(),
                                appearance.ui_font_family(),
                                CONTENT_FONT_SIZE,
                            )
                            .with_color(theme.active_ui_text_color().into())
                            .finish(),
                        )
                        .with_margin_left(8.)
                        .with_margin_right(8.)
                        .finish(),
                    )
                    .finish(),
                )
                .with_child(Self::button(
                    "Dismiss",
                    ClinchSettingsPageAction::RemoteControlDismissError,
                    self.dismiss_error_mouse_state.clone(),
                    appearance,
                    ButtonVariant::Text,
                    false,
                ))
                .finish(),
        )
        .with_background(theme.surface_1())
        .with_border(Border::all(1.).with_border_fill(theme.ui_error_color()))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
        .with_uniform_padding(10.)
        .with_margin_bottom(12.)
        .finish()
    }

    #[cfg(feature = "local_fs")]
    fn render_qr_card(
        invitation: &clinch_companion_protocol::PairingInvitation,
        now: DateTime<Utc>,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        let matrix = qr_matrix_for_url(&invitation.pairing_url).ok()?;
        let copy = Flex::column()
            .with_child(Self::strong_text(
                "Scan with your phone's camera",
                CONTENT_FONT_SIZE + 2.,
                appearance,
            ))
            .with_child(
                Container::new(Self::muted_text(
                    "Your phone opens Clinch in the browser and shows a short code. You'll \
                     confirm the same code here before it gets access.",
                    appearance,
                ))
                .with_margin_top(6.)
                .finish(),
            )
            .with_child(
                Container::new(Self::muted_text(
                    format!(
                        "New code in {} — it refreshes on its own.",
                        countdown(invitation.expires_at, now)
                    ),
                    appearance,
                ))
                .with_margin_top(10.)
                .finish(),
            )
            .finish();
        Some(Self::render_card(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    Container::new(Self::render_qr_matrix(&matrix))
                        .with_background(ColorU::white())
                        .with_uniform_padding(10.)
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                        .finish(),
                )
                .with_child(
                    Expanded::new(1., Container::new(copy).with_margin_left(20.).finish()).finish(),
                )
                .finish(),
            false,
            appearance,
        ))
    }

    #[cfg(feature = "local_fs")]
    fn render_pending_pairing_panel(
        &self,
        state: &RemoteControlViewState,
        now: DateTime<Utc>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let mut panel = Flex::column();
        for (index, claim) in state.pending_claims.iter().enumerate() {
            let code = Container::new(
                Text::new_inline(
                    pairing_code(&claim.public_key_fingerprint),
                    appearance.monospace_font_family(),
                    CONTENT_FONT_SIZE + 14.,
                )
                .with_style(Properties::default().weight(Weight::Bold))
                .with_color(theme.active_ui_text_color().into())
                .finish(),
            )
            .with_background(theme.surface_1())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(7.)))
            .with_padding_top(8.)
            .with_padding_bottom(8.)
            .with_padding_left(14.)
            .with_padding_right(14.)
            .finish();
            let buttons = Flex::row()
                .with_child(Self::button(
                    "Approve",
                    ClinchSettingsPageAction::RemoteControlApprove(claim.id),
                    self.dynamic_mouse_state(format!("approve-{}", claim.id)),
                    appearance,
                    ButtonVariant::Accent,
                    false,
                ))
                .with_child(
                    Container::new(Self::button(
                        "Reject",
                        ClinchSettingsPageAction::RemoteControlReject(claim.id),
                        self.dynamic_mouse_state(format!("reject-{}", claim.id)),
                        appearance,
                        ButtonVariant::Secondary,
                        false,
                    ))
                    .with_margin_left(8.)
                    .finish(),
                )
                .finish();
            panel.add_child(
                Container::new(
                    Flex::column()
                        .with_child(Self::strong_text(
                            format!(
                                "Approve {}?",
                                if claim.device_name.trim().is_empty() {
                                    platform_label(&claim.platform)
                                } else {
                                    claim.device_name.as_str()
                                }
                            ),
                            CONTENT_FONT_SIZE + 4.,
                            appearance,
                        ))
                        .with_child(
                            Container::new(Self::muted_text(
                                "Only approve if your phone shows this same code:",
                                appearance,
                            ))
                            .with_margin_top(6.)
                            .with_margin_bottom(10.)
                            .finish(),
                        )
                        .with_child(
                            Flex::row()
                                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                                .with_child(code)
                                .with_child(
                                    Container::new(Self::muted_text(
                                        format!("Expires in {}", countdown(claim.expires_at, now)),
                                        appearance,
                                    ))
                                    .with_margin_left(14.)
                                    .finish(),
                                )
                                .finish(),
                        )
                        .with_child(Container::new(buttons).with_margin_top(14.).finish())
                        .finish(),
                )
                .with_margin_top(if index == 0 { 0. } else { 16. })
                .finish(),
            );
        }
        Self::render_card(panel.finish(), true, appearance)
    }

    #[cfg(feature = "local_fs")]
    fn render_paired_devices(
        &self,
        state: &RemoteControlViewState,
        armed: Option<ArmedRemoval>,
        now: DateTime<Utc>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let mut list = Flex::column().with_child(
            Container::new(Self::strong_text(
                "Paired phones",
                CONTENT_FONT_SIZE,
                appearance,
            ))
            .with_margin_top(8.)
            .with_margin_bottom(8.)
            .finish(),
        );
        for device in &state.paired_devices {
            let armed_here = armed == Some(ArmedRemoval::Device(device.id));
            let status_dot = ConstrainedBox::new(
                Container::new(Empty::new().finish())
                    .with_background_color(if device.connected {
                        theme.ansi_fg_green()
                    } else {
                        theme.nonactive_ui_text_color().into()
                    })
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                    .finish(),
            )
            .with_width(8.)
            .with_height(8.)
            .finish();
            let name = if device.name.trim().is_empty() {
                platform_label(&device.platform).to_owned()
            } else {
                device.name.clone()
            };
            list.add_child(
                Container::new(
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(status_dot)
                        .with_child(
                            Expanded::new(
                                1.,
                                Container::new(
                                    Flex::column()
                                        .with_child(Self::strong_text(
                                            name,
                                            CONTENT_FONT_SIZE,
                                            appearance,
                                        ))
                                        .with_child(
                                            Container::new(Self::muted_text(
                                                device_activity_label(
                                                    device.connected,
                                                    device.last_seen_at,
                                                    now,
                                                ),
                                                appearance,
                                            ))
                                            .with_margin_top(2.)
                                            .finish(),
                                        )
                                        .finish(),
                                )
                                .with_margin_left(10.)
                                .finish(),
                            )
                            .finish(),
                        )
                        .with_child(Self::button(
                            if armed_here {
                                "Click again to remove"
                            } else {
                                "Remove"
                            },
                            ClinchSettingsPageAction::RemoteControlRevoke(device.id),
                            self.dynamic_mouse_state(format!("revoke-{}", device.id)),
                            appearance,
                            if armed_here {
                                ButtonVariant::Error
                            } else {
                                ButtonVariant::Secondary
                            },
                            false,
                        ))
                        .finish(),
                )
                .with_border(Border::all(1.).with_border_fill(theme.outline()))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
                .with_uniform_padding(10.)
                .with_margin_bottom(8.)
                .finish(),
            );
        }
        if state.paired_devices.len() > 1 {
            let armed_all = armed == Some(ArmedRemoval::All);
            list.add_child(Self::button(
                if armed_all {
                    "Click again to remove every phone"
                } else {
                    "Remove all phones"
                },
                ClinchSettingsPageAction::RemoteControlRevokeAll,
                self.revoke_all_mouse_state.clone(),
                appearance,
                if armed_all {
                    ButtonVariant::Error
                } else {
                    ButtonVariant::Secondary
                },
                false,
            ));
        }
        list.add_child(
            Container::new(Self::muted_text(
                "A removed phone must scan a new code to connect again.",
                appearance,
            ))
            .with_margin_top(6.)
            .with_margin_bottom(8.)
            .finish(),
        );
        list.finish()
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
                    Container::new(Self::muted_text(
                        "No Clinch account, relay, or Remote Control analytics are involved. \
                         The gateway listens only on this Mac and Tailscale privately proxies \
                         it inside your tailnet. Tailscale requires its own account and is \
                         governed by its own plan and privacy terms.",
                        appearance,
                    ))
                    .with_margin_left(8.)
                    .finish(),
                )
                .finish(),
        )
        .with_margin_top(4.)
        .finish()
    }

    #[cfg(feature = "local_fs")]
    fn render_native(
        &self,
        view: &ClinchSettingsPageView,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let state = RemoteControlService::as_ref(app).view_state().clone();
        let now = Utc::now();
        let mut content = Flex::column().with_child(Self::render_section_header(appearance));

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
            1,
            state.enabled,
            false,
            "Turn on Remote Control",
            "Starts a private companion on this Mac. Turning it off ends live phone sessions; \
             paired phones stay paired."
                .to_owned(),
            vec![enable_switch],
            appearance,
        ));

        let tailscale = tailscale_step_copy(&state.status);
        let mut tailscale_actions = Vec::new();
        if let Some((label, action)) = tailscale.action {
            tailscale_actions.push(Self::button(
                label,
                action,
                self.status_action_mouse_state.clone(),
                appearance,
                ButtonVariant::Secondary,
                false,
            ));
        }
        if let RemoteControlStatus::Ready { remote_url, .. } = &state.status {
            tailscale_actions.push(Self::button(
                if view.link_copied {
                    "Copied"
                } else {
                    "Copy link"
                },
                ClinchSettingsPageAction::RemoteControlCopyLink(remote_url.clone()),
                self.copy_link_mouse_state.clone(),
                appearance,
                ButtonVariant::Secondary,
                false,
            ));
        }
        content.add_child(Self::render_step(
            2,
            tailscale.done,
            tailscale.busy,
            "Tailscale on this Mac",
            tailscale.description,
            tailscale_actions,
            appearance,
        ));

        let pairing_active = state.active_invitation.is_some() || !state.pending_claims.is_empty();
        let pair_description = if !state.status.is_ready() {
            "Available once the steps above are done.".to_owned()
        } else if !state.pending_claims.is_empty() {
            "A phone scanned the code — confirm it below.".to_owned()
        } else {
            "Your phone needs Tailscale too, signed in to the same account.".to_owned()
        };
        let mut pair_actions = vec![Self::button(
            "Get Tailscale for iPhone",
            ClinchSettingsPageAction::OpenUrl(TAILSCALE_IOS_DOWNLOAD_URL.to_owned()),
            self.phone_download_mouse_state.clone(),
            appearance,
            ButtonVariant::Secondary,
            false,
        )];
        pair_actions.push(if pairing_active {
            Self::button(
                "Cancel",
                ClinchSettingsPageAction::RemoteControlCancelPairing,
                self.pair_mouse_state.clone(),
                appearance,
                ButtonVariant::Secondary,
                false,
            )
        } else {
            Self::button(
                "Pair a phone",
                ClinchSettingsPageAction::RemoteControlPair,
                self.pair_mouse_state.clone(),
                appearance,
                ButtonVariant::Accent,
                !state.status.is_ready(),
            )
        });
        content.add_child(Self::render_step(
            3,
            !state.paired_devices.is_empty(),
            false,
            "Pair a phone",
            pair_description,
            pair_actions,
            appearance,
        ));

        if let Some(message) = &state.pairing_error {
            content.add_child(self.render_pairing_error(message, appearance));
        }
        if !state.pending_claims.is_empty() {
            content.add_child(self.render_pending_pairing_panel(&state, now, appearance));
        } else if let Some(card) = state
            .active_invitation
            .as_ref()
            .and_then(|invitation| Self::render_qr_card(invitation, now, appearance))
        {
            content.add_child(card);
        }

        if !state.paired_devices.is_empty() {
            content.add_child(self.render_paired_devices(
                &state,
                view.armed_removal,
                now,
                appearance,
            ));
        }

        content.add_child(Self::render_privacy_note(appearance));
        content.add_child(
            Container::new(Self::button(
                "Setup and security guide",
                ClinchSettingsPageAction::OpenUrl(CLINCH_REMOTE_CONTROL_GUIDE_URL.to_owned()),
                self.guide_mouse_state.clone(),
                appearance,
                ButtonVariant::Secondary,
                false,
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
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        #[cfg(feature = "local_fs")]
        {
            self.render_native(view, appearance, app)
        }
        #[cfg(not(feature = "local_fs"))]
        {
            let _ = (view, app);
            Flex::column()
                .with_child(Self::render_section_header(appearance))
                .with_child(Self::render_card(
                    Self::muted_text(
                        "Remote Control requires the native Clinch app on macOS, which hosts \
                         the private companion.",
                        appearance,
                    ),
                    false,
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
        "clinch update updates automatic check daily github network privacy offline telemetry \
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
                "Ask GitHub at most once a day whether a signed Clinch update exists. Nothing is \
                 downloaded until you approve it, and no unique identifier or usage data is sent. Turn \
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
