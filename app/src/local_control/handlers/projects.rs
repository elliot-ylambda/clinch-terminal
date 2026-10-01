//! Project-scoped control uses the same parent container and live panes as the UI.
use std::str::FromStr as _;
use std::sync::Arc;

use ::local_control::agents::AgentScope;
use ::local_control::projects::{ProjectCreateParams, ProjectRestoreParams, TabTransferParams};
use ::local_control::{Action, ActionKind, ControlError, ErrorCode, InstanceId, TargetSelector};
use serde_json::{json, Value};
use uuid::Uuid;
use warp_core::ui::theme::AnsiColorIdentifier;
use warpui::{AppContext, ModelContext, ViewHandle, WindowId};

use crate::local_control::resolver::{
    reject_target_families, tab_index_from_target, target_window_id_for_target, target_workspace,
};
use crate::local_control::LocalControlBridge;
use crate::project_window::{ProjectId, ProjectWindow};
use crate::root_view::{NewWorkspaceSource, RootView};
use crate::tab::SelectedTabColor;
use crate::workspace::tab_group::TabGroupId;
use crate::workspace::Workspace;

#[path = "project_layout.rs"]
mod layout;
#[cfg(test)]
#[path = "projects_tests.rs"]
mod tests;

pub(crate) struct ProjectTarget {
    pub window_id: WindowId,
    pub parent: ViewHandle<ProjectWindow>,
    pub id: ProjectId,
    pub workspace: ViewHandle<Workspace>,
}

pub(crate) fn resolve_project(id: &str, ctx: &AppContext) -> Result<ProjectTarget, ControlError> {
    for window_id in ctx.window_ids() {
        let Some(parent) = ctx
            .root_view::<RootView>(window_id)
            .and_then(|root| root.as_ref(ctx).project_window())
        else {
            continue;
        };
        let found = parent
            .as_ref(ctx)
            .projects()
            .find(|(project_id, _)| project_id.opaque_id() == id)
            .map(|(id, workspace)| (id, workspace.clone()));
        if let Some((id, workspace)) = found {
            return Ok(ProjectTarget {
                window_id,
                parent,
                id,
                workspace,
            });
        }
    }
    Err(ControlError::new(
        ErrorCode::StaleTarget,
        "project is no longer present; discover current IDs with project list",
    ))
}

pub(super) fn activate_selected_project(
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<(), ControlError> {
    if let Some(id) = &target.project {
        let project = resolve_project(id, ctx)?;
        project
            .parent
            .update(ctx, |parent, ctx| parent.activate_project(project.id, ctx));
    }
    Ok(())
}

fn selected_project(
    target: &TargetSelector,
    action: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<ProjectTarget, ControlError> {
    let window_id = target_window_id_for_target(ctx, target, action)?;
    if let Some(id) = &target.project {
        return resolve_project(id, ctx);
    }
    let parent = project_parent(window_id, ctx)?;
    let (id, workspace) = parent
        .as_ref(ctx)
        .projects()
        .nth(parent.as_ref(ctx).active_project_index())
        .map(|(id, workspace)| (id, workspace.clone()))
        .ok_or_else(|| {
            ControlError::new(ErrorCode::MissingTarget, "window has no active project")
        })?;
    Ok(ProjectTarget {
        window_id,
        parent,
        id,
        workspace,
    })
}

fn project_parent(
    window_id: WindowId,
    ctx: &AppContext,
) -> Result<ViewHandle<ProjectWindow>, ControlError> {
    ctx.root_view::<RootView>(window_id)
        .and_then(|root| root.as_ref(ctx).project_window())
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::MissingTarget,
                "selected window has no project container",
            )
        })
}

fn inspect(
    instance: &InstanceId,
    project_id: &str,
    ctx: &AppContext,
) -> Result<Value, ControlError> {
    let snapshot = crate::local_control::agents::snapshot(
        instance,
        &AgentScope {
            projects: vec![project_id.to_owned()],
            sections: vec![],
        },
        ctx,
    )?;
    for window in snapshot.tree["windows"].as_array().into_iter().flatten() {
        if let Some(project) = window["projects"]
            .as_array()
            .and_then(|projects| projects.first())
        {
            let mut project = project.clone();
            project["window_id"] = window["window_id"].clone();
            return Ok(project);
        }
    }
    Err(ControlError::new(
        ErrorCode::StaleTarget,
        "project is no longer present",
    ))
}

pub(crate) fn handle(
    instance: &InstanceId,
    action: &Action,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<Value, ControlError> {
    reject_target_families(
        action.kind,
        target.pane.is_some()
            || target.session.is_some()
            || (action.kind != ActionKind::TabTransfer && target.tab.is_some()),
        "tab, pane, or session selectors (only tab.transfer accepts a tab)",
    )?;
    if action.kind == ActionKind::TabTransfer {
        return transfer(instance, action, target, ctx);
    }
    if matches!(
        action.kind,
        ActionKind::ProjectCreate | ActionKind::ProjectRestore
    ) {
        reject_target_families(
            action.kind,
            target.project.is_some(),
            "project selectors when creating a new project; use --window",
        )?;
        let window_id = target_window_id_for_target(ctx, target, action.kind)?;
        let parent = project_parent(window_id, ctx)?;
        let source = if action.kind == ActionKind::ProjectCreate {
            let params: ProjectCreateParams = action.params_as()?;
            match params.cwd {
                Some(cwd) => NewWorkspaceSource::Restored {
                    window_snapshot: layout::new_project(cwd)?,
                    block_lists: Arc::new(Default::default()),
                },
                None => NewWorkspaceSource::Empty {
                    previous_active_window: None,
                    shell: None,
                },
            }
        } else {
            let params: ProjectRestoreParams = action.params_as()?;
            NewWorkspaceSource::Restored {
                window_snapshot: layout::restore(params.layout, params.resume_agents)?,
                block_lists: Arc::new(Default::default()),
            }
        };
        let id = parent.update(ctx, |parent, ctx| {
            parent.add_project_from_source(source, ctx)
        });
        return Ok(
            json!({"action": action.kind.as_str(), "created": true, "project": inspect(instance, &id.opaque_id(), ctx)?}),
        );
    }
    let project = selected_project(target, action.kind, ctx)?;
    if matches!(
        action.kind,
        ActionKind::ProjectTaskList
            | ActionKind::ProjectTaskCreate
            | ActionKind::ProjectTaskUpdate
            | ActionKind::ProjectTaskComplete
            | ActionKind::ProjectTaskDelete
    ) {
        return tasks(action, &project, ctx);
    }
    let id = project.id.opaque_id();
    match action.kind {
        ActionKind::ProjectInspect => {
            Ok(json!({"action": action.kind.as_str(), "project": inspect(instance, &id, ctx)?}))
        }
        ActionKind::ProjectActivate => {
            project
                .parent
                .update(ctx, |parent, ctx| parent.activate_project(project.id, ctx));
            Ok(json!({"action": action.kind.as_str(), "project_id": id, "activated": true}))
        }
        ActionKind::ProjectClose => {
            project.parent.update(ctx, |parent, ctx| {
                parent.request_close_project(project.id, ctx)
            });
            let closed = resolve_project(&id, ctx).is_err();
            Ok(
                json!({"action": action.kind.as_str(), "project_id": id, "closed": closed, "confirmation_pending": !closed}),
            )
        }
        ActionKind::ProjectExport => {
            let has_remote_terminal = project.workspace.read(ctx, |workspace, ctx| {
                workspace.tab_views().any(|tab| {
                    tab.as_ref(ctx).terminal_views(ctx).iter().any(|terminal| {
                        terminal.as_ref(ctx).active_session_is_local(ctx) == Some(false)
                    })
                })
            });
            if has_remote_terminal {
                return Err(ControlError::new(
                    ErrorCode::UnsupportedAction,
                    "portable project export supports local terminals only",
                ));
            }
            let snapshot = project.workspace.read(ctx, |workspace, ctx| {
                workspace.snapshot(project.window_id, false, ctx)
            });
            Ok(
                json!({"action": action.kind.as_str(), "project_id": id, "layout": layout::export(snapshot)?}),
            )
        }
        _ => Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            "not a project action",
        )),
    }
}

fn transfer(
    instance: &InstanceId,
    action: &Action,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<Value, ControlError> {
    let params: TabTransferParams = action.params_as()?;
    let source = target_workspace(action.kind, target, ctx)?;
    let destination = resolve_project(&params.destination_project, ctx)?;
    let source_window = target_window_id_for_target(ctx, target, action.kind)?;
    if source.id() == destination.workspace.id() {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            "tab.transfer requires a different destination project",
        ));
    }
    let tab_id = source.read(ctx, |workspace, ctx| {
        tab_index_from_target(target, workspace, ctx)
            .map(|index| workspace.tabs[index].pane_group.id())
    })?;
    let section = params
        .section_id
        .map(|id| {
            Uuid::parse_str(&id).map(TabGroupId).map_err(|_| {
                ControlError::new(ErrorCode::InvalidParams, "invalid destination section ID")
            })
        })
        .transpose()?;
    // Resolve every placement constraint before extracting a live pane from its source.
    let insertion = destination.workspace.read(ctx, |workspace, _| {
        transfer_index(workspace, section, params.index)
    })?;
    let transferred = if source_window == destination.window_id {
        destination.parent.update(ctx, |parent, ctx| {
            parent.move_inner_tab_to_project(source.id(), tab_id, destination.id, ctx)
        })
    } else {
        transfer_across_windows(&source, source_window, tab_id, &destination, ctx)?
    };
    if !transferred {
        return Err(ControlError::new(
            ErrorCode::StaleTarget,
            "source session could not be transferred",
        ));
    }
    destination.workspace.update(ctx, |workspace, ctx| {
        let moved = workspace
            .tabs
            .iter()
            .position(|tab| tab.pane_group.id() == tab_id)
            .expect("transferred tab is present");
        workspace.tabs[moved].group_id = section;
        if let Some(group) = section.and_then(|id| workspace.tab_groups.get_mut(&id)) {
            group.collapsed = false;
        }
        workspace.move_tab_to_index(moved, insertion, ctx);
        ctx.dispatch_global_action("workspace:save_app", ());
        ctx.notify();
    });
    Ok(
        json!({"action": action.kind.as_str(), "transferred": true, "tab_id": tab_id.to_string(), "project": inspect(instance, &params.destination_project, ctx)?}),
    )
}

fn transfer_index(
    workspace: &Workspace,
    section: Option<TabGroupId>,
    index: Option<usize>,
) -> Result<usize, ControlError> {
    if section.is_some_and(|id| !workspace.tab_groups.contains_key(&id)) {
        return Err(ControlError::new(
            ErrorCode::StaleTarget,
            "destination section is not in the destination project",
        ));
    }
    let members: Vec<_> = workspace
        .tabs
        .iter()
        .enumerate()
        .filter(|(_, tab)| tab.group_id == section && !tab.pinned)
        .map(|(index, _)| index)
        .collect();
    let index = index.unwrap_or(members.len());
    if index > members.len() {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            "destination position exceeds the section/session count",
        ));
    }
    Ok(members.get(index).copied().unwrap_or_else(|| {
        if section.is_some() {
            members.last().map_or(workspace.tabs.len(), |last| last + 1)
        } else {
            workspace.tabs.len()
        }
    }))
}

pub(crate) fn tab_color_value(color: SelectedTabColor) -> Option<String> {
    match color {
        SelectedTabColor::Unset => None,
        SelectedTabColor::Cleared => Some("none".to_owned()),
        SelectedTabColor::Color(color) => Some(color.to_string().to_ascii_lowercase()),
    }
}

fn parse_tab_color(color: Option<String>) -> Result<SelectedTabColor, ControlError> {
    match color.as_deref() {
        None | Some("default" | "unset") => Ok(SelectedTabColor::Unset),
        Some("none") => Ok(SelectedTabColor::Cleared),
        Some(color) => AnsiColorIdentifier::from_str(color)
            .map(SelectedTabColor::Color)
            .map_err(|_| ControlError::new(ErrorCode::InvalidParams, "invalid tab color")),
    }
}

fn tasks(
    action: &Action,
    project: &ProjectTarget,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<Value, ControlError> {
    use ::local_control::projects::{
        ProjectTaskCreateParams, ProjectTaskIdParams, ProjectTaskUpdateParams,
    };

    use crate::workspace::task::{WorkspaceTask, WorkspaceTaskId};
    let invalid = || {
        ControlError::new(
            ErrorCode::InvalidParams,
            "task text must be nonempty and at most 256 KiB",
        )
    };
    let stale = || {
        ControlError::new(
            ErrorCode::StaleTarget,
            "task is not in the selected project",
        )
    };
    let parse_id = |id: &str| {
        Uuid::parse_str(id)
            .map(WorkspaceTaskId)
            .map_err(|_| ControlError::new(ErrorCode::InvalidParams, "invalid task ID"))
    };
    project.workspace.update(ctx, |workspace, ctx| {
        let task_id = match action.kind {
            ActionKind::ProjectTaskList => None,
            ActionKind::ProjectTaskCreate => {
                let params: ProjectTaskCreateParams = action.params_as()?;
                Some(
                    workspace
                        .add_workspace_task(params.text, ctx)
                        .ok_or_else(invalid)?,
                )
            }
            ActionKind::ProjectTaskUpdate => {
                let params: ProjectTaskUpdateParams = action.params_as()?;
                let id = parse_id(&params.task_id)?;
                let task = WorkspaceTask::new(params.text).ok_or_else(invalid)?;
                if !workspace.update_workspace_task(id, task.text, ctx) {
                    return Err(stale());
                }
                Some(id)
            }
            _ => {
                let params: ProjectTaskIdParams = action.params_as()?;
                let id = parse_id(&params.task_id)?;
                if !workspace.remove_workspace_task(id, ctx) {
                    return Err(stale());
                }
                Some(id)
            }
        };
        Ok(
            json!({"action": action.kind.as_str(), "project_id": project.id.opaque_id(),
            "task_id": task_id, "tasks": workspace.workspace_tasks(),
            "completed": action.kind == ActionKind::ProjectTaskComplete,
            "completion_behavior": "remove_from_pending_tasks"}),
        )
    })
}

fn transfer_across_windows(
    source: &ViewHandle<Workspace>,
    source_window: WindowId,
    tab_id: warpui::EntityId,
    destination: &ProjectTarget,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<bool, ControlError> {
    let source_parent = project_parent(source_window, ctx)?;
    let colors = source
        .as_ref(ctx)
        .tabs
        .iter()
        .find(|tab| tab.pane_group.id() == tab_id)
        .map(|tab| (tab.selected_color, tab.default_directory_color))
        .ok_or_else(|| ControlError::new(ErrorCode::StaleTarget, "source tab disappeared"))?;
    // Move the view tree before extraction: failure leaves all sidebar bookkeeping untouched.
    // Framework effects run after this synchronous transaction finishes.
    let moved = ctx.transfer_view_tree_to_window(tab_id, source_window, destination.window_id);
    if !moved.contains(&tab_id) {
        for id in moved.into_iter().rev() {
            ctx.transfer_view_to_window(id, destination.window_id, source_window);
        }
        return Ok(false);
    }
    let Some(tab) = source.update(ctx, |workspace, ctx| {
        workspace.take_tab_for_project_transfer(tab_id, ctx)
    }) else {
        for id in moved.into_iter().rev() {
            ctx.transfer_view_to_window(id, destination.window_id, source_window);
        }
        ctx.reparent_view(source_window, tab_id, source.id());
        return Ok(false);
    };
    ctx.reparent_view(destination.window_id, tab_id, destination.workspace.id());
    destination.workspace.update(ctx, |workspace, ctx| {
        workspace.accept_project_tab_drag(tab, ctx);
        if let Some(tab) = workspace
            .tabs
            .iter_mut()
            .find(|tab| tab.pane_group.id() == tab_id)
        {
            tab.selected_color = colors.0;
            tab.default_directory_color = colors.1;
        }
    });
    source_parent.update(ctx, |parent, ctx| {
        parent.settle_empty_workspace_after_tab_transfer(source.id(), ctx)
    });
    destination.parent.update(ctx, |parent, ctx| {
        parent.activate_project(destination.id, ctx)
    });
    Ok(true)
}
