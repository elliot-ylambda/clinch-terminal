use std::path::PathBuf;

use serde::{Deserialize, Deserializer, Serialize};

use crate::app_state::{
    AppState, LeafContents, PaneNodeSnapshot, ProjectWindowSnapshot,
    SplitDirection as StateSplitDirection, TabSnapshot, WindowSnapshot,
};
use crate::themes::theme::AnsiColorIdentifier;

#[cfg(test)]
#[path = "launch_config_tests.rs"]
mod tests;

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct LaunchConfig {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub active_window_index: Option<usize>,
    pub windows: Vec<ProjectWindowTemplate>,
}

impl LaunchConfig {
    pub fn from_snapshot(name: String, app_state: &AppState) -> Self {
        let mut active_window_index = None;
        let mut windows = Vec::new();
        for (original_index, window) in app_state.windows.iter().enumerate() {
            if let Some(window) = ProjectWindowTemplate::from_snapshot(window) {
                if app_state.active_window_index == Some(original_index) {
                    active_window_index = Some(windows.len());
                }
                windows.push(window);
            }
        }

        Self {
            name,
            active_window_index,
            windows,
        }
    }
}

/// A physical window in a launch configuration.
///
/// The legacy representation stored one [`WindowTemplate`] directly in each
/// `windows` entry. The untagged `Legacy` variant keeps those files readable,
/// while newly saved configurations use `Grouped` to preserve project order.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(untagged)]
pub enum ProjectWindowTemplate {
    Grouped {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        active_project_index: Option<usize>,
        projects: Vec<WindowTemplate>,
    },
    Legacy(WindowTemplate),
}

impl ProjectWindowTemplate {
    pub fn grouped(projects: Vec<WindowTemplate>, active_project_index: Option<usize>) -> Self {
        Self::Grouped {
            active_project_index,
            projects,
        }
    }

    pub fn singleton(project: WindowTemplate) -> Self {
        Self::Legacy(project)
    }

    fn from_snapshot(snapshot: &ProjectWindowSnapshot) -> Option<Self> {
        let kept_projects = snapshot
            .projects
            .iter()
            .enumerate()
            .filter(|(_, project)| !project.quake_mode)
            .map(|(original_index, project)| (original_index, project.clone().into()))
            .collect::<Vec<_>>();
        if kept_projects.is_empty() {
            return None;
        }

        let active_project_index = kept_projects
            .iter()
            .position(|(original_index, _)| *original_index == snapshot.active_project_index)
            .unwrap_or_else(|| {
                kept_projects
                    .iter()
                    .take_while(|(original_index, _)| {
                        *original_index < snapshot.active_project_index
                    })
                    .count()
                    .saturating_sub(1)
            });
        Some(Self::Grouped {
            active_project_index: Some(active_project_index),
            projects: kept_projects
                .into_iter()
                .map(|(_, project)| project)
                .collect(),
        })
    }

    pub fn projects(&self) -> &[WindowTemplate] {
        match self {
            Self::Grouped { projects, .. } => projects,
            Self::Legacy(project) => std::slice::from_ref(project),
        }
    }

    pub fn into_projects(self) -> Vec<WindowTemplate> {
        match self {
            Self::Grouped { projects, .. } => projects,
            Self::Legacy(project) => vec![project],
        }
    }

    pub fn active_project_index(&self) -> usize {
        match self {
            Self::Grouped {
                active_project_index,
                projects,
            } => active_project_index
                .unwrap_or_default()
                .min(projects.len().saturating_sub(1)),
            Self::Legacy(_) => 0,
        }
    }

    pub fn active_project(&self) -> Option<&WindowTemplate> {
        self.projects().get(self.active_project_index())
    }

    pub fn active_project_mut(&mut self) -> Option<&mut WindowTemplate> {
        match self {
            Self::Grouped {
                active_project_index,
                projects,
            } => {
                let index = active_project_index
                    .unwrap_or_default()
                    .min(projects.len().saturating_sub(1));
                projects.get_mut(index)
            }
            Self::Legacy(project) => Some(project),
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct WindowTemplate {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub active_tab_index: Option<usize>,
    pub tabs: Vec<TabTemplate>,
}

impl From<WindowSnapshot> for WindowTemplate {
    fn from(snapshot: WindowSnapshot) -> Self {
        let mut active_tab_index = None;
        let mut num_valid_tabs = 0;

        let tabs = snapshot
            .tabs
            .into_iter()
            .enumerate()
            .filter_map(|(i, tab)| {
                let tab = tab.try_into().ok()?;

                if i == snapshot.active_tab_index {
                    active_tab_index = Some(num_valid_tabs);
                }

                num_valid_tabs += 1;

                Some(tab)
            })
            .collect::<Vec<TabTemplate>>();

        Self {
            active_tab_index,
            tabs,
        }
    }
}

fn is_falsey(val: &Option<bool>) -> bool {
    val.is_none_or(|v| !v)
}

/// The mode a leaf pane opens in.
///
/// Used by tab configs to distinguish terminal, agent, and cloud panes.
/// Launch configs always produce `Terminal` (the default).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PaneMode {
    /// A standard terminal shell session.
    #[default]
    Terminal,
    /// A terminal that immediately enters Agent Mode.
    Agent,
    /// A cloud-mode (ambient agent) pane with no local shell.
    Cloud,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(untagged, rename_all = "lowercase")]
pub enum PaneTemplateType {
    PaneTemplate {
        #[serde(deserialize_with = "deserialize_path")]
        cwd: PathBuf,
        #[serde(skip_serializing_if = "Vec::is_empty", default)]
        commands: Vec<CommandTemplate>,
        #[serde(skip_serializing_if = "is_falsey", default)]
        is_focused: Option<bool>,
        #[serde(default)]
        pane_mode: PaneMode,
        /// Optional shell override for this pane (e.g. `"pwsh"`, `"zsh"`).
        /// Sourced from the `shell` field of a tab config pane node.
        #[serde(skip_serializing_if = "Option::is_none", default)]
        shell: Option<String>,
    },
    PaneBranchTemplate {
        split_direction: SplitDirection,
        panes: Vec<PaneTemplateType>,
    },
}

impl TryFrom<PaneNodeSnapshot> for PaneTemplateType {
    type Error = ();

    #[allow(clippy::unwrap_in_result)]
    fn try_from(snapshot: PaneNodeSnapshot) -> Result<Self, ()> {
        match snapshot {
            PaneNodeSnapshot::Branch(branch) => {
                let panes = branch
                    .children
                    .iter()
                    .filter_map(|(_, snapshot)| snapshot.clone().try_into().ok())
                    .collect::<Vec<PaneTemplateType>>();
                match panes.len() {
                    0 => Err(()),
                    1 => Ok(panes
                        .into_iter()
                        .next()
                        .expect("Checked that panes has 1 element")),
                    _ => Ok(Self::PaneBranchTemplate {
                        split_direction: branch.direction.into(),
                        panes,
                    }),
                }
            }
            PaneNodeSnapshot::Leaf(leaf) => match leaf.contents {
                LeafContents::Terminal(terminal) => Ok(Self::PaneTemplate {
                    cwd: PathBuf::from(terminal.cwd.unwrap_or_default()),
                    commands: Vec::new(),
                    is_focused: Some(leaf.is_focused),
                    pane_mode: PaneMode::Terminal,
                    shell: None,
                }),
                // Currently, notebook panes cannot be saved in launch configurations.
                LeafContents::Notebook(_)
                | LeafContents::EnvVarCollection(_)
                | LeafContents::Code(_)
                | LeafContents::Workflow(_)
                | LeafContents::Settings(_)
                | LeafContents::AIFact(_)
                | LeafContents::CodeReview(_)
                | LeafContents::ExecutionProfileEditor
                | LeafContents::GetStarted
                | LeafContents::NetworkLog
                | LeafContents::AIDocument(_)
                | LeafContents::EnvironmentManagement(_)
                | LeafContents::AmbientAgent(_)
                | LeafContents::ImageViewer(_) => {
                    // TODO: Handle AIDocument in launch config
                    Err(())
                }
            },
        }
    }
}

/// Deserializes a string that semantically represents a path, expanding ~ as
/// needed.
fn deserialize_path<'de, D>(deserializer: D) -> Result<PathBuf, D::Error>
where
    D: Deserializer<'de>,
{
    let raw_path = String::deserialize(deserializer)?;
    Ok(PathBuf::from(shellexpand::tilde(&raw_path).into_owned()))
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct TabTemplate {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub title: Option<String>,
    pub layout: PaneTemplateType,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub color: Option<AnsiColorIdentifier>,
}

impl TryFrom<TabSnapshot> for TabTemplate {
    type Error = ();

    fn try_from(snapshot: TabSnapshot) -> Result<Self, ()> {
        let color = snapshot.color();
        Ok(Self {
            title: snapshot.custom_title,
            layout: snapshot.root.try_into()?,
            color,
        })
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SplitDirection {
    Vertical,
    Horizontal,
}

impl From<StateSplitDirection> for SplitDirection {
    fn from(snapshot: StateSplitDirection) -> Self {
        match snapshot {
            StateSplitDirection::Horizontal => Self::Horizontal,
            StateSplitDirection::Vertical => Self::Vertical,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct CommandTemplate {
    pub exec: String,
}

impl From<&str> for CommandTemplate {
    fn from(s: &str) -> CommandTemplate {
        CommandTemplate {
            exec: s.to_string(),
        }
    }
}

// TODO add extra elements to the mock (split panes, multiple tabs, multiple windows)
pub fn make_mock_single_window_launch_config() -> LaunchConfig {
    LaunchConfig {
        name: "Mocked Config".to_string(),
        active_window_index: Some(0),
        windows: vec![ProjectWindowTemplate::singleton(WindowTemplate {
            active_tab_index: Some(0),
            tabs: vec![
                TabTemplate {
                    title: Some("First Tab".to_string()),
                    layout: PaneTemplateType::PaneTemplate {
                        is_focused: Some(true),
                        cwd: PathBuf::from("/some/path"),
                        commands: vec!["echo test_command".into()],
                        pane_mode: PaneMode::Terminal,
                        shell: None,
                    },
                    color: None,
                },
                TabTemplate {
                    title: Some("Second Tab".to_string()),
                    layout: PaneTemplateType::PaneTemplate {
                        is_focused: Some(true),
                        cwd: PathBuf::from("/some/path"),
                        commands: vec!["echo test_command_on_another_tab".into()],
                        pane_mode: PaneMode::Terminal,
                        shell: None,
                    },
                    color: None,
                },
            ],
        })],
    }
}
