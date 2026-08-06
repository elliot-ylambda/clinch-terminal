use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const MAX_WORKSPACE_TASK_TEXT_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct WorkspaceTaskId(pub Uuid);

impl WorkspaceTaskId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for WorkspaceTaskId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for WorkspaceTaskId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkspaceTask {
    pub id: WorkspaceTaskId,
    pub text: String,
}

impl WorkspaceTask {
    pub fn new(text: impl Into<String>) -> Option<Self> {
        let text = text.into();
        let text = text.trim();
        if text.is_empty() || text.len() > MAX_WORKSPACE_TASK_TEXT_BYTES {
            return None;
        }
        Some(Self {
            id: WorkspaceTaskId::new(),
            text: text.to_owned(),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceTaskAgent {
    Claude,
    Codex,
}

#[cfg(test)]
mod tests {
    use super::{WorkspaceTask, MAX_WORKSPACE_TASK_TEXT_BYTES};

    #[test]
    fn task_text_is_trimmed_and_empty_text_is_rejected() {
        assert_eq!(
            WorkspaceTask::new("  ship it  ").map(|task| task.text),
            Some("ship it".to_owned())
        );
        assert!(WorkspaceTask::new("  \n\t ").is_none());
    }

    #[test]
    fn oversized_task_text_is_rejected() {
        assert!(WorkspaceTask::new("x".repeat(MAX_WORKSPACE_TASK_TEXT_BYTES + 1)).is_none());
    }
}
