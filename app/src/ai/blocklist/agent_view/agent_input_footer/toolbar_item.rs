use serde::{Deserialize, Serialize};
use warpui::SingletonEntity;

use super::editor::AgentToolbarEditorMode;
use crate::context_chips::{agent_footer_available_chips, available_chips, ContextChipKind};
use crate::features::FeatureFlag;
use crate::settings::AISettings;
use crate::terminal::shared_session::SharedSessionStatus;
use crate::ui_components::icons::Icon;

/// Declares which footer(s) a toolbar item is available in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolbarAvailability {
    AgentViewOnly,
    CLIAgentOnly,
    Both,
}

impl ToolbarAvailability {
    pub fn is_available_for_agent_view(self) -> bool {
        matches!(self, Self::AgentViewOnly | Self::Both)
    }

    pub fn is_available_for_cli(self) -> bool {
        matches!(self, Self::CLIAgentOnly | Self::Both)
    }
}

/// A configurable item
///
/// This unifies context-chip data displays with interactive control buttons so
/// they can all be arranged through the same drag-and-drop editor.
#[derive(
    Clone,
    Debug,
    Eq,
    PartialEq,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "An item that can appear in the agent toolbar.",
    rename_all = "snake_case"
)]
pub enum AgentToolbarItemKind {
    #[schemars(description = "A prompt context chip.")]
    ContextChip(ContextChipKind),
    // Agent view only
    ModelSelector,
    NLDToggle,
    ContextWindowUsage,

    // CLI agent only
    FileExplorer,
    RichInput,

    // Both
    VoiceInput,
    // Renamed from ImageAttach; alias preserves existing user toolbar configs.
    #[serde(alias = "ImageAttach")]
    FileAttach,
    ShareSession,

    // CLI agent only – opens settings to the Coding Agents section.
    Settings,

    // CLI agent only – sends `/compact` to the running agent.
    Compact,

    // CLI agent only – forks this session into a new tab.
    ForkSession,

    // CLI agent only – submits "Continue" to the running agent.
    ContinuePrompt,

    // CLI agent only – submits "Looks good to me, continue" to the running agent.
    LooksGoodPrompt,

    // CLI agent only – exits Claude Code or Codex and continues in the other agent.
    TransferAgent,

    // CLI agent only – user-defined button that inserts-and-sends saved text.
    CustomInsert {
        label: String,
        text: String,
    },

    // Agent view only – shows fast-forward (auto-approve) toggle in the footer
    FastForwardToggle,

    // Agent view only – "Hand off to cloud" chip.
    HandoffToCloud,
}

impl AgentToolbarItemKind {
    pub fn available_in(&self) -> ToolbarAvailability {
        match self {
            Self::ContextChip(_) | Self::VoiceInput | Self::FileAttach | Self::ShareSession => {
                ToolbarAvailability::Both
            }
            Self::ModelSelector
            | Self::NLDToggle
            | Self::ContextWindowUsage
            | Self::FastForwardToggle
            | Self::HandoffToCloud => ToolbarAvailability::AgentViewOnly,
            Self::FileExplorer
            | Self::RichInput
            | Self::Settings
            | Self::Compact
            | Self::ForkSession
            | Self::ContinuePrompt
            | Self::TransferAgent
            | Self::CustomInsert { .. }
            | Self::LooksGoodPrompt => ToolbarAvailability::CLIAgentOnly,
        }
    }

    pub fn is_available_for_terminal(&self) -> bool {
        matches!(self, Self::CustomInsert { .. })
    }

    /// Whether this item should be visible to session viewers.
    /// Items that control host settings or initiate actions on the host's
    /// behalf are hidden from viewers.
    pub fn available_to_session_viewer(
        &self,
        status: &SharedSessionStatus,
        is_cloud_mode: bool,
    ) -> bool {
        match self {
            Self::Settings
            | Self::ShareSession
            | Self::FileExplorer
            | Self::Compact
            | Self::ForkSession
            | Self::ContinuePrompt
            | Self::TransferAgent
            | Self::CustomInsert { .. }
            | Self::LooksGoodPrompt => !status.is_viewer(),
            Self::FileAttach => !status.is_viewer() || is_cloud_mode,
            Self::FastForwardToggle => !status.is_viewer() || status.is_executor(),
            // Handoff is host-initiated; viewers cannot hand off another user's conversation.
            Self::HandoffToCloud => !status.is_viewer(),
            Self::ContextChip(_)
            | Self::ModelSelector
            | Self::NLDToggle
            | Self::ContextWindowUsage
            | Self::RichInput
            | Self::VoiceInput => true,
        }
    }

    pub fn display_label(&self) -> std::borrow::Cow<'static, str> {
        use std::borrow::Cow;
        match self {
            Self::ContextChip(_) => Cow::Borrowed("Context Chip"),
            Self::ModelSelector => Cow::Borrowed("Model Selector"),
            Self::NLDToggle => Cow::Borrowed("Autodetection"),
            Self::VoiceInput => Cow::Borrowed("Voice Input"),
            Self::FileAttach => Cow::Borrowed("Attach File"),
            Self::ContextWindowUsage => Cow::Borrowed("Context Usage"),
            Self::FileExplorer => Cow::Borrowed("File Explorer"),
            Self::RichInput => Cow::Borrowed("Rich Input"),
            Self::ShareSession => Cow::Borrowed("/remote-control"),
            Self::Settings => Cow::Borrowed("Settings"),
            Self::Compact => Cow::Borrowed("Compact"),
            Self::ForkSession => Cow::Borrowed("Fork in New Tab"),
            Self::ContinuePrompt => Cow::Borrowed("Continue"),
            Self::LooksGoodPrompt => Cow::Borrowed("LGTM"),
            Self::TransferAgent => Cow::Borrowed("Transfer agent"),
            Self::FastForwardToggle => Cow::Borrowed("Fast Forward"),
            Self::HandoffToCloud => Cow::Borrowed("Hand off to cloud"),
            Self::CustomInsert { label, .. } => Cow::Owned(label.clone()),
        }
    }

    pub fn icon(&self) -> Option<Icon> {
        match self {
            Self::ContextChip(kind) => kind.udi_icon(),
            Self::ModelSelector => Some(Icon::Oz),
            Self::NLDToggle => Some(Icon::NLD),
            Self::VoiceInput => Some(Icon::Microphone),
            Self::FileAttach => Some(Icon::Plus),
            Self::ContextWindowUsage => Some(Icon::ConversationContext0),
            Self::FileExplorer => Some(Icon::FileCopy),
            Self::RichInput => Some(Icon::TextInput),
            Self::ShareSession => Some(Icon::Phone01),
            Self::Settings => Some(Icon::Settings),
            Self::Compact => Some(Icon::Minimize),
            Self::ForkSession => Some(Icon::GitBranch),
            Self::ContinuePrompt => Some(Icon::Play),
            Self::LooksGoodPrompt => Some(Icon::ThumbsUp),
            Self::TransferAgent => Some(Icon::SwitchHorizontal01),
            Self::FastForwardToggle => Some(Icon::FastForward),
            // The bundled `upload-cloud-01.svg` (cloud-with-upward-arrow) is the
            // closest fit among the existing icons for V0; design may swap it later.
            Self::HandoffToCloud => Some(Icon::UploadCloud),
            Self::CustomInsert { .. } => Some(Icon::Play),
        }
    }

    /// Whether this item should remain visible during `&` handoff-compose mode.
    /// Only items relevant to composing a cloud run are shown.
    pub(super) fn is_available_during_handoff_compose(&self) -> bool {
        match self {
            Self::ContextChip(
                ContextChipKind::ShellGitBranch | ContextChipKind::GitBranchStatus,
            ) => true,
            Self::ModelSelector | Self::VoiceInput | Self::FileAttach => true,
            Self::ContextChip(_)
            | Self::NLDToggle
            | Self::ContextWindowUsage
            | Self::FastForwardToggle
            | Self::HandoffToCloud
            | Self::ShareSession
            | Self::FileExplorer
            | Self::RichInput
            | Self::Settings
            | Self::Compact
            | Self::ForkSession
            | Self::ContinuePrompt
            | Self::TransferAgent
            | Self::CustomInsert { .. }
            | Self::LooksGoodPrompt => false,
        }
    }

    /// Whether this item should be included in the toolbar given the current app state.
    /// Feature-flag checks live in `all_available()` / `default_*()`. This method
    /// handles runtime conditions that depend on user settings or workspace state.
    pub fn is_available(&self, app: &warpui::AppContext) -> bool {
        match self {
            Self::HandoffToCloud => AISettings::as_ref(app).is_cloud_handoff_enabled(app),
            _ => true,
        }
    }

    pub fn is_context_chip(&self) -> bool {
        matches!(self, Self::ContextChip(_))
    }

    pub fn context_chip_kind(&self) -> Option<&ContextChipKind> {
        match self {
            Self::ContextChip(kind) => Some(kind),
            _ => None,
        }
    }

    /// Default left-side items for the agent view footer.
    pub fn default_left() -> Vec<Self> {
        let mut items = vec![
            Self::ContextChip(ContextChipKind::Ssh),
            Self::ContextChip(ContextChipKind::WorkingDirectory),
            Self::ContextChip(ContextChipKind::ShellGitBranch),
            Self::ContextChip(ContextChipKind::GitDiffStats),
        ];
        if FeatureFlag::GithubPrPromptChip.is_enabled() {
            items.push(Self::ContextChip(ContextChipKind::GithubPullRequest));
        }
        items.push(Self::NLDToggle);
        items
    }

    /// Default right-side items for the agent view footer.
    pub fn default_right() -> Vec<Self> {
        let mut items = vec![
            Self::ContextChip(ContextChipKind::AgentPlanAndTodoList),
            Self::ContextWindowUsage,
            Self::ModelSelector,
        ];
        if FeatureFlag::CreatingSharedSessions.is_enabled()
            && FeatureFlag::HOARemoteControl.is_enabled()
        {
            items.push(Self::ShareSession);
        }
        if FeatureFlag::OzHandoff.is_enabled()
            && FeatureFlag::HandoffLocalCloud.is_enabled()
            && cfg!(all(feature = "local_fs", not(target_family = "wasm")))
        {
            items.push(Self::HandoffToCloud);
        }
        items.push(Self::VoiceInput);
        items.push(Self::FileAttach);
        items
    }

    /// All items available for the agent view footer configurator.
    pub fn all_available() -> Vec<Self> {
        let mut items: Vec<Self> = agent_footer_available_chips()
            .into_iter()
            .map(Self::ContextChip)
            .collect();
        items.extend([
            Self::ModelSelector,
            Self::NLDToggle,
            Self::VoiceInput,
            Self::FileAttach,
            Self::ContextWindowUsage,
        ]);
        if FeatureFlag::FastForwardAutoexecuteButton.is_enabled() {
            items.push(Self::FastForwardToggle);
        }
        if FeatureFlag::CreatingSharedSessions.is_enabled()
            && FeatureFlag::HOARemoteControl.is_enabled()
        {
            items.push(Self::ShareSession);
        }
        if FeatureFlag::OzHandoff.is_enabled()
            && FeatureFlag::HandoffLocalCloud.is_enabled()
            && cfg!(all(feature = "local_fs", not(target_family = "wasm")))
        {
            items.push(Self::HandoffToCloud);
        }
        items
    }

    /// Default left-side items for the CLI agent footer.
    ///
    /// `FileAttach` (+), the `GitDiffStats` (±) chip, `FileExplorer`, and
    /// `RichInput` are intentionally omitted from the default layout — the file
    /// explorer now lives in the header toolbar. All four remain in
    /// `all_available_for_cli_input`, so they can be dragged back via the footer
    /// toolbar editor.
    pub fn cli_default_left() -> Vec<Self> {
        vec![
            Self::ForkSession,
            Self::Compact,
            Self::ContinuePrompt,
            Self::LooksGoodPrompt,
            Self::TransferAgent,
            Self::CustomInsert {
                label: "Create a PR".to_owned(),
                text: "Create a PR, then merge main into this PR".to_owned(),
            },
            Self::CustomInsert {
                label: "Review w/ Codex Sol Max".to_owned(),
                text: "Review w/ Codex Sol Max".to_owned(),
            },
            Self::CustomInsert {
                label: "Review w/ Claude Code Fable".to_owned(),
                text: "Review w/ Claude Code Fable".to_owned(),
            },
            Self::CustomInsert {
                label: "Debug w/ Ultracode".to_owned(),
                text: "Investigate with Ultra Code and use subagents".to_owned(),
            },
            Self::CustomInsert {
                label: "Git Worktree".to_owned(),
                text: "Move our current work and code into an isolated git work tree. And create a branch. Work out of the git worktree".to_owned(),
            },
            Self::VoiceInput,
        ]
    }

    /// Default right-side items for the CLI agent footer.
    pub fn cli_default_right() -> Vec<Self> {
        let mut items = vec![
            Self::ContextChip(ContextChipKind::WorkingDirectory),
            Self::ContextChip(ContextChipKind::ShellGitBranch),
            Self::Settings,
        ];
        if FeatureFlag::CreatingSharedSessions.is_enabled()
            && FeatureFlag::HOARemoteControl.is_enabled()
        {
            items.push(Self::ShareSession);
        }
        items
    }

    /// Default left-side items for the plain terminal footer.
    pub fn terminal_default_left() -> Vec<Self> {
        [
            ("Claude", "ca"),
            ("Codex", "cx"),
            ("Claude resume", "ca --resume"),
            ("Codex resume", "codex resume"),
            ("Open", "open ."),
        ]
        .into_iter()
        .map(|(label, text)| Self::CustomInsert {
            label: label.to_owned(),
            text: text.to_owned(),
        })
        .collect()
    }

    /// Default right-side items for the plain terminal footer.
    pub fn terminal_default_right() -> Vec<Self> {
        Vec::new()
    }

    /// Items available when resetting the plain terminal footer configurator.
    pub fn all_available_for_terminal_input() -> Vec<Self> {
        Self::terminal_default_left()
    }

    /// All items available for the CLI agent footer configurator.
    pub fn all_available_for_cli_input() -> Vec<Self> {
        let mut items: Vec<Self> = available_chips()
            .into_iter()
            .map(Self::ContextChip)
            .collect();
        items.extend([
            Self::FileExplorer,
            Self::RichInput,
            Self::FileAttach,
            Self::VoiceInput,
            Self::ForkSession,
            Self::Compact,
            Self::ContinuePrompt,
            Self::LooksGoodPrompt,
            Self::TransferAgent,
            Self::Settings,
        ]);
        if FeatureFlag::CreatingSharedSessions.is_enabled()
            && FeatureFlag::HOARemoteControl.is_enabled()
        {
            items.push(Self::ShareSession);
        }
        items
    }

    /// Returns the appropriate defaults and available items for a given editor mode.
    pub fn defaults_for_mode(mode: AgentToolbarEditorMode) -> (Vec<Self>, Vec<Self>, Vec<Self>) {
        match mode {
            AgentToolbarEditorMode::AgentView => (
                Self::default_left(),
                Self::default_right(),
                Self::all_available(),
            ),
            AgentToolbarEditorMode::CLIAgent => (
                Self::cli_default_left(),
                Self::cli_default_right(),
                Self::all_available_for_cli_input(),
            ),
        }
    }
}

impl From<ContextChipKind> for AgentToolbarItemKind {
    fn from(kind: ContextChipKind) -> Self {
        Self::ContextChip(kind)
    }
}

#[cfg(test)]
#[path = "toolbar_item_tests.rs"]
mod tests;
