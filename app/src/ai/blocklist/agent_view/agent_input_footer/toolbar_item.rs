use serde::{Deserialize, Serialize};
use warpui::SingletonEntity;

use super::editor::AgentToolbarEditorMode;
use crate::context_chips::{agent_footer_available_chips, ContextChipKind};
use crate::features::FeatureFlag;
use crate::settings::AISettings;
use crate::terminal::shared_session::SharedSessionStatus;
use crate::ui_components::icons::Icon;

/// Exact quick-insert definitions that once shipped as mandatory CLI-footer defaults.
/// Never mutate or remove entries: this is persisted-format migration data, not the live catalog.
const RETIRED_CLI_DEFAULT_QUICK_INSERT_SNAPSHOTS: &[(&str, &str)] = &[
    ("/codex", "/codex"),
    (
        "Make No Mistakes",
        "Do it all for me. I'm stepping away. Don't make any mistakes.",
    ),
    ("Create a Plan", "Create a Plan"),
    ("Build w/ Sub-agents", "Build w/ Sub-agents"),
    (
        "Create a PR",
        "Create a PR, then merge main into this PR",
    ),
    (
        "Worktree-Build",
        "OK go into an isolated work tree. Plan this out, then implement it and create a pull request.",
    ),
    ("Review w/ Codex Sol Max", "Review w/ Codex Sol Max"),
    (
        "Review w/ Claude Code Fable",
        "Review w/ Claude Code Fable",
    ),
    (
        "Debug w/ Ultracode",
        "Investigate with Ultra Code and use subagents",
    ),
    (
        "Git Worktree",
        "Move our current work and code into an isolated git work tree. And create a branch. Work out of the git worktree",
    ),
    (
        "Fix & Verify",
        "Implement the requested fix, run the most relevant checks, and summarize what changed.",
    ),
    (
        "Simplify",
        "Simplify the current implementation without changing behavior, then run the relevant tests.",
    ),
    ("Push2Main", "Push all these changes to main."),
];

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

    // CLI agent only – toggles this Claude Code or Codex conversation in finder bookmarks.
    BookmarkConversation,

    // CLI agent only – submits "Continue" to the running agent.
    ContinuePrompt,

    // CLI agent only – submits "Looks good to me, continue" to the running agent.
    LooksGoodPrompt,

    // CLI agent only – copies the unsent draft and clears the composer.
    CopyAndClearDraft,

    // CLI agent only – exits Claude Code or Codex and continues in the other agent.
    TransferAgent,

    // CLI agent only – configurable button that either pre-fills or submits saved text.
    CustomInsert {
        label: String,
        text: String,
        /// Whether activating the button immediately submits the text. This defaults to true so
        /// existing settings retain their historical insert-and-send behavior.
        #[serde(
            default = "default_custom_insert_auto_send",
            skip_serializing_if = "custom_insert_auto_send_is_default"
        )]
        auto_send: bool,
    },

    // Agent view only – shows fast-forward (auto-approve) toggle in the footer
    FastForwardToggle,

    // Agent view only – "Hand off to cloud" chip.
    HandoffToCloud,
}

const TERMINAL_GIT_COMMIT_COMMAND: &str = "git add -A && printf 'Commit message [Update changes]: ' && IFS= read -r clinch_commit_message && git commit -m \"${clinch_commit_message:-Update changes}\"";
const TERMINAL_GIT_COMMIT_AND_PUSH_COMMAND: &str = "git add -A && printf 'Commit message [Update changes]: ' && IFS= read -r clinch_commit_message && git commit -m \"${clinch_commit_message:-Update changes}\" && git push";

impl AgentToolbarItemKind {
    pub fn custom_insert(label: impl Into<String>, text: impl Into<String>) -> Self {
        Self::CustomInsert {
            label: label.into(),
            text: text.into(),
            auto_send: true,
        }
    }

    /// Stable identity used when matching a persisted layout against shipped defaults. Quick
    /// inserts use their label so prompt and auto-send edits do not turn one button into two.
    pub fn has_same_toolbar_identity(&self, other: &Self) -> bool {
        match (
            self.quick_insert_identity_label(),
            other.quick_insert_identity_label(),
        ) {
            (Some(left_label), Some(right_label)) => left_label == right_label,
            _ => self == other,
        }
    }

    fn quick_insert_identity_label(&self) -> Option<&str> {
        match self {
            Self::Compact => Some("Compact"),
            Self::ContinuePrompt => Some("Continue"),
            Self::LooksGoodPrompt => Some("LGTM"),
            Self::CustomInsert { label, .. } => Some(label),
            _ => None,
        }
    }

    /// Returns the current auto-send choice for controls that insert prompt text.
    pub fn auto_send_behavior(&self) -> Option<bool> {
        match self {
            Self::Compact | Self::ContinuePrompt | Self::LooksGoodPrompt => Some(true),
            Self::CustomInsert { auto_send, .. } => Some(*auto_send),
            _ => None,
        }
    }

    /// Returns an equivalent prompt button with the requested auto-send behavior.
    ///
    /// Built-in prompt actions are represented as custom inserts after the user changes their
    /// behavior. Their stable identity still matches the shipped button, so the override survives
    /// settings reloads without producing a duplicate.
    pub fn with_auto_send_behavior(&self, auto_send: bool) -> Option<Self> {
        match self {
            Self::Compact => Some(Self::CustomInsert {
                label: "Compact".to_owned(),
                text: "/compact".to_owned(),
                auto_send,
            }),
            Self::ContinuePrompt => Some(Self::CustomInsert {
                label: "Continue".to_owned(),
                text: "Continue".to_owned(),
                auto_send,
            }),
            Self::LooksGoodPrompt => Some(Self::CustomInsert {
                label: "LGTM".to_owned(),
                text: "Looks good to me, continue".to_owned(),
                auto_send,
            }),
            Self::CustomInsert { label, text, .. } => {
                if auto_send {
                    match (label.as_str(), text.as_str()) {
                        ("Compact", "/compact") => return Some(Self::Compact),
                        ("Continue", "Continue") => return Some(Self::ContinuePrompt),
                        ("LGTM", "Looks good to me, continue") => {
                            return Some(Self::LooksGoodPrompt);
                        }
                        _ => {}
                    }
                }
                Some(Self::CustomInsert {
                    label: label.clone(),
                    text: text.clone(),
                    auto_send,
                })
            }
            _ => None,
        }
    }

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
            | Self::BookmarkConversation
            | Self::ContinuePrompt
            | Self::CopyAndClearDraft
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
            | Self::BookmarkConversation
            | Self::ContinuePrompt
            | Self::CopyAndClearDraft
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
            Self::BookmarkConversation => Cow::Borrowed("Bookmark Session"),
            Self::ContinuePrompt => Cow::Borrowed("Continue"),
            Self::LooksGoodPrompt => Cow::Borrowed("LGTM"),
            Self::CopyAndClearDraft => Cow::Borrowed("Copy & Clear"),
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
            Self::BookmarkConversation => Some(Icon::Bookmark),
            Self::ContinuePrompt => Some(Icon::Play),
            Self::LooksGoodPrompt => Some(Icon::ThumbsUp),
            Self::CopyAndClearDraft => Some(Icon::Copy),
            Self::TransferAgent => Some(Icon::SwitchHorizontal01),
            Self::FastForwardToggle => Some(Icon::FastForward),
            // The bundled `upload-cloud-01.svg` (cloud-with-upward-arrow) is the
            // closest fit among the existing icons for V0; design may swap it later.
            Self::HandoffToCloud => Some(Icon::UploadCloud),
            Self::CustomInsert {
                auto_send: true, ..
            } => Some(Icon::Play),
            Self::CustomInsert {
                auto_send: false, ..
            } => Some(Icon::TextInput),
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
            | Self::BookmarkConversation
            | Self::ContinuePrompt
            | Self::CopyAndClearDraft
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

    /// Session-management controls rendered in the leading CLI-footer action cluster.
    pub fn is_cli_session_action(&self) -> bool {
        matches!(
            self,
            Self::BookmarkConversation | Self::ForkSession | Self::TransferAgent
        )
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

    /// Default items for the CLI agent footer.
    ///
    /// This footer intentionally contains only Clinch session actions. Generic
    /// input controls and context/status chips belong in their dedicated UI,
    /// not in the quick-insert editor.
    pub fn cli_default_left() -> Vec<Self> {
        vec![
            Self::BookmarkConversation,
            Self::ForkSession,
            Self::TransferAgent,
            Self::Compact,
            Self::ContinuePrompt,
            Self::LooksGoodPrompt,
            Self::CopyAndClearDraft,
        ]
    }

    /// Shipped quick-insert recipes offered by the CLI agent footer editor.
    ///
    /// These are intentionally not part of the default layout: users can opt into the prompts
    /// they want without every preset consuming footer space on a fresh install.
    pub fn cli_quick_insert_presets() -> Vec<Self> {
        // Append new presets here rather than changing the historical snapshot catalog below.
        // Migration needs the old definitions to remain byte-for-byte stable.
        Self::retired_cli_default_quick_insert_snapshot_items()
    }

    /// Immutable prompt definitions that shipped as mandatory CLI-footer defaults before the
    /// live-default overlay format. Keep this catalog stable even when the current preset library
    /// grows or a recipe is revised; settings migration uses exact equality to avoid deleting
    /// user-authored variants.
    pub(crate) fn retired_cli_default_quick_insert_snapshot_items() -> Vec<Self> {
        RETIRED_CLI_DEFAULT_QUICK_INSERT_SNAPSHOTS
            .iter()
            .map(|(label, text)| Self::custom_insert(*label, *text))
            .collect()
    }

    /// Items that were once injected into every CLI-agent footer but are now optional.
    ///
    /// Old settings stored the entire effective footer without recording whether an item came
    /// from Clinch or the user. Exact historical definitions can therefore look user-authored
    /// after an upgrade. This list lets the settings compatibility layer remove only those exact
    /// snapshots; edited prompt text or auto-send behavior remains a genuine user override.
    pub fn is_retired_cli_default_snapshot_item(&self) -> bool {
        matches!(
            self,
            Self::FileAttach
                | Self::VoiceInput
                | Self::ContextChip(ContextChipKind::GitDiffStats)
                | Self::ShareSession
                | Self::FileExplorer
                | Self::RichInput
                | Self::ContextChip(ContextChipKind::WorkingDirectory)
                | Self::ContextChip(ContextChipKind::ShellGitBranch)
                | Self::Settings
        ) || matches!(
            self,
            Self::CustomInsert {
                label,
                text,
                auto_send: true,
            } if RETIRED_CLI_DEFAULT_QUICK_INSERT_SNAPSHOTS
                .contains(&(label.as_str(), text.as_str()))
        )
    }

    /// Default right-side items for the CLI agent footer.
    pub fn cli_default_right() -> Vec<Self> {
        Vec::new()
    }

    /// Default left-side items for the plain terminal footer.
    pub fn terminal_default_left() -> Vec<Self> {
        [
            ("Claude", "claude --dangerously-skip-permissions"),
            ("Codex", "codex --dangerously-bypass-approvals-and-sandbox"),
            (
                "Claude resume",
                "claude --dangerously-skip-permissions --resume",
            ),
            ("Codex resume", "codex resume"),
            ("Open", "open ."),
            ("Commit & Push", TERMINAL_GIT_COMMIT_AND_PUSH_COMMAND),
            ("Commit", TERMINAL_GIT_COMMIT_COMMAND),
            ("Status", "git status --short --branch"),
        ]
        .into_iter()
        .map(|(label, text)| Self::custom_insert(label, text))
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
        let mut items = vec![
            Self::BookmarkConversation,
            Self::ForkSession,
            Self::TransferAgent,
            Self::Compact,
            Self::ContinuePrompt,
            Self::LooksGoodPrompt,
            Self::CopyAndClearDraft,
        ];
        items.extend(Self::cli_quick_insert_presets());
        items
    }

    /// Whether a quick insert is supplied by Clinch rather than created by the user.
    pub fn is_shipped_quick_insert_for_mode(&self, mode: AgentToolbarEditorMode) -> bool {
        if !matches!(self, Self::CustomInsert { .. }) {
            return false;
        }
        let mut shipped = match mode {
            AgentToolbarEditorMode::AgentView => Vec::new(),
            AgentToolbarEditorMode::CLIAgent
            | AgentToolbarEditorMode::ClaudeCode
            | AgentToolbarEditorMode::Codex => Self::cli_default_left(),
            AgentToolbarEditorMode::Terminal => Self::terminal_default_left(),
        };
        if matches!(
            mode,
            AgentToolbarEditorMode::CLIAgent
                | AgentToolbarEditorMode::ClaudeCode
                | AgentToolbarEditorMode::Codex
        ) {
            shipped.extend(Self::cli_quick_insert_presets());
        }
        shipped
            .iter()
            .any(|item| item.has_same_toolbar_identity(self))
    }

    /// Returns the appropriate defaults and available items for a given editor mode.
    pub fn defaults_for_mode(mode: AgentToolbarEditorMode) -> (Vec<Self>, Vec<Self>, Vec<Self>) {
        match mode {
            AgentToolbarEditorMode::AgentView => (
                Self::default_left(),
                Self::default_right(),
                Self::all_available(),
            ),
            AgentToolbarEditorMode::CLIAgent
            | AgentToolbarEditorMode::ClaudeCode
            | AgentToolbarEditorMode::Codex => (
                Self::cli_default_left(),
                Self::cli_default_right(),
                Self::all_available_for_cli_input(),
            ),
            AgentToolbarEditorMode::Terminal => (
                Self::terminal_default_left(),
                Self::terminal_default_right(),
                Self::all_available_for_terminal_input(),
            ),
        }
    }
}

fn default_custom_insert_auto_send() -> bool {
    true
}

fn custom_insert_auto_send_is_default(value: &bool) -> bool {
    *value
}

impl From<ContextChipKind> for AgentToolbarItemKind {
    fn from(kind: ContextChipKind) -> Self {
        Self::ContextChip(kind)
    }
}

#[cfg(test)]
#[path = "toolbar_item_tests.rs"]
mod tests;
