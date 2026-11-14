//! The Coordinator's start screen, after T3 Code's new thread: until its first message, a fresh
//! Coordinator shows a headline and Clinch's input box in the middle of the pane, with a picker
//! inside the box for the agent harness that runs it.
//!
//! The agent starts at once underneath, so the first message goes straight to a warm session. The
//! start screen covers only the agent's output area, so the PTY keeps its size. It steps aside for
//! anything the agent shows in place of its empty composer (a trust dialog, a picker, its `/` menu)
//! and is gone for good once a message is sent or the session has a conversation.
use std::path::Path;

use warp_core::ui::tokens::{RADIUS_LG, SPACE_4, SPACE_6, SPACE_9};
use warpui::elements::{
    Align, Border, ChildView, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
    DispatchEventResult, EventDispatchMode, EventHandler, Expanded, Fill, Flex, ParentElement as _,
    SavePosition, Stack, Text,
};
use warpui::{AppContext, Element, SingletonEntity as _, ViewContext, ViewHandle};

use super::harness_picker::{
    coordinator_choices, HarnessPicker, HarnessPickerEvent, PickerPlacement, StartChoice,
};
use super::TerminalView;
use crate::agent_resume::AgentResumeProvider;
use crate::appearance::Appearance;
use crate::settings::AISettings;
use crate::terminal::cli_agent_sessions::{
    provider_for_agent, CLIAgentSessionsModel, PromptHistoryLoadState,
};
use crate::terminal::input::{
    CLI_AGENT_RICH_INPUT_EDITOR_BOTTOM_PADDING, CLI_AGENT_RICH_INPUT_EDITOR_TOP_PADDING,
};
use crate::terminal::one_box_composer::{supports_one_box, OneBoxComposer};
use crate::workspace::WorkspaceAction;

/// The box's placeholder on the start screen.
const START_SCREEN_PLACEHOLDER: &str = "Type here";
/// The widest the headline and box get, so a wide pane keeps them readable.
const START_SCREEN_MAX_WIDTH: f32 = 760.;
const HEADLINE_FONT_SIZE: f32 = 26.;
/// Where the input's text starts inside its own padding.
const START_SCREEN_TEXT_INSET: f32 = 19.;
/// The editor's line box is a little taller than a text element's at the same line height.
const START_SCREEN_EDITOR_LINE_EXTRA: f32 = 2.;
/// Where integration tests click to open the harness picker.
pub const COORDINATOR_HARNESS_PICKER_POSITION_ID: &str = "coordinator_harness_picker";

/// A Coordinator pane's start screen.
pub(super) struct CoordinatorStart {
    /// Started without a conversation to resume, so the screen shows from the first frame. A
    /// restored Coordinator shows it only once its history proves it has none.
    known_fresh: bool,
    /// The agent has been detected, so no session afterwards means it exited.
    agent_started: bool,
    /// A message went out; the screen never comes back.
    dismissed: bool,
    /// The project named in the headline.
    project: Option<String>,
    /// Text typed on the start screen of the Coordinator this one replaced.
    pending_draft: Option<String>,
    /// The first message that starts auto-coordinate's loop, offered in the box while
    /// auto-coordinate is on.
    kickoff: Option<String>,
    /// The kickoff went into the box once; clearing it is the user's choice.
    kickoff_offered: bool,
    picker: ViewHandle<HarnessPicker>,
}

/// What the start screen's box can do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StartScreenPhase {
    /// The agent is starting, so the box can't take text yet.
    Starting,
    /// The box replaces the agent's empty composer.
    Ready,
}

impl TerminalView {
    /// Shows the start screen on this Coordinator pane until its first message.
    pub(crate) fn show_coordinator_start_screen(
        &mut self,
        provider: AgentResumeProvider,
        known_fresh: bool,
        project: Option<String>,
        draft: Option<String>,
        kickoff: Option<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        let picker = ctx.add_typed_action_view(|ctx| {
            HarnessPicker::new(
                StartChoice::Agent(provider),
                coordinator_choices(provider),
                PickerPlacement::StartScreen,
                ctx,
            )
        });
        ctx.subscribe_to_view(&picker, |me, _, event, ctx| match event {
            HarnessPickerEvent::Selected(StartChoice::Agent(provider)) => {
                // The workspace restarts the Coordinator; it does so after this update, since it
                // closes this pane.
                let draft = Some(me.input.as_ref(ctx).buffer_text(ctx))
                    .filter(|draft| !draft.trim().is_empty());
                ctx.dispatch_typed_action_deferred(WorkspaceAction::SetCoordinatorAgent {
                    provider: *provider,
                    draft,
                });
            }
            HarnessPickerEvent::Selected(StartChoice::Terminal) => {}
            HarnessPickerEvent::Closed => {
                me.redetermine_terminal_focus(ctx);
            }
        });
        self.coordinator_start = Some(CoordinatorStart {
            known_fresh,
            agent_started: false,
            dismissed: false,
            project,
            pending_draft: draft.filter(|draft| !draft.trim().is_empty()),
            kickoff,
            kickoff_offered: false,
            picker,
        });
        self.input.update(ctx, |input, ctx| {
            input.set_cli_agent_hint_override(Some(START_SCREEN_PLACEHOLDER), ctx);
        });
        ctx.notify();
    }

    /// Offers `kickoff` as the start screen's first message, replacing one offered before, or
    /// withdraws it when `None`. Text the user typed or changed is never touched.
    pub(crate) fn set_coordinator_kickoff(
        &mut self,
        kickoff: Option<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(start) = self
            .coordinator_start
            .as_mut()
            .filter(|start| !start.dismissed)
        else {
            return;
        };
        let previous = std::mem::replace(&mut start.kickoff, kickoff);
        start.kickoff_offered = false;
        if let Some(previous) = previous {
            if start.pending_draft.as_ref() == Some(&previous) {
                start.pending_draft = None;
            }
            self.withdraw_coordinator_kickoff(&previous, ctx);
        }
        self.offer_coordinator_kickoff(ctx);
    }

    /// Puts the kickoff in the start screen's box once the screen is sure to show, if the box is
    /// empty. A draft carried over from a replaced Coordinator goes first.
    pub(super) fn offer_coordinator_kickoff(&mut self, ctx: &mut ViewContext<Self>) {
        let view_id = self.view_id;
        // Before the agent starts there is no session to hold the text; it is offered then.
        if !self.coordinator_awaiting_first_message(ctx)
            || CLIAgentSessionsModel::as_ref(ctx)
                .session(view_id)
                .is_none()
        {
            return;
        }
        let Some(start) = self.coordinator_start.as_mut() else {
            return;
        };
        if start.kickoff_offered || start.pending_draft.is_some() {
            return;
        }
        let Some(kickoff) = start.kickoff.clone() else {
            return;
        };
        start.kickoff_offered = true;
        if self.has_active_cli_agent_input_session(ctx) {
            self.input.update(ctx, |input, ctx| {
                if input.buffer_text(ctx).trim().is_empty() {
                    input.replace_buffer_content(&kickoff, ctx);
                }
            });
        } else {
            // The box takes the session's draft when it opens.
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, _| {
                let empty = sessions
                    .session(view_id)
                    .is_some_and(|session| session.draft_text.is_none());
                if empty {
                    sessions.set_draft(view_id, kickoff);
                }
            });
        }
        ctx.notify();
    }

    /// Takes `kickoff` back out of the box, if it is still there unchanged.
    fn withdraw_coordinator_kickoff(&mut self, kickoff: &str, ctx: &mut ViewContext<Self>) {
        let view_id = self.view_id;
        if self.has_active_cli_agent_input_session(ctx) {
            self.input.update(ctx, |input, ctx| {
                if input.buffer_text(ctx) == kickoff {
                    input.replace_buffer_content("", ctx);
                }
            });
        } else {
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, _| {
                let unchanged = sessions
                    .session(view_id)
                    .is_some_and(|session| session.draft_text.as_deref() == Some(kickoff));
                if unchanged {
                    sessions.clear_draft(view_id);
                }
            });
        }
        ctx.notify();
    }

    /// The agent was detected: carries over a draft from the Coordinator this one replaced, and
    /// shows the harness that actually runs.
    pub(super) fn note_coordinator_agent_started(&mut self, ctx: &mut ViewContext<Self>) {
        let view_id = self.view_id;
        let Some(agent) = CLIAgentSessionsModel::as_ref(ctx)
            .session(view_id)
            .map(|session| session.agent)
        else {
            return;
        };
        let Some(start) = self.coordinator_start.as_mut() else {
            return;
        };
        start.agent_started = true;
        let carried_draft = start.pending_draft.take();
        if let Some(provider) = provider_for_agent(agent) {
            start.picker.update(ctx, |picker, ctx| {
                picker.set_choice(StartChoice::Agent(provider), ctx)
            });
        }
        match carried_draft {
            Some(draft) => {
                // The draft may be the kickoff itself, carried over from the replaced
                // Coordinator; either way it stands in for the offer.
                start.kickoff_offered = true;
                CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, _| {
                    sessions.set_draft(view_id, draft);
                });
            }
            None => self.offer_coordinator_kickoff(ctx),
        }
    }

    /// A message is going to the agent: the start screen gives way to the conversation for good.
    pub(super) fn dismiss_coordinator_start_screen(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(start) = self
            .coordinator_start
            .as_mut()
            .filter(|start| !start.dismissed)
        else {
            return;
        };
        start.dismissed = true;
        self.input.update(ctx, |input, ctx| {
            input.set_cli_agent_hint_override(None, ctx);
        });
        ctx.notify();
    }

    /// Whether this Coordinator has had no conversation yet: it started fresh, or its restored
    /// history proved empty, and nothing has been sent to it since. Its first message is the one
    /// that starts auto-coordinate's loop, with the start screen or without it.
    pub(crate) fn coordinator_has_no_conversation(&self, app: &AppContext) -> bool {
        let Some(start) = self
            .coordinator_start
            .as_ref()
            .filter(|start| !start.dismissed)
        else {
            return false;
        };
        let Some(session) = CLIAgentSessionsModel::as_ref(app).session(self.view_id) else {
            return start.known_fresh;
        };
        let has_conversation = session.prompt_count() > 0 || session.has_observed_turn_activity;
        let known_empty = start.known_fresh
            || matches!(
                session.prompt_history_load_state,
                PromptHistoryLoadState::Ready
            );
        !has_conversation && known_empty
    }

    /// Whether this Coordinator is on its start screen with no conversation yet, setting aside
    /// moments when the agent's own screen covers it. Auto-coordinate waits for the user's first
    /// message rather than starting the conversation itself.
    pub(crate) fn coordinator_awaiting_first_message(&self, app: &AppContext) -> bool {
        if !*AISettings::as_ref(app).one_box_agent_input {
            return false;
        }
        let Some(start) = self
            .coordinator_start
            .as_ref()
            .filter(|start| !start.dismissed)
        else {
            return false;
        };
        let Some(session) = CLIAgentSessionsModel::as_ref(app).session(self.view_id) else {
            return start.known_fresh && !start.agent_started;
        };
        supports_one_box(session.agent)
            && !session.is_remote()
            && session.should_auto_toggle_input
            && self.coordinator_has_no_conversation(app)
    }

    /// Whether this frame shows the start screen, and in which phase.
    pub(super) fn coordinator_start_screen_phase(
        &self,
        one_box_composer: Option<OneBoxComposer>,
        app: &AppContext,
    ) -> Option<StartScreenPhase> {
        if !self.coordinator_awaiting_first_message(app)
            || self.one_box.native_screen
            // The agent's command menu shows over the conversation area.
            || self.has_one_box_command()
        {
            return None;
        }
        let has_session = CLIAgentSessionsModel::as_ref(app)
            .session(self.view_id)
            .is_some();
        Some(if has_session && one_box_composer.is_some() {
            StartScreenPhase::Ready
        } else {
            StartScreenPhase::Starting
        })
    }

    /// Lays the start screen over the agent's output area. Clicks and scrolls stop at it.
    pub(super) fn with_coordinator_start_screen(
        &self,
        output_area: Box<dyn Element>,
        phase: StartScreenPhase,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let Some(start) = self.coordinator_start.as_ref() else {
            return output_area;
        };
        let headline = match &start.project {
            Some(project) => format!("What should we work on in {project}?"),
            None => "What should we work on?".to_owned(),
        };
        let field = match phase {
            StartScreenPhase::Ready => self.render_input(),
            StartScreenPhase::Starting => {
                let agent = start.picker.as_ref(app).choice().name();
                start_screen_note(format!("Starting {agent}…"), app)
            }
        };
        with_start_screen(
            output_area,
            headline,
            field,
            &start.picker,
            COORDINATOR_HARNESS_PICKER_POSITION_ID,
            None,
            app,
        )
    }
}

/// The project a start screen's headline names: the folder's name, or none for the home folder,
/// where a coordinator or tab outside any repository works.
pub(crate) fn start_screen_project_name(directory: &Path) -> Option<String> {
    if dirs::home_dir().is_some_and(|home| home == directory) {
        return None;
    }
    directory
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
}

/// A line of dim text in the start screen's box, laid out like the input's own placeholder line
/// so the box keeps its size and the text its place when the input arrives.
pub(super) fn start_screen_note(text: String, app: &AppContext) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    Container::new(
        Text::new_inline(
            text,
            appearance.monospace_font_family(),
            appearance.monospace_font_size(),
        )
        .with_line_height_ratio(appearance.line_height_ratio())
        .with_color(appearance.tokens().text_3)
        .finish(),
    )
    .with_padding_top(CLI_AGENT_RICH_INPUT_EDITOR_TOP_PADDING + START_SCREEN_EDITOR_LINE_EXTRA)
    .with_padding_bottom(
        CLI_AGENT_RICH_INPUT_EDITOR_BOTTOM_PADDING + START_SCREEN_EDITOR_LINE_EXTRA,
    )
    .with_padding_left(START_SCREEN_TEXT_INSET)
    .finish()
}

/// Lays a start screen over `output_area`: `headline`, and below it a box holding `field` with
/// `picker` at its right edge, saved at `picker_position_id`, and `status` under the box. Clicks and
/// scrolls stop at the screen.
pub(super) fn with_start_screen(
    output_area: Box<dyn Element>,
    headline: String,
    field: Box<dyn Element>,
    picker: &ViewHandle<HarnessPicker>,
    picker_position_id: &'static str,
    status: Option<Box<dyn Element>>,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let tokens = appearance.tokens();
    let headline = Text::new_inline(headline, appearance.ui_font_family(), HEADLINE_FONT_SIZE)
        .with_color(tokens.text_1)
        .finish();
    let picker = SavePosition::new(ChildView::new(picker).finish(), picker_position_id).finish();
    let field_row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(Expanded::new(1., field).finish())
        .with_child(Container::new(picker).with_margin_right(SPACE_6).finish())
        .finish();
    let card = Container::new(field_row)
        .with_vertical_padding(SPACE_4)
        .with_background(Fill::Solid(tokens.surface_field))
        .with_border(Border::all(1.).with_border_fill(appearance.theme().outline()))
        .with_corner_radius(CornerRadius::with_all(RADIUS_LG))
        .finish();
    let mut column = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(headline)
        .with_child(Container::new(card).with_margin_top(SPACE_9).finish());
    if let Some(status) = status {
        column.add_child(Container::new(status).with_margin_top(SPACE_4).finish());
    }
    let screen = Container::new(
        Align::new(
            Container::new(
                ConstrainedBox::new(column.finish())
                    .with_max_width(START_SCREEN_MAX_WIDTH)
                    .finish(),
            )
            .with_uniform_padding(SPACE_9)
            // A little above center, where the eye lands first.
            .with_margin_bottom(SPACE_9 * 2.)
            .finish(),
        )
        .finish(),
    )
    .with_background(appearance.theme().background())
    .finish();
    let screen = EventHandler::new(screen)
        .on_left_mouse_down(|_, _, _| DispatchEventResult::StopPropagation)
        .on_right_mouse_down(|_, _, _| DispatchEventResult::StopPropagation)
        .on_scroll_wheel(|_, _, _, _| DispatchEventResult::StopPropagation)
        .finish();
    Stack::new()
        .with_event_dispatch_mode(EventDispatchMode::Waterfall)
        .with_child(output_area)
        .with_child(screen)
        .finish()
}
