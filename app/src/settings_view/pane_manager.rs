use std::collections::HashMap;

use warpui::{Entity, EntityId, ModelContext, SingletonEntity, ViewHandle, WindowId};

use super::SettingsView;
use crate::pane_group::{PaneContent, PaneId, SettingsPane};
use crate::PaneViewLocator;

struct SettingsPaneData {
    window_id: WindowId,
    locator: Option<PaneViewLocator>,
    settings_view: ViewHandle<SettingsView>,
}

/// Singleton model to manage state of settings panes across workspaces. Specifically:
/// - Maintains settings view handles to preserve state when panes are hidden
/// - Tracks currently open settings panes and their location
#[derive(Default)]
pub struct SettingsPaneManager {
    panes: HashMap<EntityId, SettingsPaneData>,
    active_view_by_window: HashMap<WindowId, EntityId>,
}

impl SettingsPaneManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn settings_view(&self, window_id: WindowId) -> ViewHandle<SettingsView> {
        let settings_view_id = self
            .active_view_by_window
            .get(&window_id)
            .expect("Window should have an active settings view");
        self.panes
            .get(settings_view_id)
            .expect("Active settings view should be registered")
            .settings_view
            .clone()
    }

    pub fn settings_views(&self, window_id: WindowId) -> Vec<ViewHandle<SettingsView>> {
        self.panes
            .values()
            .filter(|data| data.window_id == window_id)
            .map(|data| data.settings_view.clone())
            .collect()
    }

    pub fn register_view(&mut self, window_id: WindowId, view: ViewHandle<SettingsView>) {
        let settings_view_id = view.id();
        self.panes.insert(
            settings_view_id,
            SettingsPaneData {
                window_id,
                locator: None,
                settings_view: view,
            },
        );
        self.active_view_by_window
            .insert(window_id, settings_view_id);
    }

    pub fn activate_view(&mut self, window_id: WindowId, settings_view_id: EntityId) {
        let belongs_to_window = self
            .panes
            .get(&settings_view_id)
            .is_some_and(|data| data.window_id == window_id);
        if belongs_to_window {
            self.active_view_by_window
                .insert(window_id, settings_view_id);
        } else {
            log::warn!(
                "Cannot activate unregistered settings view {settings_view_id:?} in {window_id:?}"
            );
        }
    }

    pub fn unregister_view(&mut self, settings_view_id: EntityId) {
        let Some(data) = self.panes.remove(&settings_view_id) else {
            return;
        };
        if self.active_view_by_window.get(&data.window_id) == Some(&settings_view_id) {
            if let Some(replacement) = self.panes.iter().find_map(|(view_id, candidate)| {
                (candidate.window_id == data.window_id).then_some(*view_id)
            }) {
                self.active_view_by_window
                    .insert(data.window_id, replacement);
            } else {
                self.active_view_by_window.remove(&data.window_id);
            }
        }
    }

    pub fn move_view(
        &mut self,
        settings_view_id: EntityId,
        source_window_id: WindowId,
        target_window_id: WindowId,
    ) {
        let Some(data) = self.panes.get_mut(&settings_view_id) else {
            log::warn!("Cannot move unregistered settings view {settings_view_id:?}");
            return;
        };
        data.window_id = target_window_id;

        if self.active_view_by_window.get(&source_window_id) == Some(&settings_view_id) {
            if let Some(replacement) = self.panes.iter().find_map(|(view_id, candidate)| {
                (candidate.window_id == source_window_id).then_some(*view_id)
            }) {
                self.active_view_by_window
                    .insert(source_window_id, replacement);
            } else {
                self.active_view_by_window.remove(&source_window_id);
            }
        }
        self.active_view_by_window
            .insert(target_window_id, settings_view_id);
    }

    pub fn find_pane(&self, settings_view_id: EntityId) -> Option<PaneViewLocator> {
        self.panes
            .get(&settings_view_id)
            .and_then(|data| data.locator)
    }

    pub fn register_pane(
        &mut self,
        settings_view_id: EntityId,
        pane: &SettingsPane,
        pane_group_id: EntityId,
        window_id: WindowId,
        _ctx: &mut ModelContext<Self>,
    ) {
        if let Some(data) = self.panes.get_mut(&settings_view_id) {
            data.window_id = window_id;
            data.locator = Some(PaneViewLocator {
                pane_group_id,
                pane_id: pane.id(),
            });
        } else {
            log::warn!("Settings view should already exist for settings pane");
        }
    }

    pub fn deregister_pane(
        &mut self,
        settings_view_id: EntityId,
        pane_group_id: EntityId,
        pane_id: PaneId,
        _ctx: &mut ModelContext<Self>,
    ) {
        if let Some(data) = self.panes.get_mut(&settings_view_id) {
            let locator = PaneViewLocator {
                pane_group_id,
                pane_id,
            };
            if data.locator == Some(locator) {
                data.locator = None;
            }
        }
    }
}

impl Entity for SettingsPaneManager {
    type Event = ();
}

/// Mark SettingsPaneManager as global application state.
impl SingletonEntity for SettingsPaneManager {}
