use std::collections::HashMap;

use warpui::{AppContext, Entity, EntityId, SingletonEntity, WeakViewHandle, WindowId};

use super::Workspace;

/// A registry that tracks every project workspace by physical window ID while
/// retaining an O(1) lookup for the active workspace in each window.
pub struct WorkspaceRegistry {
    windows: HashMap<WindowId, WindowWorkspaces>,
}

struct WindowWorkspaces {
    active_workspace_id: EntityId,
    workspaces: HashMap<EntityId, WeakViewHandle<Workspace>>,
}

impl Default for WorkspaceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkspaceRegistry {
    pub fn new() -> Self {
        Self {
            windows: HashMap::new(),
        }
    }

    /// Registers a workspace for the given window and makes it active.
    ///
    /// Project windows may own more than one live workspace for the same
    /// physical window. The latest workspace constructed is active until the
    /// project container explicitly selects another one.
    pub fn register(&mut self, window_id: WindowId, workspace: WeakViewHandle<Workspace>) {
        let workspace_id = workspace.id();
        let entry = self
            .windows
            .entry(window_id)
            .or_insert_with(|| WindowWorkspaces {
                active_workspace_id: workspace_id,
                workspaces: HashMap::new(),
            });
        entry.active_workspace_id = workspace_id;
        entry.workspaces.insert(workspace_id, workspace);
    }

    /// Registers (if necessary) and marks a workspace as active for a physical
    /// window.
    pub fn set_active(&mut self, window_id: WindowId, workspace: WeakViewHandle<Workspace>) {
        self.register(window_id, workspace);
    }

    pub fn is_active(&self, window_id: WindowId, workspace_id: EntityId) -> bool {
        self.windows
            .get(&window_id)
            .is_some_and(|entry| entry.active_workspace_id == workspace_id)
    }

    /// Removes one project workspace without unregistering its siblings.
    pub fn unregister_workspace(&mut self, window_id: WindowId, workspace_id: EntityId) {
        let Some(entry) = self.windows.get_mut(&window_id) else {
            return;
        };
        entry.workspaces.remove(&workspace_id);
        if entry.workspaces.is_empty() {
            self.windows.remove(&window_id);
            return;
        }
        if entry.active_workspace_id == workspace_id {
            entry.active_workspace_id = *entry
                .workspaces
                .keys()
                .next()
                .expect("non-empty workspace registry entry");
        }
    }

    /// Unregisters every project workspace for the given physical window.
    pub fn unregister(&mut self, window_id: WindowId) {
        self.windows.remove(&window_id);
    }

    /// Returns the active workspace for the given physical window.
    pub fn get(
        &self,
        window_id: WindowId,
        app: &AppContext,
    ) -> Option<warpui::ViewHandle<Workspace>> {
        let entry = self.windows.get(&window_id)?;
        entry
            .workspaces
            .get(&entry.active_workspace_id)?
            .upgrade(app)
    }

    /// Returns every live project workspace hosted by a physical window.
    pub fn get_all(
        &self,
        window_id: WindowId,
        app: &AppContext,
    ) -> Vec<warpui::ViewHandle<Workspace>> {
        self.windows
            .get(&window_id)
            .into_iter()
            .flat_map(|entry| entry.workspaces.values())
            .filter_map(|weak_handle| weak_handle.upgrade(app))
            .collect()
    }

    /// Returns all registered project workspaces that are still alive.
    /// A physical `WindowId` may therefore occur more than once.
    pub fn all_workspaces(
        &self,
        app: &AppContext,
    ) -> Vec<(WindowId, warpui::ViewHandle<Workspace>)> {
        self.windows
            .iter()
            .flat_map(|(window_id, entry)| {
                entry.workspaces.values().filter_map(move |weak_handle| {
                    weak_handle
                        .upgrade(app)
                        .map(|handle| (*window_id, handle))
                })
            })
            .collect()
    }
}

impl Entity for WorkspaceRegistry {
    type Event = ();
}

impl SingletonEntity for WorkspaceRegistry {}
