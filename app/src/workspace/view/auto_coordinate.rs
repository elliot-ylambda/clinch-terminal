//! Auto-coordinate: the project coordinator acting on its own.
//!
//! When a session in the project finishes a turn or starts waiting on the user, Clinch wakes the
//! coordinator with a one-line update so it can run the tactics the user switched on (naming new
//! sessions, spotting conflicts, lining up merges and releases, offering to clean up). Updates are
//! batched and spaced out, because each wakeup spends a turn of the user's own Claude or Codex
//! plan. A wakeup is typed only when the coordinator is idle and its composer is empty; anything
//! else, including the user typing, holds it for a later retry rather than interrupting.
//!
//! The coordinator's instructions for these wakeups live in the managed `clinch-coordinate` skill,
//! so improving them never requires restarting a coordinator.

use std::time::Duration;

use instant::Instant;
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use settings::Setting as _;
use warp_core::ui::icons::Icon;
use warp_core::ui::theme::Fill;
use warp_core::ui::tokens::{corners_md, corners_pill, Tokens};
use warpui::elements::{
    ChildAnchor, ConstrainedBox, Container, CrossAxisAlignment, Expanded, Flex, Hoverable,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning, ParentAnchor,
    ParentElement, ParentOffsetBounds, Rect, SavePosition, Stack, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::platform::Cursor;
use warpui::r#async::Timer;
use warpui::ui_components::components::UiComponent;
use warpui::ui_components::switch::SwitchStateHandle;
use warpui::{AppContext, Element, EntityId, SingletonEntity, ViewContext};

use super::vertical_tabs::SidebarPopover;
use super::Workspace;
use crate::agent_resume::AUTO_COORDINATE_MARKER;
use crate::appearance::Appearance;
use crate::settings::ClinchSettings;
use crate::terminal::cli_agent_sessions::{
    CLIAgentSessionStatus, CLIAgentSessionsModel, CLIAgentSessionsModelEvent,
};
use crate::workspace::action::WorkspaceAction;

/// Updates that arrive close together go out as one wakeup.
const BATCH_WINDOW: Duration = Duration::from_secs(5);
/// The least time between two automatic wakeups.
const MIN_WAKE_INTERVAL: Duration = Duration::from_secs(120);
/// How soon a wakeup that is due but held (coordinator busy, user typing) is tried again.
const RETRY_INTERVAL: Duration = Duration::from_secs(10);
/// Session titles are quoted in the wakeup; long ones are cut.
const MAX_TITLE_CHARS: usize = 60;

/// What the coordinator may do on its own. Each one is a global default the user switches in the
/// coordinator's settings popover.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tactic {
    Watch,
    NameSessions,
    SpotConflicts,
    LineUpShipping,
    CleanUp,
}

impl Tactic {
    pub(crate) const ALL: [Tactic; 5] = [
        Tactic::Watch,
        Tactic::NameSessions,
        Tactic::SpotConflicts,
        Tactic::LineUpShipping,
        Tactic::CleanUp,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Tactic::Watch => "Watch and report",
            Tactic::NameSessions => "Name new sessions",
            Tactic::SpotConflicts => "Spot conflicts",
            Tactic::LineUpShipping => "Line up shipping",
            Tactic::CleanUp => "Clean up finished work",
        }
    }

    pub(crate) fn description(self) -> &'static str {
        match self {
            Tactic::Watch => "Tells you when sessions finish, get stuck, or need you.",
            Tactic::NameSessions => {
                "Gives a session a short name after its first turn. Never renames a tab you named."
            }
            Tactic::SpotConflicts => "Warns when two sessions change the same files or branch.",
            Tactic::LineUpShipping => {
                "One merge or release at a time, in a sensible order. Asks before each one."
            }
            Tactic::CleanUp => "Offers to close sessions whose work has merged. Asks first.",
        }
    }

    /// How the wakeup names this tactic to the coordinator.
    fn wakeup_name(self) -> &'static str {
        match self {
            Tactic::Watch => "watch",
            Tactic::NameSessions => "name sessions",
            Tactic::SpotConflicts => "spot conflicts",
            Tactic::LineUpShipping => "line up shipping (ask first)",
            Tactic::CleanUp => "clean up (ask first)",
        }
    }

    pub(crate) fn is_enabled(self, app: &AppContext) -> bool {
        let settings = ClinchSettings::as_ref(app);
        *match self {
            Tactic::Watch => settings.coordinator_auto_watch.value(),
            Tactic::NameSessions => settings.coordinator_auto_name_sessions.value(),
            Tactic::SpotConflicts => settings.coordinator_auto_spot_conflicts.value(),
            Tactic::LineUpShipping => settings.coordinator_auto_line_up_shipping.value(),
            Tactic::CleanUp => settings.coordinator_auto_clean_up.value(),
        }
    }

    fn set_enabled(self, enabled: bool, ctx: &mut AppContext) {
        ClinchSettings::handle(ctx).update(ctx, |settings, ctx| {
            let result = match self {
                Tactic::Watch => settings.coordinator_auto_watch.set_value(enabled, ctx),
                Tactic::NameSessions => settings
                    .coordinator_auto_name_sessions
                    .set_value(enabled, ctx),
                Tactic::SpotConflicts => settings
                    .coordinator_auto_spot_conflicts
                    .set_value(enabled, ctx),
                Tactic::LineUpShipping => settings
                    .coordinator_auto_line_up_shipping
                    .set_value(enabled, ctx),
                Tactic::CleanUp => settings.coordinator_auto_clean_up.set_value(enabled, ctx),
            };
            if let Err(error) = result {
                log::warn!("could not save coordinator tactic {self:?}: {error:#}");
            }
        });
    }
}

fn enabled_tactics(app: &AppContext) -> Vec<Tactic> {
    Tactic::ALL
        .into_iter()
        .filter(|tactic| tactic.is_enabled(app))
        .collect()
}

/// Why a session is worth the coordinator's attention.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UpdateKind {
    FinishedTurn,
    NeedsYou,
}

impl UpdateKind {
    fn from_status(status: &CLIAgentSessionStatus) -> Option<Self> {
        match status {
            CLIAgentSessionStatus::Success => Some(Self::FinishedTurn),
            CLIAgentSessionStatus::Blocked { .. } => Some(Self::NeedsYou),
            CLIAgentSessionStatus::InProgress => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SessionUpdate {
    pub(crate) session: EntityId,
    pub(crate) title: String,
    pub(crate) kind: UpdateKind,
}

/// Decides when a wakeup may go out. Pure, so the batching and spacing rules are tested directly.
///
/// Updates are kept one per session, the latest winning. The first update opens a short batch
/// window; a wakeup is due once that window closes and the previous wakeup is far enough behind.
#[derive(Debug, Default)]
pub(crate) struct WakePlanner {
    pending: Vec<SessionUpdate>,
    first_pending_at: Option<Instant>,
    last_wake_at: Option<Instant>,
}

impl WakePlanner {
    pub(crate) fn record(&mut self, update: SessionUpdate, now: Instant) {
        match self
            .pending
            .iter_mut()
            .find(|pending| pending.session == update.session)
        {
            Some(pending) => *pending = update,
            None => self.pending.push(update),
        }
        self.first_pending_at.get_or_insert(now);
    }

    /// When the pending updates may go out, if there are any.
    pub(crate) fn due_at(&self) -> Option<Instant> {
        let batch_closes = self.first_pending_at? + BATCH_WINDOW;
        Some(match self.last_wake_at {
            Some(last) => batch_closes.max(last + MIN_WAKE_INTERVAL),
            None => batch_closes,
        })
    }

    /// The pending updates, if they are due. They leave the planner; hand them back with
    /// [`Self::hold`] if the coordinator cannot take them yet.
    pub(crate) fn take_due(&mut self, now: Instant) -> Option<Vec<SessionUpdate>> {
        if self.due_at()? > now {
            return None;
        }
        self.first_pending_at = None;
        Some(std::mem::take(&mut self.pending))
    }

    /// Puts back updates that could not be delivered. Anything newer for the same session wins,
    /// and they stay due, so the next attempt can send them right away.
    pub(crate) fn hold(&mut self, updates: Vec<SessionUpdate>, since: Instant) {
        for update in updates.into_iter().rev() {
            if !self
                .pending
                .iter()
                .any(|pending| pending.session == update.session)
            {
                self.pending.insert(0, update);
            }
        }
        if !self.pending.is_empty() {
            let since = since.checked_sub(BATCH_WINDOW).unwrap_or(since);
            self.first_pending_at = Some(self.first_pending_at.map_or(since, |at| at.min(since)));
        }
    }

    /// Everything pending, due or not: a run the user asked for goes out right away.
    pub(crate) fn take_all(&mut self) -> Vec<SessionUpdate> {
        self.first_pending_at = None;
        std::mem::take(&mut self.pending)
    }

    pub(crate) fn mark_woken(&mut self, now: Instant) {
        self.last_wake_at = Some(now);
    }

    pub(crate) fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.pending.clear();
        self.first_pending_at = None;
    }
}

fn quoted_title(title: &str) -> String {
    let clean = title
        .split(|ch: char| ch.is_whitespace() || ch.is_control())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .replace('"', "'");
    let mut short: String = clean.chars().take(MAX_TITLE_CHARS).collect();
    if clean.chars().count() > MAX_TITLE_CHARS {
        short.push('…');
    }
    if short.is_empty() {
        short.push_str("Untitled session");
    }
    format!("\"{short}\"")
}

/// The one-line message typed into the coordinator. It names what changed and which tactics are
/// on; the skill says what to do with it.
pub(crate) fn wakeup_text(
    updates: &[SessionUpdate],
    tactics: &[Tactic],
    requested: bool,
) -> String {
    let mut text = String::from(AUTO_COORDINATE_MARKER);
    if requested {
        text.push_str(" Run requested.");
    }
    if !updates.is_empty() {
        let changes = updates
            .iter()
            .map(|update| {
                let what = match update.kind {
                    UpdateKind::FinishedTurn => "finished a turn",
                    UpdateKind::NeedsYou => "is waiting on the user",
                };
                format!("{} {what}", quoted_title(&update.title))
            })
            .collect::<Vec<_>>()
            .join("; ");
        let count = match updates.len() {
            1 => "1 update".to_owned(),
            n => format!("{n} updates"),
        };
        text.push_str(&format!(" {count}: {changes}."));
    }
    let tactics = tactics
        .iter()
        .map(|tactic| tactic.wakeup_name())
        .collect::<Vec<_>>()
        .join(", ");
    text.push_str(&format!(
        " Tactics on: {tactics}. Follow the Auto-coordinate section of the clinch-coordinate \
         skill, and reply in one short line when nothing needs the user."
    ));
    text
}

/// What the coordinator row and popover say about auto-coordinate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum AutoCoordinateStatus {
    #[default]
    Idle,
    /// Something is due, but the coordinator is mid-turn.
    WaitingForCoordinator,
    /// Something is due, but the user is typing to the coordinator.
    WaitingForUser,
    /// The coordinator is not running a Claude Code or Codex session.
    CoordinatorUnavailable,
}

impl AutoCoordinateStatus {
    /// What holds a wakeup back, given why `local_control_agent_readiness` says the coordinator
    /// cannot take text right now.
    fn held_by(unavailable_reason: &str) -> Self {
        match unavailable_reason {
            "human_input_or_draft"
            | "saved_draft"
            | "native_composer"
            | "rich_input"
            | "recent_input"
            | "unverified_composer" => Self::WaitingForUser,
            "no_agent" | "unsupported_provider" | "remote_session" | "agent_not_writable" => {
                Self::CoordinatorUnavailable
            }
            _ => Self::WaitingForCoordinator,
        }
    }
}

/// Per-project auto-coordinate state. The on/off choice itself lives on the coordinator tab so
/// it is saved with the project.
#[derive(Default)]
pub(crate) struct AutoCoordinateState {
    planner: WakePlanner,
    timer_scheduled: bool,
    /// A run the user asked for (Run now, or switching Auto on) that has not gone out yet.
    run_requested: bool,
    pub(crate) status: AutoCoordinateStatus,
    pub(crate) last_run: Option<chrono::DateTime<chrono::Local>>,
}

impl AutoCoordinateState {
    /// Forgets queued updates and any requested run.
    fn drop_pending(&mut self) {
        self.planner.clear();
        self.run_requested = false;
    }
}

impl Workspace {
    /// Whether this project's coordinator acts on its own. A coordinator the user never switched
    /// follows the `clinch.coordinator.auto.default_on` setting.
    pub(crate) fn auto_coordinate_enabled(&self, app: &AppContext) -> bool {
        self.coordinator_tab_index().is_some_and(|index| {
            self.tabs[index]
                .auto_coordinate
                .unwrap_or(*ClinchSettings::as_ref(app).coordinator_auto_default_on)
        })
    }

    /// Claude Code and Codex sessions the coordinator is watching: every one in the project
    /// except its own.
    pub(crate) fn auto_coordinate_watched_sessions(&self, app: &AppContext) -> usize {
        let sessions = CLIAgentSessionsModel::as_ref(app);
        self.tabs
            .iter()
            .filter(|tab| !tab.coordinator)
            .flat_map(|tab| tab.pane_group.as_ref(app).terminal_views(app))
            .filter(|terminal| sessions.session(terminal.id()).is_some())
            .count()
    }

    pub(crate) fn set_auto_coordinate(&mut self, enabled: bool, ctx: &mut ViewContext<Self>) {
        let Some(index) = self.coordinator_tab_index() else {
            return;
        };
        let was_enabled = self.auto_coordinate_enabled(ctx);
        self.tabs[index].auto_coordinate = Some(enabled);
        if enabled && !was_enabled {
            // A first look right away, so turning it on visibly does something (for example,
            // naming the sessions already open) instead of waiting for the next finished turn.
            self.run_coordinator_now(ctx);
        } else if !enabled {
            self.auto_coordinate.drop_pending();
            self.auto_coordinate.status = AutoCoordinateStatus::Idle;
        }
        ctx.dispatch_global_action("workspace:save_app", ());
        ctx.notify();
    }

    pub(crate) fn set_coordinator_tactic(
        &mut self,
        tactic: Tactic,
        enabled: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        tactic.set_enabled(enabled, ctx);
        ctx.notify();
    }

    /// Records a session change for the coordinator. Called for every agent event in this
    /// project; only finished turns and sessions waiting on the user count, and never the
    /// coordinator's own.
    pub(super) fn note_session_for_auto_coordinate(
        &mut self,
        event: &CLIAgentSessionsModelEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        let CLIAgentSessionsModelEvent::StatusChanged {
            terminal_view_id,
            status,
            ..
        } = event
        else {
            return;
        };
        let Some(kind) = UpdateKind::from_status(status) else {
            return;
        };
        if !self.auto_coordinate_enabled(ctx) {
            return;
        }
        let Some(tab) = self.tabs.iter().find(|tab| {
            tab.pane_group
                .as_ref(ctx)
                .terminal_views(ctx)
                .iter()
                .any(|terminal| terminal.id() == *terminal_view_id)
        }) else {
            return;
        };
        if tab.coordinator {
            // Its own turns end constantly; they are never news to it.
            return;
        }
        let title = tab.pane_group.as_ref(ctx).display_title(ctx);
        self.auto_coordinate.planner.record(
            SessionUpdate {
                session: *terminal_view_id,
                title,
                kind,
            },
            Instant::now(),
        );
        self.schedule_auto_coordinate_attempt(ctx);
    }

    fn schedule_auto_coordinate_attempt(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(due_at) = self.auto_coordinate.planner.due_at() {
            let delay = due_at.saturating_duration_since(Instant::now());
            self.schedule_auto_coordinate_timer(delay, ctx);
        }
    }

    /// One timer at a time. When it fires it sends whatever is outstanding: due updates, a
    /// requested run, or both.
    fn schedule_auto_coordinate_timer(&mut self, delay: Duration, ctx: &mut ViewContext<Self>) {
        if self.auto_coordinate.timer_scheduled {
            return;
        }
        self.auto_coordinate.timer_scheduled = true;
        ctx.spawn(async move { Timer::after(delay).await }, |me, _, ctx| {
            me.auto_coordinate.timer_scheduled = false;
            let requested = me.auto_coordinate.run_requested;
            if requested || me.auto_coordinate.planner.has_pending() {
                me.attempt_auto_coordinate_wakeup(requested, ctx);
            }
        });
    }

    /// Runs the coordinator now with whatever has changed so far, whether or not auto-coordinate
    /// is on.
    pub(crate) fn run_coordinator_now(&mut self, ctx: &mut ViewContext<Self>) {
        self.attempt_auto_coordinate_wakeup(true, ctx);
    }

    fn attempt_auto_coordinate_wakeup(&mut self, requested: bool, ctx: &mut ViewContext<Self>) {
        let Some(coordinator_index) = self.coordinator_tab_index() else {
            // The coordinator was closed; there is no one to wake.
            self.auto_coordinate.drop_pending();
            return;
        };
        if !requested && !self.auto_coordinate_enabled(ctx) {
            self.auto_coordinate.drop_pending();
            return;
        }
        let tactics = enabled_tactics(ctx);
        if tactics.is_empty() {
            // Nothing is switched on, so there is nothing to wake it for.
            self.auto_coordinate.drop_pending();
            return;
        }
        let now = Instant::now();
        let updates = if requested {
            self.auto_coordinate.run_requested = true;
            self.auto_coordinate.planner.take_all()
        } else {
            let Some(updates) = self.auto_coordinate.planner.take_due(now) else {
                self.schedule_auto_coordinate_attempt(ctx);
                return;
            };
            updates
        };

        let Some(terminal) = self.tabs[coordinator_index]
            .pane_group
            .as_ref(ctx)
            .focused_session_view(ctx)
        else {
            self.hold_auto_coordinate_wakeup(
                AutoCoordinateStatus::CoordinatorUnavailable,
                updates,
                now,
                ctx,
            );
            return;
        };

        let (revision, unavailable) = terminal.as_ref(ctx).local_control_agent_readiness(ctx);
        if let Some(reason) = unavailable {
            if reason == "not_started" {
                // A restored coordinator waits to start until its tab is shown. Start it, the
                // same way a queued message does, and deliver once it is up.
                terminal.update(ctx, |terminal, ctx| terminal.start_deferred_agent(ctx));
            }
            self.hold_auto_coordinate_wakeup(
                AutoCoordinateStatus::held_by(reason),
                updates,
                now,
                ctx,
            );
            return;
        }

        let text = wakeup_text(&updates, &tactics, requested);
        let sent = terminal.update(ctx, |terminal, ctx| {
            terminal.local_control_send_agent_text(text, &revision, |_| {}, ctx)
        });
        if !sent {
            self.hold_auto_coordinate_wakeup(
                AutoCoordinateStatus::WaitingForCoordinator,
                updates,
                now,
                ctx,
            );
            return;
        }
        self.auto_coordinate.planner.mark_woken(now);
        self.auto_coordinate.run_requested = false;
        self.auto_coordinate.status = AutoCoordinateStatus::Idle;
        self.auto_coordinate.last_run = Some(chrono::Local::now());
        ctx.notify();
    }

    /// Puts back updates the coordinator could not take and tries again shortly.
    fn hold_auto_coordinate_wakeup(
        &mut self,
        status: AutoCoordinateStatus,
        updates: Vec<SessionUpdate>,
        now: Instant,
        ctx: &mut ViewContext<Self>,
    ) {
        self.auto_coordinate.status = status;
        self.auto_coordinate.planner.hold(updates, now);
        self.schedule_auto_coordinate_timer(RETRY_INTERVAL, ctx);
        ctx.notify();
    }

    /// One line for the popover header: what auto-coordinate is doing right now.
    pub(crate) fn auto_coordinate_summary(&self, app: &AppContext) -> String {
        if !self.auto_coordinate_enabled(app) {
            return "Off. The coordinator only acts when you ask it.".to_owned();
        }
        match self.auto_coordinate.status {
            AutoCoordinateStatus::WaitingForUser => {
                "Waiting for you to finish typing to the coordinator.".to_owned()
            }
            AutoCoordinateStatus::WaitingForCoordinator => {
                "Waiting for the coordinator to finish its turn.".to_owned()
            }
            AutoCoordinateStatus::CoordinatorUnavailable => {
                "The coordinator isn't running Claude Code or Codex.".to_owned()
            }
            AutoCoordinateStatus::Idle => {
                let watched = match self.auto_coordinate_watched_sessions(app) {
                    1 => "Watching 1 session".to_owned(),
                    n => format!("Watching {n} sessions"),
                };
                match self.auto_coordinate.last_run {
                    Some(at) => format!("{watched} · last run {}", at.format("%-I:%M %p")),
                    None => watched,
                }
            }
        }
    }
}

/// The coordinator row's Auto pill. The settings popover hangs below it, and tests click it.
pub const AUTO_COORDINATE_PILL_POSITION_ID: &str = "vertical_tabs:auto_coordinate";

/// Hover and switch states for the Auto pill and its popover.
#[derive(Default)]
pub(crate) struct AutoCoordinateUiState {
    pill: MouseStateHandle,
    switch: MouseStateHandle,
    master_switch: SwitchStateHandle,
    tactic_switches: [SwitchStateHandle; Tactic::ALL.len()],
    run_now: MouseStateHandle,
}

/// What the coordinator row needs to draw its Auto pill.
#[derive(Clone)]
pub(crate) struct AutoPillProps {
    pub(crate) enabled: bool,
    pub(crate) settings_open: bool,
    pill: MouseStateHandle,
    switch: MouseStateHandle,
}

impl AutoCoordinateUiState {
    pub(crate) fn pill_props(&self, enabled: bool, settings_open: bool) -> AutoPillProps {
        AutoPillProps {
            enabled,
            settings_open,
            pill: self.pill.clone(),
            switch: self.switch.clone(),
        }
    }
}

/// The coordinator row's Auto pill. Its dot shows whether auto-coordinate is on; clicking it opens
/// the settings popover, whose first switch turns it on or off. It is a status chip rather than a
/// second toggle, because the row's own switch already turns the coordinator itself on and off.
pub(crate) fn render_auto_controls(
    props: AutoPillProps,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let tokens = *appearance.tokens();
    let font_family = appearance.ui_font_family();
    let ui_builder = appearance.ui_builder().clone();
    let enabled = props.enabled;
    let settings_open = props.settings_open;

    let pill = Hoverable::new(props.pill, move |state| {
        let hovered = state.is_hovered();
        let active = hovered || settings_open;
        let (text, background) = if active {
            (tokens.text_1, tokens.surface_hover)
        } else {
            (tokens.text_2, ColorU::transparent_black())
        };
        let content = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(3.)
            .with_child(
                Text::new_inline("Auto".to_owned(), font_family, 11.)
                    .with_style(Properties::default().weight(Weight::Medium))
                    .with_color(text)
                    .finish(),
            )
            .with_child(
                ConstrainedBox::new(Icon::ChevronDown.to_warpui_icon(Fill::Solid(text)).finish())
                    .with_width(10.)
                    .with_height(10.)
                    .finish(),
            )
            .finish();
        let pill = Container::new(content)
            .with_horizontal_padding(7.)
            .with_vertical_padding(3.)
            .with_background(Fill::Solid(background))
            .with_corner_radius(corners_pill())
            .finish();
        // The tooltip points to the settings, so it stays hidden while they are open.
        if !hovered || settings_open {
            return pill;
        }
        let mut stack = Stack::new().with_child(pill);
        stack.add_positioned_overlay_child(
            ui_builder
                .tool_tip("Choose what auto-coordinate does".to_owned())
                .build()
                .finish(),
            OffsetPositioning::offset_from_parent(
                vec2f(0., -6.),
                ParentOffsetBounds::WindowByPosition,
                ParentAnchor::TopMiddle,
                ChildAnchor::BottomMiddle,
            ),
        );
        stack.finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(|ctx, _, _| {
        ctx.dispatch_typed_action(WorkspaceAction::ToggleSidebarPopover(
            SidebarPopover::AutoCoordinate,
        ));
    })
    .finish();

    // One switch, and it means auto-coordinate: the same one as in the settings it sits beside. The
    // coordinator tab itself is turned off from its right-click menu.
    Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(6.)
        .with_child(SavePosition::new(pill, AUTO_COORDINATE_PILL_POSITION_ID).finish())
        .with_child(super::vertical_tabs::render_row_switch(
            enabled,
            if enabled {
                "Turn auto-coordinate off"
            } else {
                "Turn auto-coordinate on"
            },
            WorkspaceAction::SetAutoCoordinate(!enabled),
            props.switch,
            appearance,
        ))
        .finish()
}

fn render_switch_row(
    title: &str,
    description: Option<&str>,
    checked: bool,
    switch: SwitchStateHandle,
    action: WorkspaceAction,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let tokens = appearance.tokens();
    let font_family = appearance.ui_font_family();
    let mut text = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_spacing(2.)
        .with_child(
            Text::new_inline(title.to_owned(), font_family, 13.)
                .with_color(tokens.text_1)
                .finish(),
        );
    if let Some(description) = description {
        text.add_child(
            Text::new(description.to_owned(), font_family, 12.)
                .soft_wrap(true)
                .with_color(tokens.text_3)
                .finish(),
        );
    }
    Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_size(MainAxisSize::Max)
        .with_spacing(12.)
        .with_child(Expanded::new(1., text.finish()).finish())
        .with_child(
            appearance
                .ui_builder()
                .switch(switch)
                .check(checked)
                .build()
                .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
                .finish(),
        )
        .finish()
}

fn render_rule(tokens: &Tokens) -> Box<dyn Element> {
    Container::new(
        ConstrainedBox::new(
            Container::new(Rect::new().finish())
                .with_background(Fill::Solid(tokens.hairline))
                .finish(),
        )
        .with_height(1.)
        .finish(),
    )
    .with_vertical_margin(10.)
    .finish()
}

/// The auto-coordinate settings popover: the project's switch, what it is doing, and the
/// tactics it may use, which apply to every project.
pub(crate) fn render_auto_coordinate_popover(
    ui: &AutoCoordinateUiState,
    workspace: &Workspace,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let tokens = *appearance.tokens();
    let font_family = appearance.ui_font_family();
    let enabled = workspace.auto_coordinate_enabled(app);

    let mut column = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_child(render_switch_row(
            "Auto-coordinate",
            None,
            enabled,
            ui.master_switch.clone(),
            WorkspaceAction::SetAutoCoordinate(!enabled),
            appearance,
        ))
        .with_child(
            Container::new(
                Text::new(workspace.auto_coordinate_summary(app), font_family, 12.)
                    .soft_wrap(true)
                    .with_color(tokens.text_2)
                    .finish(),
            )
            .with_margin_top(2.)
            .finish(),
        )
        .with_child(render_rule(&tokens))
        .with_child(
            Container::new(
                Text::new_inline("In every project".to_owned(), font_family, 11.)
                    .with_color(tokens.text_3)
                    .finish(),
            )
            .with_margin_bottom(8.)
            .finish(),
        );
    for (tactic, switch) in Tactic::ALL.into_iter().zip(ui.tactic_switches.iter()) {
        let checked = tactic.is_enabled(app);
        column.add_child(
            Container::new(render_switch_row(
                tactic.label(),
                Some(tactic.description()),
                checked,
                switch.clone(),
                WorkspaceAction::SetCoordinatorTactic {
                    tactic,
                    enabled: !checked,
                },
                appearance,
            ))
            .with_margin_bottom(10.)
            .finish(),
        );
    }
    column.add_child(render_rule(&tokens));

    let run_now = Hoverable::new(ui.run_now.clone(), move |state| {
        let content = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.)
            .with_child(
                ConstrainedBox::new(
                    Icon::Play
                        .to_warpui_icon(Fill::Solid(tokens.text_1))
                        .finish(),
                )
                .with_width(12.)
                .with_height(12.)
                .finish(),
            )
            .with_child(
                Text::new_inline("Run now".to_owned(), font_family, 12.)
                    .with_style(Properties::default().weight(Weight::Medium))
                    .with_color(tokens.text_1)
                    .finish(),
            )
            .finish();
        Container::new(content)
            .with_horizontal_padding(10.)
            .with_vertical_padding(6.)
            .with_background(Fill::Solid(if state.is_hovered() {
                tokens.surface_raised
            } else {
                tokens.surface_field
            }))
            .with_corner_radius(corners_md())
            .finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(|ctx, _, _| ctx.dispatch_typed_action(WorkspaceAction::RunCoordinatorNow))
    .finish();

    column.add_child(
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(12.)
            .with_child(
                Expanded::new(
                    1.,
                    Text::new(
                        "Runs after sessions finish a turn, at most every 2 minutes, using your \
                         Claude or Codex plan."
                            .to_owned(),
                        font_family,
                        11.,
                    )
                    .soft_wrap(true)
                    .with_color(tokens.text_3)
                    .finish(),
                )
                .finish(),
            )
            .with_child(run_now)
            .finish(),
    );

    Container::new(column.finish())
        .with_uniform_padding(8.)
        .finish()
}

#[cfg(test)]
#[path = "auto_coordinate_tests.rs"]
mod tests;
