use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use pathfinder_geometry::rect::RectF;
use serde::{Deserialize, Serialize};
use warpui::platform::FullscreenState;
use warpui::AppContext;

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent_conversations_model::AgentManagementFilters;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::blocklist::{InputConfig, SerializedBlockListItem};
use crate::code::editor_management::CodeSource;
use crate::drive::OpenWarpDriveObjectSettings;
use crate::root_view::{quake_mode_window_id, RootView};
use crate::server::ids::SyncId;
use crate::settings_view::environments_page::EnvironmentsPage;
use crate::settings_view::SettingsSection;
use crate::tab::SelectedTabColor;
use crate::terminal::ShellLaunchData;
use crate::themes::theme::AnsiColorIdentifier;
use crate::workspace::tab_group::{SelectedSectionColor, TabGroupId};
use crate::workspace::task::WorkspaceTask;
use crate::workspace::view::left_panel::ToolPanelView;

#[derive(Debug, Clone, PartialEq)]
pub struct AppState {
    pub windows: Vec<ProjectWindowSnapshot>,
    pub active_window_index: Option<usize>,
    pub block_lists: Arc<HashMap<PaneUuid, Vec<SerializedBlockListItem>>>,
    pub running_mcp_servers: Vec<uuid::Uuid>,
}

impl AppState {
    /// Stable pane UUIDs that participate in this restorable snapshot. Agent-resume uses
    /// this set as its ownership boundary so append-only history and registry files left by
    /// closed panes cannot block a live pane from recovering its conversation.
    pub(crate) fn terminal_pane_uuids(&self) -> Vec<Vec<u8>> {
        let mut uuids = Vec::new();
        for window in &self.windows {
            for project in &window.projects {
                for tab in &project.tabs {
                    tab.root.collect_terminal_pane_uuids(&mut uuids);
                }
            }
        }
        uuids
    }
}

/// Snapshot of one physical window and the ordered project workspaces it
/// contains. `WindowSnapshot` remains the snapshot of one project workspace.
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectWindowSnapshot {
    pub projects: Vec<WindowSnapshot>,
    pub active_project_index: usize,
}

impl ProjectWindowSnapshot {
    pub fn singleton(project: WindowSnapshot) -> Self {
        Self {
            projects: vec![project],
            active_project_index: 0,
        }
    }

    pub fn active_project(&self) -> Option<&WindowSnapshot> {
        self.projects
            .get(self.active_project_index)
            .or_else(|| self.projects.first())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PaneUuid(pub Vec<u8>);

/// Wrapper for persisting agent management filters to restore.
#[derive(Default, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedAgentManagementFilters {
    pub filters: AgentManagementFilters,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowSnapshot {
    pub tabs: Vec<TabSnapshot>,
    pub active_tab_index: usize,
    pub bounds: Option<RectF>,
    pub fullscreen_state: FullscreenState,
    pub quake_mode: bool,
    pub universal_search_width: Option<f32>,
    pub warp_ai_width: Option<f32>,
    pub voltron_width: Option<f32>,
    pub warp_drive_index_width: Option<f32>,
    pub left_panel_open: bool,
    pub vertical_tabs_panel_open: bool,
    pub left_panel_width: Option<f32>,
    pub right_panel_width: Option<f32>,
    pub agent_management_filters: Option<PersistedAgentManagementFilters>,
    /// Tab groups defined in this window. Group order is implicit from
    /// member tabs' positions, so no explicit ordering is persisted.
    pub tab_groups: Vec<TabGroupSnapshot>,
    pub tasks: Vec<WorkspaceTask>,
    pub tasks_collapsed: bool,
    pub bookmarked_sessions_color: SelectedSectionColor,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TabGroupSnapshot {
    pub id: TabGroupId,
    pub name: Option<String>,
    pub color: SelectedSectionColor,
    pub collapsed: bool,
    pub pinned: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TabSnapshot {
    pub custom_title: Option<String>,
    pub root: PaneNodeSnapshot,
    pub default_directory_color: Option<AnsiColorIdentifier>,
    pub selected_color: SelectedTabColor,
    pub left_panel: Option<LeftPanelSnapshot>,
    pub right_panel: Option<RightPanelSnapshot>,
    /// Tab group this tab belongs to, if any.
    pub group_id: Option<TabGroupId>,
    /// True when this tab is pinned to the front of the tab list.
    pub pinned: bool,
}

impl TabSnapshot {
    pub(crate) fn color(&self) -> Option<AnsiColorIdentifier> {
        self.selected_color.resolve(self.default_directory_color)
    }
}

#[derive(Clone, Debug, PartialEq)]
#[allow(
    clippy::large_enum_variant,
    reason = "LeafSnapshot is significantly larger than BranchSnapshot due to nested snapshot types."
)]
pub enum PaneNodeSnapshot {
    Branch(BranchSnapshot),
    Leaf(LeafSnapshot),
}

impl PaneNodeSnapshot {
    pub fn has_horizontal_split(&self) -> bool {
        match self {
            PaneNodeSnapshot::Leaf(_) => false,
            PaneNodeSnapshot::Branch(BranchSnapshot {
                direction,
                children,
            }) => {
                let self_has_split = *direction == SplitDirection::Horizontal && children.len() > 1;
                self_has_split
                    || children
                        .iter()
                        .any(|(_, child)| child.has_horizontal_split())
            }
        }
    }

    fn collect_terminal_pane_uuids(&self, uuids: &mut Vec<Vec<u8>>) {
        match self {
            PaneNodeSnapshot::Branch(branch) => {
                for (_, child) in &branch.children {
                    child.collect_terminal_pane_uuids(uuids);
                }
            }
            PaneNodeSnapshot::Leaf(LeafSnapshot {
                contents: LeafContents::Terminal(terminal),
                ..
            }) => uuids.push(terminal.uuid.clone()),
            PaneNodeSnapshot::Leaf(_) => {}
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BranchSnapshot {
    pub direction: SplitDirection,
    pub children: Vec<(PaneFlex, PaneNodeSnapshot)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LeafSnapshot {
    pub is_focused: bool,
    pub custom_vertical_tabs_title: Option<String>,
    pub contents: LeafContents,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LeafContents {
    Terminal(TerminalPaneSnapshot),
    Notebook(NotebookPaneSnapshot),
    AIDocument(AIDocumentPaneSnapshot),
    Code(CodePaneSnapShot),
    EnvVarCollection(EnvVarCollectionPaneSnapshot),
    EnvironmentManagement(EnvironmentManagementPaneSnapshot),
    Workflow(WorkflowPaneSnapshot),
    Settings(SettingsPaneSnapshot),
    AIFact(AIFactPaneSnapshot),
    ExecutionProfileEditor,
    CodeReview(CodeReviewPaneSnapshot),
    AmbientAgent(AmbientAgentPaneSnapshot),
    /// The in-app network log pane. Not persisted across restarts because the
    /// backing log is an in-memory ring buffer that starts empty on launch.
    NetworkLog,
    /// A new first-time user experience which prioritizes choosing a coding repository.
    GetStarted,
    /// An image preview pane. Only the local path is persisted; zoom/pan/backdrop
    /// reset to defaults on restore.
    ImageViewer(ImagePaneSnapshot),
}

#[cfg(feature = "local_fs")]
impl LeafContents {
    /// Whether this pane content should be written to (and later restored
    /// from) the SQLite app-state database.
    ///
    /// Non-persisted pane types are skipped entirely during the pane tree
    /// traversal in `save_app_state`, so no `pane_nodes` row is inserted for
    /// them. This is important: inserting a `pane_nodes` row with
    /// `is_leaf = true` but no matching `pane_leaves` row leaves an orphan
    /// that `read_node` cannot resolve, which causes the surrounding tab's
    /// restoration to fail and the whole tab to disappear on restart.
    pub(crate) fn is_persisted(&self) -> bool {
        match self {
            // Network log: the backing log is an in-memory ring buffer that
            // starts empty on launch; persisting would also regress back to
            // an on-disk log via the app-state database.
            LeafContents::NetworkLog
            // Environment management panes are opened on-demand via workspace
            // actions and have no persistable state.
            | LeafContents::EnvironmentManagement(_) => false,
            LeafContents::Terminal(_)
            | LeafContents::Notebook(_)
            | LeafContents::AIDocument(_)
            | LeafContents::Code(_)
            | LeafContents::EnvVarCollection(_)
            | LeafContents::Workflow(_)
            | LeafContents::Settings(_)
            | LeafContents::AIFact(_)
            | LeafContents::ExecutionProfileEditor
            | LeafContents::CodeReview(_)
            | LeafContents::AmbientAgent(_)
            | LeafContents::GetStarted
            | LeafContents::ImageViewer(_) => true,
        }
    }
}

/// Snapshot of an ambient agent pane.
#[derive(Clone, Debug, PartialEq)]
pub struct AmbientAgentPaneSnapshot {
    pub uuid: Vec<u8>,
    // `task_id` is purposefully optional,
    // as you can have a valid state (i.e. an empty cloud mode pane) where it is None.
    pub task_id: Option<AmbientAgentTaskId>,
}

/// Snapshot of the contents of a terminal pane.
#[derive(Clone, Debug, PartialEq)]
pub struct TerminalPaneSnapshot {
    pub uuid: Vec<u8>,
    pub cwd: Option<String>,
    pub shell_launch_data: Option<ShellLaunchData>,
    pub is_active: bool,
    pub is_read_only: bool,
    pub input_config: Option<InputConfig>,
    pub llm_model_override: Option<String>,
    pub active_profile_id: Option<SyncId>,
    pub conversation_ids_to_restore: Vec<AIConversationId>,
    /// The active conversation ID if the agent view was open in fullscreen mode.
    /// When `Some`, the agent view should be restored to fullscreen for this conversation.
    pub active_conversation_id: Option<AIConversationId>,
    /// Command to auto-run after the restored shell boots (e.g. `claude --resume <id>`).
    pub on_restore_command: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum NotebookPaneSnapshot {
    CloudNotebook {
        /// The ID of the notebook that was open in this pane. There are 3 possibilities:
        /// 1. The pane contains a newly-created notebook that has not been edited yet. It might not
        ///    have an ID yet (client or server), so this will be `None`.
        /// 2. The pane contains a notebook that hasn't been synced to the server yet, so this will
        ///    contain a client ID that should exist in SQLite.
        /// 3. The pane contains a notebook that's known to the server, so this will contain the
        ///    server ID.
        notebook_id: Option<SyncId>,
        // Settings for the notebook pane when it's opened (such as a folder to focus upon opening)
        settings: OpenWarpDriveObjectSettings,
    },
    LocalFileNotebook {
        /// The path to the local file that was open in this pane. This may be `None` if
        /// the pane contained an unreadable file.
        path: Option<PathBuf>,
    },
}

/// Snapshot of an image preview pane. Only the local path is persisted;
/// zoom/pan/backdrop reset to defaults on restore.
#[derive(Clone, Debug, PartialEq)]
pub struct ImagePaneSnapshot {
    /// `None` if the pane held an unreadable/remote image.
    pub path: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AIDocumentPaneSnapshot {
    Local {
        document_id: String,
        version: i32,
        content: Option<String>,
        title: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct CodePaneTabSnapshot {
    pub path: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CodePaneSnapShot {
    Local {
        tabs: Vec<CodePaneTabSnapshot>,
        active_tab_index: usize,
        /// The full `CodeSource` for this pane, serialized as JSON in the DB.
        source: Option<CodeSource>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum WorkflowPaneSnapshot {
    CloudWorkflow {
        workflow_id: Option<SyncId>,
        // Settings for the workflow pane when it's opened (such as a folder to focus upon opening)
        settings: OpenWarpDriveObjectSettings,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum EnvVarCollectionPaneSnapshot {
    // CloudEnvVarCollection snapshots operate under the same heuristics
    // as NotebookPaneSnapshot::CloudNotebook
    CloudEnvVarCollection {
        env_var_collection_id: Option<SyncId>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct EnvironmentManagementPaneSnapshot {
    pub mode: EnvironmentsPage,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SettingsPaneSnapshot {
    Local {
        current_page: SettingsSection,
        search_query: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum AIFactPaneSnapshot {
    Personal,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CodeReviewPaneSnapshot {
    Local {
        terminal_uuid: Vec<u8>,
        repo_path: PathBuf,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LeftPanelDisplayedTab {
    FileTree,
    GlobalSearch,
    WarpDrive,
    ConversationListView,
    Skills,
}

impl From<ToolPanelView> for LeftPanelDisplayedTab {
    fn from(view: ToolPanelView) -> Self {
        match view {
            ToolPanelView::ProjectExplorer => LeftPanelDisplayedTab::FileTree,
            ToolPanelView::GlobalSearch { .. } => LeftPanelDisplayedTab::GlobalSearch,
            ToolPanelView::WarpDrive => LeftPanelDisplayedTab::WarpDrive,
            ToolPanelView::ConversationListView => LeftPanelDisplayedTab::ConversationListView,
            ToolPanelView::Skills => LeftPanelDisplayedTab::Skills,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LeftPanelSnapshot {
    pub left_panel_displayed_tab: LeftPanelDisplayedTab,
    pub pane_group_id: String,
    pub width: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RightPanelSnapshot {
    pub pane_group_id: String,
    pub width: usize,
    pub is_maximized: bool,
}

/// Copied from pane group model, which should be private to pane group.
#[derive(Clone, Debug, PartialEq)]
pub enum SplitDirection {
    Horizontal,
    Vertical,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PaneFlex(pub f32);

pub fn get_app_state(app: &AppContext) -> AppState {
    let active_window_id = app.windows().active_window();
    let quake_mode_id = quake_mode_window_id();

    let mut active_window_index = None;

    let mut windows = vec![];

    for window_id in app.window_ids() {
        let Some(root_view) = app.root_view::<RootView>(window_id) else {
            continue;
        };
        let Some(project_window) = root_view.as_ref(app).project_window() else {
            continue;
        };
        let project_window = project_window.as_ref(app);
        if project_window.is_empty() {
            continue;
        }
        if project_window
            .active_workspace()
            .as_ref(app)
            .is_tab_drag_preview()
        {
            continue;
        }

        let kept_projects = project_window
            .projects()
            .enumerate()
            .filter_map(|(original_index, (_, workspace))| {
                let workspace = workspace.as_ref(app);
                if workspace.is_tab_drag_preview() {
                    return None;
                }
                let snapshot = workspace.snapshot(
                    window_id,
                    quake_mode_id.is_some_and(|id| id == window_id),
                    app,
                );
                (!snapshot.tabs.is_empty()).then_some((original_index, snapshot))
            })
            .collect::<Vec<_>>();
        if kept_projects.is_empty() {
            continue;
        }

        // The saved index must address the FILTERED list: drag previews and empty
        // snapshots were dropped above, so the in-memory active index may not survive
        // as-is. If the active project itself was dropped, fall back to its nearest
        // surviving predecessor.
        let active_project_index = project_window.active_project_index();
        let saved_active_index = kept_projects
            .iter()
            .position(|(original_index, _)| *original_index == active_project_index)
            .unwrap_or_else(|| {
                kept_projects
                    .iter()
                    .take_while(|(original_index, _)| *original_index < active_project_index)
                    .count()
                    .saturating_sub(1)
            });

        if active_window_id == Some(window_id) {
            active_window_index = Some(windows.len());
        }
        windows.push(ProjectWindowSnapshot {
            projects: kept_projects
                .into_iter()
                .map(|(_, snapshot)| snapshot)
                .collect(),
            active_project_index: saved_active_index,
        });
    }

    AppState {
        windows,
        active_window_index,
        block_lists: Default::default(),
        running_mcp_servers: Vec::new(),
    }
}

#[cfg(test)]
#[path = "app_state_tests.rs"]
mod tests;
