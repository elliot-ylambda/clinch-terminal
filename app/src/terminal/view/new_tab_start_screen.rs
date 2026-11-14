//! A fresh tab's empty state. A toggle at the top switches the tab between its terminal, the
//! default, and a chat view laid out the way a Coordinator starts: a headline naming the project,
//! and Clinch's input box in the middle of the pane with a picker for the agent Enter starts there,
//! Claude Code or Codex. Escape in the chat view goes back to the terminal. The toggle's choice is
//! what later tabs open in, and the empty state ends once the tab is put to use.
//!
//! A new tab in a project works in a worktree of its own, which
//! [`crate::managed_worktrees`] gets ready in the background; its status shows under the box, or
//! under the toggle in the terminal. Anything sent before the worktree is ready, a command in the
//! terminal included, waits for it. The shell moves into the worktree as soon as it is ready.
use std::path::Path;

use settings::Setting as _;
use warp_core::ui::icons::Icon;
use warp_core::ui::theme::Fill;
use warp_core::ui::tokens::{corners_pill, SPACE_2, SPACE_4};
use warpui::elements::{
    Border, ChildView, ConstrainedBox, Container, CrossAxisAlignment, Expanded, Flex, Hoverable,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement as _, SavePosition, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::platform::Cursor;
use warpui::{
    AppContext, Element, Entity, SingletonEntity as _, TypedActionView, View, ViewContext,
    ViewHandle,
};

use super::coordinator_start_screen::{start_screen_note, with_start_screen};
use super::harness_picker::{HarnessPicker, HarnessPickerEvent, PickerPlacement, StartChoice};
use super::TerminalView;
use crate::agent_resume::AgentResumeProvider;
use crate::appearance::Appearance;
use crate::managed_worktrees::{
    release_taken_worktree, worktree_is_on_branch, ManagedWorktreesEvent, ManagedWorktreesModel,
    PreparedWorktree, Repository,
};
use crate::settings::ClinchSettings;
use crate::terminal::shell::ShellType;

#[cfg(test)]
#[path = "new_tab_start_screen_tests.rs"]
mod tests;

/// The agents the chat view's picker offers, in picker order. The terminal is the toggle's other
/// side.
const NEW_TAB_CHOICES: &[StartChoice] = &[
    StartChoice::Agent(AgentResumeProvider::Claude),
    StartChoice::Agent(AgentResumeProvider::Codex),
];
/// Where integration tests click to open the chat view's picker.
pub const NEW_TAB_START_PICKER_POSITION_ID: &str = "new_tab_start_picker";
/// Where integration tests click the toggle's two sides.
pub const NEW_TAB_TERMINAL_TOGGLE_POSITION_ID: &str = "new_tab_view_toggle:terminal";
pub const NEW_TAB_CHAT_TOGGLE_POSITION_ID: &str = "new_tab_view_toggle:chat";
const TOGGLE_FONT_SIZE: f32 = 13.;
const TOGGLE_ICON_SIZE: f32 = 14.;

/// What a fresh tab shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewTabView {
    Terminal,
    Chat,
}

impl NewTabView {
    const ALL: [NewTabView; 2] = [NewTabView::Terminal, NewTabView::Chat];

    fn label(self) -> &'static str {
        match self {
            NewTabView::Terminal => "Terminal",
            NewTabView::Chat => "Chat",
        }
    }

    fn icon(self) -> Icon {
        match self {
            NewTabView::Terminal => Icon::Terminal,
            NewTabView::Chat => Icon::MessageText,
        }
    }

    fn position_id(self) -> &'static str {
        match self {
            NewTabView::Terminal => NEW_TAB_TERMINAL_TOGGLE_POSITION_ID,
            NewTabView::Chat => NEW_TAB_CHAT_TOGGLE_POSITION_ID,
        }
    }

    /// The view new tabs open in.
    fn configured(app: &AppContext) -> Self {
        if *ClinchSettings::as_ref(app).new_tab_chat_view {
            NewTabView::Chat
        } else {
            NewTabView::Terminal
        }
    }
}

/// A fresh tab's empty state and worktree.
pub(super) struct NewTabStart {
    project: Option<String>,
    /// `None` outside a project, where the tab works where it opened.
    worktree: Option<WorktreeStatus>,
    view: NewTabView,
    toggle: ViewHandle<NewTabViewToggle>,
    picker: ViewHandle<HarnessPicker>,
    /// Sent before the worktree was ready: what to start, and the text.
    waiting: Option<(StartChoice, String)>,
    /// A command for the shell once it is ready for one.
    shell_command: Option<ShellCommand>,
    /// The command the empty state last ran, so it isn't taken for someone else's.
    ran: Option<String>,
    /// The tab is not in use yet, so the toggle shows.
    fresh: bool,
}

enum WorktreeStatus {
    Preparing,
    Ready(PreparedWorktree),
    Failed,
}

/// What the empty state leaves for the shell. It becomes a command line once the shell runs it,
/// quoted for that shell.
#[derive(Default)]
struct ShellCommand {
    /// The worktree to move into first.
    enter: Option<PreparedWorktree>,
    /// Then what to start, and the text.
    start: Option<(StartChoice, String)>,
}

impl TerminalView {
    /// Gives this new tab its empty state, opening in the view the setting names, and gets a
    /// worktree of `repository` ready for it.
    pub(crate) fn start_new_tab(
        &mut self,
        project: Option<String>,
        repository: Option<Repository>,
        ctx: &mut ViewContext<Self>,
    ) {
        let view = NewTabView::configured(ctx);
        let choice = configured_choice(&ClinchSettings::as_ref(ctx).new_tab_start_with);
        let toggle = ctx.add_typed_action_view(|_| NewTabViewToggle::new(view));
        ctx.subscribe_to_view(&toggle, |me, _, event, ctx| {
            let NewTabViewToggleEvent::Selected(view) = event;
            me.set_new_tab_view(*view, true, ctx);
        });
        let picker = ctx.add_typed_action_view(|ctx| {
            HarnessPicker::new(
                choice,
                NEW_TAB_CHOICES.to_vec(),
                PickerPlacement::StartScreen,
                ctx,
            )
        });
        ctx.subscribe_to_view(&picker, |me, picker, event, ctx| match event {
            HarnessPickerEvent::Selected(choice) => {
                picker.update(ctx, |picker, ctx| picker.set_choice(*choice, ctx));
                if let Some(setting) = choice_setting(*choice) {
                    ClinchSettings::handle(ctx).update(ctx, |settings, ctx| {
                        if let Err(err) = settings
                            .new_tab_start_with
                            .set_value(setting.to_owned(), ctx)
                        {
                            log::warn!("could not save what new tabs start: {err}");
                        }
                    });
                }
                me.sync_new_tab_input(ctx);
                me.redetermine_terminal_focus(ctx);
                ctx.notify();
            }
            HarnessPickerEvent::Closed => {
                me.redetermine_terminal_focus(ctx);
            }
        });
        let worktree = repository.map(|repository| {
            let view_id = self.view_id;
            let worktrees = ManagedWorktreesModel::handle(ctx);
            ctx.subscribe_to_model(&worktrees, move |me, _, event, ctx| {
                let ManagedWorktreesEvent::Prepared {
                    terminal_view_id,
                    result,
                } = event;
                if *terminal_view_id == view_id {
                    me.note_new_tab_worktree(result.clone(), ctx);
                }
            });
            worktrees.update(ctx, |worktrees, ctx| {
                worktrees.prepare_for(view_id, repository, ctx)
            });
            WorktreeStatus::Preparing
        });
        self.new_tab_start = Some(NewTabStart {
            project,
            worktree,
            view,
            toggle,
            picker,
            waiting: None,
            shell_command: None,
            ran: None,
            fresh: true,
        });
        self.sync_new_tab_input(ctx);
        ctx.notify();
    }

    /// Switches a fresh tab between its terminal and the chat view. `remember` makes it the view
    /// later tabs open in, as a click on the toggle does.
    pub(super) fn set_new_tab_view(
        &mut self,
        view: NewTabView,
        remember: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(start) = self.new_tab_start.as_mut().filter(|start| start.fresh) else {
            return;
        };
        start.view = view;
        start
            .toggle
            .update(ctx, |toggle, ctx| toggle.set_view(view, ctx));
        if remember {
            ClinchSettings::handle(ctx).update(ctx, |settings, ctx| {
                if let Err(err) = settings
                    .new_tab_chat_view
                    .set_value(view == NewTabView::Chat, ctx)
                {
                    log::warn!("could not save the view new tabs open in: {err}");
                }
            });
        }
        self.sync_new_tab_input(ctx);
        self.redetermine_terminal_focus(ctx);
        ctx.notify();
    }

    /// Fits the input to the empty state: on the chat view it takes Enter and Escape, with the
    /// picker's placeholder; in the terminal it holds commands until the worktree is ready.
    fn sync_new_tab_input(&mut self, ctx: &mut ViewContext<Self>) {
        let (placeholder, hold) = match self.new_tab_start.as_ref().filter(|start| start.fresh) {
            Some(start) if start.view == NewTabView::Chat => {
                (Some(placeholder(start.picker.as_ref(ctx).choice())), false)
            }
            Some(start) => (
                None,
                matches!(start.worktree, Some(WorktreeStatus::Preparing)),
            ),
            None => (None, false),
        };
        self.input.update(ctx, |input, ctx| {
            input.set_start_screen(placeholder, ctx);
            input.set_holds_commands(hold);
        });
    }

    /// The worktree is ready, or couldn't be made: the shell moves into it, and whatever was sent
    /// before starts there.
    fn note_new_tab_worktree(
        &mut self,
        result: Result<PreparedWorktree, String>,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(start) = self.new_tab_start.as_mut() else {
            return;
        };
        let waiting = start.waiting.take();
        let enter = match result {
            Ok(prepared) => {
                start.worktree = Some(WorktreeStatus::Ready(prepared.clone()));
                Some(prepared)
            }
            Err(err) => {
                log::warn!("could not get a worktree ready for a new tab: {err}");
                start.worktree = Some(WorktreeStatus::Failed);
                None
            }
        };
        if waiting.is_some() {
            self.end_new_tab_empty_state(ctx);
        } else {
            self.sync_new_tab_input(ctx);
        }
        self.run_new_tab_command(enter, waiting, ctx);
        ctx.notify();
    }

    /// Enter in the empty state: the chat view starts the picker's agent with the box's text, and
    /// the terminal runs its command. Either waits for the worktree.
    pub(super) fn submit_new_tab_start(&mut self, text: String, ctx: &mut ViewContext<Self>) {
        let Some(start) = self.new_tab_start.as_mut().filter(|start| start.fresh) else {
            return;
        };
        let choice = match start.view {
            NewTabView::Chat => start.picker.as_ref(ctx).choice(),
            NewTabView::Terminal => StartChoice::Terminal,
        };
        if choice == StartChoice::Terminal && text.trim().is_empty() {
            return;
        }
        self.input.update(ctx, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx)
        });
        if matches!(start.worktree, Some(WorktreeStatus::Preparing)) {
            start.waiting = Some((choice, text));
            ctx.notify();
            return;
        }
        self.end_new_tab_empty_state(ctx);
        self.run_new_tab_command(None, Some((choice, text)), ctx);
    }

    /// Escape on the chat view: back to the terminal, which still moves into the worktree once it
    /// is ready. Something already sent from the chat view stops waiting.
    pub(super) fn dismiss_new_tab_start_screen(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(start) = self.new_tab_start.as_mut() {
            start.waiting = None;
        }
        self.set_new_tab_view(NewTabView::Terminal, false, ctx);
    }

    /// A command started in the shell. One the empty state didn't run puts the tab to use, so the
    /// empty state ends.
    pub(super) fn note_new_tab_command_started(
        &mut self,
        command: &str,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(start) = self.new_tab_start.as_mut() else {
            return;
        };
        if start.ran.as_deref() == Some(command) {
            start.ran = None;
            if let Some(WorktreeStatus::Ready(prepared)) = &start.worktree {
                release_taken_worktree(&prepared.path);
            }
            return;
        }
        start.waiting = None;
        self.end_new_tab_empty_state(ctx);
    }

    fn end_new_tab_empty_state(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(start) = self.new_tab_start.as_mut().filter(|start| start.fresh) else {
            return;
        };
        start.fresh = false;
        self.sync_new_tab_input(ctx);
        self.redetermine_terminal_focus(ctx);
        ctx.notify();
    }

    /// Leaves the shell a command: move into `enter`, then start `start`. Joins one still waiting,
    /// so moving into the worktree is never dropped.
    fn run_new_tab_command(
        &mut self,
        enter: Option<PreparedWorktree>,
        start: Option<(StartChoice, String)>,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(new_tab) = self.new_tab_start.as_mut() {
            let command = new_tab
                .shell_command
                .get_or_insert_with(ShellCommand::default);
            command.enter = enter.or(command.enter.take());
            command.start = start.or(command.start.take());
        }
        self.run_waiting_new_tab_command(ctx);
    }

    /// Call when the shell finishes a command: running one clears the box's placeholder, and the
    /// empty state may have one waiting.
    pub(super) fn note_new_tab_block_completed(&mut self, ctx: &mut ViewContext<Self>) {
        if self.new_tab_start_screen_shows() {
            self.input
                .update(ctx, |input, ctx| input.set_zero_state_hint_text(ctx));
        }
        self.run_waiting_new_tab_command(ctx);
    }

    /// Runs the command the start screen left for the shell, once the shell is ready for one.
    /// Call when the shell finishes starting up or finishes a command.
    pub(super) fn run_waiting_new_tab_command(&mut self, ctx: &mut ViewContext<Self>) {
        if self
            .new_tab_start
            .as_ref()
            .is_none_or(|start| start.shell_command.is_none())
        {
            return;
        }
        let shell_is_idle = self.is_login_shell_bootstrapped()
            && !self
                .model
                .lock()
                .block_list()
                .active_block()
                .is_active_and_long_running()
            && !self.has_pending_command_or_awaiting_completion(ctx);
        if !shell_is_idle {
            return;
        }
        let shell = self.active_session_shell_type(ctx);
        let Some(new_tab) = self.new_tab_start.as_mut() else {
            return;
        };
        let Some(waiting) = new_tab.shell_command.take() else {
            return;
        };
        // The shell may have waited long enough for another tab to take the worktree.
        let enter = waiting.enter.as_ref().filter(|prepared| {
            let still_ours = worktree_is_on_branch(&prepared.path, &prepared.branch);
            if !still_ours {
                log::warn!(
                    "the worktree {} moved off {} before the new tab could use it",
                    prepared.path.display(),
                    prepared.branch
                );
                new_tab.worktree = Some(WorktreeStatus::Failed);
            }
            still_ours
        });
        let command = shell_command_line(
            enter.map(|prepared| prepared.path.as_path()),
            waiting.start.as_ref(),
            shell,
        );
        let Some(command) = command else {
            return;
        };
        let ran = self.input.update(ctx, |input, ctx| {
            input.execute_command_keeping_text(&command, ctx)
        });
        if ran {
            new_tab.ran = Some(command);
        } else {
            new_tab.shell_command = Some(waiting);
        }
    }

    /// Whether this frame shows the chat view's start screen, which places the input in the
    /// middle of the pane.
    pub(crate) fn new_tab_start_screen_shows(&self) -> bool {
        self.new_tab_start
            .as_ref()
            .is_some_and(|start| start.fresh && start.view == NewTabView::Chat)
    }

    /// Whether this tab is fresh, so its toggle shows.
    pub(crate) fn new_tab_empty_state_shows(&self) -> bool {
        self.new_tab_start.as_ref().is_some_and(|start| start.fresh)
    }

    /// The view a fresh tab shows, or `None` once it is in use. Tests read it.
    #[cfg(any(test, feature = "integration_tests"))]
    pub(crate) fn new_tab_view(&self) -> Option<NewTabView> {
        self.new_tab_start
            .as_ref()
            .filter(|start| start.fresh)
            .map(|start| start.view)
    }

    /// Puts the toggle above the shell's output area, with the chat view's start screen over the
    /// output area when it is chosen, or the worktree's status under the toggle in the terminal.
    pub(super) fn with_new_tab_empty_state(
        &self,
        output_area: Box<dyn Element>,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let Some(start) = self.new_tab_start.as_ref() else {
            return output_area;
        };
        let mut header = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(ChildView::new(&start.toggle).finish());
        let body = match start.view {
            NewTabView::Chat => self.with_new_tab_chat_view(start, output_area, app),
            NewTabView::Terminal => {
                let status = match &start.waiting {
                    Some((choice, text)) => Some(waiting_status(*choice, text, app)),
                    None => start
                        .worktree
                        .as_ref()
                        .map(|worktree| worktree_status(worktree, app)),
                };
                if let Some(status) = status {
                    header.add_child(Container::new(status).with_margin_top(SPACE_2).finish());
                }
                output_area
            }
        };
        let header = Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::Center)
                .with_child(header.finish())
                .finish(),
        )
        .with_padding_top(SPACE_4)
        .with_padding_bottom(SPACE_2)
        .finish();
        Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(header)
            .with_child(Expanded::new(1., body).finish())
            .finish()
    }

    /// The chat view: a headline and the input in the middle of the pane, over the output area.
    fn with_new_tab_chat_view(
        &self,
        start: &NewTabStart,
        output_area: Box<dyn Element>,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let headline = match &start.project {
            Some(project) => format!("What's next in {project}?"),
            None => "What's next?".to_owned(),
        };
        let field = match &start.waiting {
            Some((choice, _)) => start_screen_note(waiting_note(*choice), app),
            // Only the editor: the shell prompt and its chips stay off the start screen.
            None => ChildView::new(self.input.as_ref(app).editor()).finish(),
        };
        let field = Container::new(field)
            .with_horizontal_padding(SPACE_4)
            .finish();
        let status = start
            .worktree
            .as_ref()
            .map(|worktree| worktree_status(worktree, app));
        with_start_screen(
            output_area,
            headline,
            field,
            &start.picker,
            NEW_TAB_START_PICKER_POSITION_ID,
            status,
            app,
        )
    }
}

/// The toggle at the top of a fresh tab: its terminal or the chat view, as two halves of a pill.
pub struct NewTabViewToggle {
    view: NewTabView,
    mouse_states: [MouseStateHandle; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewTabViewToggleAction {
    Select(NewTabView),
}

pub enum NewTabViewToggleEvent {
    /// The user picked the side that wasn't showing.
    Selected(NewTabView),
}

impl NewTabViewToggle {
    fn new(view: NewTabView) -> Self {
        Self {
            view,
            mouse_states: Default::default(),
        }
    }

    fn set_view(&mut self, view: NewTabView, ctx: &mut ViewContext<Self>) {
        if self.view != view {
            self.view = view;
            ctx.notify();
        }
    }
}

impl Entity for NewTabViewToggle {
    type Event = NewTabViewToggleEvent;
}

impl TypedActionView for NewTabViewToggle {
    type Action = NewTabViewToggleAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        let NewTabViewToggleAction::Select(view) = *action;
        if view != self.view {
            ctx.emit(NewTabViewToggleEvent::Selected(view));
        }
    }
}

impl View for NewTabViewToggle {
    fn ui_name() -> &'static str {
        "NewTabViewToggle"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let tokens = *appearance.tokens();
        let font_family = appearance.ui_font_family();
        let outline = appearance.theme().outline();
        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(2.);
        for (view, mouse_state) in NewTabView::ALL.into_iter().zip(&self.mouse_states) {
            let selected = view == self.view;
            let segment = Hoverable::new(mouse_state.clone(), move |state| {
                let hovered = state.is_hovered();
                let color = if selected || hovered {
                    tokens.text_1
                } else {
                    tokens.text_2
                };
                let content = Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(6.)
                    .with_child(
                        ConstrainedBox::new(
                            view.icon().to_warpui_icon(Fill::Solid(color)).finish(),
                        )
                        .with_width(TOGGLE_ICON_SIZE)
                        .with_height(TOGGLE_ICON_SIZE)
                        .finish(),
                    )
                    .with_child(
                        Text::new_inline(view.label().to_owned(), font_family, TOGGLE_FONT_SIZE)
                            .with_style(Properties::default().weight(if selected {
                                Weight::Semibold
                            } else {
                                Weight::Medium
                            }))
                            .with_color(color)
                            .finish(),
                    )
                    .finish();
                let segment = Container::new(content)
                    .with_horizontal_padding(14.)
                    .with_vertical_padding(6.)
                    .with_corner_radius(corners_pill());
                if selected {
                    segment
                        .with_background(Fill::Solid(tokens.surface_raised))
                        .with_border(Border::all(1.).with_border_fill(outline))
                        .finish()
                } else if hovered {
                    segment
                        .with_background(Fill::Solid(tokens.surface_hover))
                        .finish()
                } else {
                    segment.finish()
                }
            })
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(NewTabViewToggleAction::Select(view));
            })
            .finish();
            row.add_child(SavePosition::new(segment, view.position_id()).finish());
        }
        Container::new(row.finish())
            .with_uniform_padding(3.)
            .with_background(Fill::Solid(tokens.surface_field))
            .with_border(Border::all(1.).with_border_fill(outline))
            .with_corner_radius(corners_pill())
            .finish()
    }
}

fn worktree_status(worktree: &WorktreeStatus, app: &AppContext) -> Box<dyn Element> {
    let text = match worktree {
        WorktreeStatus::Preparing => "Getting a worktree ready…".to_owned(),
        WorktreeStatus::Ready(prepared) => worktree_label(prepared),
        WorktreeStatus::Failed => {
            "Couldn't make a worktree, so this tab works in the main checkout".to_owned()
        }
    };
    status_text(text, app)
}

/// A dim status line under the box or the toggle.
fn status_text(text: String, app: &AppContext) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    Text::new_inline(text, appearance.ui_font_family(), appearance.ui_font_size())
        .with_color(appearance.tokens().text_3)
        .finish()
}

/// Under the box, once the worktree is ready: its branch and what it started from.
fn worktree_label(prepared: &PreparedWorktree) -> String {
    let folder = prepared
        .path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    if folder == prepared.branch {
        format!("Worktree {} · from {}", prepared.branch, prepared.base)
    } else {
        format!(
            "Worktree {} on {} · from {}",
            folder, prepared.branch, prepared.base
        )
    }
}

/// Under the terminal's toggle, once something was sent before the worktree was ready.
fn waiting_status(choice: StartChoice, text: &str, app: &AppContext) -> Box<dyn Element> {
    let text = match choice {
        StartChoice::Terminal => format!("Runs once the worktree is ready: {}", text.trim()),
        StartChoice::Agent(_) => waiting_note(choice),
    };
    status_text(text, app)
}

fn waiting_note(choice: StartChoice) -> String {
    match choice {
        StartChoice::Terminal => "Runs once the worktree is ready…".to_owned(),
        StartChoice::Agent(_) => format!("{} starts once the worktree is ready…", choice.name()),
    }
}

fn placeholder(choice: StartChoice) -> &'static str {
    match choice {
        StartChoice::Agent(_) => "Describe a task",
        StartChoice::Terminal => "Run a command",
    }
}

/// The agent the chat view's picker starts on: the saved one, else Claude Code.
fn configured_choice(setting: &str) -> StartChoice {
    StartChoice::Agent(
        AgentResumeProvider::from_agent_name(setting).unwrap_or(AgentResumeProvider::Claude),
    )
}

/// How the picker's choice is saved; the picker offers only agents.
fn choice_setting(choice: StartChoice) -> Option<&'static str> {
    match choice {
        StartChoice::Agent(provider) => Some(provider.as_str()),
        StartChoice::Terminal => None,
    }
}

/// The command line that moves into `enter`, then starts what `start` names. `None` when there
/// is nothing to run.
fn shell_command_line(
    enter: Option<&Path>,
    start: Option<&(StartChoice, String)>,
    shell: Option<ShellType>,
) -> Option<String> {
    let enter = enter.map(|path| format!("cd {}", quote_for(shell, &path.to_string_lossy())));
    let start = start.and_then(|(choice, text)| start_command(*choice, text, shell));
    match (enter, start) {
        (Some(enter), Some(start)) => Some(format!("{enter} && {start}")),
        (enter, start) => enter.or(start),
    }
}

/// The shell command that starts `choice` with `text`: the agent, with the text as its first
/// message, or the text itself as a command. `None` when there is nothing to run.
fn start_command(choice: StartChoice, text: &str, shell: Option<ShellType>) -> Option<String> {
    let text = text.trim();
    match choice {
        StartChoice::Terminal => (!text.is_empty()).then(|| text.to_owned()),
        StartChoice::Agent(provider) => {
            let launch = provider.launch_command_prefix(true);
            Some(if text.is_empty() {
                launch.to_owned()
            } else if text.starts_with('-') {
                // Read as a message, not an option.
                format!("{launch} -- {}", quote_for(shell, text))
            } else {
                format!("{launch} {}", quote_for(shell, text))
            })
        }
    }
}

/// `text` as one word for `shell`, newlines and all.
fn quote_for(shell: Option<ShellType>, text: &str) -> String {
    match shell {
        // In fish, a backslash escapes `\` and `'` inside single quotes.
        Some(ShellType::Fish) => {
            format!("'{}'", text.replace('\\', "\\\\").replace('\'', "\\'"))
        }
        // PowerShell ends a single-quoted string at any single quote, curly ones included, and
        // reads one doubled as the quote itself.
        Some(ShellType::PowerShell) => {
            let mut quoted = String::from("'");
            for c in text.chars() {
                if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{201b}') {
                    quoted.push(c);
                }
                quoted.push(c);
            }
            quoted.push('\'');
            quoted
        }
        Some(ShellType::Bash | ShellType::Zsh) | None => shell_words::quote(text).into_owned(),
    }
}
