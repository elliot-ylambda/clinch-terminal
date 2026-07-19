use serde::{Deserialize, Serialize};
use settings::Setting as _;
use warpui::{AppContext, SingletonEntity};

use crate::auth::AuthStateProvider;
use crate::features::FeatureFlag;
use crate::settings::{AISettings, CodeSettings};
use crate::tab::uses_vertical_tabs;
use crate::ui_components::icons::Icon;
use crate::workspace::tab_settings::TabSettings;

/// A configurable item in the vertical tabs header toolbar.
///
/// Each variant represents a control that can be placed on either the left or
/// right side of the toolbar. Panel controls open on the configured side.
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
#[schemars(rename_all = "snake_case")]
pub enum HeaderToolbarItemKind {
    TabsPanel,
    FileExplorer,
    Skills,
    ConversationFinder,
    ToolsPanel,
    AgentManagement,
    CodeReview,
    IMessageStatus,
    NotificationsMailbox,
}

impl HeaderToolbarItemKind {
    pub fn display_label(&self) -> &'static str {
        match self {
            Self::TabsPanel => "Tabs Panel",
            Self::FileExplorer => "File Explorer",
            Self::Skills => "Skills",
            Self::ConversationFinder => "Conversation Finder",
            Self::ToolsPanel => "Tools Panel",
            Self::AgentManagement => "Agent Management",
            Self::CodeReview => "Code Review",
            Self::IMessageStatus => "iMessage Status",
            Self::NotificationsMailbox => "Notifications",
        }
    }

    pub fn icon(&self) -> Icon {
        match self {
            Self::TabsPanel => Icon::Menu,
            Self::FileExplorer => Icon::Folder,
            Self::Skills => Icon::Stars,
            Self::ConversationFinder => Icon::Conversation,
            Self::ToolsPanel => Icon::Tool2,
            Self::AgentManagement => Icon::Grid,
            Self::CodeReview => Icon::Diff,
            Self::IMessageStatus => Icon::Phone01,
            Self::NotificationsMailbox => Icon::Inbox,
        }
    }

    /// Whether this item is supported on the current platform/configuration
    /// (feature flags, compile-time features, AI enabled, auth state).
    /// Does not check user show/hide preferences — use `is_available` for that.
    pub fn is_supported(&self, app: &AppContext) -> bool {
        match self {
            Self::TabsPanel => uses_vertical_tabs(),
            // The file tree is reached through the same left (tools) panel as
            // `ToolsPanel`, so it is supported wherever that panel is. Real
            // availability is gated by `show_project_explorer` in `is_available`.
            Self::FileExplorer => true,
            Self::Skills => FeatureFlag::SkillsPanel.is_enabled(),
            Self::ConversationFinder => true,
            Self::ToolsPanel => true,
            Self::AgentManagement => {
                let is_web_anonymous_user = AuthStateProvider::as_ref(app)
                    .get()
                    .is_user_web_anonymous_user()
                    .unwrap_or_default();
                AISettings::as_ref(app).is_any_ai_enabled(app)
                    && FeatureFlag::AgentManagementView.is_enabled()
                    && !is_web_anonymous_user
            }
            Self::CodeReview => cfg!(feature = "local_fs"),
            // Keep deserializing toolbar settings written by development builds
            // that exposed the removed iMessage integration.
            Self::IMessageStatus => false,
            // Clinch does not surface agent notifications. Keep this variant so
            // older serialized toolbar settings still deserialize successfully.
            Self::NotificationsMailbox => false,
        }
    }

    /// Whether this item should be shown in the toolbar.
    /// Checks both `is_supported` and user show/hide preferences.
    pub fn is_available(&self, app: &AppContext) -> bool {
        if !self.is_supported(app) {
            return false;
        }
        match self {
            Self::FileExplorer => *CodeSettings::as_ref(app).show_project_explorer,
            Self::CodeReview => *TabSettings::as_ref(app).show_code_review_button.value(),
            Self::IMessageStatus => true,
            Self::NotificationsMailbox => *AISettings::as_ref(app).show_agent_notifications,
            Self::TabsPanel
            | Self::Skills
            | Self::ConversationFinder
            | Self::ToolsPanel
            | Self::AgentManagement => true,
        }
    }

    /// Whether this item opens a side panel (as opposed to replacing the content
    /// area or opening a popover).
    ///
    /// `FileExplorer`, `Skills`, and `ToolsPanel` share the same left-panel
    /// view. When several are configured, one owns the renderer; see
    /// `HeaderToolbarChipSelection::is_shared_left_panel_owner`.
    pub fn is_panel(&self) -> bool {
        matches!(
            self,
            Self::TabsPanel
                | Self::FileExplorer
                | Self::Skills
                | Self::ToolsPanel
                | Self::CodeReview
        )
    }

    pub fn default_left() -> Vec<Self> {
        vec![
            Self::TabsPanel,
            Self::FileExplorer,
            Self::Skills,
            Self::ConversationFinder,
            Self::AgentManagement,
        ]
    }

    pub fn default_right() -> Vec<Self> {
        vec![Self::CodeReview]
    }

    /// Toolbar items offered by Clinch's configurator (availability filtering
    /// is done at the call site).
    pub fn all_items() -> Vec<Self> {
        vec![
            Self::TabsPanel,
            Self::FileExplorer,
            Self::Skills,
            Self::ConversationFinder,
            Self::ToolsPanel,
            Self::AgentManagement,
            Self::CodeReview,
        ]
    }
}

#[cfg(test)]
#[path = "header_toolbar_item_tests.rs"]
mod tests;
