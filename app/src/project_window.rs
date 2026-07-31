use std::sync::Arc;

use pathfinder_geometry::rect::RectF as GeometryRect;
use pathfinder_geometry::vector::{vec2f, Vector2F};
use uuid::Uuid;
use warp_core::ui::theme::Fill;
use warpui::accessibility::{AccessibilityContent, ActionAccessibilityContent, WarpA11yRole};
use warpui::elements::{
    Align, Border, ChildAnchor, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox,
    Container, CornerRadius, CrossAxisAlignment, Draggable, DraggableState, DropShadow, Empty,
    Flex, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning, Padding,
    ParentAnchor, ParentElement, ParentOffsetBounds, Radius, Rect, SavePosition, ScrollTarget,
    ScrollToPositionMode, ScrollbarWidth, Shrinkable, Stack, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::keymap::{BindingDescription, EditableBinding};
use warpui::platform::{Cursor, TerminationMode};
use warpui::presenter::ChildView;
use warpui::text_layout::ClipConfig;
use warpui::windowing::WindowManager;
use warpui::{
    id, AppContext, Element, Entity, EntityId, FocusContext, SingletonEntity, TypedActionView,
    View, ViewContext, ViewHandle, WindowId,
};

use crate::appearance::Appearance;
use crate::root_view::NewWorkspaceSource;
use crate::server::server_api::ServerTime;
use crate::ui_components::icons::Icon;
use crate::ui_components::{CLINCH_DONE_BLUE, CLINCH_LOGO_GREEN};
use crate::util::bindings::{self, CustomAction};
use crate::workspace::view::{
    ProjectCliAgentActivity, ProjectCliAgentCounts, ProjectCliAgentSummary, TransferredTab,
    TAB_BAR_POSITION_ID,
};
use crate::workspace::{Workspace, WorkspaceEvent, WorkspaceRegistry};
use crate::GlobalResourceHandles;

pub(crate) const ACTIVATE_PREVIOUS_PROJECT_MAC_KEY_BINDING: &str = "cmd-[";
pub(crate) const ACTIVATE_NEXT_PROJECT_MAC_KEY_BINDING: &str = "cmd-]";

pub(crate) fn init(app: &mut AppContext) {
    app.register_editable_bindings([
        EditableBinding::new(
            "project_window:new_project",
            BindingDescription::new("New Project"),
            ProjectWindowAction::Add,
        )
        .with_custom_action(CustomAction::NewProject)
        .with_context_predicate(id!(ProjectWindow::ui_name())),
        EditableBinding::new(
            "project_window:activate_previous_project",
            "Activate previous project",
            ProjectWindowAction::ActivatePrevious,
        )
        .with_group(bindings::BindingGroup::Navigation.as_str())
        .with_context_predicate(id!(ProjectWindow::ui_name()))
        .with_mac_key_binding(ACTIVATE_PREVIOUS_PROJECT_MAC_KEY_BINDING),
        EditableBinding::new(
            "project_window:activate_next_project",
            "Activate next project",
            ProjectWindowAction::ActivateNext,
        )
        .with_group(bindings::BindingGroup::Navigation.as_str())
        .with_context_predicate(id!(ProjectWindow::ui_name()))
        .with_mac_key_binding(ACTIVATE_NEXT_PROJECT_MAC_KEY_BINDING),
        EditableBinding::new(
            "project_window:cancel_project_drag",
            "Cancel project drag",
            ProjectWindowAction::CancelDrag,
        )
        .with_context_predicate(
            id!(ProjectWindow::ui_name()) & id!("ProjectWindow_ProjectDragging"),
        )
        .with_key_binding("escape"),
        EditableBinding::new(
            "project_window:close_active_project",
            "Close active project",
            ProjectWindowAction::CloseActive,
        )
        .with_context_predicate(id!(ProjectWindow::ui_name())),
        EditableBinding::new(
            "project_window:move_active_project_left",
            "Move active project left",
            ProjectWindowAction::MoveActiveLeft,
        )
        .with_group(bindings::BindingGroup::Navigation.as_str())
        .with_context_predicate(id!(ProjectWindow::ui_name())),
        EditableBinding::new(
            "project_window:move_active_project_right",
            "Move active project right",
            ProjectWindowAction::MoveActiveRight,
        )
        .with_group(bindings::BindingGroup::Navigation.as_str())
        .with_context_predicate(id!(ProjectWindow::ui_name())),
    ]);
}

/// Stable identity for a project while the application is running.
///
/// Project ordering is persisted separately, so this ID intentionally does not
/// cross launches.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ProjectId(Uuid);

impl ProjectId {
    fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

#[derive(Clone)]
struct Project {
    id: ProjectId,
    workspace: ViewHandle<Workspace>,
    mouse_state: MouseStateHandle,
    close_mouse_state: MouseStateHandle,
    draggable_state: DraggableState,
}

#[derive(Clone, Copy)]
struct ProjectDragState {
    id: ProjectId,
    original_index: usize,
    last_position: GeometryRect,
    detached_from_source_strip: bool,
    hover_target_window_id: Option<WindowId>,
}

struct ProjectAttachTarget {
    window_id: WindowId,
    insertion_index: usize,
    project_window: ViewHandle<ProjectWindow>,
}

fn project_tab_position_id(id: ProjectId) -> String {
    format!("project-tab:{}", id.0)
}

const PROJECT_TAB_STRIP_POSITION_ID: &str = "project-window:tab-strip";
const PROJECT_HEADER_DROP_TARGET_POSITION_ID: &str = "project-window:header-drop-target";
const PROJECT_TAB_CLOSE_BUTTON_SIZE: f32 = 16.;
const PROJECT_TAB_CLOSE_BUTTON_GAP: f32 = 6.;
// Preserve enough label space after the symmetric close-button reservation and
// tab chrome so short directory names do not collapse into an empty clip.
const PROJECT_TAB_MIN_WIDTH: f32 = 96.;
const PROJECT_TAB_VERTICAL_NUDGE: f32 = 2.;
const PROJECT_TAB_VERTICAL_PADDING: f32 = 6.;
const PROJECT_TAB_BORDER_WIDTH: f32 = 1.;
const PROJECT_AGENT_COUNT_BADGE_BORDER_WIDTH: f32 = 1.;
const PROJECT_AGENT_HOVER_CARD_WIDTH: f32 = 320.;
const PROJECT_AGENT_HOVER_CARD_PADDING: f32 = 10.;
const PROJECT_AGENT_HOVER_CARD_BORDER_WIDTH: f32 = 1.;
const PROJECT_AGENT_HOVER_CARD_CONTENT_WIDTH: f32 = PROJECT_AGENT_HOVER_CARD_WIDTH
    - 2. * (PROJECT_AGENT_HOVER_CARD_PADDING + PROJECT_AGENT_HOVER_CARD_BORDER_WIDTH);
const PROJECT_AGENT_HOVER_CARD_MAX_ROWS: usize = 8;

fn project_agent_hover_summary(
    tab_count: usize,
    agent_count: usize,
    counts: ProjectCliAgentCounts,
) -> String {
    let mut parts = vec![if tab_count == 1 {
        "1 open tab".to_string()
    } else {
        format!("{tab_count} open tabs")
    }];
    if agent_count > 0 {
        parts.push(if agent_count == 1 {
            "1 agent".to_string()
        } else {
            format!("{agent_count} agents")
        });
    }
    if counts.working > 0 {
        parts.push(format!("{} working", counts.working));
    }
    if counts.done > 0 {
        parts.push(format!("{} done", counts.done));
    }
    if counts.running_commands > 0 {
        parts.push(if counts.running_commands == 1 {
            "1 command running".to_string()
        } else {
            format!("{} commands running", counts.running_commands)
        });
    }
    parts.join(" · ")
}

fn render_project_agent_hover_card(
    project_title: &str,
    tab_count: usize,
    agents: &[ProjectCliAgentSummary],
    counts: ProjectCliAgentCounts,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let background = theme.tooltip_background();
    let background_fill = Fill::Solid(background);
    let main_text_color = theme.main_text_color(background_fill).into_solid();
    let sub_text_color = theme.sub_text_color(background_fill).into_solid();
    let disabled_text_color = theme.disabled_text_color(background_fill).into_solid();
    let font_family = appearance.ui_font_family();

    let header = Flex::column()
        .with_main_axis_size(MainAxisSize::Min)
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_spacing(2.)
        .with_child(
            Text::new_inline(project_title.to_string(), font_family, 12.)
                .with_clip(ClipConfig::ellipsis())
                .with_color(main_text_color)
                .with_style(Properties::default().weight(Weight::Semibold))
                .finish(),
        )
        .with_child(
            Text::new_inline(
                project_agent_hover_summary(tab_count, agents.len(), counts),
                font_family,
                10.,
            )
            .with_color(sub_text_color)
            .finish(),
        )
        .finish();

    let mut content = Flex::column()
        .with_main_axis_size(MainAxisSize::Min)
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_spacing(8.)
        .with_child(header);

    if agents.is_empty() {
        content.add_child(
            Text::new_inline(
                "No Claude Code or Codex tabs open".to_string(),
                font_family,
                11.,
            )
            .with_color(sub_text_color)
            .finish(),
        );
    } else {
        content.add_child(
            ConstrainedBox::new(Rect::new().with_background(theme.outline()).finish())
                .with_height(1.)
                .finish(),
        );

        let mut rows = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(8.);
        for summary in agents.iter().take(PROJECT_AGENT_HOVER_CARD_MAX_ROWS) {
            let icon = summary
                .agent
                .icon()
                .map(|icon| {
                    ConstrainedBox::new(icon.to_warpui_icon(Fill::Solid(main_text_color)).finish())
                        .with_width(16.)
                        .with_height(16.)
                        .finish()
                })
                .unwrap_or_else(|| Empty::new().finish());
            let activity_color = match summary.activity {
                ProjectCliAgentActivity::Working => CLINCH_LOGO_GREEN,
                ProjectCliAgentActivity::Done | ProjectCliAgentActivity::NeedsAttention => {
                    CLINCH_DONE_BLUE
                }
                ProjectCliAgentActivity::Idle => disabled_text_color,
            };
            let metadata = Flex::row()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(4.)
                .with_child(
                    ConstrainedBox::new(
                        Rect::new()
                            .with_background(Fill::Solid(activity_color))
                            .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
                            .finish(),
                    )
                    .with_width(6.)
                    .with_height(6.)
                    .finish(),
                )
                .with_child(
                    Text::new_inline(
                        format!(
                            "{} · {}",
                            summary.agent.display_name(),
                            summary.activity.label()
                        ),
                        font_family,
                        10.,
                    )
                    .with_color(sub_text_color)
                    .finish(),
                )
                .finish();
            let labels = Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_spacing(2.)
                .with_child(
                    Text::new_inline(summary.title.clone(), font_family, 11.)
                        .with_clip(ClipConfig::ellipsis())
                        .with_color(main_text_color)
                        .finish(),
                )
                .with_child(metadata)
                .finish();
            // Positioned overlays inside the horizontally scrolling project
            // strip are initially measured with an unbounded width. This row
            // contains a flexible label, so give it the hover card's known
            // inner width before Flex lays it out. Without this constraint,
            // beginning a project-tab drag can hit Flex's infinite-constraint
            // assertion and crash the app.
            rows.add_child(
                ConstrainedBox::new(
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_cross_axis_alignment(CrossAxisAlignment::Start)
                        .with_spacing(8.)
                        .with_child(Container::new(icon).with_margin_top(1.).finish())
                        .with_child(Shrinkable::new(1., labels).finish())
                        .finish(),
                )
                .with_width(PROJECT_AGENT_HOVER_CARD_CONTENT_WIDTH)
                .finish(),
            );
        }
        if agents.len() > PROJECT_AGENT_HOVER_CARD_MAX_ROWS {
            rows.add_child(
                Text::new_inline(
                    format!(
                        "+ {} more open agent tabs",
                        agents.len() - PROJECT_AGENT_HOVER_CARD_MAX_ROWS
                    ),
                    font_family,
                    10.,
                )
                .with_color(sub_text_color)
                .finish(),
            );
        }
        content.add_child(rows.finish());
    }

    ConstrainedBox::new(
        Container::new(content.finish())
            .with_background(background)
            .with_border(
                Border::all(PROJECT_AGENT_HOVER_CARD_BORDER_WIDTH)
                    .with_border_fill(theme.outline()),
            )
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
            .with_padding(Padding::uniform(PROJECT_AGENT_HOVER_CARD_PADDING))
            .with_drop_shadow(DropShadow::default())
            .finish(),
    )
    .with_width(PROJECT_AGENT_HOVER_CARD_WIDTH)
    .finish()
}

fn previous_project_index(active_index: usize, project_count: usize) -> Option<usize> {
    (project_count > 1).then(|| active_index.checked_sub(1).unwrap_or(project_count - 1))
}

fn next_project_index(active_index: usize, project_count: usize) -> Option<usize> {
    (project_count > 1).then(|| (active_index + 1) % project_count)
}

fn active_index_after_removal(
    active_index: usize,
    removed_index: usize,
    remaining_count: usize,
) -> Option<usize> {
    if remaining_count == 0 {
        None
    } else if active_index > removed_index {
        Some(active_index - 1)
    } else if active_index == removed_index {
        Some(removed_index.min(remaining_count - 1))
    } else {
        Some(active_index)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CloseProjectDecision {
    NotFound,
    CloseWindow,
    Project(usize),
}

fn close_project_decision(
    project_count: usize,
    project_index: Option<usize>,
) -> CloseProjectDecision {
    match project_index {
        None => CloseProjectDecision::NotFound,
        Some(_) if project_count == 1 => CloseProjectDecision::CloseWindow,
        Some(index) => CloseProjectDecision::Project(index),
    }
}

/// Owns all of the project workspaces hosted by one physical Clinch window.
///
/// Only the active workspace is rendered, but inactive workspace handles stay
/// strongly owned here so their terminals and agent sessions remain live.
pub(crate) struct ProjectWindow {
    window_id: WindowId,
    projects: Vec<Project>,
    active_project_index: usize,
    project_tab_scroll_state: ClippedScrollStateHandle,
    active_drag: Option<ProjectDragState>,
    incoming_drag_insertion_index: Option<usize>,
    new_project_mouse_state: MouseStateHandle,
    global_resource_handles: GlobalResourceHandles,
    server_time: Option<Arc<ServerTime>>,
}

impl ProjectWindow {
    fn subscribe_to_workspace_metadata(
        workspace: &ViewHandle<Workspace>,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.subscribe_to_view(workspace, |me: &mut Self, _, event, ctx| match event {
            WorkspaceEvent::ProjectMetadataChanged => {
                // The strip is mounted in ProjectWindow's own render for the
                // horizontal header, but inside the *active* workspace's tab
                // bar for vertical tabs. A background project's change must
                // invalidate the active workspace too, or its pill stays
                // stale until something else redraws the header.
                ctx.notify();
                if let Some(active) = me.projects.get(me.active_project_index) {
                    active.workspace.update(ctx, |_, ctx| ctx.notify());
                }
            }
        });
    }

    pub(crate) fn new(
        global_resource_handles: GlobalResourceHandles,
        server_time: Option<Arc<ServerTime>>,
        workspace_sources: Vec<NewWorkspaceSource>,
        active_project_index: usize,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        debug_assert!(!workspace_sources.is_empty());

        let projects = workspace_sources
            .into_iter()
            .map(|source| {
                let workspace = ctx.add_typed_action_view(|ctx| {
                    Workspace::new(
                        global_resource_handles.clone(),
                        server_time.clone(),
                        source,
                        ctx,
                    )
                });
                Self::subscribe_to_workspace_metadata(&workspace, ctx);
                Project {
                    id: ProjectId::new(),
                    workspace,
                    mouse_state: Default::default(),
                    close_mouse_state: Default::default(),
                    draggable_state: Default::default(),
                }
            })
            .collect::<Vec<_>>();
        let active_project_index = active_project_index.min(projects.len().saturating_sub(1));

        let project_window = Self {
            window_id: ctx.window_id(),
            projects,
            active_project_index,
            project_tab_scroll_state: Default::default(),
            active_drag: None,
            incoming_drag_insertion_index: None,
            new_project_mouse_state: Default::default(),
            global_resource_handles,
            server_time,
        };
        project_window
            .project_tab_scroll_state
            .scroll_to_position(ScrollTarget {
                position_id: project_tab_position_id(
                    project_window.projects[active_project_index].id,
                ),
                mode: ScrollToPositionMode::FullyIntoView,
            });
        project_window.register_active_workspace(ctx);
        project_window
            .active_workspace()
            .update(ctx, |workspace, ctx| {
                workspace.handle_project_activated(ctx)
            });
        project_window
    }

    pub(crate) fn active_workspace(&self) -> ViewHandle<Workspace> {
        self.projects[self.active_project_index].workspace.clone()
    }

    fn accessibility_summary(&self, app: &AppContext) -> String {
        let Some(project) = self.projects.get(self.active_project_index) else {
            return "No projects".to_string();
        };
        let workspace = project.workspace.as_ref(app);
        let agent_counts = workspace.project_cli_agent_counts(app);
        let working = match agent_counts.working {
            0 => String::new(),
            1 => ", 1 agent working".to_string(),
            count => format!(", {count} agents working"),
        };
        let done = match agent_counts.done {
            0 => String::new(),
            1 => ", 1 agent done".to_string(),
            count => format!(", {count} agents done"),
        };
        let running_commands = match agent_counts.running_commands {
            0 => String::new(),
            1 => ", 1 command running".to_string(),
            count => format!(", {count} commands running"),
        };
        let unread = if workspace.has_unread_project_activity(app) {
            ", unread activity"
        } else {
            ""
        };
        format!(
            "Project {} of {}: {}{}{}{}{}",
            self.active_project_index + 1,
            self.projects.len(),
            workspace.project_display_name(app),
            working,
            done,
            running_commands,
            unread
        )
    }

    fn accessibility_project_list(&self, app: &AppContext) -> String {
        let projects = self
            .projects
            .iter()
            .enumerate()
            .map(|(index, project)| {
                let workspace = project.workspace.as_ref(app);
                let selected = if index == self.active_project_index {
                    ", selected"
                } else {
                    ""
                };
                let agent_counts = workspace.project_cli_agent_counts(app);
                let working = match agent_counts.working {
                    0 => String::new(),
                    1 => ", 1 agent working".to_string(),
                    count => format!(", {count} agents working"),
                };
                let done = match agent_counts.done {
                    0 => String::new(),
                    1 => ", 1 agent done".to_string(),
                    count => format!(", {count} agents done"),
                };
                let running_commands = match agent_counts.running_commands {
                    0 => String::new(),
                    1 => ", 1 command running".to_string(),
                    count => format!(", {count} commands running"),
                };
                let unread = if workspace.has_unread_project_activity(app) {
                    ", unread activity"
                } else {
                    ""
                };
                format!(
                    "{} of {}: {}{}{}{}{}{}",
                    index + 1,
                    self.projects.len(),
                    workspace.project_display_name(app),
                    selected,
                    working,
                    done,
                    running_commands,
                    unread
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        format!("Projects: {projects}")
    }

    pub(crate) fn active_project_index(&self) -> usize {
        self.active_project_index
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.projects.is_empty()
    }

    pub(crate) fn is_any_drag_active(app: &AppContext) -> bool {
        app.window_ids().any(|window_id| {
            app.root_view::<crate::root_view::RootView>(window_id)
                .and_then(|root_view| root_view.as_ref(app).project_window())
                .is_some_and(|project_window| project_window.as_ref(app).active_drag.is_some())
        })
    }

    pub(crate) fn projects(&self) -> impl Iterator<Item = (ProjectId, &ViewHandle<Workspace>)> {
        self.projects
            .iter()
            .map(|project| (project.id, &project.workspace))
    }

    fn register_active_workspace(&self, ctx: &mut ViewContext<Self>) {
        let workspace = self.active_workspace();
        WorkspaceRegistry::handle(ctx).update(ctx, |registry, _| {
            registry.set_active(self.window_id, workspace.downgrade());
        });
    }

    fn notify_project_header(&self, ctx: &mut ViewContext<Self>) {
        if let Some(project) = self.projects.get(self.active_project_index) {
            project.workspace.update(ctx, |_, ctx| ctx.notify());
        }
        ctx.notify();
    }

    fn skip_next_notification_read_interaction(&self, ctx: &mut ViewContext<Self>) {
        self.active_workspace().update(ctx, |workspace, _| {
            workspace.skip_next_notification_read_interaction();
        });
    }

    pub(crate) fn focus_active_workspace(&self, ctx: &mut ViewContext<Self>) {
        let workspace = self.active_workspace();
        ctx.focus(&workspace);
        workspace.update(ctx, |workspace, ctx| workspace.focus_active_tab(ctx));
    }

    pub(crate) fn handle_reopen(&mut self, ctx: &mut ViewContext<Self>) {
        let workspaces = self
            .projects
            .iter()
            .map(|project| project.workspace.clone())
            .collect::<Vec<_>>();
        for workspace in workspaces {
            workspace.update(ctx, |workspace, ctx| workspace.handle_reopen(ctx));
        }
        self.register_active_workspace(ctx);
        self.active_workspace().update(ctx, |workspace, ctx| {
            workspace.handle_project_activated(ctx);
        });
        self.notify_project_header(ctx);
    }

    pub(crate) fn add_project(&mut self, ctx: &mut ViewContext<Self>) -> ProjectId {
        self.add_project_from_source(
            NewWorkspaceSource::Empty {
                previous_active_window: Some(self.window_id),
                shell: None,
            },
            ctx,
        )
    }

    pub(crate) fn add_project_from_source(
        &mut self,
        source: NewWorkspaceSource,
        ctx: &mut ViewContext<Self>,
    ) -> ProjectId {
        let insertion_index = self.projects.len();
        let (id, _, insertion_index) =
            self.insert_project_from_source(source, insertion_index, ctx);
        self.activate_project_index(insertion_index, ctx);
        id
    }

    fn insert_project_from_source(
        &mut self,
        source: NewWorkspaceSource,
        insertion_index: usize,
        ctx: &mut ViewContext<Self>,
    ) -> (ProjectId, ViewHandle<Workspace>, usize) {
        let workspace = ctx.add_typed_action_view(|ctx| {
            Workspace::new(
                self.global_resource_handles.clone(),
                self.server_time.clone(),
                source,
                ctx,
            )
        });
        Self::subscribe_to_workspace_metadata(&workspace, ctx);
        let id = ProjectId::new();
        let insertion_index = insertion_index.min(self.projects.len());
        self.projects.insert(
            insertion_index,
            Project {
                id,
                workspace: workspace.clone(),
                mouse_state: Default::default(),
                close_mouse_state: Default::default(),
                draggable_state: Default::default(),
            },
        );
        (id, workspace, insertion_index)
    }

    /// Creates a project around an existing live tab without moving it through
    /// a temporary native window. The destination workspace starts with the
    /// same transfer placeholder used by cross-window tab drag, then adopts the
    /// source pane group after its structural parent is updated.
    pub(crate) fn add_project_from_transferred_tab(
        &mut self,
        transferred_tab: TransferredTab,
        insertion_index: usize,
        ctx: &mut ViewContext<Self>,
    ) -> ProjectId {
        let TransferredTab {
            pane_group,
            color,
            custom_title,
            left_panel_open,
            vertical_tabs_panel_open,
            right_panel_open,
            is_right_panel_maximized,
            draggable_state,
        } = transferred_tab;
        draggable_state.cancel_drag();

        let (id, workspace, insertion_index) = self.insert_project_from_source(
            NewWorkspaceSource::TransferredTab {
                tab_color: color,
                custom_title,
                left_panel_open,
                vertical_tabs_panel_open,
                right_panel_open,
                is_right_panel_maximized,
                is_tab_drag_preview: false,
            },
            insertion_index,
            ctx,
        );
        ctx.reparent_view(self.window_id, pane_group.id(), workspace.id());
        workspace.update(ctx, |workspace, ctx| {
            workspace.adopt_transferred_pane_group(pane_group, ctx);
        });
        self.activate_project_index(insertion_index, ctx);
        id
    }

    /// Commits a deferred inner-tab promotion after the source workspace's
    /// action update has completed. ProjectWindow owns this transaction so it
    /// can update the source child and then create the destination without a
    /// child-to-parent re-entrant view borrow.
    fn promote_inner_tab(
        &mut self,
        source_workspace_id: EntityId,
        pane_group_id: EntityId,
        insertion_index: usize,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        if !self.supports_project_tabs(ctx) {
            return false;
        }
        let Some(source_workspace) = self
            .projects
            .iter()
            .find(|project| project.workspace.id() == source_workspace_id)
            .map(|project| project.workspace.clone())
        else {
            return false;
        };
        let Some(transferred_tab) = source_workspace.update(ctx, |workspace, ctx| {
            workspace.take_tab_for_new_project(pane_group_id, ctx)
        }) else {
            return false;
        };

        self.add_project_from_transferred_tab(transferred_tab, insertion_index, ctx);
        true
    }

    pub(crate) fn activate_project(&mut self, id: ProjectId, ctx: &mut ViewContext<Self>) {
        if let Some(index) = self.projects.iter().position(|project| project.id == id) {
            self.activate_project_index(index, ctx);
        }
    }

    pub(crate) fn activate_project_containing_pane_group(
        &mut self,
        pane_group_id: warpui::EntityId,
        ctx: &mut ViewContext<Self>,
    ) -> Option<ViewHandle<Workspace>> {
        let index = self.projects.iter().position(|project| {
            project
                .workspace
                .as_ref(ctx)
                .contains_pane_group(pane_group_id)
        })?;
        self.activate_project_index(index, ctx);
        Some(self.projects[index].workspace.clone())
    }

    pub(crate) fn activate_project_containing_workspace(
        &mut self,
        workspace_id: warpui::EntityId,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let Some(index) = self
            .projects
            .iter()
            .position(|project| project.workspace.id() == workspace_id)
        else {
            return false;
        };
        self.activate_project_index(index, ctx);
        true
    }

    pub(crate) fn activate_previous_project(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(index) = previous_project_index(self.active_project_index, self.projects.len())
        {
            self.activate_project_index(index, ctx);
        }
    }

    pub(crate) fn activate_next_project(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(index) = next_project_index(self.active_project_index, self.projects.len()) {
            self.activate_project_index(index, ctx);
        }
    }

    fn move_active_project(&mut self, direction: isize, ctx: &mut ViewContext<Self>) {
        let target_index = self.active_project_index as isize + direction;
        if target_index < 0 || target_index >= self.projects.len() as isize {
            return;
        }
        let target_index = target_index as usize;
        self.projects.swap(self.active_project_index, target_index);
        self.active_project_index = target_index;
        self.project_tab_scroll_state
            .scroll_to_position(ScrollTarget {
                position_id: project_tab_position_id(self.projects[target_index].id),
                mode: ScrollToPositionMode::FullyIntoView,
            });
        self.notify_project_header(ctx);
        ctx.dispatch_global_action("workspace:save_app", ());
    }

    pub(crate) fn request_close_project(&mut self, id: ProjectId, ctx: &mut ViewContext<Self>) {
        let Some(index) = self.project_index_for_close(id, ctx) else {
            return;
        };

        // Prompts are owned by Workspace, so make the project visible before
        // asking it to show shared-session or unsaved-work confirmation.
        self.activate_project_index(index, ctx);
        let close_immediately = self.projects[index]
            .workspace
            .update(ctx, |workspace, ctx| {
                workspace.request_project_close(id, ctx)
            });
        if close_immediately {
            self.commit_close_project(id, ctx);
        }
    }

    pub(crate) fn commit_close_project(&mut self, id: ProjectId, ctx: &mut ViewContext<Self>) {
        let Some(index) = self.project_index_for_close(id, ctx) else {
            return;
        };

        let workspace = self.projects[index].workspace.clone();
        workspace.update(ctx, |workspace, ctx| {
            workspace.prepare_for_project_close(ctx);
        });
        self.finalize_prepared_project_close(id, ctx);
    }

    pub(crate) fn finalize_prepared_project_close(
        &mut self,
        id: ProjectId,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(index) = self.project_index_for_close(id, ctx) else {
            return;
        };
        let removed_project = self.projects.remove(index);
        ctx.unsubscribe_to_view(&removed_project.workspace);

        self.active_project_index =
            active_index_after_removal(self.active_project_index, index, self.projects.len())
                .expect("closing one of multiple projects leaves a project");
        self.activate_project_index(self.active_project_index, ctx);
    }

    fn project_index_for_close(&self, id: ProjectId, ctx: &mut ViewContext<Self>) -> Option<usize> {
        match close_project_decision(
            self.projects.len(),
            self.projects.iter().position(|project| project.id == id),
        ) {
            CloseProjectDecision::NotFound => None,
            CloseProjectDecision::CloseWindow => {
                ctx.close_window();
                None
            }
            CloseProjectDecision::Project(index) => Some(index),
        }
    }

    fn reorder_project_from_drag(
        &mut self,
        id: ProjectId,
        drag_position: GeometryRect,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(index) = self.projects.iter().position(|project| project.id == id) else {
            return;
        };
        let active_id = self.projects[self.active_project_index].id;
        let drag_center_x = drag_position.center().x();

        let target_index = if index > 0
            && ctx
                .element_position_by_id_at_last_frame(
                    self.window_id,
                    project_tab_position_id(self.projects[index - 1].id),
                )
                .is_some_and(|bounds| drag_center_x < bounds.center().x())
        {
            index - 1
        } else if index + 1 < self.projects.len()
            && ctx
                .element_position_by_id_at_last_frame(
                    self.window_id,
                    project_tab_position_id(self.projects[index + 1].id),
                )
                .is_some_and(|bounds| drag_center_x > bounds.center().x())
        {
            index + 1
        } else {
            index
        };

        if target_index != index {
            self.projects.swap(index, target_index);
            self.active_project_index = self
                .projects
                .iter()
                .position(|project| project.id == active_id)
                .unwrap_or(0);
            self.notify_project_header(ctx);
        }
    }

    fn project_tab_rects(&self, app: &AppContext) -> Vec<GeometryRect> {
        self.projects
            .iter()
            .filter_map(|project| {
                app.element_position_by_id_at_last_frame(
                    self.window_id,
                    project_tab_position_id(project.id),
                )
            })
            .collect()
    }

    fn project_strip_bounds(&self, app: &AppContext) -> Option<GeometryRect> {
        if let Some(bounds) =
            app.element_position_by_id_at_last_frame(self.window_id, PROJECT_TAB_STRIP_POSITION_ID)
        {
            return Some(bounds);
        }
        let mut rects = self.project_tab_rects(app).into_iter();
        let first = rects.next()?;
        let (min_x, min_y, max_x, max_y) = rects.fold(
            (first.min_x(), first.min_y(), first.max_x(), first.max_y()),
            |(min_x, min_y, max_x, max_y), rect| {
                (
                    min_x.min(rect.min_x()),
                    min_y.min(rect.min_y()),
                    max_x.max(rect.max_x()),
                    max_y.max(rect.max_y()),
                )
            },
        );
        Some(GeometryRect::new(
            vec2f(min_x, min_y),
            vec2f(max_x - min_x, max_y - min_y),
        ))
    }

    fn project_header_drop_bounds(&self, app: &AppContext) -> Option<GeometryRect> {
        let position_id = if crate::tab::uses_vertical_tabs() {
            TAB_BAR_POSITION_ID
        } else {
            PROJECT_HEADER_DROP_TARGET_POSITION_ID
        };
        app.element_position_by_id_at_last_frame(self.window_id, position_id)
            .or_else(|| self.project_strip_bounds(app))
    }

    /// Returns the project insertion index for an inner tab dragged over this
    /// window's header. The complete header is accepted so users need not aim
    /// at the narrow project pills themselves.
    pub(crate) fn inner_tab_drop_insertion_index(
        &self,
        tab_position: GeometryRect,
        app: &AppContext,
    ) -> Option<usize> {
        // The caller is the active Workspace and may currently be removed
        // from AppContext for its action update. Avoid reading it back through
        // `supports_project_tabs`; Workspace validates its own preview state.
        if !self.supports_project_container() {
            return None;
        }
        let header_bounds = self.project_header_drop_bounds(app)?;
        if !header_bounds.contains_point(tab_position.center()) {
            return None;
        }
        let window_bounds = app.window_bounds(&self.window_id)?;
        Some(self.insertion_index_at_screen_position(
            window_bounds.origin() + tab_position.center(),
            app,
        ))
    }

    pub(crate) fn can_promote_tab_from_workspace(&self, workspace_id: warpui::EntityId) -> bool {
        self.supports_project_container()
            && self
                .projects
                .iter()
                .any(|project| project.workspace.id() == workspace_id)
    }

    pub(crate) fn insertion_index_after_workspace(
        &self,
        workspace_id: warpui::EntityId,
    ) -> Option<usize> {
        self.projects
            .iter()
            .position(|project| project.workspace.id() == workspace_id)
            .map(|index| index + 1)
    }

    pub(crate) fn set_inner_tab_drop_indicator(
        &mut self,
        insertion_index: Option<usize>,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.active_drag.is_some() || self.incoming_drag_insertion_index == insertion_index {
            return;
        }
        self.incoming_drag_insertion_index = insertion_index;
        // The active Workspace also requests a redraw from its drag handler.
        // Avoid updating it from here because this method is called while that
        // child workspace is already handling the pointer action.
        ctx.notify();
    }

    fn handle_project_drag(
        &mut self,
        id: ProjectId,
        position: GeometryRect,
        ctx: &mut ViewContext<Self>,
    ) {
        const DETACH_SENSITIVITY: f32 = 10.;
        let center_y = position.center().y();
        let detached_from_source_strip = self.project_strip_bounds(ctx).is_some_and(|bounds| {
            center_y < bounds.min_y() - DETACH_SENSITIVITY
                || center_y > bounds.max_y() + DETACH_SENSITIVITY
        });
        let hovered_target = ctx
            .window_bounds(&self.window_id)
            .map(|window_bounds| window_bounds.origin() + position.center())
            .filter(|_| detached_from_source_strip)
            .and_then(|cursor| self.attach_target_at_screen_position(cursor, ctx))
            .filter(|target| target.window_id != self.window_id);
        let previous_target_window_id = self
            .active_drag
            .and_then(|drag| drag.hover_target_window_id);
        let hover_target_window_id = hovered_target.as_ref().map(|target| target.window_id);

        if previous_target_window_id != hover_target_window_id {
            if let Some(previous_target_window_id) = previous_target_window_id {
                Self::set_incoming_drag_indicator(previous_target_window_id, None, ctx);
            }
        }
        if let Some(target) = &hovered_target {
            target.project_window.update(ctx, |project_window, ctx| {
                project_window.incoming_drag_insertion_index = Some(target.insertion_index);
                project_window.notify_project_header(ctx);
            });
        }
        let original_index = self
            .active_drag
            .map(|drag| drag.original_index)
            .or_else(|| self.projects.iter().position(|project| project.id == id))
            .unwrap_or(0);
        self.active_drag = Some(ProjectDragState {
            id,
            original_index,
            last_position: position,
            detached_from_source_strip,
            hover_target_window_id,
        });

        if !detached_from_source_strip {
            self.reorder_project_from_drag(id, position, ctx);
        }
    }

    fn set_incoming_drag_indicator(
        window_id: WindowId,
        insertion_index: Option<usize>,
        ctx: &mut AppContext,
    ) {
        let Some(root_view) = ctx.root_view::<crate::root_view::RootView>(window_id) else {
            return;
        };
        let Some(project_window) = root_view.as_ref(ctx).project_window() else {
            return;
        };
        project_window.update(ctx, |project_window, ctx| {
            project_window.incoming_drag_insertion_index = insertion_index;
            project_window.notify_project_header(ctx);
        });
    }

    fn cancel_project_drag(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(drag) = self.active_drag.take() else {
            return;
        };
        if let Some(target_window_id) = drag.hover_target_window_id {
            Self::set_incoming_drag_indicator(target_window_id, None, ctx);
        }

        let active_id = self
            .projects
            .get(self.active_project_index)
            .map(|project| project.id);
        if let Some(current_index) = self
            .projects
            .iter()
            .position(|project| project.id == drag.id)
        {
            self.projects[current_index].draggable_state.cancel_drag();
            let project = self.projects.remove(current_index);
            let original_index = drag.original_index.min(self.projects.len());
            self.projects.insert(original_index, project);
            if let Some(active_id) = active_id {
                self.active_project_index = self
                    .projects
                    .iter()
                    .position(|project| project.id == active_id)
                    .unwrap_or(0);
            }
        }
        self.notify_project_header(ctx);
    }

    fn insertion_index_at_screen_position(
        &self,
        cursor_on_screen: Vector2F,
        app: &AppContext,
    ) -> usize {
        let Some(window_bounds) = app.window_bounds(&self.window_id) else {
            return self.projects.len();
        };
        let cursor_x = cursor_on_screen.x() - window_bounds.min_x();
        self.projects
            .iter()
            .position(|project| {
                app.element_position_by_id_at_last_frame(
                    self.window_id,
                    project_tab_position_id(project.id),
                )
                .is_some_and(|bounds| cursor_x < bounds.center().x())
            })
            .unwrap_or(self.projects.len())
    }

    fn attach_target_at_screen_position(
        &self,
        cursor_on_screen: Vector2F,
        app: &AppContext,
    ) -> Option<ProjectAttachTarget> {
        const HIT_MARGIN: f32 = 12.;
        for window_id in WindowManager::as_ref(app).ordered_window_ids() {
            let Some(root_view) = app.root_view::<crate::root_view::RootView>(window_id) else {
                continue;
            };
            let Some(project_window) = root_view.as_ref(app).project_window() else {
                continue;
            };
            let candidate = project_window.as_ref(app);
            if candidate.projects.is_empty() || !candidate.supports_project_tabs(app) {
                continue;
            }
            let Some(window_bounds) = app.window_bounds(&window_id) else {
                continue;
            };
            let Some(strip_bounds) = candidate.project_strip_bounds(app) else {
                continue;
            };
            let strip_on_screen = GeometryRect::new(
                window_bounds.origin() + strip_bounds.origin(),
                strip_bounds.size(),
            );
            let expanded = GeometryRect::new(
                strip_on_screen.origin() - vec2f(HIT_MARGIN, HIT_MARGIN),
                strip_on_screen.size() + vec2f(HIT_MARGIN * 2., HIT_MARGIN * 2.),
            );
            if expanded.contains_point(cursor_on_screen) {
                return Some(ProjectAttachTarget {
                    window_id,
                    insertion_index: candidate
                        .insertion_index_at_screen_position(cursor_on_screen, app),
                    project_window,
                });
            }
        }
        None
    }

    fn take_project_for_transfer(
        &mut self,
        id: ProjectId,
        ctx: &mut ViewContext<Self>,
    ) -> Option<Project> {
        let index = self.projects.iter().position(|project| project.id == id)?;
        let project = self.projects.remove(index);
        ctx.unsubscribe_to_view(&project.workspace);
        self.active_project_index =
            active_index_after_removal(self.active_project_index, index, self.projects.len())
                .unwrap_or(0);
        Some(project)
    }

    fn settle_after_project_transfer_out(&mut self, ctx: &mut ViewContext<Self>) {
        if self.projects.is_empty() {
            ctx.windows()
                .close_window(self.window_id, TerminationMode::ContentTransferred);
            return;
        }
        self.register_active_workspace(ctx);
        self.active_workspace().update(ctx, |workspace, ctx| {
            workspace.handle_project_activated(ctx);
        });
        ctx.notify();
    }

    fn insert_transferred_project(
        &mut self,
        project: Project,
        insertion_index: usize,
        ctx: &mut ViewContext<Self>,
    ) {
        project.draggable_state.cancel_drag();
        self.incoming_drag_insertion_index = None;
        let project_window_id = ctx.handle().id();
        ctx.reparent_view(self.window_id, project.workspace.id(), project_window_id);
        Self::subscribe_to_workspace_metadata(&project.workspace, ctx);
        let insertion_index = insertion_index.min(self.projects.len());
        self.projects.insert(insertion_index, project);
        if self.set_active_project_index(insertion_index, ctx) {
            self.notify_project_header(ctx);
        }
    }

    fn replace_placeholder_with_transferred_project(
        &mut self,
        project: Project,
        ctx: &mut ViewContext<Self>,
    ) {
        let placeholders = std::mem::take(&mut self.projects);
        for placeholder in placeholders {
            ctx.unsubscribe_to_view(&placeholder.workspace);
            placeholder.workspace.update(ctx, |workspace, ctx| {
                workspace.prepare_for_project_close(ctx);
            });
        }
        self.insert_transferred_project(project, 0, ctx);
    }

    fn transfer_project_to_target(
        &mut self,
        id: ProjectId,
        target: ProjectAttachTarget,
        replace_target_placeholder: bool,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        if target.window_id == self.window_id {
            return false;
        }
        let source_index = self
            .projects
            .iter()
            .position(|project| project.id == id)
            .unwrap_or(0);
        let source_active_id = self
            .projects
            .get(self.active_project_index)
            .map(|project| project.id);
        let Some(project) = self.take_project_for_transfer(id, ctx) else {
            return false;
        };
        project.draggable_state.cancel_drag();
        let workspace_id = project.workspace.id();
        let transferred =
            ctx.transfer_view_tree_to_window(workspace_id, self.window_id, target.window_id);
        if !transferred.contains(&workspace_id) {
            for view_id in transferred.into_iter().rev() {
                ctx.transfer_view_to_window(view_id, target.window_id, self.window_id);
            }
            let source_index = source_index.min(self.projects.len());
            Self::subscribe_to_workspace_metadata(&project.workspace, ctx);
            self.projects.insert(source_index, project);
            if let Some(source_active_id) = source_active_id {
                self.active_project_index = self
                    .projects
                    .iter()
                    .position(|project| project.id == source_active_id)
                    .unwrap_or(source_index);
            }
            self.register_active_workspace(ctx);
            self.notify_project_header(ctx);
            log::warn!(
                "failed to transfer project workspace {workspace_id:?} from window {:?} to {:?}; \
                 restored it to the source window",
                self.window_id,
                target.window_id
            );
            return false;
        }

        target
            .project_window
            .update(ctx, move |project_window, ctx| {
                if replace_target_placeholder {
                    project_window.replace_placeholder_with_transferred_project(project, ctx);
                } else {
                    project_window.insert_transferred_project(project, target.insertion_index, ctx);
                }
            });
        self.settle_after_project_transfer_out(ctx);
        ctx.windows().show_window_and_focus_app(target.window_id);
        ctx.dispatch_global_action("workspace:save_app", ());
        true
    }

    fn detach_project_to_new_window(
        &mut self,
        drag: ProjectDragState,
        cursor_on_screen: Vector2F,
        ctx: &mut ViewContext<Self>,
    ) {
        let source_size = ctx
            .window_bounds(&self.window_id)
            .map(|bounds| bounds.size())
            .unwrap_or_else(|| vec2f(1000., 700.));
        let (window_id, root_view) = crate::root_view::open_new_window_get_handles(None, ctx);
        let origin = cursor_on_screen - vec2f(drag.last_position.width() / 2., 20.);
        ctx.set_and_cache_window_bounds(window_id, GeometryRect::new(origin, source_size));
        let Some(project_window) = root_view.as_ref(ctx).project_window() else {
            return;
        };
        let transferred = self.transfer_project_to_target(
            drag.id,
            ProjectAttachTarget {
                window_id,
                insertion_index: 0,
                project_window,
            },
            true,
            ctx,
        );
        if !transferred {
            ctx.windows()
                .close_window(window_id, TerminationMode::ContentTransferred);
        }
    }

    fn finish_project_drag(&mut self, ctx: &mut ViewContext<Self>) {
        // Keep the drag marked active until the drop has fully committed. New
        // window creation and source-window closure can synchronously request
        // persistence, and those intermediate snapshots must be suppressed.
        let Some(drag) = self.active_drag else {
            return;
        };
        if let Some(target_window_id) = drag.hover_target_window_id {
            Self::set_incoming_drag_indicator(target_window_id, None, ctx);
        }
        if !drag.detached_from_source_strip {
            self.active_drag = None;
            ctx.dispatch_global_action("workspace:save_app", ());
            ctx.notify();
            return;
        }

        let Some(window_bounds) = ctx.window_bounds(&self.window_id) else {
            self.cancel_project_drag(ctx);
            return;
        };
        let cursor_on_screen = window_bounds.origin() + drag.last_position.center();
        if let Some(target) = self.attach_target_at_screen_position(cursor_on_screen, ctx) {
            if target.window_id != self.window_id {
                self.transfer_project_to_target(drag.id, target, false, ctx);
            }
        } else if self.projects.len() == 1 {
            if let Some(bounds) = ctx.window_bounds(&self.window_id) {
                let origin = cursor_on_screen - vec2f(drag.last_position.width() / 2., 20.);
                ctx.set_and_cache_window_bounds(
                    self.window_id,
                    GeometryRect::new(origin, bounds.size()),
                );
                ctx.windows().show_window_and_focus_app(self.window_id);
            }
        } else {
            self.detach_project_to_new_window(drag, cursor_on_screen, ctx);
        }
        self.active_drag = None;
        ctx.dispatch_global_action("workspace:save_app", ());
        ctx.notify();
    }

    fn activate_project_index(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        if self.set_active_project_index(index, ctx) {
            ctx.dispatch_global_action("workspace:save_app", ());
            ctx.notify();
        }
    }

    fn set_active_project_index(&mut self, index: usize, ctx: &mut ViewContext<Self>) -> bool {
        if index >= self.projects.len() {
            return false;
        }
        self.active_project_index = index;
        self.project_tab_scroll_state
            .scroll_to_position(ScrollTarget {
                position_id: project_tab_position_id(self.projects[index].id),
                mode: ScrollToPositionMode::FullyIntoView,
            });
        self.register_active_workspace(ctx);
        let workspace = self.active_workspace();
        workspace.update(ctx, |workspace, ctx| {
            workspace.handle_project_activated(ctx);
        });
        ctx.focus(&workspace);
        true
    }

    fn supports_project_container(&self) -> bool {
        !self.projects.is_empty()
            && !cfg!(target_family = "wasm")
            && crate::root_view::quake_mode_window_id() != Some(self.window_id)
    }

    pub(crate) fn supports_project_tabs(&self, app: &AppContext) -> bool {
        self.supports_project_container()
            && !self.active_workspace().as_ref(app).is_tab_drag_preview()
    }

    pub(crate) fn render_project_tab_strip(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let mut tabs = Flex::row()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);
        let insertion_indicator = || {
            Container::new(
                ConstrainedBox::new(
                    Rect::new()
                        .with_background(theme.accent())
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(1.)))
                        .finish(),
                )
                .with_width(2.)
                .with_height(24.)
                .finish(),
            )
            .with_margin_right(2.)
            .finish()
        };

        for (index, project) in self.projects.iter().enumerate() {
            if self.incoming_drag_insertion_index == Some(index) {
                tabs.add_child(insertion_indicator());
            }
            let workspace = project.workspace.as_ref(app);
            let title = workspace.project_display_name(app);
            let has_other_unread = workspace.has_other_unread_project_activity(app);
            let agent_counts = workspace.project_cli_agent_counts(app);
            let agent_summaries = workspace.project_cli_agent_summaries(app);
            let tab_count = workspace.tab_count();
            let is_active = index == self.active_project_index;
            let is_dragging = project.draggable_state.is_dragging();
            let text_color = if is_active {
                theme.active_ui_text_color()
            } else {
                theme.nonactive_ui_text_color()
            };
            let (hover_parent_anchor, hover_child_anchor) = if self.projects.len() == 1 {
                (ParentAnchor::BottomMiddle, ChildAnchor::TopMiddle)
            } else if index == 0 {
                (ParentAnchor::BottomLeft, ChildAnchor::TopLeft)
            } else if index + 1 == self.projects.len() {
                (ParentAnchor::BottomRight, ChildAnchor::TopRight)
            } else {
                (ParentAnchor::BottomMiddle, ChildAnchor::TopMiddle)
            };

            let project_id = project.id;
            let close_mouse_state = project.close_mouse_state.clone();
            let font_family = appearance.ui_font_family();
            let font_size = appearance.ui_font_size();
            let accent = theme.accent();
            let outline = theme.outline();
            let active_background = theme.surface_2();
            let inactive_background = theme.background();
            let hover_background = theme.surface_3();
            let command_badge_color = theme.disabled_text_color(theme.background()).into_solid();

            let tab = Hoverable::new(project.mouse_state.clone(), move |mouse_state| {
                let mut label = Flex::row()
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center);
                // A blue completed count is the quantitative form of a completed-turn dot. Keep
                // the plain dot for any other unread state, including mixed done + blocked cases.
                // Both use the fixed done-blue rather than the theme accent: an accent that
                // resolves to lime is indistinguishable from the green working badge.
                if has_other_unread {
                    label.add_child(
                        Container::new(
                            ConstrainedBox::new(
                                Rect::new()
                                    .with_background(Fill::Solid(CLINCH_DONE_BLUE))
                                    .with_corner_radius(CornerRadius::with_all(Radius::Percentage(
                                        50.,
                                    )))
                                    .finish(),
                            )
                            .with_width(6.)
                            .with_height(6.)
                            .finish(),
                        )
                        .with_margin_right(6.)
                        .finish(),
                    );
                }
                if agent_counts.done > 0 {
                    label.add_child(
                        Container::new(
                            Text::new_inline(
                                agent_counts.done.to_string(),
                                font_family,
                                (font_size - 2.).max(8.),
                            )
                            .with_color(CLINCH_DONE_BLUE)
                            .finish(),
                        )
                        .with_horizontal_padding(4.)
                        .with_border(
                            Border::all(PROJECT_AGENT_COUNT_BADGE_BORDER_WIDTH)
                                .with_border_fill(Fill::Solid(CLINCH_DONE_BLUE)),
                        )
                        .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
                        .with_margin_right(6.)
                        .finish(),
                    );
                }
                if agent_counts.working > 0 {
                    label.add_child(
                        Container::new(
                            Text::new_inline(
                                agent_counts.working.to_string(),
                                font_family,
                                (font_size - 2.).max(8.),
                            )
                            .with_color(CLINCH_LOGO_GREEN)
                            .finish(),
                        )
                        .with_horizontal_padding(4.)
                        .with_border(
                            Border::all(PROJECT_AGENT_COUNT_BADGE_BORDER_WIDTH)
                                .with_border_fill(Fill::Solid(CLINCH_LOGO_GREEN)),
                        )
                        .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
                        .with_margin_right(6.)
                        .finish(),
                    );
                }
                if agent_counts.running_commands > 0 {
                    label.add_child(
                        Container::new(
                            Text::new_inline(
                                agent_counts.running_commands.to_string(),
                                font_family,
                                (font_size - 2.).max(8.),
                            )
                            .with_color(command_badge_color)
                            .finish(),
                        )
                        .with_horizontal_padding(4.)
                        .with_border(
                            Border::all(PROJECT_AGENT_COUNT_BADGE_BORDER_WIDTH)
                                .with_border_fill(Fill::Solid(command_badge_color)),
                        )
                        .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
                        .with_margin_right(6.)
                        .finish(),
                    );
                }
                label.add_child(
                    Shrinkable::new(
                        1.,
                        Text::new_inline(title.clone(), font_family, font_size)
                            .with_color(text_color.into())
                            .with_clip(ClipConfig::ellipsis())
                            .finish(),
                    )
                    .finish(),
                );

                let close_button = (is_active || mouse_state.is_hovered()).then(|| {
                    Hoverable::new(close_mouse_state.clone(), move |close_state| {
                        let icon = ConstrainedBox::new(
                            Icon::X
                                .to_warpui_icon(Fill::Solid(text_color.into()))
                                .finish(),
                        )
                        .with_width(14.)
                        .with_height(14.)
                        .finish();
                        let button = Container::new(icon)
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(3.)));
                        if close_state.is_hovered() {
                            button.with_background(hover_background).finish()
                        } else {
                            button.finish()
                        }
                    })
                    .with_cursor(Cursor::PointingHand)
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(ProjectWindowAction::RequestClose(project_id));
                    })
                    .finish()
                });

                // Match the proven horizontal-tab layout: the label is the Stack's
                // in-flow child, while the close button is a positioned overlay.
                // Symmetric inner padding keeps the label centered and reserves its
                // full allocation even while the close button is hidden.
                let side_slot_width = PROJECT_TAB_CLOSE_BUTTON_SIZE + PROJECT_TAB_CLOSE_BUTTON_GAP;
                let label_row = Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::Center)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(Shrinkable::new(1., label.finish()).finish())
                    .finish();
                let mut contents = Stack::new().with_child(
                    Container::new(label_row)
                        .with_horizontal_padding(side_slot_width)
                        .finish(),
                );
                if let Some(close_button) = close_button {
                    contents.add_positioned_child(
                        ConstrainedBox::new(Align::new(close_button).finish())
                            .with_width(PROJECT_TAB_CLOSE_BUTTON_SIZE)
                            .with_height(PROJECT_TAB_CLOSE_BUTTON_SIZE)
                            .finish(),
                        OffsetPositioning::offset_from_parent(
                            vec2f(0., 0.),
                            ParentOffsetBounds::ParentByPosition,
                            ParentAnchor::MiddleRight,
                            ChildAnchor::MiddleRight,
                        ),
                    )
                }

                let background = if is_active {
                    active_background
                } else if mouse_state.is_hovered() {
                    hover_background
                } else {
                    inactive_background
                };
                let border_fill = if is_active {
                    Fill::Solid(accent.into_solid())
                } else if mouse_state.is_hovered() {
                    outline
                } else {
                    inactive_background
                };
                let tab_body = ConstrainedBox::new(
                    Container::new(contents.finish())
                        .with_background(background)
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
                        .with_border(
                            Border::all(PROJECT_TAB_BORDER_WIDTH).with_border_fill(border_fill),
                        )
                        .with_horizontal_padding(12.)
                        .with_vertical_padding(PROJECT_TAB_VERTICAL_PADDING)
                        .with_margin_right(2.)
                        .finish(),
                )
                .with_min_width(PROJECT_TAB_MIN_WIDTH)
                .with_max_width(180.)
                .finish();

                if mouse_state.is_hovered() && !is_dragging {
                    let mut stack = Stack::new().with_child(tab_body);
                    stack.add_positioned_overlay_child(
                        render_project_agent_hover_card(
                            &title,
                            tab_count,
                            &agent_summaries,
                            agent_counts,
                            appearance,
                        ),
                        OffsetPositioning::offset_from_parent(
                            vec2f(0., 6.),
                            ParentOffsetBounds::WindowByPosition,
                            hover_parent_anchor,
                            hover_child_anchor,
                        ),
                    );
                    stack.finish()
                } else {
                    tab_body
                }
            })
            .with_defer_events_to_children()
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(ProjectWindowAction::Activate(project_id));
            })
            .finish();

            let draggable = Draggable::new(project.draggable_state.clone(), tab)
                .on_drag(move |ctx, _, position, _| {
                    ctx.dispatch_typed_action(ProjectWindowAction::Reorder {
                        id: project_id,
                        position,
                    });
                })
                .on_drop(|ctx, _, _, _| {
                    ctx.dispatch_typed_action(ProjectWindowAction::FinishDrag);
                })
                .finish();
            tabs.add_child(
                SavePosition::new(draggable, &project_tab_position_id(project_id)).finish(),
            );
        }
        if self.incoming_drag_insertion_index == Some(self.projects.len()) {
            tabs.add_child(insertion_indicator());
        }

        let scrollable = ClippedScrollable::horizontal(
            self.project_tab_scroll_state.clone(),
            tabs.finish(),
            ScrollbarWidth::None,
            Fill::Solid(theme.outline().into()).into(),
            Fill::Solid(theme.active_ui_text_color().into()).into(),
            Fill::Solid(theme.background().into()).into(),
        )
        // Scrollable reserves 4px of cross-axis gutter for a scrollbar even
        // with ScrollbarWidth::None. The title bar's height budget is tight:
        // losing those 4px leaves the tab label less than one line height and
        // Text then drops the line entirely (blank pills).
        .with_padding_start(0.)
        .with_padding_end(0.)
        .finish();
        let scrollable = SavePosition::new(scrollable, PROJECT_TAB_STRIP_POSITION_ID).finish();
        let add_button = Hoverable::new(self.new_project_mouse_state.clone(), move |mouse_state| {
            let icon = ConstrainedBox::new(
                Icon::Plus
                    .to_warpui_icon(Fill::Solid(theme.nonactive_ui_text_color().into()))
                    .finish(),
            )
            .with_width(16.)
            .with_height(16.)
            .finish();
            let button = Container::new(icon)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                .with_uniform_padding(5.);
            if mouse_state.is_hovered() {
                button.with_background(theme.surface_3()).finish()
            } else {
                button.finish()
            }
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| ctx.dispatch_typed_action(ProjectWindowAction::Add))
        .finish();
        let add_button = appearance.ui_builder().overlay_tool_tip_on_element(
            "Add Project".to_string(),
            self.new_project_mouse_state.clone(),
            add_button,
            ParentAnchor::BottomMiddle,
            ChildAnchor::TopMiddle,
            vec2f(0., 4.),
        );

        Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(Shrinkable::new(1., scrollable).finish())
                .with_child(Container::new(add_button).with_margin_left(4.).finish())
                .finish(),
        )
        // The strip is geometrically centered, but its pills read slightly high
        // beside the surrounding title-bar controls. Nudge the whole strip down.
        .with_margin_top(PROJECT_TAB_VERTICAL_NUDGE)
        .finish()
    }

    fn render_horizontal_project_header(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let header = Container::new(self.render_project_tab_strip(appearance, app))
            .with_background(appearance.theme().background())
            .with_padding_left(16.)
            .with_padding_right(8.)
            .with_border(
                Border::bottom(crate::tab::TAB_BAR_BORDER_HEIGHT)
                    .with_border_fill(crate::workspace::chrome_divider_fill(appearance.theme())),
            )
            .finish();
        SavePosition::new(
            ConstrainedBox::new(header)
                .with_height(crate::workspace::view::TAB_BAR_HEIGHT)
                .finish(),
            PROJECT_HEADER_DROP_TARGET_POSITION_ID,
        )
        .finish()
    }
}

impl Entity for ProjectWindow {
    type Event = ();
}

/// Project actions are added here rather than to `WorkspaceAction` so the two
/// tab layers keep separate identities and shortcut semantics.
#[derive(Clone, Debug)]
pub(crate) enum ProjectWindowAction {
    Add,
    Activate(ProjectId),
    RequestClose(ProjectId),
    Reorder {
        id: ProjectId,
        position: GeometryRect,
    },
    FinishDrag,
    CancelDrag,
    CloseActive,
    MoveActiveLeft,
    MoveActiveRight,
    ActivatePrevious,
    ActivateNext,
    PromoteInnerTab {
        source_workspace_id: EntityId,
        pane_group_id: EntityId,
        insertion_index: usize,
    },
}

impl TypedActionView for ProjectWindow {
    type Action = ProjectWindowAction;

    fn handle_action(&mut self, action: &ProjectWindowAction, ctx: &mut ViewContext<Self>) {
        match action {
            ProjectWindowAction::Add => {
                if self.supports_project_tabs(ctx) {
                    self.add_project(ctx);
                } else {
                    ctx.dispatch_global_action("root_view:open_new_project", ());
                }
            }
            ProjectWindowAction::Activate(id) => {
                // .get(): the action can arrive while `projects` is empty (e.g. queued
                // against a drained window awaiting its deferred close).
                let previous_active_id = self
                    .projects
                    .get(self.active_project_index)
                    .map(|project| project.id);
                self.activate_project(*id, ctx);
                let new_active_id = self
                    .projects
                    .get(self.active_project_index)
                    .map(|project| project.id);
                if previous_active_id.is_some() && previous_active_id != new_active_id {
                    self.skip_next_notification_read_interaction(ctx);
                }
            }
            ProjectWindowAction::RequestClose(id) => {
                let previous_active_id = self
                    .projects
                    .get(self.active_project_index)
                    .map(|project| project.id);
                self.request_close_project(*id, ctx);
                if previous_active_id != Some(*id)
                    && self
                        .projects
                        .get(self.active_project_index)
                        .is_some_and(|project| project.id == *id)
                {
                    self.skip_next_notification_read_interaction(ctx);
                }
            }
            ProjectWindowAction::Reorder { id, position } => {
                self.handle_project_drag(*id, *position, ctx)
            }
            ProjectWindowAction::FinishDrag => self.finish_project_drag(ctx),
            ProjectWindowAction::CancelDrag => self.cancel_project_drag(ctx),
            ProjectWindowAction::CloseActive => {
                if let Some(project_id) = self
                    .projects
                    .get(self.active_project_index)
                    .map(|project| project.id)
                {
                    self.request_close_project(project_id, ctx);
                }
            }
            ProjectWindowAction::MoveActiveLeft => self.move_active_project(-1, ctx),
            ProjectWindowAction::MoveActiveRight => self.move_active_project(1, ctx),
            ProjectWindowAction::ActivatePrevious => {
                if self.projects.len() > 1 {
                    self.activate_previous_project(ctx);
                    self.skip_next_notification_read_interaction(ctx);
                }
            }
            ProjectWindowAction::ActivateNext => {
                if self.projects.len() > 1 {
                    self.activate_next_project(ctx);
                    self.skip_next_notification_read_interaction(ctx);
                }
            }
            ProjectWindowAction::PromoteInnerTab {
                source_workspace_id,
                pane_group_id,
                insertion_index,
            } => {
                self.promote_inner_tab(*source_workspace_id, *pane_group_id, *insertion_index, ctx);
            }
        }
    }

    fn action_accessibility_contents(
        &mut self,
        action: &ProjectWindowAction,
        ctx: &mut ViewContext<Self>,
    ) -> ActionAccessibilityContent {
        match action {
            ProjectWindowAction::Add
            | ProjectWindowAction::Activate(_)
            | ProjectWindowAction::ActivatePrevious
            | ProjectWindowAction::ActivateNext => {
                ActionAccessibilityContent::Custom(AccessibilityContent::new_without_help(
                    self.accessibility_summary(ctx),
                    WarpA11yRole::UserAction,
                ))
            }
            ProjectWindowAction::RequestClose(_) => {
                ActionAccessibilityContent::Custom(AccessibilityContent::new_without_help(
                    "Close project requested",
                    WarpA11yRole::UserAction,
                ))
            }
            ProjectWindowAction::CloseActive => {
                ActionAccessibilityContent::Custom(AccessibilityContent::new_without_help(
                    "Close active project requested",
                    WarpA11yRole::UserAction,
                ))
            }
            ProjectWindowAction::MoveActiveLeft | ProjectWindowAction::MoveActiveRight => {
                ActionAccessibilityContent::Custom(AccessibilityContent::new_without_help(
                    self.accessibility_summary(ctx),
                    WarpA11yRole::UserAction,
                ))
            }
            ProjectWindowAction::CancelDrag => {
                ActionAccessibilityContent::Custom(AccessibilityContent::new_without_help(
                    "Project drag canceled",
                    WarpA11yRole::UserAction,
                ))
            }
            ProjectWindowAction::FinishDrag => {
                ActionAccessibilityContent::Custom(AccessibilityContent::new_without_help(
                    "Project move complete",
                    WarpA11yRole::UserAction,
                ))
            }
            ProjectWindowAction::Reorder { .. } | ProjectWindowAction::PromoteInnerTab { .. } => {
                ActionAccessibilityContent::Empty
            }
        }
    }
}

impl View for ProjectWindow {
    fn ui_name() -> &'static str {
        "ProjectWindow"
    }

    fn accessibility_contents(&self, app: &AppContext) -> Option<AccessibilityContent> {
        Some(AccessibilityContent::new(
            self.accessibility_project_list(app),
            "Use Command left brace and Command right brace to switch projects. Close and move project actions are available in the command palette and keybinding settings.",
            WarpA11yRole::ListRole,
        ))
    }

    fn keymap_context(&self, _app: &AppContext) -> warpui::keymap::Context {
        let mut context = Self::default_keymap_context();
        if self.active_drag.is_some() {
            context.set.insert("ProjectWindow_ProjectDragging");
        }
        context
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            self.focus_active_workspace(ctx);
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        if self.projects.is_empty() {
            return Empty::new().finish();
        }
        let workspace =
            ChildView::new(&self.projects[self.active_project_index].workspace).finish();
        if self.supports_project_tabs(app) && !crate::tab::uses_vertical_tabs() {
            Flex::column()
                .with_child(self.render_horizontal_project_header(app))
                .with_child(Shrinkable::new(1., workspace).finish())
                .finish()
        } else {
            workspace
        }
    }

    fn child_view_ids(&self, _app: &AppContext) -> Vec<warpui::EntityId> {
        self.projects
            .iter()
            .map(|project| project.workspace.id())
            .collect()
    }
}

#[cfg(test)]
#[path = "project_window_tests.rs"]
mod tests;
