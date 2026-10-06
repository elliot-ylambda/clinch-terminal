pub mod new_session_shell;
pub mod startup_shell;
pub mod working_directory_config;

use instant::Duration;
use lazy_static::lazy_static;
pub use new_session_shell::*;
use serde::{Deserialize, Serialize};
use settings::Setting as _;
pub use startup_shell::*;
use warp_core::settings::macros::define_settings_group;
use warp_core::settings::{RespectUserSyncSetting, SupportedPlatforms, SyncToCloud};
pub use working_directory_config::*;

use crate::ai::blocklist::agent_view::toolbar_item::AgentToolbarItemKind;
use crate::context_chips::prompt::PromptSelection;
use crate::context_chips::ContextChipKind;

lazy_static! {
    pub static ref DEFAULT_THRESHOLD_FOR_LONG_RUNNING_NOTIFICATION: Duration =
        Duration::from_secs(30);
}

#[derive(
    Copy,
    Clone,
    Debug,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Whether the user has enabled or disabled notifications.",
    rename_all = "snake_case"
)]
pub enum NotificationsMode {
    // User has not been shown notifications banner before or has seen it before but decided not to dismiss it.
    #[schemars(description = "Notifications have not been configured yet.")]
    Unset,

    // User has asked not to be shown notifications banner again.
    #[schemars(description = "The notifications banner has been dismissed.")]
    Dismissed,

    // User has enabled system notifications and wants to receive notifications.
    #[schemars(description = "Notifications are enabled.")]
    Enabled,

    // User had previously enabled notifications, but has now disabled them.
    #[schemars(description = "Notifications are disabled.")]
    Disabled,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, settings_value::SettingsValue)]
/**
 * Added [serde(default)] to ensure that new notification settings are backwards compatible with old clients.
 * Otherwise, clients will fail to deserialize existing settings after updating.
 *
 * @see https://github.com/warpdotdev/warp-internal/pull/14596/files#diff-90221c7ecae01c6faf8f170158dea3e49d34d40225a306da42ccc76489d1f84cR43-R44
 *
 * Alternative considered: Using Option<bool> fields would have required more
 * complex defaulting logic to set the default value to true.
 *
 */
#[serde(default)]
#[derive(schemars::JsonSchema)]
#[schemars(description = "Notification preferences for terminal events.")]
pub struct NotificationsSettings {
    #[schemars(
        description = "Whether notifications are enabled, disabled, or not yet configured."
    )]
    pub mode: NotificationsMode,

    #[schemars(description = "Whether to notify when a long-running command completes.")]
    pub is_long_running_enabled: bool,
    #[schemars(
        with = "u64",
        description = "Threshold in seconds for long-running command notifications."
    )]
    pub long_running_threshold: Duration,

    /// Legacy. To be combined with `is_needs_attention_enabled` when desktop notifs are unflagged.
    #[schemars(description = "Whether to notify when a password prompt is detected.")]
    pub is_password_prompt_enabled: bool,

    #[schemars(description = "Whether to notify when an agent task completes.")]
    pub is_agent_task_completed_enabled: bool,
    #[schemars(description = "Whether to notify when a session needs attention.")]
    pub is_needs_attention_enabled: bool,

    #[schemars(description = "Whether to play a sound with notifications.")]
    pub play_notification_sound: bool,

    #[schemars(description = "Whether to show CLI-agent (Claude/Codex) status badges on tabs.")]
    pub show_agent_status_on_tabs: bool,
}

impl Default for NotificationsSettings {
    fn default() -> Self {
        Self {
            mode: NotificationsMode::Unset,
            is_long_running_enabled: true,
            long_running_threshold: *DEFAULT_THRESHOLD_FOR_LONG_RUNNING_NOTIFICATION,
            is_password_prompt_enabled: true,
            is_agent_task_completed_enabled: true,
            is_needs_attention_enabled: true,
            play_notification_sound: true,
            show_agent_status_on_tabs: true,
        }
    }
}

#[derive(
    Copy,
    Clone,
    Debug,
    Default,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
pub enum GithubPrPromptChipDefaultValidation {
    #[default]
    Unvalidated,
    Validated,
    Suppressed,
}

impl GithubPrPromptChipDefaultValidation {
    pub fn is_suppressed(self) -> bool {
        matches!(self, Self::Suppressed)
    }
}

/// Shared behavior for toolbar chip selection types.
/// Each variant stores either a `Default` (resolved via type-specific defaults) or `Custom` left/right item lists.
pub trait ToolbarChipSelection {
    fn default_left_items() -> Vec<AgentToolbarItemKind>;
    fn default_right_items() -> Vec<AgentToolbarItemKind>;
    fn left_items(&self) -> Vec<AgentToolbarItemKind>;
    fn right_items(&self) -> Vec<AgentToolbarItemKind>;

    fn left_chips(&self) -> Vec<ContextChipKind> {
        self.left_items()
            .into_iter()
            .filter_map(|item| match item {
                AgentToolbarItemKind::ContextChip(kind) => Some(kind),
                _ => None,
            })
            .collect()
    }

    fn right_chips(&self) -> Vec<ContextChipKind> {
        self.right_items()
            .into_iter()
            .filter_map(|item| match item {
                AgentToolbarItemKind::ContextChip(kind) => Some(kind),
                _ => None,
            })
            .collect()
    }

    fn all_chips(&self) -> Vec<ContextChipKind> {
        let mut chips = self.left_chips();
        chips.extend(self.right_chips());
        chips
    }

    fn all_items(&self) -> Vec<AgentToolbarItemKind> {
        let mut items = self.left_items();
        items.extend(self.right_items());
        items
    }
}

/// Toolbar defaults are live product-owned entries, while a user's custom entries are an
/// overlay. Custom inserts use their label as their stable identity so a shipped prompt can be
/// revised without resurrecting a button the user explicitly hid.
fn toolbar_items_have_same_identity(
    left: &AgentToolbarItemKind,
    right: &AgentToolbarItemKind,
) -> bool {
    left.has_same_toolbar_identity(right)
}

fn contains_toolbar_item_identity(
    items: &[AgentToolbarItemKind],
    candidate: &AgentToolbarItemKind,
) -> bool {
    items
        .iter()
        .any(|item| toolbar_items_have_same_identity(item, candidate))
}

/// Resolves a persisted toolbar overlay against the defaults in this build.
///
/// Legacy `custom` settings stored a full snapshot and did not carry `inherit_defaults`. Treat
/// those snapshots as an overlay on first load so defaults added by newer Clinch releases appear
/// immediately. New settings additionally persist `hidden_defaults`, which distinguishes an
/// intentional removal from a default that simply did not exist when the snapshot was written.
fn merge_toolbar_defaults_and_custom_items(
    default_left: Vec<AgentToolbarItemKind>,
    default_right: Vec<AgentToolbarItemKind>,
    saved_left: &[AgentToolbarItemKind],
    saved_right: &[AgentToolbarItemKind],
    hidden_defaults: &[AgentToolbarItemKind],
    inherit_defaults: bool,
) -> (Vec<AgentToolbarItemKind>, Vec<AgentToolbarItemKind>) {
    let all_defaults = default_left
        .iter()
        .chain(&default_right)
        .cloned()
        .collect::<Vec<_>>();
    let effective_hidden = if inherit_defaults {
        hidden_defaults
    } else {
        Default::default()
    };

    let mut left = default_left
        .into_iter()
        .filter(|item| !contains_toolbar_item_identity(effective_hidden, item))
        .collect::<Vec<_>>();
    let mut right = default_right
        .into_iter()
        .filter(|item| !contains_toolbar_item_identity(effective_hidden, item))
        .collect::<Vec<_>>();

    // Saved default entries are snapshots, not custom buttons. Drop them here so current shipped
    // definitions and ordering win, then append only the user's additions after the defaults.
    left.extend(
        saved_left
            .iter()
            .filter(|item| !contains_toolbar_item_identity(&all_defaults, item))
            .cloned(),
    );
    right.extend(
        saved_right
            .iter()
            .filter(|item| !contains_toolbar_item_identity(&all_defaults, item))
            .cloned(),
    );

    (left, right)
}

/// Resolves a user-saved explicit layout while keeping shipped definitions live.
///
/// Saved default items act as ordered references. Their current build definition is used unless
/// the user explicitly changed that item and it appears in `overridden_defaults`. Defaults added
/// after the layout was saved are appended to their shipped side, while `hidden_defaults` keeps
/// intentional removals hidden.
fn merge_ordered_toolbar_defaults_and_custom_items(
    default_left: Vec<AgentToolbarItemKind>,
    default_right: Vec<AgentToolbarItemKind>,
    saved_left: &[AgentToolbarItemKind],
    saved_right: &[AgentToolbarItemKind],
    hidden_defaults: &[AgentToolbarItemKind],
    overridden_defaults: &[AgentToolbarItemKind],
    inherit_defaults: bool,
) -> (Vec<AgentToolbarItemKind>, Vec<AgentToolbarItemKind>) {
    let all_defaults = default_left
        .iter()
        .chain(&default_right)
        .cloned()
        .collect::<Vec<_>>();
    let effective_hidden = if inherit_defaults {
        hidden_defaults
    } else {
        Default::default()
    };
    let mut represented_defaults = Vec::new();

    let mut resolve_side = |saved: &[AgentToolbarItemKind]| {
        let mut resolved = Vec::with_capacity(saved.len());
        for item in saved {
            let Some(default_item) = all_defaults
                .iter()
                .find(|default| toolbar_items_have_same_identity(default, item))
            else {
                resolved.push(item.clone());
                continue;
            };
            if contains_toolbar_item_identity(effective_hidden, default_item)
                || contains_toolbar_item_identity(&represented_defaults, default_item)
            {
                continue;
            }
            represented_defaults.push(default_item.clone());
            let item = overridden_defaults
                .iter()
                .find(|overridden| toolbar_items_have_same_identity(default_item, overridden))
                .unwrap_or(default_item);
            resolved.push(item.clone());
        }
        resolved
    };

    let mut left = resolve_side(saved_left);
    let mut right = resolve_side(saved_right);

    for default in default_left {
        if !contains_toolbar_item_identity(effective_hidden, &default)
            && !contains_toolbar_item_identity(&represented_defaults, &default)
        {
            represented_defaults.push(default.clone());
            left.push(default);
        }
    }
    for default in default_right {
        if !contains_toolbar_item_identity(effective_hidden, &default)
            && !contains_toolbar_item_identity(&represented_defaults, &default)
        {
            represented_defaults.push(default.clone());
            right.push(default);
        }
    }

    (left, right)
}

fn hidden_toolbar_defaults(
    default_left: &[AgentToolbarItemKind],
    default_right: &[AgentToolbarItemKind],
    selected_left: &[AgentToolbarItemKind],
    selected_right: &[AgentToolbarItemKind],
) -> Vec<AgentToolbarItemKind> {
    let selected = selected_left
        .iter()
        .chain(selected_right)
        .cloned()
        .collect::<Vec<_>>();
    default_left
        .iter()
        .chain(default_right)
        .filter(|item| !contains_toolbar_item_identity(&selected, item))
        .cloned()
        .collect()
}

fn overridden_toolbar_defaults(
    default_left: &[AgentToolbarItemKind],
    default_right: &[AgentToolbarItemKind],
    selected_left: &[AgentToolbarItemKind],
    selected_right: &[AgentToolbarItemKind],
) -> Vec<AgentToolbarItemKind> {
    let defaults = default_left.iter().chain(default_right);
    selected_left
        .iter()
        .chain(selected_right)
        .filter(|selected| {
            defaults.clone().any(|default| {
                toolbar_items_have_same_identity(default, selected) && default != *selected
            })
        })
        .cloned()
        .collect()
}

fn normalize_hidden_custom_inserts(
    selected_left: &[AgentToolbarItemKind],
    selected_right: &[AgentToolbarItemKind],
    hidden_custom_inserts: Vec<AgentToolbarItemKind>,
) -> Vec<AgentToolbarItemKind> {
    let selected = selected_left
        .iter()
        .chain(selected_right)
        .cloned()
        .collect::<Vec<_>>();
    let mut normalized = Vec::new();
    for item in hidden_custom_inserts {
        if !matches!(item, AgentToolbarItemKind::CustomInsert { .. })
            || contains_toolbar_item_identity(&selected, &item)
            || contains_toolbar_item_identity(&normalized, &item)
        {
            continue;
        }
        normalized.push(item);
    }
    normalized
}

#[derive(
    Clone,
    Debug,
    Default,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Agent toolbar layout configuration.",
    rename_all = "snake_case"
)]
pub enum AgentToolbarChipSelection {
    #[default]
    #[schemars(description = "Use the default toolbar layout.")]
    Default,
    #[schemars(description = "Use a custom arrangement of toolbar items.")]
    Custom {
        left: Vec<AgentToolbarItemKind>,
        right: Vec<AgentToolbarItemKind>,
    },
}

impl ToolbarChipSelection for AgentToolbarChipSelection {
    fn default_left_items() -> Vec<AgentToolbarItemKind> {
        AgentToolbarItemKind::default_left()
    }

    fn default_right_items() -> Vec<AgentToolbarItemKind> {
        AgentToolbarItemKind::default_right()
    }

    fn left_items(&self) -> Vec<AgentToolbarItemKind> {
        match self {
            Self::Default => Self::default_left_items(),
            Self::Custom { left, .. } => left.clone(),
        }
    }

    fn right_items(&self) -> Vec<AgentToolbarItemKind> {
        match self {
            Self::Default => Self::default_right_items(),
            Self::Custom { right, .. } => right.clone(),
        }
    }
}

#[derive(
    Clone,
    Debug,
    Default,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "CLI agent toolbar layout configuration.",
    rename_all = "snake_case"
)]
pub enum CLIAgentToolbarChipSelection {
    #[default]
    #[schemars(description = "Use the default toolbar layout.")]
    Default,
    #[schemars(description = "Use a custom arrangement of toolbar items.")]
    Custom {
        left: Vec<AgentToolbarItemKind>,
        right: Vec<AgentToolbarItemKind>,
        /// Shipped defaults the user explicitly removed. Kept separate from custom entries so
        /// defaults introduced by future releases can still appear automatically.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        hidden_defaults: Vec<AgentToolbarItemKind>,
        /// Absent in legacy full-snapshot settings. New settings opt into live default merging.
        #[serde(default)]
        inherit_defaults: bool,
        /// Shipped items whose editable fields the user explicitly changed.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        overridden_defaults: Vec<AgentToolbarItemKind>,
        /// User-created quick inserts saved in the editor but not shown in the footer.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        hidden_custom_inserts: Vec<AgentToolbarItemKind>,
        /// Whether `left` and `right` are an explicit ordering of the effective toolbar.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        preserve_order: bool,
    },
}

impl CLIAgentToolbarChipSelection {
    #[cfg(test)]
    pub(crate) fn custom_from_effective_items(
        left: Vec<AgentToolbarItemKind>,
        right: Vec<AgentToolbarItemKind>,
    ) -> Self {
        Self::custom_from_effective_items_and_hidden_custom_inserts(left, right, Vec::new())
    }

    pub(crate) fn custom_from_effective_items_and_hidden_custom_inserts(
        left: Vec<AgentToolbarItemKind>,
        right: Vec<AgentToolbarItemKind>,
        hidden_custom_inserts: Vec<AgentToolbarItemKind>,
    ) -> Self {
        let default_left = AgentToolbarItemKind::cli_default_left();
        let default_right = AgentToolbarItemKind::cli_default_right();
        let hidden_defaults = hidden_toolbar_defaults(&default_left, &default_right, &left, &right);
        let overridden_defaults =
            overridden_toolbar_defaults(&default_left, &default_right, &left, &right);
        let hidden_custom_inserts =
            normalize_hidden_custom_inserts(&left, &right, hidden_custom_inserts);
        Self::Custom {
            left,
            right,
            hidden_defaults,
            inherit_defaults: true,
            overridden_defaults,
            hidden_custom_inserts,
            preserve_order: true,
        }
    }

    pub(crate) fn hidden_custom_inserts(&self) -> Vec<AgentToolbarItemKind> {
        match self {
            Self::Default => Vec::new(),
            Self::Custom {
                hidden_custom_inserts,
                ..
            } => hidden_custom_inserts.clone(),
        }
    }

    fn retired_default_snapshot_item_count(&self) -> usize {
        match self {
            Self::Default => 0,
            Self::Custom { left, right, .. } => left
                .iter()
                .chain(right)
                .filter(|item| item.is_retired_cli_default_snapshot_item())
                .count(),
        }
    }

    fn should_normalize_retired_default_snapshot(&self) -> bool {
        let Self::Custom {
            left,
            right,
            inherit_defaults,
            ..
        } = self
        else {
            return false;
        };
        if !*inherit_defaults {
            return true;
        }

        // During the provider-split release, opening and saving an inherited legacy footer could
        // materialize it as a modern live overlay. Recognize that otherwise-unversioned snapshot
        // by the complete historical quick-insert catalog plus at least one retired host control.
        // A user who opted into one (or even several) presets does not match this signature.
        let presets = AgentToolbarItemKind::retired_cli_default_quick_insert_snapshot_items();
        let contains_all_presets = presets
            .iter()
            .all(|preset| left.iter().chain(right).any(|item| item == preset));
        let contains_retired_host_control = left
            .iter()
            .chain(right)
            .any(|item| item.is_retired_cli_default_snapshot_item() && !presets.contains(item));
        contains_all_presets && contains_retired_host_control
    }

    fn effective_items(&self) -> (Vec<AgentToolbarItemKind>, Vec<AgentToolbarItemKind>) {
        let normalize_retired_snapshot = self.should_normalize_retired_default_snapshot();
        match self {
            Self::Default => (
                AgentToolbarItemKind::cli_default_left(),
                AgentToolbarItemKind::cli_default_right(),
            ),
            Self::Custom {
                left,
                right,
                hidden_defaults,
                inherit_defaults,
                overridden_defaults,
                hidden_custom_inserts: _,
                preserve_order,
            } => {
                let default_left = AgentToolbarItemKind::cli_default_left();
                let default_right = AgentToolbarItemKind::cli_default_right();
                // Before live-default overlays existed, settings serialized the complete footer;
                // the provider-split release could then re-save that inherited snapshot as an
                // overlay. Items Clinch shipped at the time otherwise survive forever as apparent
                // user customizations when product defaults change. Strip only exact historical
                // definitions from recognized snapshots; edited/custom variants remain.
                let normalize_legacy_snapshot = |items: &[AgentToolbarItemKind]| {
                    items
                        .iter()
                        .filter(|item| {
                            !normalize_retired_snapshot
                                || !item.is_retired_cli_default_snapshot_item()
                        })
                        .cloned()
                        .collect::<Vec<_>>()
                };
                let left = normalize_legacy_snapshot(left);
                let right = normalize_legacy_snapshot(right);
                if *preserve_order {
                    merge_ordered_toolbar_defaults_and_custom_items(
                        default_left,
                        default_right,
                        &left,
                        &right,
                        hidden_defaults,
                        overridden_defaults,
                        *inherit_defaults,
                    )
                } else {
                    merge_toolbar_defaults_and_custom_items(
                        default_left,
                        default_right,
                        &left,
                        &right,
                        hidden_defaults,
                        *inherit_defaults,
                    )
                }
            }
        }
    }
}

impl ToolbarChipSelection for CLIAgentToolbarChipSelection {
    fn default_left_items() -> Vec<AgentToolbarItemKind> {
        AgentToolbarItemKind::cli_default_left()
    }

    fn default_right_items() -> Vec<AgentToolbarItemKind> {
        AgentToolbarItemKind::cli_default_right()
    }

    fn left_items(&self) -> Vec<AgentToolbarItemKind> {
        self.effective_items().0
    }

    fn right_items(&self) -> Vec<AgentToolbarItemKind> {
        self.effective_items().1
    }
}

#[derive(
    Clone,
    Debug,
    Default,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Terminal toolbar layout configuration.",
    rename_all = "snake_case"
)]
pub enum TerminalToolbarChipSelection {
    #[default]
    #[schemars(description = "Use the default toolbar layout.")]
    Default,
    #[schemars(description = "Use a custom arrangement of toolbar items.")]
    Custom {
        left: Vec<AgentToolbarItemKind>,
        right: Vec<AgentToolbarItemKind>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        hidden_defaults: Vec<AgentToolbarItemKind>,
        #[serde(default)]
        inherit_defaults: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        overridden_defaults: Vec<AgentToolbarItemKind>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        hidden_custom_inserts: Vec<AgentToolbarItemKind>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        preserve_order: bool,
    },
}

impl TerminalToolbarChipSelection {
    #[cfg(test)]
    pub(crate) fn custom_from_effective_items(
        left: Vec<AgentToolbarItemKind>,
        right: Vec<AgentToolbarItemKind>,
    ) -> Self {
        Self::custom_from_effective_items_and_hidden_custom_inserts(left, right, Vec::new())
    }

    pub(crate) fn custom_from_effective_items_and_hidden_custom_inserts(
        left: Vec<AgentToolbarItemKind>,
        right: Vec<AgentToolbarItemKind>,
        hidden_custom_inserts: Vec<AgentToolbarItemKind>,
    ) -> Self {
        let default_left = AgentToolbarItemKind::terminal_default_left();
        let default_right = AgentToolbarItemKind::terminal_default_right();
        let hidden_defaults = hidden_toolbar_defaults(&default_left, &default_right, &left, &right);
        let overridden_defaults =
            overridden_toolbar_defaults(&default_left, &default_right, &left, &right);
        let hidden_custom_inserts =
            normalize_hidden_custom_inserts(&left, &right, hidden_custom_inserts);
        Self::Custom {
            left,
            right,
            hidden_defaults,
            inherit_defaults: true,
            overridden_defaults,
            hidden_custom_inserts,
            preserve_order: true,
        }
    }

    pub(crate) fn hidden_custom_inserts(&self) -> Vec<AgentToolbarItemKind> {
        match self {
            Self::Default => Vec::new(),
            Self::Custom {
                hidden_custom_inserts,
                ..
            } => hidden_custom_inserts.clone(),
        }
    }

    fn effective_items(&self) -> (Vec<AgentToolbarItemKind>, Vec<AgentToolbarItemKind>) {
        match self {
            Self::Default => (
                AgentToolbarItemKind::terminal_default_left(),
                AgentToolbarItemKind::terminal_default_right(),
            ),
            Self::Custom {
                left,
                right,
                hidden_defaults,
                inherit_defaults,
                overridden_defaults,
                hidden_custom_inserts: _,
                preserve_order,
            } => {
                let default_left = AgentToolbarItemKind::terminal_default_left();
                let default_right = AgentToolbarItemKind::terminal_default_right();
                if *preserve_order {
                    merge_ordered_toolbar_defaults_and_custom_items(
                        default_left,
                        default_right,
                        left,
                        right,
                        hidden_defaults,
                        overridden_defaults,
                        *inherit_defaults,
                    )
                } else {
                    merge_toolbar_defaults_and_custom_items(
                        default_left,
                        default_right,
                        left,
                        right,
                        hidden_defaults,
                        *inherit_defaults,
                    )
                }
            }
        }
    }
}

impl ToolbarChipSelection for TerminalToolbarChipSelection {
    fn default_left_items() -> Vec<AgentToolbarItemKind> {
        AgentToolbarItemKind::terminal_default_left()
    }

    fn default_right_items() -> Vec<AgentToolbarItemKind> {
        AgentToolbarItemKind::terminal_default_right()
    }

    fn left_items(&self) -> Vec<AgentToolbarItemKind> {
        self.effective_items().0
    }

    fn right_items(&self) -> Vec<AgentToolbarItemKind> {
        self.effective_items().1
    }
}

fn choose_pre_unified_coding_agent_selection<'a>(
    legacy: &'a CLIAgentToolbarChipSelection,
    claude: Option<&'a CLIAgentToolbarChipSelection>,
    codex: Option<&'a CLIAgentToolbarChipSelection>,
) -> &'a CLIAgentToolbarChipSelection {
    match (claude, codex) {
        (Some(claude), None) => claude,
        (None, Some(codex)) => codex,
        (Some(claude), Some(codex)) if claude == codex => claude,
        (Some(claude), Some(codex)) => {
            // If the short-lived split settings disagree, prefer the side with fewer exact
            // retired product defaults. That recovers the deliberately cleaned layout from the
            // common "one provider reset, one provider inherited the old snapshot" case. Equal
            // ambiguity falls back to the pre-split shared preference without deleting either
            // deprecated value.
            match claude
                .retired_default_snapshot_item_count()
                .cmp(&codex.retired_default_snapshot_item_count())
            {
                std::cmp::Ordering::Less => claude,
                std::cmp::Ordering::Greater => codex,
                std::cmp::Ordering::Equal => legacy,
            }
        }
        (None, None) => legacy,
    }
}

#[cfg(test)]
mod toolbar_chip_selection_tests {
    use settings_value::SettingsValue;

    use super::*;

    fn custom_insert(label: &str, text: &str) -> AgentToolbarItemKind {
        AgentToolbarItemKind::custom_insert(label, text)
    }

    #[test]
    fn legacy_cli_snapshot_drops_retired_defaults_and_keeps_custom_items_last() {
        let mut edited_retired_preset =
            AgentToolbarItemKind::custom_insert("Push2Main", "Push all these changes to main.");
        let AgentToolbarItemKind::CustomInsert { auto_send, .. } = &mut edited_retired_preset
        else {
            unreachable!();
        };
        *auto_send = false;
        // This reproduces the full shipped recipe set from before live-default overlays, plus
        // context/input controls that appeared in still-older snapshots. None of those product
        // entries should be promoted into permanent user-authored buttons during migration.
        let mut retired_presets =
            AgentToolbarItemKind::retired_cli_default_quick_insert_snapshot_items();
        let push_to_main = retired_presets
            .pop()
            .expect("the historical preset catalog includes Push2Main");
        let mut legacy_left = vec![
            AgentToolbarItemKind::ForkSession,
            AgentToolbarItemKind::Compact,
            AgentToolbarItemKind::ContinuePrompt,
            AgentToolbarItemKind::LooksGoodPrompt,
            AgentToolbarItemKind::TransferAgent,
        ];
        legacy_left.extend(retired_presets);
        legacy_left.extend([
            AgentToolbarItemKind::VoiceInput,
            push_to_main,
            AgentToolbarItemKind::FileAttach,
            AgentToolbarItemKind::ShareSession,
            AgentToolbarItemKind::FileExplorer,
            AgentToolbarItemKind::RichInput,
            AgentToolbarItemKind::Settings,
            custom_insert("Mine", "mine"),
            edited_retired_preset.clone(),
        ]);
        let legacy = CLIAgentToolbarChipSelection::Custom {
            left: legacy_left,
            right: vec![
                AgentToolbarItemKind::ContextChip(ContextChipKind::WorkingDirectory),
                AgentToolbarItemKind::ContextChip(ContextChipKind::ShellGitBranch),
                AgentToolbarItemKind::ContextChip(ContextChipKind::GitDiffStats),
            ],
            hidden_defaults: vec![],
            inherit_defaults: false,
            overridden_defaults: vec![],
            hidden_custom_inserts: vec![],
            preserve_order: false,
        };
        let mut serialized = legacy.to_file_value();
        let custom = serialized
            .get_mut("custom")
            .and_then(serde_json::Value::as_object_mut)
            .unwrap();
        custom.remove("hidden_defaults");
        custom.remove("inherit_defaults");

        let migrated = CLIAgentToolbarChipSelection::from_file_value(&serialized).unwrap();
        let mut expected_left = AgentToolbarItemKind::cli_default_left();
        expected_left.push(custom_insert("Mine", "mine"));
        expected_left.push(edited_retired_preset);
        assert_eq!(migrated.left_items(), expected_left);
        assert!(migrated.right_items().is_empty());
    }

    #[test]
    fn pre_unified_provider_resolution_prefers_the_cleaned_split_layout() {
        let legacy = CLIAgentToolbarChipSelection::Default;
        let claude = CLIAgentToolbarChipSelection::Custom {
            left: AgentToolbarItemKind::retired_cli_default_quick_insert_snapshot_items(),
            right: vec![AgentToolbarItemKind::ContextChip(
                ContextChipKind::WorkingDirectory,
            )],
            hidden_defaults: Vec::new(),
            inherit_defaults: true,
            overridden_defaults: Vec::new(),
            hidden_custom_inserts: Vec::new(),
            preserve_order: true,
        };
        let codex = CLIAgentToolbarChipSelection::custom_from_effective_items(
            AgentToolbarItemKind::cli_default_left(),
            Vec::new(),
        );

        assert!(std::ptr::eq(
            choose_pre_unified_coding_agent_selection(&legacy, Some(&claude), Some(&codex),),
            &codex,
        ));
        assert!(std::ptr::eq(
            choose_pre_unified_coding_agent_selection(&legacy, Some(&codex), None),
            &codex,
        ));
    }

    #[test]
    fn hidden_defaults_survive_while_new_defaults_are_added() {
        let old_default = AgentToolbarItemKind::ForkSession;
        let hidden_default = AgentToolbarItemKind::Compact;
        let future_default = AgentToolbarItemKind::ContinuePrompt;
        let custom = custom_insert("Mine", "mine");

        let (left, right) = merge_toolbar_defaults_and_custom_items(
            vec![
                old_default.clone(),
                hidden_default.clone(),
                future_default.clone(),
            ],
            vec![],
            &[old_default],
            std::slice::from_ref(&custom),
            &[hidden_default],
            true,
        );

        assert_eq!(
            left,
            vec![AgentToolbarItemKind::ForkSession, future_default]
        );
        assert_eq!(right, vec![custom]);
    }

    #[test]
    fn current_default_definition_wins_over_saved_snapshot_with_same_label() {
        let current = custom_insert("Review", "new prompt");
        let saved = custom_insert("Review", "old prompt");
        let custom = custom_insert("Mine", "mine");

        let (left, _) = merge_toolbar_defaults_and_custom_items(
            vec![current.clone()],
            vec![],
            &[saved, custom.clone()],
            &[],
            &[],
            false,
        );

        assert_eq!(left, vec![current, custom]);
    }

    #[test]
    fn effective_selection_records_intentional_default_removals() {
        let mut selected = AgentToolbarItemKind::cli_default_left();
        let removed = selected.remove(0);
        let custom = custom_insert("Mine", "mine");
        selected.push(custom.clone());

        let selection = CLIAgentToolbarChipSelection::custom_from_effective_items(selected, vec![]);
        let CLIAgentToolbarChipSelection::Custom {
            hidden_defaults,
            inherit_defaults,
            ..
        } = &selection
        else {
            unreachable!();
        };

        assert!(*inherit_defaults);
        assert_eq!(hidden_defaults, std::slice::from_ref(&removed));
        assert!(!contains_toolbar_item_identity(
            &selection.left_items(),
            &removed
        ));
        assert_eq!(selection.left_items().last(), Some(&custom));

        let restored =
            CLIAgentToolbarChipSelection::from_file_value(&selection.to_file_value()).unwrap();
        assert_eq!(restored, selection);
        assert!(!contains_toolbar_item_identity(
            &restored.left_items(),
            &removed
        ));
    }

    #[test]
    fn explicitly_selected_historical_preset_survives_live_default_overlay() {
        let preset = AgentToolbarItemKind::cli_quick_insert_presets()
            .into_iter()
            .find(|item| item.display_label() == "Create a Plan")
            .expect("Create a Plan is a shipped optional preset");
        let mut selected = AgentToolbarItemKind::cli_default_left();
        selected.push(preset.clone());

        let selection =
            CLIAgentToolbarChipSelection::custom_from_effective_items(selected, Vec::new());
        assert!(selection.left_items().contains(&preset));

        let restored =
            CLIAgentToolbarChipSelection::from_file_value(&selection.to_file_value()).unwrap();
        assert!(restored.left_items().contains(&preset));
    }

    #[test]
    fn resaved_full_historical_snapshot_is_normalized_without_losing_custom_buttons() {
        let custom = custom_insert("Mine", "mine");
        let mut selected = AgentToolbarItemKind::cli_default_left();
        selected.extend(AgentToolbarItemKind::retired_cli_default_quick_insert_snapshot_items());
        selected.extend([AgentToolbarItemKind::VoiceInput, custom.clone()]);

        let selection =
            CLIAgentToolbarChipSelection::custom_from_effective_items(selected, Vec::new());
        let mut expected = AgentToolbarItemKind::cli_default_left();
        expected.push(custom);
        assert_eq!(selection.left_items(), expected);
    }

    #[test]
    fn explicit_terminal_layout_preserves_reordering() {
        let mut left = AgentToolbarItemKind::terminal_default_left();
        left.swap(0, 1);
        let selection =
            TerminalToolbarChipSelection::custom_from_effective_items(left.clone(), vec![]);

        assert_eq!(selection.left_items(), left);
        let restored =
            TerminalToolbarChipSelection::from_file_value(&selection.to_file_value()).unwrap();
        assert_eq!(restored.left_items(), left);
    }

    #[test]
    fn explicit_default_override_preserves_auto_send_choice() {
        let mut left = AgentToolbarItemKind::terminal_default_left();
        let AgentToolbarItemKind::CustomInsert { auto_send, .. } = &mut left[0] else {
            unreachable!();
        };
        *auto_send = false;
        let selection =
            TerminalToolbarChipSelection::custom_from_effective_items(left.clone(), vec![]);

        assert_eq!(selection.left_items(), left);
        let TerminalToolbarChipSelection::Custom {
            overridden_defaults,
            preserve_order,
            ..
        } = &selection
        else {
            unreachable!();
        };
        assert!(*preserve_order);
        assert_eq!(overridden_defaults, &left[..1]);
        let restored =
            TerminalToolbarChipSelection::from_file_value(&selection.to_file_value()).unwrap();
        assert_eq!(restored.left_items(), left);
    }

    #[test]
    fn cli_built_in_prompt_override_preserves_auto_send_choice() {
        let mut left = AgentToolbarItemKind::cli_default_left();
        let compact_index = left
            .iter()
            .position(|item| item == &AgentToolbarItemKind::Compact)
            .unwrap();
        left[compact_index] = AgentToolbarItemKind::Compact
            .with_auto_send_behavior(false)
            .unwrap();
        let selection =
            CLIAgentToolbarChipSelection::custom_from_effective_items(left.clone(), vec![]);

        assert_eq!(selection.left_items(), left);
        let restored =
            CLIAgentToolbarChipSelection::from_file_value(&selection.to_file_value()).unwrap();
        assert_eq!(restored.left_items(), left);
    }

    #[test]
    fn untouched_ordered_default_uses_the_current_definition() {
        let current = custom_insert("Review", "new prompt");
        let saved = custom_insert("Review", "old prompt");
        let (left, _) = merge_ordered_toolbar_defaults_and_custom_items(
            vec![current.clone()],
            vec![],
            &[saved],
            &[],
            &[],
            &[],
            true,
        );

        assert_eq!(left, vec![current]);
    }
}

define_settings_group!(SessionSettings, settings: [
    working_directory_config: WorkingDirectoryConfig,
    startup_shell_override: StartupShellOverride {
        type: StartupShell,
        default: StartupShell::default(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "session.startup_shell_override",
        description: "The shell to use when Warp starts up.",
    },
    new_session_shell_override: NewSessionShellOverride {
        type: Option<NewSessionShell>,
        default: None,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "session.new_session_shell_override",
        description: "The shell to use when opening a new session.",
    }
    honor_ps1: HonorPS1 {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "terminal.input.honor_ps1",
        description: "Whether to use your shell's PS1 prompt instead of the Warp prompt.",
    },
    saved_prompt: SavedPrompt {
        type: PromptSelection,
        default: PromptSelection::default(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: true,
    },
    should_add_agent_mode_chip: ShouldAddAgentModeChip {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: true,
    },
    should_confirm_close_session: ShouldConfirmCloseSession {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "general.should_confirm_close_session",
        description: "Whether to show a confirmation dialog when closing a session.",
    },
    // Value is saved here but not shown in ui (can't be toggled in settings)
    should_confirm_shared_session_edit_access: ShouldConfirmSharedSessionEditAccess {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: true,
    }
    notifications: Notifications {
        type: NotificationsSettings,
        default: NotificationsSettings::default(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "notifications.preferences",
        max_table_depth: 1,
        description: "Notification preferences for terminal events.",
    }
    // This is a legacy setting that we no longer allow users to toggle after
    // context chips were introduced. We keep it only to respect users who
    // had previously disabled the dirty files chip via this setting.
    git_prompt_dirty_indicator: LegacyGitPromptDirtyIndicator {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: true,
        storage_key: "GitPromptDirtyIndicator",
    },
    // TODO: Remove this setting when `FeatureFlag::ProfilesDesignRevamp` is cleaned up.
    // When ProfilesDesignRevamp is enabled, model selectors are always shown in the prompt.
    // This setting only controls visibility when ProfilesDesignRevamp is disabled.
    show_model_selectors_in_prompt: ShowModelSelectorsInPrompt {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.warp_agent.input.show_model_selectors_in_prompt",
        description: "Whether to show AI model selectors in the input prompt.",
    },
    agent_footer_chip_selection: AgentToolbarChipSelectionSetting {
        type: AgentToolbarChipSelection,
        default: AgentToolbarChipSelection::default(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.warp_agent.input.agent_toolbar_chip_selection_setting",
        description: "Controls the layout of context chips in the Agent Mode toolbar.",
    },
    cli_agent_footer_chip_selection: CLIAgentToolbarChipSelectionSetting {
        type: CLIAgentToolbarChipSelection,
        default: CLIAgentToolbarChipSelection::default(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.third_party.cli_agent_toolbar_chip_selection_setting",
        description: "Legacy shared coding-agent toolbar layout used during compatibility migration.",
    },
    // Canonical coding-agent footer introduced after the provider-specific experiment. `None`
    // means this profile has not yet written the unified format and should use the compatibility
    // resolver in `coding_agent_footer_chip_selection_value`.
    coding_agent_footer_chip_selection: CodingAgentToolbarChipSelectionSetting {
        type: Option<CLIAgentToolbarChipSelection>,
        default: None,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.third_party.coding_agent_toolbar_chip_selection_setting",
        description: "Controls the shared quick-action layout for coding-agent toolbars.",
    },
    // Retained for backwards-compatible settings deserialization. A short-lived release allowed
    // Claude Code and Codex to diverge; the effective coding-agent footer is shared again through
    // `coding_agent_footer_chip_selection` so every coding-agent tab and window stays consistent.
    claude_code_footer_chip_selection: ClaudeCodeToolbarChipSelectionSetting {
        type: Option<CLIAgentToolbarChipSelection>,
        default: None,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.third_party.claude_code_toolbar_chip_selection_setting",
        description: "Deprecated provider-specific Claude Code quick-action layout.",
    },
    // See `claude_code_footer_chip_selection` above. Do not use this as an effective selection.
    codex_footer_chip_selection: CodexToolbarChipSelectionSetting {
        type: Option<CLIAgentToolbarChipSelection>,
        default: None,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "agents.third_party.codex_toolbar_chip_selection_setting",
        description: "Deprecated provider-specific Codex quick-action layout.",
    },
    terminal_footer_chip_selection: TerminalToolbarChipSelectionSetting {
        type: TerminalToolbarChipSelection,
        default: TerminalToolbarChipSelection::default(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "terminal.footer_toolbar_chip_selection",
        description: "Controls the layout of quick actions in the terminal toolbar.",
    },
    show_terminal_footer: ShowTerminalFooter {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "terminal.show_terminal_footer",
        description: "Whether to show the quick-actions toolbar in plain terminal panes.",
    },
    notification_toast_duration_secs: NotificationToastDurationSecs {
        type: u64,
        default: 8,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "notifications.toast_duration_secs",
        description: "How long notification toasts are displayed, in seconds.",
    },
    // Tracks whether the `gh` CLI is installed and authenticated on this machine.
    // Not synced because `gh` CLI availability is machine-specific.
    github_pr_chip_default_validation: GithubPrChipDefaultValidation {
        type: GithubPrPromptChipDefaultValidation,
        default: GithubPrPromptChipDefaultValidation::Unvalidated,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        private: true,
    },
    // One-time flag: whether we've already migrated the handoff-to-cloud chip
    // into a user's custom agent toolbar layout. When `Default`, the chip is
    // already present via `AgentToolbarItemKind::default_right()`, so this
    // only matters for `Custom` layouts that were saved before the chip existed.
    did_add_handoff_chip_to_toolbar: DidAddHandoffChipToToolbar {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::No),
        private: true,
    },
]);

impl SessionSettings {
    pub fn coding_agent_footer_chip_selection_value(&self) -> &CLIAgentToolbarChipSelection {
        if let Some(selection) = self.coding_agent_footer_chip_selection.value().as_ref() {
            return selection;
        }

        let legacy = self.cli_agent_footer_chip_selection.value();
        let claude = self.claude_code_footer_chip_selection.value().as_ref();
        let codex = self.codex_footer_chip_selection.value().as_ref();
        choose_pre_unified_coding_agent_selection(legacy, claude, codex)
    }

    /// Claude Code and Codex share one coding-agent footer layout. The provider-specific fields
    /// above remain readable only so settings written by the short-lived split-layout release do
    /// not fail to deserialize.
    pub fn claude_code_footer_chip_selection_value(&self) -> &CLIAgentToolbarChipSelection {
        self.coding_agent_footer_chip_selection_value()
    }

    pub fn codex_footer_chip_selection_value(&self) -> &CLIAgentToolbarChipSelection {
        self.coding_agent_footer_chip_selection_value()
    }

    pub fn footer_chip_selection_for_cli_agent(
        &self,
        _agent: crate::terminal::CLIAgent,
    ) -> &CLIAgentToolbarChipSelection {
        self.coding_agent_footer_chip_selection_value()
    }
}

settings::macros::implement_setting_for_enum!(
    WorkingDirectoryConfig,
    SessionSettings,
    SupportedPlatforms::ALL,
    SyncToCloud::Never,
    private: false,
    toml_path: "session.working_directory_config",
    max_table_depth: 1,
    description: "Controls the working directory used when opening new sessions.",
);
