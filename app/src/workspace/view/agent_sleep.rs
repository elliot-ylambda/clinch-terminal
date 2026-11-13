//! Putting agent tabs to sleep and waking them, at the level of tabs and projects.
//!
//! A pane knows whether its own agent may be stopped (`terminal/view/agent_sleep.rs`). This file
//! adds what only the project knows: which tab is the Coordinator, which one the user is looking
//! at, and how long each has sat idle. It also decides what counts as the user choosing a tab,
//! which wakes it, as opposed to a tab merely ending up on screen, which does not.
use std::time::Duration;

use warpui::r#async::Timer;
use warpui::{AppContext, EntityId, SingletonEntity, ViewContext, ViewHandle};

use super::Workspace;
use crate::pane_group::PaneGroup;
use crate::settings::ClinchSettings;
use crate::terminal::view::agent_sleep::SleepBlocker;
use crate::terminal::TerminalView;
use crate::view_components::DismissibleToast;
use crate::workspace::WorkspaceRegistry;

/// How often each project looks for agent tabs that have been idle long enough to sleep.
const IDLE_CHECK_INTERVAL: Duration = Duration::from_secs(60);
/// How long a chosen tab must stay chosen before it wakes. Stepping past a row of asleep tabs
/// with the keyboard then wakes only the one the user stops on.
const SELECT_WAKE_DWELL: Duration = Duration::from_millis(700);

/// What asking some tabs to sleep came to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SleepSummary {
    /// Sessions whose agents were asked to stop.
    pub(crate) slept: usize,
    /// Sessions left running, and why.
    pub(crate) kept_awake: Vec<SleepBlocker>,
}

impl SleepSummary {
    fn add(&mut self, other: SleepSummary) {
        self.slept += other.slept;
        self.kept_awake.extend(other.kept_awake);
    }
}

/// The short confirmation shown after sleeping several tabs at once.
pub(crate) fn bulk_sleep_message(summary: &SleepSummary) -> String {
    // A tab that was already asleep, or has no agent, was never a candidate.
    let busy = summary
        .kept_awake
        .iter()
        .filter(|blocker| !matches!(blocker, SleepBlocker::AlreadyAsleep | SleepBlocker::NoAgent))
        .count();
    let slept = match summary.slept {
        0 => "No idle sessions to put to sleep".to_owned(),
        1 => "Put 1 session to sleep".to_owned(),
        count => format!("Put {count} sessions to sleep"),
    };
    match busy {
        0 => slept,
        1 => format!("{slept}. 1 stayed awake because it's in use."),
        count => format!("{slept}. {count} stayed awake because they're in use."),
    }
}

/// Why a single tab the user asked to sleep stayed awake, as a sentence.
pub(crate) fn tab_stayed_awake_message(blocker: SleepBlocker) -> String {
    format!(
        "This session can't sleep yet: it {}.",
        blocker.explanation()
    )
}

/// How long an agent tab must sit idle before it sleeps on its own, or `None` when that is off.
pub(crate) fn idle_sleep_threshold(hours: u32) -> Option<Duration> {
    (hours > 0).then(|| Duration::from_secs(u64::from(hours) * 60 * 60))
}

impl Workspace {
    /// Whether tab `index` is the one on screen in this window.
    fn is_tab_on_screen(&self, index: usize, ctx: &ViewContext<Self>) -> bool {
        index == self.active_tab_index
            && WorkspaceRegistry::as_ref(ctx).is_active(ctx.window_id(), ctx.handle().id())
    }

    /// Puts one agent of tab `index` to sleep, if the tab and the agent allow it. The reasons are
    /// checked in the order that says most: the Coordinator first, then whatever the agent itself
    /// is doing, and only then that its tab happens to be on screen.
    pub(crate) fn sleep_agent_in_tab(
        &mut self,
        index: usize,
        terminal: &ViewHandle<TerminalView>,
        direct_request: bool,
        ctx: &mut ViewContext<Self>,
    ) -> Result<(), SleepBlocker> {
        let tab = self.tabs.get(index).ok_or(SleepBlocker::NoAgent)?;
        if tab.coordinator {
            return Err(SleepBlocker::Coordinator);
        }
        let group = tab.pane_group.clone();
        if let Some(blocker) = group
            .as_ref(ctx)
            .agent_sleep_blocker(terminal, direct_request, ctx)
        {
            return Err(blocker);
        }
        if !direct_request && self.is_tab_on_screen(index, ctx) {
            return Err(SleepBlocker::Visible);
        }
        group.update(ctx, |group, ctx| {
            group.sleep_agent(terminal, direct_request, ctx)
        })
    }

    /// Puts every agent in tab `index` to sleep that may be. `direct_request` is the user asking
    /// for this tab by name, which is allowed even for the tab they are looking at. `min_idle`
    /// skips agents used more recently than that.
    pub(crate) fn sleep_tab(
        &mut self,
        index: usize,
        direct_request: bool,
        min_idle: Option<Duration>,
        ctx: &mut ViewContext<Self>,
    ) -> SleepSummary {
        let mut summary = SleepSummary::default();
        let Some(group) = self.tabs.get(index).map(|tab| tab.pane_group.clone()) else {
            return summary;
        };
        for terminal in group.as_ref(ctx).agent_terminal_views(ctx) {
            if min_idle.is_some_and(|min_idle| terminal.as_ref(ctx).agent_idle_for() < min_idle) {
                continue;
            }
            match self.sleep_agent_in_tab(index, &terminal, direct_request, ctx) {
                Ok(()) => summary.slept += 1,
                Err(blocker) => summary.kept_awake.push(blocker),
            }
        }
        summary
    }

    /// Sleeps every agent tab in this project that may sleep and has been idle for `min_idle`.
    pub(crate) fn sleep_idle_tabs(
        &mut self,
        min_idle: Option<Duration>,
        ctx: &mut ViewContext<Self>,
    ) -> SleepSummary {
        let mut summary = SleepSummary::default();
        for index in 0..self.tabs.len() {
            summary.add(self.sleep_tab(index, false, min_idle, ctx));
        }
        summary
    }

    /// The tab menu's Sleep: this one tab, with a word about why if it stays awake.
    pub(super) fn sleep_tab_from_menu(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        let summary = self.sleep_tab(index, true, None, ctx);
        if summary.slept == 0 {
            if let Some(blocker) = summary.kept_awake.first() {
                self.show_sleep_toast(tab_stayed_awake_message(*blocker), ctx);
            }
        }
    }

    /// The tab menu's bulk commands: every idle agent tab in this project, or in every project of
    /// every window, with a short count of what happened.
    pub(super) fn sleep_idle_tabs_from_menu(
        &mut self,
        every_project: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        let mut summary = self.sleep_idle_tabs(None, ctx);
        if every_project {
            let this_project = ctx.handle().id();
            let others: Vec<ViewHandle<Workspace>> = WorkspaceRegistry::as_ref(ctx)
                .all_workspaces(ctx)
                .into_iter()
                .map(|(_, workspace)| workspace)
                .filter(|workspace| workspace.id() != this_project)
                .collect();
            for workspace in others {
                summary.add(
                    workspace.update(ctx, |workspace, ctx| workspace.sleep_idle_tabs(None, ctx)),
                );
            }
        }
        self.show_sleep_toast(bulk_sleep_message(&summary), ctx);
    }

    fn show_sleep_toast(&mut self, message: String, ctx: &mut ViewContext<Self>) {
        self.toast_stack.update(ctx, |toast_stack, ctx| {
            toast_stack.add_ephemeral_toast(DismissibleToast::default(message), ctx);
        });
    }

    /// Wakes every asleep agent in tab `index`. Returns how many it started.
    pub(crate) fn wake_tab(&mut self, index: usize, ctx: &mut ViewContext<Self>) -> usize {
        let Some(group) = self.tabs.get(index).map(|tab| tab.pane_group.clone()) else {
            return 0;
        };
        wake_agents(&group, ctx)
    }

    /// The user chose the tab that is now on screen: by clicking it, or with a tab shortcut. If
    /// it is asleep it wakes once the choice has held for a moment.
    pub(super) fn wake_chosen_tab_soon(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(group) = self
            .tabs
            .get(self.active_tab_index)
            .map(|tab| tab.pane_group.clone())
        else {
            return;
        };
        if group.as_ref(ctx).asleep_agent_panes(ctx).is_empty() {
            return;
        }
        let chosen = group.id();
        ctx.spawn(Timer::after(SELECT_WAKE_DWELL), move |me, _, ctx| {
            me.wake_tab_if_still_chosen(chosen, ctx);
        });
    }

    /// Wakes the tab the user chose, unless they have moved on since. Returns how many agents it
    /// started.
    pub(crate) fn wake_tab_if_still_chosen(
        &mut self,
        chosen: EntityId,
        ctx: &mut ViewContext<Self>,
    ) -> usize {
        let index = self.active_tab_index;
        let still_chosen = self
            .tabs
            .get(index)
            .is_some_and(|tab| tab.pane_group.id() == chosen)
            && self.is_tab_on_screen(index, ctx);
        if still_chosen {
            self.wake_tab(index, ctx)
        } else {
            0
        }
    }

    /// Starts the once-a-minute look for tabs that have been idle long enough to sleep.
    pub(super) fn schedule_idle_sleep_check(ctx: &mut ViewContext<Self>) {
        ctx.spawn(Timer::after(IDLE_CHECK_INTERVAL), |me, _, ctx| {
            me.run_idle_sleep_check(ctx);
            Self::schedule_idle_sleep_check(ctx);
        });
    }

    /// One pass of automatic sleep. An agent that is busy right now restarts its idle clock here,
    /// because nothing else may report the moment a turn without hooks ends.
    pub(crate) fn run_idle_sleep_check(&mut self, ctx: &mut ViewContext<Self>) -> SleepSummary {
        let groups: Vec<ViewHandle<PaneGroup>> =
            self.tabs.iter().map(|tab| tab.pane_group.clone()).collect();
        for group in &groups {
            for terminal in group.as_ref(ctx).agent_terminal_views(ctx) {
                if agent_is_in_use(&terminal, ctx) {
                    terminal.update(ctx, |view, _| view.touch_agent_activity());
                }
            }
        }
        // The tab on screen counts as in use for as long as it stays there.
        if let Some(group) = groups
            .get(self.active_tab_index)
            .filter(|_| self.is_tab_on_screen(self.active_tab_index, ctx))
        {
            for terminal in group.as_ref(ctx).agent_terminal_views(ctx) {
                terminal.update(ctx, |view, _| view.touch_agent_activity());
            }
        }
        let hours = *ClinchSettings::as_ref(ctx).sleep_idle_agent_tabs_after_hours;
        match idle_sleep_threshold(hours) {
            Some(min_idle) => self.sleep_idle_tabs(Some(min_idle), ctx),
            None => SleepSummary::default(),
        }
    }
}

/// Whether an agent is doing something or being waited on, as opposed to sitting idle.
fn agent_is_in_use(terminal: &ViewHandle<TerminalView>, ctx: &AppContext) -> bool {
    matches!(
        terminal.as_ref(ctx).agent_sleep_blocker(ctx),
        Some(
            SleepBlocker::Working
                | SleepBlocker::WaitingOnYou
                | SleepBlocker::BackgroundWork
                | SleepBlocker::Draft
                | SleepBlocker::QueuedMessage
                | SleepBlocker::InUseElsewhere
                | SleepBlocker::Restoring
        )
    )
}

/// Wakes every asleep agent in a tab. Returns how many it started.
pub(crate) fn wake_agents(group: &ViewHandle<PaneGroup>, ctx: &mut AppContext) -> usize {
    group
        .as_ref(ctx)
        .agent_terminal_views(ctx)
        .into_iter()
        .filter(|terminal| terminal.update(ctx, |view, ctx| view.wake_agent(ctx)))
        .count()
}

#[cfg(test)]
#[path = "agent_sleep_tests.rs"]
mod tests;
