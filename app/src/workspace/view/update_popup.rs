//! The small pop-up every Clinch update goes through: it asks, shows the download, then says Clinch
//! is about to quit and reopen. The Update button, the Clinch menu, Settings and banners all land
//! here, whether or not the update is already downloaded.

use pathfinder_color::ColorU;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::Fill;
use warp_core::ui::tokens::{corners_lg, corners_md, corners_pill, Tokens};
use warpui::elements::{
    Border, ConstrainedBox, Container, CrossAxisAlignment, Dismiss, Empty, Flex, Hoverable,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement as _, Rect, Stack, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::platform::Cursor;
use warpui::{AppContext, Element, SingletonEntity as _};

use crate::autoupdate::{self, AutoupdateStage, ClinchDownloadProgress, ClinchUpdateFailure};
use crate::settings::ClinchSettings;
use crate::terminal::cli_agent_sessions::restore::{
    mid_turn_panes, running_commands_outside_agents,
};
use crate::workspace::action::WorkspaceAction;

const POPUP_WIDTH: f32 = 300.;
const POPUP_PADDING: f32 = 14.;
const PROGRESS_BAR_HEIGHT: f32 = 5.;
const TITLE_SIZE: f32 = 13.;
const BODY_SIZE: f32 = 12.;
const BYTES_PER_MB: u64 = 1_000_000;

/// What the pop-up is showing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum UpdatePopupStep {
    Checking,
    CheckFailed,
    Confirm { version: String },
    Downloading { received: u64, total: u64 },
    Verifying,
    Installing,
    Failed(ClinchUpdateFailure),
}

impl UpdatePopupStep {
    /// Whether a click elsewhere closes the pop-up. Nothing has been agreed yet in these steps.
    /// Once an install is running the pop-up stays up, so the quit is never a surprise, and it
    /// doesn't get in the way of the terminal.
    fn closes_on_click_outside(&self) -> bool {
        matches!(
            self,
            Self::Checking | Self::CheckFailed | Self::Confirm { .. }
        )
    }
}

/// Decides the pop-up's step. Progress and failures of an update the user agreed to show in every
/// window; the steps before that only show where the user asked.
pub(super) fn update_popup_step(
    stage: &AutoupdateStage,
    progress: Option<ClinchDownloadProgress>,
    failure: Option<ClinchUpdateFailure>,
    opened_here: bool,
) -> Option<UpdatePopupStep> {
    if let Some(failure) = failure {
        return Some(UpdatePopupStep::Failed(failure));
    }
    match stage {
        AutoupdateStage::DownloadingUpdate => Some(match progress {
            Some(ClinchDownloadProgress::Downloading { received, total }) => {
                UpdatePopupStep::Downloading { received, total }
            }
            Some(ClinchDownloadProgress::Verifying) => UpdatePopupStep::Verifying,
            None => UpdatePopupStep::Downloading {
                received: 0,
                total: 0,
            },
        }),
        AutoupdateStage::Updating { .. } | AutoupdateStage::UpdatedPendingRestart { .. } => {
            Some(UpdatePopupStep::Installing)
        }
        _ if !opened_here => None,
        AutoupdateStage::CheckingForUpdate => Some(UpdatePopupStep::Checking),
        AutoupdateStage::UnableToCheckForUpdate => Some(UpdatePopupStep::CheckFailed),
        AutoupdateStage::UpdateAvailable { new_version, .. }
        | AutoupdateStage::UpdateReady { new_version, .. } => Some(UpdatePopupStep::Confirm {
            version: new_version.version.clone(),
        }),
        AutoupdateStage::NoUpdateAvailable
        | AutoupdateStage::UnableToUpdateToNewVersion { .. }
        | AutoupdateStage::UnableToLaunchNewVersion { .. } => None,
    }
}

/// What restarting does to the work running right now, in a sentence or two.
pub(super) fn restart_note(
    running_sessions: usize,
    running_commands: usize,
    sessions_continue: bool,
) -> String {
    let mut note = "Clinch will quit and reopen with your tabs.".to_owned();
    match (running_sessions, sessions_continue) {
        (0, _) => {}
        (1, true) => note.push_str(" 1 running session will continue."),
        (count, true) => note.push_str(&format!(" {count} running sessions will continue.")),
        (1, false) => note.push_str(" 1 running session will stop and reopen."),
        (count, false) => {
            note.push_str(&format!(" {count} running sessions will stop and reopen."))
        }
    }
    match running_commands {
        0 => {}
        1 => note.push_str(" 1 running command will stop."),
        count => note.push_str(&format!(" {count} running commands will stop.")),
    }
    note
}

/// The line under the progress bar, or `None` before the download has its first byte count.
pub(super) fn download_status(received: u64, total: u64) -> Option<String> {
    (total > 0).then(|| {
        format!(
            "{}% · {} of {} MB",
            received.saturating_mul(100) / total,
            received / BYTES_PER_MB,
            total.div_ceil(BYTES_PER_MB),
        )
    })
}

/// Hover states for the pop-up's buttons, kept by the workspace between frames.
#[derive(Default)]
pub(super) struct UpdatePopupMouseStates {
    primary: MouseStateHandle,
    secondary: MouseStateHandle,
    link: MouseStateHandle,
}

/// The pop-up for this window, if it should show one.
pub(super) fn render_clinch_update_popup(
    opened_here: bool,
    mouse_states: &UpdatePopupMouseStates,
    app: &AppContext,
) -> Option<Box<dyn Element>> {
    let step = update_popup_step(
        &autoupdate::get_update_state(app),
        autoupdate::clinch_download_progress(app),
        autoupdate::clinch_update_failure(app),
        opened_here,
    )?;
    let card = render_step(&step, mouse_states, app);
    Some(if step.closes_on_click_outside() {
        Dismiss::new(card)
            .on_dismiss(|ctx, _| {
                ctx.dispatch_typed_action(WorkspaceAction::CloseClinchUpdatePopup);
            })
            // A click on the Update button underneath only closes the pop-up, rather than closing
            // it and reopening it straight away.
            .prevent_interaction_with_other_elements()
            .finish()
    } else {
        card
    })
}

/// A button in the pop-up's bottom row.
struct PopupButton {
    label: &'static str,
    action: WorkspaceAction,
}

/// What a step of the pop-up shows. Anything left out isn't drawn.
#[derive(Default)]
struct StepContent {
    title: String,
    body: Option<String>,
    /// How full the progress bar is, from 0 to 1.
    bar: Option<f32>,
    primary: Option<PopupButton>,
    secondary: Option<PopupButton>,
    /// A link at the start of the button row: its label and the page it opens.
    link: Option<(&'static str, String)>,
}

fn render_step(
    step: &UpdatePopupStep,
    mouse_states: &UpdatePopupMouseStates,
    app: &AppContext,
) -> Box<dyn Element> {
    let close = |label| PopupButton {
        label,
        action: WorkspaceAction::CloseClinchUpdatePopup,
    };
    let retry = |action| PopupButton {
        label: "Try again",
        action,
    };
    let release_page = autoupdate::clinch_release_page_url(app);
    let open_release_page = |label| release_page.clone().map(|url| (label, url));

    let StepContent {
        title,
        body,
        bar,
        primary,
        secondary,
        link,
    } = match step {
        UpdatePopupStep::Checking => StepContent {
            title: "Checking for updates…".to_owned(),
            secondary: Some(close("Close")),
            ..Default::default()
        },
        UpdatePopupStep::CheckFailed => StepContent {
            title: "Couldn't check for updates".to_owned(),
            body: Some("Check your connection and try again.".to_owned()),
            primary: Some(retry(WorkspaceAction::CheckForUpdate)),
            secondary: Some(close("Close")),
            ..Default::default()
        },
        UpdatePopupStep::Confirm { version } => StepContent {
            title: format!("Update to {version}"),
            body: Some(restart_note(
                mid_turn_panes(app).len(),
                running_commands_outside_agents(app),
                *ClinchSettings::as_ref(app).continue_sessions_after_restart,
            )),
            primary: Some(PopupButton {
                label: "Update",
                action: WorkspaceAction::StartClinchUpdate,
            }),
            secondary: Some(close("Later")),
            link: open_release_page("What's new"),
            ..Default::default()
        },
        UpdatePopupStep::Downloading { received, total } => StepContent {
            title: "Downloading update".to_owned(),
            body: Some(
                download_status(*received, *total)
                    .unwrap_or_else(|| "Starting the download…".to_owned()),
            ),
            bar: Some(if *total > 0 {
                *received as f32 / *total as f32
            } else {
                0.
            }),
            secondary: Some(PopupButton {
                label: "Cancel",
                action: WorkspaceAction::CancelClinchDownload,
            }),
            ..Default::default()
        },
        UpdatePopupStep::Verifying => StepContent {
            title: "Verifying update".to_owned(),
            body: Some("Clinch will quit and reopen when this is done.".to_owned()),
            bar: Some(1.),
            ..Default::default()
        },
        UpdatePopupStep::Installing => StepContent {
            title: "Installing update".to_owned(),
            body: Some("Clinch will quit and reopen on its own.".to_owned()),
            bar: Some(1.),
            ..Default::default()
        },
        UpdatePopupStep::Failed(ClinchUpdateFailure::Download) => StepContent {
            title: "Couldn't download the update".to_owned(),
            body: Some("Nothing was changed. Check your connection and try again.".to_owned()),
            primary: Some(retry(WorkspaceAction::StartClinchUpdate)),
            secondary: Some(close("Close")),
            ..Default::default()
        },
        UpdatePopupStep::Failed(ClinchUpdateFailure::Install) => StepContent {
            title: "Couldn't install the update".to_owned(),
            body: Some("Nothing was changed, and Clinch is still running this version.".to_owned()),
            primary: Some(retry(WorkspaceAction::StartClinchUpdate)),
            secondary: Some(close("Close")),
            link: open_release_page("Get it manually"),
            ..Default::default()
        },
        UpdatePopupStep::Failed(ClinchUpdateFailure::NotWritable) => StepContent {
            title: "Clinch can't update itself here".to_owned(),
            body: Some(
                "It doesn't have permission to replace itself where it's installed.".to_owned(),
            ),
            secondary: Some(close("Close")),
            link: open_release_page("Get it manually"),
            ..Default::default()
        },
    };

    let appearance = Appearance::as_ref(app);
    let tokens = *appearance.tokens();
    let font_family = appearance.ui_font_family();
    let mut column = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_main_axis_size(MainAxisSize::Min)
        .with_spacing(8.)
        .with_child(
            Text::new(title, font_family, TITLE_SIZE)
                .soft_wrap(true)
                .with_style(Properties::default().weight(Weight::Medium))
                .with_color(tokens.text_1)
                .finish(),
        );
    if let Some(fraction) = bar {
        column.add_child(render_progress_bar(fraction, &tokens));
    }
    if let Some(body) = body {
        column.add_child(
            Text::new(body, font_family, BODY_SIZE)
                .soft_wrap(true)
                .with_color(tokens.text_2)
                .finish(),
        );
    }
    if primary.is_some() || secondary.is_some() || link.is_some() {
        column.add_child(render_button_row(
            primary,
            secondary,
            link,
            mouse_states,
            appearance,
        ));
    }

    ConstrainedBox::new(
        Container::new(column.finish())
            .with_uniform_padding(POPUP_PADDING)
            .with_background(Fill::Solid(tokens.surface_overlay))
            .with_corner_radius(corners_lg())
            .with_border(Border::all(1.).with_border_color(tokens.hairline))
            .with_drop_shadow(tokens.overlay_shadow())
            .finish(),
    )
    .with_width(POPUP_WIDTH)
    .finish()
}

fn render_progress_bar(fraction: f32, tokens: &Tokens) -> Box<dyn Element> {
    let width = POPUP_WIDTH - 2. * POPUP_PADDING;
    let segment = |color: ColorU, width: f32| {
        ConstrainedBox::new(
            Rect::new()
                .with_background(Fill::Solid(color))
                .with_corner_radius(corners_pill())
                .finish(),
        )
        .with_width(width)
        .with_height(PROGRESS_BAR_HEIGHT)
        .finish()
    };
    let mut bar = Stack::new().with_child(segment(tokens.control_track, width));
    // Nothing filled until there is something to show: a pill can't get narrower than its height.
    if fraction > 0. {
        bar.add_child(segment(
            tokens.accent,
            (width * fraction.min(1.)).max(PROGRESS_BAR_HEIGHT),
        ));
    }
    bar.finish()
}

fn render_button_row(
    primary: Option<PopupButton>,
    secondary: Option<PopupButton>,
    link: Option<(&'static str, String)>,
    mouse_states: &UpdatePopupMouseStates,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let tokens = *appearance.tokens();
    let font_family = appearance.ui_font_family();
    let link = link.map_or_else(
        || Empty::new().finish(),
        |(label, url)| {
            Hoverable::new(mouse_states.link.clone(), move |state| {
                Text::new_inline(label, font_family, BODY_SIZE)
                    .with_color(if state.is_hovered() {
                        tokens.text_1
                    } else {
                        tokens.text_3
                    })
                    .finish()
            })
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(WorkspaceAction::OpenLink(url.clone()));
            })
            .finish()
        },
    );

    let mut buttons = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_size(MainAxisSize::Min)
        .with_spacing(6.);
    if let Some(button) = secondary {
        buttons.add_child(render_button(
            button,
            false,
            mouse_states.secondary.clone(),
            appearance,
        ));
    }
    if let Some(button) = primary {
        buttons.add_child(render_button(
            button,
            true,
            mouse_states.primary.clone(),
            appearance,
        ));
    }

    Container::new(
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(link)
            .with_child(buttons.finish())
            .finish(),
    )
    .with_margin_top(4.)
    .finish()
}

fn render_button(
    button: PopupButton,
    is_primary: bool,
    mouse_state: MouseStateHandle,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let tokens = *appearance.tokens();
    let font_family = appearance.ui_font_family();
    let PopupButton { label, action } = button;
    Hoverable::new(mouse_state, move |state| {
        let (background, text) = match (is_primary, state.is_hovered()) {
            (true, _) => (tokens.accent, tokens.text_on_accent),
            (false, true) => (tokens.surface_raised, tokens.text_1),
            (false, false) => (tokens.surface_field, tokens.text_1),
        };
        Container::new(
            Text::new_inline(label, font_family, BODY_SIZE)
                .with_style(Properties::default().weight(Weight::Medium))
                .with_color(text)
                .finish(),
        )
        .with_horizontal_padding(12.)
        .with_vertical_padding(6.)
        .with_background(Fill::Solid(background))
        .with_corner_radius(corners_md())
        .finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
    .finish()
}

#[cfg(test)]
#[path = "update_popup_tests.rs"]
mod tests;
