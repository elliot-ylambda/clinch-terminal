use std::sync::Arc;

use uuid::Uuid;
use warpui::presenter::ChildView;
use warpui::{
    AppContext, Element, Entity, FocusContext, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle, WindowId,
};

use crate::root_view::NewWorkspaceSource;
use crate::server::server_api::ServerTime;
use crate::workspace::{Workspace, WorkspaceRegistry};
use crate::GlobalResourceHandles;

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

    pub(crate) fn project_count(&self) -> usize {
        self.projects.len()
    }

    pub(crate) fn active_project_index(&self) -> usize {
        self.active_project_index
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
}

impl Entity for ProjectWindow {
    type Event = ();
}

/// Project actions are added here rather than to `WorkspaceAction` so the two
/// tab layers keep separate identities and shortcut semantics.
#[derive(Clone, Debug)]
pub(crate) enum ProjectWindowAction {}

impl TypedActionView for ProjectWindow {
    type Action = ProjectWindowAction;

    fn handle_action(
        &mut self,
        action: &ProjectWindowAction,
        _ctx: &mut ViewContext<Self>,
    ) {
        match *action {}
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

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        ChildView::new(&self.projects[self.active_project_index].workspace).finish()
    }

    fn child_view_ids(&self, _app: &AppContext) -> Vec<warpui::EntityId> {
        self.projects
            .iter()
            .map(|project| project.workspace.id())
            .collect()
    }
}
