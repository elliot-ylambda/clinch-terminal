//! Portable project layouts and exact live-tab transfers.
use serde::{Deserialize, Serialize};

pub const PROJECT_LAYOUT_VERSION: u32 = 1;
pub const MAX_PROJECT_LAYOUT_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectCreateParams {
    pub cwd: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectRestoreParams {
    pub layout: ProjectLayout,
    #[serde(default)]
    pub resume_agents: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TabTransferParams {
    pub destination_project: String,
    pub section_id: Option<String>,
    /// Zero-based position in the destination section, or the ungrouped tab list.
    pub index: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectLayout {
    pub version: u32,
    pub active_tab: usize,
    pub sections: Vec<SectionLayout>,
    pub tabs: Vec<TabLayout>,
    #[serde(default)]
    pub tasks: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SectionLayout {
    pub id: String,
    pub name: Option<String>,
    /// null inherits the default; "none" explicitly clears it.
    pub color: Option<String>,
    pub collapsed: bool,
    pub pinned: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TabLayout {
    pub title: Option<String>,
    pub color: Option<String>,
    pub pinned: bool,
    pub section: Option<String>,
    pub panes: PaneLayout,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PaneLayout {
    Terminal {
        cwd: Option<String>,
        title: Option<String>,
        focused: bool,
        resume: Option<AgentResume>,
    },
    Split {
        direction: SplitDirection,
        children: Vec<PaneLayout>,
        weights: Vec<f32>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitDirection {
    Horizontal,
    Vertical,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentResume {
    pub provider: ResumeProvider,
    pub conversation_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResumeProvider {
    Claude,
    Codex,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectTaskCreateParams {
    pub text: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectTaskUpdateParams {
    pub task_id: String,
    pub text: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectTaskIdParams {
    pub task_id: String,
}
