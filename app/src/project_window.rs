use std::sync::Arc;

use uuid::Uuid;
use warpui::elements::{
    Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, DispatchEventResult,
    EventHandler, Flex, MainAxisSize, Radius, Rect, Shrinkable, Text,
};
use warpui::keymap::{BindingDescription, EditableBinding};
use warpui::presenter::ChildView;
use warpui::platform::Cursor;
use warpui::{
    id, AppContext, Element, Entity, EntityId, FocusContext, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle, WindowId,
};

use crate::appearance::Appearance;
use crate::root_view::NewWorkspaceSource;
use crate::server::server_api::ServerTime;
use crate::util::bindings::{self, CustomAction};
use crate::workspace::{Workspace, WorkspaceRegistry};
use crate::GlobalResourceHandles;

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
        .with_mac_key_binding("cmd-{"),
        EditableBinding::new(
            "project_window:activate_next_project",
            "Activate next project",
            ProjectWindowAction::ActivateNext,
        )
        .with_group(bindings::BindingGroup::Navigation.as_str())
        .with_context_predicate(id!(ProjectWindow::ui_name()))
        .with_mac_key_binding("cmd-}"),
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
}

/// Owns all of the project workspaces hosted by one physical Clinch window.
///
/// Only the active workspace is rendered, but inactive workspace handles stay
/// strongly owned here so their terminals and agent sessions remain live.
pub(crate) struct ProjectWindow {
    window_id: WindowId,
    view_id: EntityId,
    projects: Vec<Project>,
    active_project_index: usize,
    global_resource_handles: GlobalResourceHandles,
    server_time: Option<Arc<ServerTime>>,
}

impl ProjectWindow {
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
                Project {
                    id: ProjectId::new(),
                    workspace,
                }
            })
            .collect::<Vec<_>>();
        let active_project_index = active_project_index.min(projects.len().saturating_sub(1));

        let project_window = Self {
            window_id: ctx.window_id(),
            view_id: ctx.view_id(),
            projects,
            active_project_index,
            global_resource_handles,
            server_time,
        };
        project_window.register_active_workspace(ctx);
        project_window
    }

    pub(crate) fn singleton(
        global_resource_handles: GlobalResourceHandles,
        server_time: Option<Arc<ServerTime>>,
        workspace_source: NewWorkspaceSource,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        Self::new(
            global_resource_handles,
            server_time,
            vec![workspace_source],
            0,
            ctx,
        )
    }

    pub(crate) fn active_workspace(&self) -> ViewHandle<Workspace> {
        self.projects[self.active_project_index].workspace.clone()
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

    pub(crate) fn focus_active_workspace(&self, ctx: &mut ViewContext<Self>) {
        let workspace = self.active_workspace();
        ctx.focus(&workspace);
        workspace.update(ctx, |workspace, ctx| workspace.focus_active_tab(ctx));
    }

    pub(crate) fn add_project(&mut self, ctx: &mut ViewContext<Self>) -> ProjectId {
        let workspace = ctx.add_typed_action_view(|ctx| {
            Workspace::new(
                self.global_resource_handles.clone(),
                self.server_time.clone(),
                NewWorkspaceSource::Empty {
                    previous_active_window: Some(self.window_id),
                    shell: None,
                },
                ctx,
            )
        });
        let id = ProjectId::new();
        self.projects.push(Project { id, workspace });
        self.activate_project_index(self.projects.len() - 1, ctx);
        id
    }

    pub(crate) fn activate_project(&mut self, id: ProjectId, ctx: &mut ViewContext<Self>) {
        if let Some(index) = self.projects.iter().position(|project| project.id == id) {
            self.activate_project_index(index, ctx);
        }
    }

    pub(crate) fn activate_previous_project(&mut self, ctx: &mut ViewContext<Self>) {
        if self.projects.len() <= 1 {
            return;
        }
        let index = self
            .active_project_index
            .checked_sub(1)
            .unwrap_or(self.projects.len() - 1);
        self.activate_project_index(index, ctx);
    }

    pub(crate) fn activate_next_project(&mut self, ctx: &mut ViewContext<Self>) {
        if self.projects.len() <= 1 {
            return;
        }
        let index = (self.active_project_index + 1) % self.projects.len();
        self.activate_project_index(index, ctx);
    }

    fn activate_project_index(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        if index >= self.projects.len() {
            return;
        }
        self.active_project_index = index;
        self.register_active_workspace(ctx);
        let workspace = self.active_workspace();
        workspace.update(ctx, |workspace, ctx| {
            workspace.handle_project_activated(ctx);
        });
        ctx.focus(&workspace);
        ctx.dispatch_global_action("workspace:save_app", ());
        ctx.notify();
    }

    pub(crate) fn supports_project_tabs(&self, app: &AppContext) -> bool {
        !cfg!(target_family = "wasm")
            && crate::root_view::quake_mode_window_id() != Some(self.window_id)
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

        for (index, project) in self.projects.iter().enumerate() {
            let workspace = project.workspace.as_ref(app);
            let title = workspace.project_display_name(app);
            let has_unread = workspace.has_unread_project_activity(app);
            let is_active = index == self.active_project_index;
            let text_color = if is_active {
                theme.active_ui_text_color()
            } else {
                theme.nonactive_ui_text_color()
            };

            let mut contents = Flex::row()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Center);
            if has_unread {
                contents.add_child(
                    Container::new(
                        ConstrainedBox::new(
                            Rect::new()
                                .with_background(theme.accent())
                                .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
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
            contents.add_child(
                Text::new_inline(title, appearance.ui_font_family(), appearance.ui_font_size())
                    .with_color(text_color.into())
                    .finish(),
            );

            let background = if is_active {
                theme.surface_2()
            } else {
                theme.background()
            };
            let tab = ConstrainedBox::new(
                Container::new(contents.finish())
                    .with_background(background)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
                    .with_horizontal_padding(12.)
                    .with_vertical_padding(6.)
                    .with_margin_right(2.)
                    .finish(),
            )
            .with_min_width(84.)
            .with_max_width(180.)
            .finish();

            let window_id = self.window_id;
            let project_window_view_id = self.view_id;
            let project_id = project.id;
            tabs.add_child(
                EventHandler::new(tab)
                    .on_left_mouse_down(move |ctx, _, _| {
                        ctx.dispatch_typed_action_for_view(
                            window_id,
                            project_window_view_id,
                            &ProjectWindowAction::Activate(project_id),
                        );
                        DispatchEventResult::StopPropagation
                    })
                    .with_cursor(Cursor::PointingHand)
                    .finish(),
            );
        }

        tabs.finish()
    }

    fn render_horizontal_project_header(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let header = Container::new(self.render_project_tab_strip(appearance, app))
            .with_background(appearance.theme().background())
            .with_padding_left(16.)
            .with_padding_right(8.)
            .with_border(
                Border::bottom(crate::tab::TAB_BAR_BORDER_HEIGHT)
                    .with_border_fill(appearance.theme().outline()),
            )
            .finish();
        ConstrainedBox::new(header)
            .with_height(crate::workspace::view::TAB_BAR_HEIGHT)
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
    ActivatePrevious,
    ActivateNext,
}

impl TypedActionView for ProjectWindow {
    type Action = ProjectWindowAction;

    fn handle_action(&mut self, action: &ProjectWindowAction, ctx: &mut ViewContext<Self>) {
        match action {
            ProjectWindowAction::Add => {
                self.add_project(ctx);
            }
            ProjectWindowAction::Activate(id) => self.activate_project(*id, ctx),
            ProjectWindowAction::ActivatePrevious => self.activate_previous_project(ctx),
            ProjectWindowAction::ActivateNext => self.activate_next_project(ctx),
        }
    }
}

impl View for ProjectWindow {
    fn ui_name() -> &'static str {
        "ProjectWindow"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            self.focus_active_workspace(ctx);
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
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
