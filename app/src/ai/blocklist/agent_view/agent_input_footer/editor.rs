//! Modal for customizing the agent input footer chip layout.
//!
//! Uses the shared [`ChipConfigurator`] to arrange footer controls. Agent View keeps its
//! left/right layout, while CLI-agent and terminal footers use one ordered footer zone.

use std::collections::HashMap;

use settings::Setting as _;
use warpui::keymap::FixedBinding;
use warpui::{AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext};

use super::toolbar_item::AgentToolbarItemKind;
use crate::chip_configurator::{
    render_chip_editor_modal, render_chip_editor_sections, ChipConfigurator,
    ChipConfiguratorAction, ChipConfiguratorLayout, ChipEditorModalConfig, ChipEditorMouseHandles,
    ChipEditorSectionsConfig, ChipEditorTab,
};
use crate::terminal::session_settings::{
    AgentToolbarChipSelection, CLIAgentToolbarChipSelection, SessionSettings,
    SessionSettingsChangedEvent, TerminalToolbarChipSelection, ToolbarChipSelection,
};
use crate::{report_if_error, Appearance};

const AGENT_MODAL_TITLE: &str = "Edit agent toolbelt";
const FOOTER_MODAL_TITLE: &str = "Edit footer buttons";

/// Controls which set of items and settings the editor modal operates on.
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub enum AgentToolbarEditorMode {
    #[default]
    AgentView,
    /// Legacy shared CLI-agent selection used as the migration fallback and by the AI settings
    /// inline editor. The unified popup opens provider-specific tabs instead.
    CLIAgent,
    ClaudeCode,
    Codex,
    Terminal,
}
pub enum AgentToolbarEditorEvent {
    Close,
    AddQuickInsert(AgentToolbarEditorMode),
}

pub struct AgentToolbarEditorModal {
    mouse_handles: ChipEditorMouseHandles,
    chip_configurator: ChipConfigurator,
    drafts: HashMap<AgentToolbarEditorMode, ToolbarEditorDraft>,
    dirty_modes: Vec<AgentToolbarEditorMode>,
    mode: AgentToolbarEditorMode,
    is_dirty: bool,
}

#[derive(Clone, Debug)]
struct ToolbarEditorDraft {
    left: Vec<AgentToolbarItemKind>,
    right: Vec<AgentToolbarItemKind>,
    hidden_custom_inserts: Vec<AgentToolbarItemKind>,
}

pub struct AgentToolbarInlineEditor {
    mouse_handles: ChipEditorMouseHandles,
    chip_configurator: ChipConfigurator,
    mode: AgentToolbarEditorMode,
}

#[derive(Clone, Copy, Debug)]
pub enum AgentToolbarEditorAction {
    Cancel,
    Save,
    AddQuickInsert,
    SelectMode(AgentToolbarEditorMode),
    Chip(ChipConfiguratorAction),
    ResetDefault,
    /// Dummy action used as on_click for chip bank clicks (no-op).
    Activate,
}

#[derive(Clone, Copy, Debug)]
pub enum AgentToolbarInlineEditorAction {
    Chip(ChipConfiguratorAction),
    ResetDefault,
    /// Dummy action used as on_click for chip bank clicks (no-op).
    Activate,
}

fn layout_for_mode(mode: AgentToolbarEditorMode) -> ChipConfiguratorLayout {
    match mode {
        AgentToolbarEditorMode::AgentView => ChipConfiguratorLayout::LeftRightZones,
        AgentToolbarEditorMode::CLIAgent
        | AgentToolbarEditorMode::ClaudeCode
        | AgentToolbarEditorMode::Codex
        | AgentToolbarEditorMode::Terminal => ChipConfiguratorLayout::SingleZone,
    }
}

fn selected_toolbar_items(
    mode: AgentToolbarEditorMode,
    chip_configurator: &ChipConfigurator,
) -> (Vec<AgentToolbarItemKind>, Vec<AgentToolbarItemKind>) {
    match mode {
        AgentToolbarEditorMode::AgentView => (
            chip_configurator.left_item_kinds(),
            chip_configurator.right_item_kinds(),
        ),
        AgentToolbarEditorMode::CLIAgent
        | AgentToolbarEditorMode::ClaudeCode
        | AgentToolbarEditorMode::Codex
        | AgentToolbarEditorMode::Terminal => (chip_configurator.used_item_kinds(), Vec::new()),
    }
}

fn open_toolbar_items_from_settings<V: View>(
    chip_configurator: &mut ChipConfigurator,
    mode: AgentToolbarEditorMode,
    ctx: &mut ViewContext<V>,
) {
    let appearance = Appearance::as_ref(ctx);
    let session_settings = SessionSettings::as_ref(ctx);
    let (current_left, current_right, available) = match mode {
        AgentToolbarEditorMode::AgentView => {
            let selection = session_settings.agent_footer_chip_selection.clone();
            (
                selection.left_items(),
                selection.right_items(),
                AgentToolbarItemKind::all_available(),
            )
        }
        mode @ (AgentToolbarEditorMode::CLIAgent
        | AgentToolbarEditorMode::ClaudeCode
        | AgentToolbarEditorMode::Codex) => {
            let selection = match mode {
                AgentToolbarEditorMode::CLIAgent => session_settings
                    .cli_agent_footer_chip_selection
                    .value()
                    .clone(),
                AgentToolbarEditorMode::ClaudeCode => session_settings
                    .claude_code_footer_chip_selection_value()
                    .clone(),
                AgentToolbarEditorMode::Codex => {
                    session_settings.codex_footer_chip_selection_value().clone()
                }
                _ => unreachable!(),
            };
            let mut available = AgentToolbarItemKind::all_available_for_cli_input();
            for item in selection.hidden_custom_inserts() {
                if !available
                    .iter()
                    .any(|available| available.has_same_toolbar_identity(&item))
                {
                    available.push(item);
                }
            }
            (selection.left_items(), selection.right_items(), available)
        }
        AgentToolbarEditorMode::Terminal => {
            let selection = session_settings.terminal_footer_chip_selection.clone();
            let current_left = selection.left_items();
            let current_right = selection.right_items();
            let mut available = AgentToolbarItemKind::all_available_for_terminal_input();
            for item in selection.hidden_custom_inserts() {
                if !available
                    .iter()
                    .any(|available| available.has_same_toolbar_identity(&item))
                {
                    available.push(item);
                }
            }
            for item in current_left.iter().chain(&current_right) {
                if item.is_available_for_terminal()
                    && !available
                        .iter()
                        .any(|available| available.has_same_toolbar_identity(item))
                {
                    available.push(item.clone());
                }
            }
            (current_left, current_right, available)
        }
    };

    // Filter out items that are unavailable due to runtime state (user settings,
    // workspace config, etc.) on top of the feature-flag checks in all_available().
    let available: Vec<AgentToolbarItemKind> = available
        .into_iter()
        .filter(|item| item.is_available(ctx))
        .collect();

    // Drop saved items that are no longer available (e.g. their feature flag was disabled
    // or a setting was turned off).
    let filter_unavailable = |items: Vec<AgentToolbarItemKind>| -> Vec<AgentToolbarItemKind> {
        items
            .into_iter()
            .filter(|item| {
                matches!(item, AgentToolbarItemKind::CustomInsert { .. })
                    || available.contains(item)
            })
            .collect()
    };
    let current_left = filter_unavailable(current_left);
    let current_right = filter_unavailable(current_right);

    match mode {
        AgentToolbarEditorMode::AgentView => chip_configurator.open_left_right_zones_with_items(
            current_left,
            current_right,
            available,
            appearance,
        ),
        AgentToolbarEditorMode::CLIAgent
        | AgentToolbarEditorMode::ClaudeCode
        | AgentToolbarEditorMode::Codex
        | AgentToolbarEditorMode::Terminal => {
            let current = current_left.into_iter().chain(current_right).collect();
            chip_configurator.open_single_zone_with_items(current, available, appearance);
        }
    }
}

fn open_toolbar_items_from_draft<V: View>(
    chip_configurator: &mut ChipConfigurator,
    mode: AgentToolbarEditorMode,
    draft: &ToolbarEditorDraft,
    ctx: &mut ViewContext<V>,
) {
    let appearance = Appearance::as_ref(ctx);
    let (_, _, mut available) = AgentToolbarItemKind::defaults_for_mode(mode);
    for item in &draft.hidden_custom_inserts {
        if !available
            .iter()
            .any(|available| available.has_same_toolbar_identity(item))
        {
            available.push(item.clone());
        }
    }
    let available = available
        .into_iter()
        .filter(|item| item.is_available(ctx))
        .collect();
    match mode {
        AgentToolbarEditorMode::AgentView => chip_configurator.open_left_right_zones_with_items(
            draft.left.clone(),
            draft.right.clone(),
            available,
            appearance,
        ),
        AgentToolbarEditorMode::CLIAgent
        | AgentToolbarEditorMode::ClaudeCode
        | AgentToolbarEditorMode::Codex
        | AgentToolbarEditorMode::Terminal => chip_configurator.open_single_zone_with_items(
            draft.left.iter().chain(&draft.right).cloned().collect(),
            available,
            appearance,
        ),
    }
}

fn open_default_toolbar_items<V: View>(
    chip_configurator: &mut ChipConfigurator,
    mode: AgentToolbarEditorMode,
    ctx: &mut ViewContext<V>,
) {
    let appearance = Appearance::as_ref(ctx);
    let (left, right, available) = AgentToolbarItemKind::defaults_for_mode(mode);
    let filter_runtime = |items: Vec<AgentToolbarItemKind>| -> Vec<AgentToolbarItemKind> {
        items
            .into_iter()
            .filter(|item| item.is_available(ctx))
            .collect()
    };
    let left = filter_runtime(left);
    let right = filter_runtime(right);
    let available = filter_runtime(available);
    match mode {
        AgentToolbarEditorMode::AgentView => {
            chip_configurator.open_left_right_zones_with_items(left, right, available, appearance)
        }
        AgentToolbarEditorMode::CLIAgent
        | AgentToolbarEditorMode::ClaudeCode
        | AgentToolbarEditorMode::Codex
        | AgentToolbarEditorMode::Terminal => chip_configurator.open_single_zone_with_items(
            left.into_iter().chain(right).collect(),
            available,
            appearance,
        ),
    }
}

fn is_toolbar_editor_at_defaults(
    mode: AgentToolbarEditorMode,
    chip_configurator: &ChipConfigurator,
) -> bool {
    let (left, right) = selected_toolbar_items(mode, chip_configurator);
    toolbar_items_match_defaults(mode, &left, &right)
        && hidden_custom_inserts_from_configurator(mode, chip_configurator).is_empty()
}

fn toolbar_items_match_defaults(
    mode: AgentToolbarEditorMode,
    left: &[AgentToolbarItemKind],
    right: &[AgentToolbarItemKind],
) -> bool {
    let (default_left, default_right, _) = AgentToolbarItemKind::defaults_for_mode(mode);
    default_left.as_slice() == left && default_right.as_slice() == right
}

fn hidden_custom_inserts_from_configurator(
    mode: AgentToolbarEditorMode,
    chip_configurator: &ChipConfigurator,
) -> Vec<AgentToolbarItemKind> {
    chip_configurator
        .unused_item_kinds()
        .into_iter()
        .filter(|item| {
            matches!(item, AgentToolbarItemKind::CustomInsert { .. })
                && !item.is_shipped_quick_insert_for_mode(mode)
        })
        .collect()
}

fn available_section_label(mode: AgentToolbarEditorMode) -> &'static str {
    match mode {
        AgentToolbarEditorMode::AgentView => "Available chips",
        AgentToolbarEditorMode::CLIAgent
        | AgentToolbarEditorMode::ClaudeCode
        | AgentToolbarEditorMode::Codex
        | AgentToolbarEditorMode::Terminal => "Available buttons",
    }
}

impl AgentToolbarInlineEditor {
    pub fn new(mode: AgentToolbarEditorMode, ctx: &mut ViewContext<Self>) -> Self {
        let mut editor = Self {
            mouse_handles: Default::default(),
            chip_configurator: ChipConfigurator::new(layout_for_mode(mode)),
            mode,
        };
        editor.reset_from_settings(ctx);

        ctx.subscribe_to_model(&SessionSettings::handle(ctx), |me, _, event, ctx| {
            let should_refresh = matches!(
                (me.mode, event),
                (
                    AgentToolbarEditorMode::AgentView,
                    SessionSettingsChangedEvent::AgentToolbarChipSelectionSetting { .. },
                ) | (
                    AgentToolbarEditorMode::CLIAgent,
                    SessionSettingsChangedEvent::CLIAgentToolbarChipSelectionSetting { .. },
                ) | (
                    AgentToolbarEditorMode::ClaudeCode,
                    SessionSettingsChangedEvent::ClaudeCodeToolbarChipSelectionSetting { .. },
                ) | (
                    AgentToolbarEditorMode::Codex,
                    SessionSettingsChangedEvent::CodexToolbarChipSelectionSetting { .. },
                ) | (
                    AgentToolbarEditorMode::Terminal,
                    SessionSettingsChangedEvent::TerminalToolbarChipSelectionSetting { .. },
                )
            );

            if should_refresh && me.chip_configurator.current_dragging_state.is_none() {
                me.reset_from_settings(ctx);
                ctx.notify();
            }
        });

        editor
    }

    fn reset_from_settings(&mut self, ctx: &mut ViewContext<Self>) {
        open_toolbar_items_from_settings(&mut self.chip_configurator, self.mode, ctx);
    }

    fn save_current_selection(&self, ctx: &mut ViewContext<Self>) {
        let (left, right) = selected_toolbar_items(self.mode, &self.chip_configurator);
        let hidden_custom_inserts =
            hidden_custom_inserts_from_configurator(self.mode, &self.chip_configurator);
        save_toolbar_selection(self.mode, left, right, hidden_custom_inserts, ctx);
    }

    fn is_at_defaults(&self) -> bool {
        is_toolbar_editor_at_defaults(self.mode, &self.chip_configurator)
    }
}

impl Entity for AgentToolbarInlineEditor {
    type Event = ();
}

impl TypedActionView for AgentToolbarInlineEditor {
    type Action = AgentToolbarInlineEditorAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            Self::Action::Chip(chip_action) => {
                let should_save = self.chip_configurator.handle_action(chip_action, ctx);
                if should_save {
                    self.save_current_selection(ctx);
                }
                ctx.notify();
            }
            Self::Action::ResetDefault => {
                open_default_toolbar_items(&mut self.chip_configurator, self.mode, ctx);
                self.save_current_selection(ctx);
                ctx.notify();
            }
            Self::Action::Activate => {
                // no-op — used as the on_click for chip bank items
            }
        }
    }
}

impl View for AgentToolbarInlineEditor {
    fn ui_name() -> &'static str {
        "AgentToolbarInlineEditor"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        render_chip_editor_sections(
            &self.chip_configurator,
            ChipEditorSectionsConfig {
                available_section_label: available_section_label(self.mode),
                is_at_defaults: self.is_at_defaults(),
                reset_action: AgentToolbarInlineEditorAction::ResetDefault,
                activate_action: AgentToolbarInlineEditorAction::Activate,
                chip_action_wrapper: AgentToolbarInlineEditorAction::Chip,
                mouse_handles: &self.mouse_handles,
            },
            appearance,
        )
    }
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([FixedBinding::new(
        "escape",
        AgentToolbarEditorAction::Cancel,
        id!(AgentToolbarEditorModal::ui_name()),
    )]);
}

/// Appends a new `CustomInsert` button after the CLI footer's effective defaults and custom
/// entries, then persists the selection as a live-default overlay.
///
/// Pure so it can be unit-tested without touching global settings; the
/// persisting wrapper `append_cli_custom_button` reads/writes the setting
/// around this.
pub fn next_selection_with_custom_button(
    current: CLIAgentToolbarChipSelection,
    label: String,
    text: String,
    auto_send: bool,
    visible: bool,
) -> CLIAgentToolbarChipSelection {
    let mut left = current.left_items();
    let right = current.right_items();
    let mut hidden_custom_inserts = current.hidden_custom_inserts();
    let button = AgentToolbarItemKind::CustomInsert {
        label,
        text,
        auto_send,
    };
    if visible {
        left.push(button);
    } else {
        hidden_custom_inserts.push(button);
    }
    CLIAgentToolbarChipSelection::custom_from_effective_items_and_hidden_custom_inserts(
        left,
        right,
        hidden_custom_inserts,
    )
}

/// Persists a new custom quick-insert button into a shared or provider-specific CLI footer.
pub fn append_cli_custom_button<V: View>(
    mode: AgentToolbarEditorMode,
    label: String,
    text: String,
    auto_send: bool,
    visible: bool,
    ctx: &mut ViewContext<V>,
) {
    let current = match mode {
        AgentToolbarEditorMode::CLIAgent => SessionSettings::as_ref(ctx)
            .cli_agent_footer_chip_selection
            .value()
            .clone(),
        AgentToolbarEditorMode::ClaudeCode => SessionSettings::as_ref(ctx)
            .claude_code_footer_chip_selection_value()
            .clone(),
        AgentToolbarEditorMode::Codex => SessionSettings::as_ref(ctx)
            .codex_footer_chip_selection_value()
            .clone(),
        AgentToolbarEditorMode::AgentView | AgentToolbarEditorMode::Terminal => return,
    };
    let next = next_selection_with_custom_button(current, label, text, auto_send, visible);
    SessionSettings::handle(ctx).update(ctx, |settings, ctx| match mode {
        AgentToolbarEditorMode::CLIAgent => {
            report_if_error!(settings
                .cli_agent_footer_chip_selection
                .set_value(next, ctx));
        }
        AgentToolbarEditorMode::ClaudeCode => {
            report_if_error!(settings
                .claude_code_footer_chip_selection
                .set_value(Some(next), ctx));
        }
        AgentToolbarEditorMode::Codex => {
            report_if_error!(settings
                .codex_footer_chip_selection
                .set_value(Some(next), ctx));
        }
        AgentToolbarEditorMode::AgentView | AgentToolbarEditorMode::Terminal => {}
    });
}

pub fn next_terminal_selection_with_custom_button(
    current: TerminalToolbarChipSelection,
    label: String,
    text: String,
    auto_send: bool,
    visible: bool,
) -> TerminalToolbarChipSelection {
    let mut left = current.left_items();
    let right = current.right_items();
    let mut hidden_custom_inserts = current.hidden_custom_inserts();
    let button = AgentToolbarItemKind::CustomInsert {
        label,
        text,
        auto_send,
    };
    if visible {
        left.push(button);
    } else {
        hidden_custom_inserts.push(button);
    }
    TerminalToolbarChipSelection::custom_from_effective_items_and_hidden_custom_inserts(
        left,
        right,
        hidden_custom_inserts,
    )
}

pub fn append_terminal_custom_button<V: View>(
    label: String,
    text: String,
    auto_send: bool,
    visible: bool,
    ctx: &mut ViewContext<V>,
) {
    let current = SessionSettings::as_ref(ctx)
        .terminal_footer_chip_selection
        .clone();
    let next = next_terminal_selection_with_custom_button(current, label, text, auto_send, visible);
    SessionSettings::handle(ctx).update(ctx, |settings, ctx| {
        report_if_error!(settings.terminal_footer_chip_selection.set_value(next, ctx));
    });
}

fn save_toolbar_selection<V: View>(
    mode: AgentToolbarEditorMode,
    left: Vec<AgentToolbarItemKind>,
    right: Vec<AgentToolbarItemKind>,
    hidden_custom_inserts: Vec<AgentToolbarItemKind>,
    ctx: &mut ViewContext<V>,
) {
    let is_default =
        hidden_custom_inserts.is_empty() && toolbar_items_match_defaults(mode, &left, &right);
    match mode {
        AgentToolbarEditorMode::AgentView => {
            let selection = if is_default {
                AgentToolbarChipSelection::Default
            } else {
                AgentToolbarChipSelection::Custom { left, right }
            };
            SessionSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(settings
                    .agent_footer_chip_selection
                    .set_value(selection, ctx));
            });
        }
        mode @ (AgentToolbarEditorMode::CLIAgent
        | AgentToolbarEditorMode::ClaudeCode
        | AgentToolbarEditorMode::Codex) => {
            let selection = if is_default {
                CLIAgentToolbarChipSelection::Default
            } else {
                CLIAgentToolbarChipSelection::custom_from_effective_items_and_hidden_custom_inserts(
                    left,
                    right,
                    hidden_custom_inserts,
                )
            };
            SessionSettings::handle(ctx).update(ctx, |settings, ctx| match mode {
                AgentToolbarEditorMode::CLIAgent => {
                    report_if_error!(settings
                        .cli_agent_footer_chip_selection
                        .set_value(selection, ctx));
                }
                AgentToolbarEditorMode::ClaudeCode => {
                    report_if_error!(settings
                        .claude_code_footer_chip_selection
                        .set_value(Some(selection), ctx));
                }
                AgentToolbarEditorMode::Codex => {
                    report_if_error!(settings
                        .codex_footer_chip_selection
                        .set_value(Some(selection), ctx));
                }
                AgentToolbarEditorMode::AgentView | AgentToolbarEditorMode::Terminal => {}
            });
        }
        AgentToolbarEditorMode::Terminal => {
            let selection = if is_default {
                TerminalToolbarChipSelection::Default
            } else {
                TerminalToolbarChipSelection::custom_from_effective_items_and_hidden_custom_inserts(
                    left,
                    right,
                    hidden_custom_inserts,
                )
            };
            SessionSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(settings
                    .terminal_footer_chip_selection
                    .set_value(selection, ctx));
            });
        }
    }
}

impl AgentToolbarEditorModal {
    pub fn new(_ctx: &mut ViewContext<Self>) -> Self {
        Self {
            mouse_handles: Default::default(),
            chip_configurator: ChipConfigurator::new(ChipConfiguratorLayout::LeftRightZones),
            drafts: HashMap::new(),
            dirty_modes: Vec::new(),
            mode: AgentToolbarEditorMode::default(),
            is_dirty: false,
        }
    }

    pub fn open(&mut self, mode: AgentToolbarEditorMode, ctx: &mut ViewContext<Self>) {
        self.reset();
        self.mode = match mode {
            AgentToolbarEditorMode::CLIAgent => AgentToolbarEditorMode::ClaudeCode,
            mode => mode,
        };
        open_toolbar_items_from_settings(&mut self.chip_configurator, self.mode, ctx);
        ctx.notify();
    }

    fn current_draft(&self) -> ToolbarEditorDraft {
        let (left, right) = selected_toolbar_items(self.mode, &self.chip_configurator);
        ToolbarEditorDraft {
            left,
            right,
            hidden_custom_inserts: hidden_custom_inserts_from_configurator(
                self.mode,
                &self.chip_configurator,
            ),
        }
    }

    fn mark_current_mode_dirty(&mut self) {
        if !self.dirty_modes.contains(&self.mode) {
            self.dirty_modes.push(self.mode);
        }
        self.is_dirty = true;
    }

    fn select_mode(&mut self, mode: AgentToolbarEditorMode, ctx: &mut ViewContext<Self>) {
        if mode == self.mode
            || matches!(
                mode,
                AgentToolbarEditorMode::AgentView | AgentToolbarEditorMode::CLIAgent
            )
        {
            return;
        }
        if self.dirty_modes.contains(&self.mode) {
            self.drafts.insert(self.mode, self.current_draft());
        }
        self.mode = mode;
        if let Some(draft) = self.drafts.get(&mode) {
            open_toolbar_items_from_draft(&mut self.chip_configurator, mode, draft, ctx);
        } else {
            open_toolbar_items_from_settings(&mut self.chip_configurator, mode, ctx);
        }
        ctx.notify();
    }

    fn save_to_settings(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.is_dirty {
            return;
        }
        if self.dirty_modes.contains(&self.mode) {
            self.drafts.insert(self.mode, self.current_draft());
        }
        for mode in self.dirty_modes.clone() {
            let Some(draft) = self.drafts.get(&mode).cloned() else {
                continue;
            };
            save_toolbar_selection(
                mode,
                draft.left,
                draft.right,
                draft.hidden_custom_inserts,
                ctx,
            );
        }
        self.dirty_modes.clear();
        self.is_dirty = false;
    }

    fn reset(&mut self) {
        self.chip_configurator.reset();
        self.drafts.clear();
        self.dirty_modes.clear();
        self.is_dirty = false;
    }

    fn modal_title(&self) -> &'static str {
        match self.mode {
            AgentToolbarEditorMode::AgentView => AGENT_MODAL_TITLE,
            AgentToolbarEditorMode::CLIAgent
            | AgentToolbarEditorMode::ClaudeCode
            | AgentToolbarEditorMode::Codex
            | AgentToolbarEditorMode::Terminal => FOOTER_MODAL_TITLE,
        }
    }
}

impl Entity for AgentToolbarEditorModal {
    type Event = AgentToolbarEditorEvent;
}

impl TypedActionView for AgentToolbarEditorModal {
    type Action = AgentToolbarEditorAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            Self::Action::Cancel => {
                self.reset();
                ctx.emit(AgentToolbarEditorEvent::Close);
            }
            Self::Action::Save => {
                self.save_to_settings(ctx);
                ctx.emit(AgentToolbarEditorEvent::Close);
            }
            Self::Action::AddQuickInsert => {
                self.save_to_settings(ctx);
                ctx.emit(AgentToolbarEditorEvent::AddQuickInsert(self.mode));
            }
            Self::Action::SelectMode(mode) => self.select_mode(*mode, ctx),
            Self::Action::Chip(chip_action) => {
                let mutated = self.chip_configurator.handle_action(chip_action, ctx);
                if mutated {
                    self.mark_current_mode_dirty();
                }
                ctx.notify();
            }
            Self::Action::ResetDefault => {
                self.mark_current_mode_dirty();
                open_default_toolbar_items(&mut self.chip_configurator, self.mode, ctx);
                ctx.notify();
            }
            Self::Action::Activate => {
                // no-op — used as the on_click for chip bank items
            }
        }
    }
}

impl AgentToolbarEditorModal {
    fn is_at_defaults(&self) -> bool {
        is_toolbar_editor_at_defaults(self.mode, &self.chip_configurator)
    }
}

impl View for AgentToolbarEditorModal {
    fn ui_name() -> &'static str {
        "AgentToolbarEditorModal"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let tabs = if self.mode == AgentToolbarEditorMode::AgentView {
            Vec::new()
        } else {
            vec![
                ChipEditorTab {
                    label: "Terminal",
                    selected: self.mode == AgentToolbarEditorMode::Terminal,
                    action: AgentToolbarEditorAction::SelectMode(AgentToolbarEditorMode::Terminal),
                    mouse_handle: &self.mouse_handles.terminal_tab,
                },
                ChipEditorTab {
                    label: "Codex",
                    selected: self.mode == AgentToolbarEditorMode::Codex,
                    action: AgentToolbarEditorAction::SelectMode(AgentToolbarEditorMode::Codex),
                    mouse_handle: &self.mouse_handles.codex_tab,
                },
                ChipEditorTab {
                    label: "Claude Code",
                    selected: self.mode == AgentToolbarEditorMode::ClaudeCode,
                    action: AgentToolbarEditorAction::SelectMode(
                        AgentToolbarEditorMode::ClaudeCode,
                    ),
                    mouse_handle: &self.mouse_handles.claude_code_tab,
                },
            ]
        };
        render_chip_editor_modal(
            &self.chip_configurator,
            ChipEditorModalConfig {
                title: self.modal_title(),
                available_section_label: available_section_label(self.mode),
                is_at_defaults: self.is_at_defaults(),
                is_dirty: self.is_dirty,
                cancel_action: AgentToolbarEditorAction::Cancel,
                save_action: AgentToolbarEditorAction::Save,
                add_action: (!matches!(self.mode, AgentToolbarEditorMode::AgentView))
                    .then_some(AgentToolbarEditorAction::AddQuickInsert),
                tabs,
                reset_action: AgentToolbarEditorAction::ResetDefault,
                activate_action: AgentToolbarEditorAction::Activate,
                chip_action_wrapper: AgentToolbarEditorAction::Chip,
                mouse_handles: &self.mouse_handles,
            },
            appearance,
        )
    }
}

#[cfg(test)]
#[path = "editor_tests.rs"]
mod tests;
