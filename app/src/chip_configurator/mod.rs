//! Generalized drag/drop chip configurator.
//!
//! This module extracts the reusable chip arrangement logic from the prompt
//! `EditorModal` so that it can be shared between the terminal prompt editor
//! and the agent input footer editor.
pub(crate) mod modal_shell;

use std::cell::RefCell;
use std::collections::HashMap;

pub(crate) use modal_shell::{
    render_chip_editor_modal, render_chip_editor_sections, ChipEditorModalConfig,
    ChipEditorMouseHandles, ChipEditorSectionsConfig, ChipEditorTab,
};
use pathfinder_color::ColorU;
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::vec2f;
use warp_core::ui::theme::Fill;
use warp_core::ui::tokens::{corners_sm, Tokens};
use warpui::elements::{
    Border, ChildAnchor, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox, Container,
    CrossAxisAlignment, Dash, DispatchEventResult, Draggable, DraggableState, Element, Empty,
    EventHandler, Expanded, Flex, Hoverable, MainAxisSize, MouseStateHandle, OffsetPositioning,
    ParentAnchor, ParentElement, ParentOffsetBounds, Rect, SavePosition, ScrollbarWidth, Stack,
    Text, Wrap,
};
use warpui::fonts::{FamilyId, Properties};
use warpui::platform::Cursor;
use warpui::text_layout::ClipConfig;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{Action, SingletonEntity, View, ViewContext};

use crate::ai::blocklist::agent_view::toolbar_item::AgentToolbarItemKind;
use crate::appearance::Appearance;
use crate::context_chips::display_chip::{chip_container, udi_font_size};
use crate::context_chips::renderer::{ChipDragState, Renderer as ContextChipRenderer};
use crate::context_chips::{spacing, ChipAvailability, ContextChipKind};
use crate::ui_components::icons;

const USED_CHIPS_POSITION_ID: &str = "chip_cfg_used";
const LEFT_CHIPS_POSITION_ID: &str = "chip_cfg_left";
const RIGHT_CHIPS_POSITION_ID: &str = "chip_cfg_right";
const UNUSED_CHIPS_POSITION_ID: &str = "chip_cfg_unused";

const LIST_ROW_HEIGHT: f32 = 30.;
const LIST_ROW_GAP: f32 = 4.;
const LIST_FONT_SIZE: f32 = 13.;
const LIST_CAPTION_FONT_SIZE: f32 = 12.;
const LIST_ICON_SIZE: f32 = 14.;
/// Past this the lists scroll, so the dialog still fits a short window.
const LIST_MAX_HEIGHT: f32 = 470.;
/// Room on the right so the scrollbar never sits on top of a row's buttons.
const LIST_SCROLLBAR_GUTTER: f32 = 10.;

/// How a single zone reads when it is shown as an ordered list, one item per row. Order decides
/// what shows: the first `visible_count` items sit on the footer and the rest open from More.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListPresentation {
    pub visible_count: usize,
    /// The same, when the pane is narrow.
    pub compact_visible_count: usize,
}

/// Where a dragged row lands in a list: after every other row whose middle is above the pointer.
/// `dragged_slot` is the row's own slot when it is moving within this list. Rows share one
/// height, so slots stay put while items move through them, and the answer cannot flip-flop.
fn list_insertion_index(
    slot_middles: &[f32],
    dragged_slot: Option<usize>,
    pointer_y: f32,
) -> usize {
    slot_middles
        .iter()
        .enumerate()
        .filter(|(slot, middle)| Some(*slot) != dragged_slot && **middle < pointer_y)
        .count()
}

/// An item that can be placed in the configurator — either a context chip or
/// an interactive control button.
pub enum ConfigurableItem {
    ContextChip(Box<ContextChipRenderer>),
    Control(ControlItemRenderer),
}

impl ConfigurableItem {
    pub fn from_toolbar_item(kind: AgentToolbarItemKind, appearance: &Appearance) -> Option<Self> {
        match kind {
            AgentToolbarItemKind::ContextChip(chip_kind) => {
                ContextChipRenderer::default_from_kind_with_agent_view(
                    chip_kind,
                    ChipAvailability::Enabled,
                    true,
                    appearance,
                )
                .map(Box::new)
                .map(Self::ContextChip)
            }
            control => Some(Self::Control(ControlItemRenderer::new(control))),
        }
    }

    pub fn item_kind(&self) -> Option<AgentToolbarItemKind> {
        match self {
            Self::ContextChip(r) => Some(AgentToolbarItemKind::ContextChip(r.chip_kind().clone())),
            Self::Control(r) => r.kind.clone(),
        }
    }

    pub fn chip_kind(&self) -> Option<&ContextChipKind> {
        match self {
            Self::ContextChip(r) => Some(r.chip_kind()),
            Self::Control(_) => None,
        }
    }

    pub fn is_removable(&self) -> bool {
        match self {
            Self::ContextChip(_) => true,
            Self::Control(r) => r.removable,
        }
    }

    /// Whether this chip is pinned to the left zone in a `LeftRightZones`
    /// layout — it may be reordered within the left zone but cannot be moved
    /// to the right zone. Used to lock the tabs-panel chip to the left.
    pub fn is_left_pinned(&self) -> bool {
        match self {
            Self::ContextChip(_) => false,
            Self::Control(r) => r.left_pinned,
        }
    }

    fn display_label(&self) -> String {
        match self {
            Self::ContextChip(_) => self
                .item_kind()
                .map(|kind| kind.display_label().into_owned())
                .unwrap_or_default(),
            Self::Control(r) => r.display_label().into_owned(),
        }
    }

    fn display_icon(&self) -> Option<icons::Icon> {
        match self {
            Self::ContextChip(_) => None,
            Self::Control(r) => r.display_icon(),
        }
    }

    pub fn draggable_state(&self) -> DraggableState {
        match self {
            Self::ContextChip(r) => r.draggable_state(),
            Self::Control(r) => r.draggable_state.clone(),
        }
    }

    pub fn render_unused(
        &self,
        drag_state: ChipDragState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        match self {
            Self::ContextChip(r) => r.render_unused(drag_state, appearance),
            Self::Control(r) => r.render_internal(drag_state, None, appearance),
        }
    }

    pub fn render_used<A: Action + Clone>(
        &self,
        drag_state: ChipDragState,
        on_remove_action: A,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        match self {
            Self::ContextChip(r) => r.render_used(drag_state, on_remove_action, appearance),
            Self::Control(r) => {
                let remove_button = if r.removable {
                    Some(r.render_remove_button(on_remove_action, appearance))
                } else {
                    None
                };
                r.render_internal(drag_state, remove_button, appearance)
            }
        }
    }
}

/// Lightweight renderer for non-chip control items (model selector, NLD toggle,
/// voice input, image attach, file explorer, view changes, compose, etc.)
/// inside the configurator.
pub struct ControlItemRenderer {
    kind: Option<AgentToolbarItemKind>,
    custom_label: Option<String>,
    custom_icon: Option<crate::ui_components::icons::Icon>,
    /// An opaque string identifier for round-tripping items through the configurator.
    /// Used by the header toolbar editor to recover the `HeaderToolbarItemKind`.
    identifier: Option<String>,
    removable: bool,
    /// When set (in a `LeftRightZones` layout), the chip is pinned to the left
    /// zone: it can be reordered within the left zone but never moved to the
    /// right zone. Used to lock the tabs-panel chip to the left.
    left_pinned: bool,
    draggable_state: DraggableState,
    tooltip_state_handle: MouseStateHandle,
    remove_button_state_handle: MouseStateHandle,
}

impl ControlItemRenderer {
    pub fn new(kind: AgentToolbarItemKind) -> Self {
        Self {
            kind: Some(kind),
            custom_label: None,
            custom_icon: None,
            identifier: None,
            removable: true,
            left_pinned: false,
            draggable_state: Default::default(),
            tooltip_state_handle: Default::default(),
            remove_button_state_handle: Default::default(),
        }
    }

    pub fn new_with_label_and_icon(label: String, icon: crate::ui_components::icons::Icon) -> Self {
        Self {
            kind: None,
            custom_label: Some(label),
            custom_icon: Some(icon),
            identifier: None,
            removable: true,
            left_pinned: false,
            draggable_state: Default::default(),
            tooltip_state_handle: Default::default(),
            remove_button_state_handle: Default::default(),
        }
    }

    pub fn with_identifier(mut self, id: String) -> Self {
        self.identifier = Some(id);
        self
    }

    pub(crate) fn identifier(&self) -> Option<&str> {
        self.identifier.as_deref()
    }

    pub fn non_removable(mut self) -> Self {
        self.removable = false;
        self
    }

    /// Pin this chip to the left zone in a `LeftRightZones` layout: it may be
    /// reordered within the left zone but can never be dragged to the right.
    pub fn left_pinned(mut self) -> Self {
        self.left_pinned = true;
        self
    }

    fn render_remove_button<A: Action + Clone>(
        &self,
        action: A,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let icon_size = appearance.monospace_font_size();
        let button = Hoverable::new(self.remove_button_state_handle.clone(), |_| {
            ConstrainedBox::new(
                icons::Icon::X
                    .to_warpui_icon(appearance.theme().ui_error_color().into())
                    .finish(),
            )
            .with_height(icon_size)
            .with_width(icon_size)
            .finish()
        });
        button
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
            .with_cursor(Cursor::PointingHand)
            .finish()
    }

    pub(crate) fn display_label(&self) -> std::borrow::Cow<'_, str> {
        if let Some(label) = &self.custom_label {
            std::borrow::Cow::Borrowed(label.as_str())
        } else if let Some(kind) = &self.kind {
            kind.display_label()
        } else {
            std::borrow::Cow::Borrowed("Unknown")
        }
    }

    fn display_icon(&self) -> Option<crate::ui_components::icons::Icon> {
        if let Some(icon) = self.custom_icon {
            Some(icon)
        } else {
            self.kind.as_ref().and_then(|k| k.icon())
        }
    }

    fn render_internal(
        &self,
        drag_state: ChipDragState,
        remove_button: Option<Box<dyn Element>>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let font_size = udi_font_size(appearance);
        let label = self.display_label().to_string();
        let icon = self.display_icon();
        let is_dragging = matches!(drag_state, ChipDragState::Draggable { is_dragging: true });
        let mut hoverable = Hoverable::new(self.tooltip_state_handle.clone(), move |mouse_state| {
            let show_hover = mouse_state.is_hovered() && !is_dragging;
            let background = if show_hover {
                appearance.theme().surface_2()
            } else {
                appearance.theme().surface_1()
            };
            let color = appearance.theme().sub_text_color(background).into_solid();

            let mut content = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

            if let Some(icon) = icon {
                content.add_child(
                    Container::new(
                        ConstrainedBox::new(icon.to_warpui_icon(Fill::Solid(color)).finish())
                            .with_height(font_size)
                            .with_width(font_size)
                            .finish(),
                    )
                    .with_margin_right(spacing::UDI_CHIP_ICON_GAP)
                    .finish(),
                );
            }
            let text = Text::new_inline(label.clone(), appearance.ui_font_family(), font_size)
                .with_color(color)
                .with_line_height_ratio(appearance.line_height_ratio())
                .with_style(Properties::default())
                .finish();
            content.add_child(text);

            if let Some(remove_button) = remove_button {
                content.add_child(
                    Container::new(remove_button)
                        .with_margin_left(spacing::UDI_CHIP_ICON_GAP)
                        .finish(),
                );
            }

            let button = chip_container(content.finish(), None, appearance)
                .with_background(background)
                .finish();
            if !show_hover {
                return button;
            }

            let tooltip = appearance.ui_builder().tool_tip(label.clone());
            let mut stack = Stack::new();
            stack.add_child(button);
            stack.add_positioned_overlay_child(
                tooltip.build().finish(),
                OffsetPositioning::offset_from_parent(
                    vec2f(0., -2.5 * font_size),
                    ParentOffsetBounds::Unbounded,
                    warpui::elements::ParentAnchor::Center,
                    warpui::elements::ChildAnchor::Center,
                ),
            );
            stack.finish()
        });

        if matches!(drag_state, ChipDragState::Draggable { .. }) {
            hoverable = hoverable.with_cursor(Cursor::OpenHand);
        }

        hoverable.finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChipLocation {
    Used { index: usize },
    Left { index: usize },
    Right { index: usize },
    Unused { index: usize },
}

impl ChipLocation {
    pub fn index(&self) -> usize {
        match self {
            Self::Used { index }
            | Self::Left { index }
            | Self::Right { index }
            | Self::Unused { index } => *index,
        }
    }

    pub fn in_same_area_as(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Self::Used { .. }, Self::Used { .. })
                | (Self::Left { .. }, Self::Left { .. })
                | (Self::Right { .. }, Self::Right { .. })
                | (Self::Unused { .. }, Self::Unused { .. })
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CurrentDraggingState {
    pub original_location: ChipLocation,
    pub current_location: ChipLocation,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ChipConfiguratorAction {
    StartDraggingChip { location: ChipLocation },
    DragChip { current_position: RectF },
    DropChip { position: RectF },
    RemoveFromUsed { location: ChipLocation },
    AddFromUnused { index: usize },
    ToggleAutoSend { location: ChipLocation },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChipConfiguratorLayout {
    SingleZone,
    LeftRightZones,
}

pub struct ChipConfigurator {
    layout: ChipConfiguratorLayout,

    pub used_chips: Vec<ConfigurableItem>,
    pub left_chips: Vec<ConfigurableItem>,
    pub right_chips: Vec<ConfigurableItem>,
    pub unused_chips: Vec<ConfigurableItem>,
    pub current_dragging_state: Option<CurrentDraggingState>,
    interaction_mouse_states: RefCell<HashMap<String, MouseStateHandle>>,
    list_presentation: Option<ListPresentation>,
    list_scroll_state: ClippedScrollStateHandle,
}

impl ChipConfigurator {
    pub fn new(layout: ChipConfiguratorLayout) -> Self {
        Self {
            layout,
            used_chips: vec![],
            left_chips: vec![],
            right_chips: vec![],
            unused_chips: vec![],
            current_dragging_state: None,
            interaction_mouse_states: RefCell::new(HashMap::new()),
            list_presentation: None,
            list_scroll_state: Default::default(),
        }
    }

    pub fn list_presentation(&self) -> Option<ListPresentation> {
        self.list_presentation
    }

    pub fn layout(&self) -> ChipConfiguratorLayout {
        self.layout
    }

    /// Initialize for `SingleZone` layout with pre-built context chip renderers.
    pub fn open_single_zone_with_renderers(
        &mut self,
        used_chips: Vec<ContextChipRenderer>,
        unused_chips: Vec<ContextChipRenderer>,
    ) {
        self.layout = ChipConfiguratorLayout::SingleZone;
        self.list_presentation = None;
        self.reset();
        self.used_chips = used_chips
            .into_iter()
            .map(|r| ConfigurableItem::ContextChip(Box::new(r)))
            .collect();
        self.unused_chips = unused_chips
            .into_iter()
            .map(|r| ConfigurableItem::ContextChip(Box::new(r)))
            .collect();
    }

    /// Initialize a `SingleZone` layout shown as an ordered list beside the available items.
    pub fn open_list_with_items(
        &mut self,
        used_items: Vec<AgentToolbarItemKind>,
        available: Vec<AgentToolbarItemKind>,
        presentation: ListPresentation,
        appearance: &Appearance,
    ) {
        self.open_single_zone_with_items(used_items, available, appearance);
        self.list_presentation = Some(presentation);
    }

    /// Initialize for `SingleZone` layout with toolbar items.
    pub fn open_single_zone_with_items(
        &mut self,
        used_items: Vec<AgentToolbarItemKind>,
        available: Vec<AgentToolbarItemKind>,
        appearance: &Appearance,
    ) {
        self.layout = ChipConfiguratorLayout::SingleZone;
        self.list_presentation = None;
        self.reset();
        self.used_chips = used_items
            .iter()
            .filter_map(|kind| ConfigurableItem::from_toolbar_item(kind.clone(), appearance))
            .collect();
        self.unused_chips = available
            .into_iter()
            .filter_map(|kind| {
                (!used_items
                    .iter()
                    .any(|used| used.has_same_toolbar_identity(&kind)))
                .then(|| ConfigurableItem::from_toolbar_item(kind, appearance))
                .flatten()
            })
            .collect();
    }

    /// Initialize for `LeftRightZones` layout with `AgentToolbarItemKind` lists.
    pub fn open_left_right_zones_with_items(
        &mut self,
        left_items: Vec<AgentToolbarItemKind>,
        right_items: Vec<AgentToolbarItemKind>,
        available: Vec<AgentToolbarItemKind>,
        appearance: &Appearance,
    ) {
        self.layout = ChipConfiguratorLayout::LeftRightZones;
        self.list_presentation = None;
        self.reset();
        self.left_chips = left_items
            .iter()
            .filter_map(|kind| ConfigurableItem::from_toolbar_item(kind.clone(), appearance))
            .collect();
        self.right_chips = right_items
            .iter()
            .filter_map(|kind| ConfigurableItem::from_toolbar_item(kind.clone(), appearance))
            .collect();
        let used_set: Vec<_> = left_items
            .iter()
            .chain(right_items.iter())
            .cloned()
            .collect();
        self.unused_chips = available
            .into_iter()
            .filter_map(|kind| {
                (!used_set
                    .iter()
                    .any(|used| used.has_same_toolbar_identity(&kind)))
                .then(|| ConfigurableItem::from_toolbar_item(kind, appearance))
                .flatten()
            })
            .collect();
    }

    pub fn reset(&mut self) {
        self.used_chips.clear();
        self.left_chips.clear();
        self.right_chips.clear();
        self.unused_chips.clear();
        self.current_dragging_state = None;
        self.interaction_mouse_states.borrow_mut().clear();
    }

    pub fn used_item_kinds(&self) -> Vec<AgentToolbarItemKind> {
        self.used_chips
            .iter()
            .filter_map(|r| r.item_kind())
            .collect()
    }

    pub fn left_item_kinds(&self) -> Vec<AgentToolbarItemKind> {
        self.left_chips
            .iter()
            .filter_map(|r| r.item_kind())
            .collect()
    }

    pub fn right_item_kinds(&self) -> Vec<AgentToolbarItemKind> {
        self.right_chips
            .iter()
            .filter_map(|r| r.item_kind())
            .collect()
    }

    pub fn unused_item_kinds(&self) -> Vec<AgentToolbarItemKind> {
        self.unused_chips
            .iter()
            .filter_map(|r| r.item_kind())
            .collect()
    }

    pub fn item_kind_at(&self, location: ChipLocation) -> Option<AgentToolbarItemKind> {
        self.chips_for_location(location)
            .get(location.index())
            .and_then(ConfigurableItem::item_kind)
    }

    pub fn replace_item_kind_at(
        &mut self,
        location: ChipLocation,
        item: AgentToolbarItemKind,
        appearance: &Appearance,
    ) -> bool {
        let index = location.index();
        let Some(replacement) = ConfigurableItem::from_toolbar_item(item, appearance) else {
            return false;
        };
        let items = self.chips_for_location_mut(location);
        let Some(current) = items.get_mut(index) else {
            return false;
        };
        *current = replacement;
        true
    }

    pub fn handle_action<V: View>(
        &mut self,
        action: &ChipConfiguratorAction,
        ctx: &mut ViewContext<V>,
    ) -> bool {
        match action {
            ChipConfiguratorAction::StartDraggingChip { location } => {
                self.current_dragging_state = Some(CurrentDraggingState {
                    original_location: *location,
                    current_location: *location,
                });
                false
            }
            ChipConfiguratorAction::DragChip { current_position } => {
                self.handle_chip_dragged(*current_position, ctx);
                false
            }
            ChipConfiguratorAction::DropChip { position } => {
                self.handle_chip_dropped(*position, ctx);
                true
            }
            ChipConfiguratorAction::RemoveFromUsed { location } => {
                if !self.is_chip_at_location_removable(*location) {
                    return false;
                }
                let unused_len = self.unused_chips.len();
                self.remove_and_insert_chip_at_location(
                    *location,
                    ChipLocation::Unused { index: unused_len },
                );
                true
            }
            ChipConfiguratorAction::AddFromUnused { index } => {
                if *index >= self.unused_chips.len() {
                    return false;
                }
                let destination = match self.layout {
                    ChipConfiguratorLayout::SingleZone => ChipLocation::Used {
                        index: self.used_chips.len(),
                    },
                    ChipConfiguratorLayout::LeftRightZones => ChipLocation::Left {
                        index: self.left_chips.len(),
                    },
                };
                self.remove_and_insert_chip_at_location(
                    ChipLocation::Unused { index: *index },
                    destination,
                );
                true
            }
            ChipConfiguratorAction::ToggleAutoSend { location } => {
                let Some(item) = self.item_kind_at(*location) else {
                    return false;
                };
                let Some(auto_send) = item.auto_send_behavior() else {
                    return false;
                };
                let Some(replacement) = item.with_auto_send_behavior(!auto_send) else {
                    return false;
                };
                self.replace_item_kind_at(*location, replacement, Appearance::as_ref(ctx))
            }
        }
    }

    fn is_chip_at_location_removable(&self, location: ChipLocation) -> bool {
        let chips = self.chips_for_location(location);
        let index = location.index();
        if index >= chips.len() {
            return true;
        }
        chips[index].is_removable()
    }

    fn is_chip_at_location_left_pinned(&self, location: ChipLocation) -> bool {
        let chips = self.chips_for_location(location);
        let index = location.index();
        if index >= chips.len() {
            return false;
        }
        chips[index].is_left_pinned()
    }

    fn chips_for_location(&self, location: ChipLocation) -> &Vec<ConfigurableItem> {
        match location {
            ChipLocation::Used { .. } => &self.used_chips,
            ChipLocation::Left { .. } => &self.left_chips,
            ChipLocation::Right { .. } => &self.right_chips,
            ChipLocation::Unused { .. } => &self.unused_chips,
        }
    }

    fn chips_for_location_mut(&mut self, location: ChipLocation) -> &mut Vec<ConfigurableItem> {
        match location {
            ChipLocation::Used { .. } => &mut self.used_chips,
            ChipLocation::Left { .. } => &mut self.left_chips,
            ChipLocation::Right { .. } => &mut self.right_chips,
            ChipLocation::Unused { .. } => &mut self.unused_chips,
        }
    }

    fn remove_and_insert_chip_at_location(&mut self, from: ChipLocation, to: ChipLocation) {
        let from_vec = self.chips_for_location(from);
        if from_vec.is_empty() {
            return;
        }
        let index = from.index().min(from_vec.len() - 1);

        if from.in_same_area_as(&to) {
            let vec = self.chips_for_location_mut(from);
            let removed = vec.remove(index);
            let to_index = to.index().min(vec.len());
            vec.insert(to_index, removed);
        } else {
            let from_vec = self.chips_for_location_mut(from);
            let removed = from_vec.remove(index);
            let to_vec = self.chips_for_location_mut(to);
            let to_index = to.index().min(to_vec.len());
            to_vec.insert(to_index, removed);
        }
    }

    fn chip_save_position_id(location: ChipLocation) -> String {
        let area = match location {
            ChipLocation::Used { .. } => "u",
            ChipLocation::Left { .. } => "l",
            ChipLocation::Right { .. } => "r",
            ChipLocation::Unused { .. } => "n",
        };
        format!("chip_cfg_{area}_{}", location.index())
    }

    fn drop_target_position_id(location: ChipLocation) -> &'static str {
        match location {
            ChipLocation::Used { .. } => USED_CHIPS_POSITION_ID,
            ChipLocation::Left { .. } => LEFT_CHIPS_POSITION_ID,
            ChipLocation::Right { .. } => RIGHT_CHIPS_POSITION_ID,
            ChipLocation::Unused { .. } => UNUSED_CHIPS_POSITION_ID,
        }
    }

    fn find_new_location_for_index<V: View>(
        &self,
        current_location: ChipLocation,
        dragged_position: RectF,
        index: usize,
        is_last: bool,
        chip_index_to_location: &dyn Fn(usize) -> ChipLocation,
        ctx: &mut ViewContext<V>,
    ) -> Option<ChipLocation> {
        let chip_location = chip_index_to_location(index);
        let chip_position_id = Self::chip_save_position_id(chip_location);
        let chip_position = ctx.element_position_by_id(chip_position_id)?;

        let dragged_at_adequate_height = chip_position.min_y() <= dragged_position.center().y()
            && dragged_position.center().y() <= chip_position.max_y();
        if !dragged_at_adequate_height {
            return None;
        }

        let is_to_the_right_of_last_chip =
            is_last && dragged_position.max_x() > chip_position.max_x();
        if is_to_the_right_of_last_chip {
            return Some(chip_index_to_location(index + 1));
        }

        if !chip_position.intersects(dragged_position) {
            return None;
        }

        let should_compare_index = current_location.in_same_area_as(&chip_location);
        let dragged_before = dragged_position.min_x() < chip_position.min_x()
            && (!should_compare_index || current_location.index() > index);
        let dragged_after = dragged_position.max_x() > chip_position.max_x()
            && (!should_compare_index || current_location.index() < index);

        if dragged_before || dragged_after {
            return Some(chip_location);
        }

        None
    }

    fn find_new_location_in_zone<V: View>(
        &self,
        current_location: ChipLocation,
        dragged_position: RectF,
        chips_count: usize,
        chip_index_to_location: &dyn Fn(usize) -> ChipLocation,
        ctx: &mut ViewContext<V>,
    ) -> Option<ChipLocation> {
        if chips_count == 0 {
            let target_id = Self::drop_target_position_id(chip_index_to_location(0));
            if let Some(target_pos) = ctx.element_position_by_id(target_id) {
                if target_pos.intersects(dragged_position) {
                    return Some(chip_index_to_location(0));
                }
            }
            return None;
        }

        for index in 0..chips_count {
            let is_last = index == chips_count - 1;
            if let Some(new_location) = self.find_new_location_for_index(
                current_location,
                dragged_position,
                index,
                is_last,
                chip_index_to_location,
                ctx,
            ) {
                // The "past the last chip" heuristic matches any chip dragged
                // beyond the final item in this zone. When adjacent zones share
                // a Y range (e.g. left/right drop zones on the same row), that
                // heuristic false-positives for chips that have left this zone.
                // Verify the dragged center is still within the zone's drop
                // target before accepting an append match.
                if new_location.index() >= chips_count {
                    let target_id = Self::drop_target_position_id(new_location);
                    if let Some(target_pos) = ctx.element_position_by_id(target_id) {
                        let center_x = dragged_position.center().x();
                        if center_x < target_pos.min_x() || center_x > target_pos.max_x() {
                            return None;
                        }
                    }
                }
                return Some(new_location);
            }
        }

        None
    }

    fn find_in_used<V: View>(
        &self,
        location: ChipLocation,
        dragged_position: RectF,
        ctx: &mut ViewContext<V>,
    ) -> Option<ChipLocation> {
        self.find_new_location_in_zone(
            location,
            dragged_position,
            self.used_chips.len(),
            &|i| ChipLocation::Used { index: i },
            ctx,
        )
    }

    fn find_in_left<V: View>(
        &self,
        location: ChipLocation,
        dragged_position: RectF,
        ctx: &mut ViewContext<V>,
    ) -> Option<ChipLocation> {
        self.find_new_location_in_zone(
            location,
            dragged_position,
            self.left_chips.len(),
            &|i| ChipLocation::Left { index: i },
            ctx,
        )
    }

    fn find_in_right<V: View>(
        &self,
        location: ChipLocation,
        dragged_position: RectF,
        ctx: &mut ViewContext<V>,
    ) -> Option<ChipLocation> {
        self.find_new_location_in_zone(
            location,
            dragged_position,
            self.right_chips.len(),
            &|i| ChipLocation::Right { index: i },
            ctx,
        )
    }

    fn find_in_unused<V: View>(
        &self,
        location: ChipLocation,
        dragged_position: RectF,
        ctx: &mut ViewContext<V>,
    ) -> Option<ChipLocation> {
        self.find_new_location_in_zone(
            location,
            dragged_position,
            self.unused_chips.len(),
            &|i| ChipLocation::Unused { index: i },
            ctx,
        )
    }

    fn handle_chip_dragged<V: View>(&mut self, dragged_position: RectF, ctx: &mut ViewContext<V>) {
        // Use the tracked current location rather than the closure-captured location,
        // which may be stale after chips have been reordered mid-drag.
        let location = match self.current_dragging_state {
            Some(state) => state.current_location,
            None => return,
        };
        if self.list_presentation.is_some() {
            self.handle_list_row_dragged(location, dragged_position, ctx);
            return;
        }

        // Scan the current zone first so that within-zone reordering always
        // takes priority over cross-zone moves. Without this, a chip near
        // the boundary between two zones could match in the wrong zone first,
        // and the cross-zone suppression guard would then block the move
        // entirely — preventing the within-zone reorder from ever running.
        let new_location = match self.layout {
            ChipConfiguratorLayout::SingleZone => match location {
                ChipLocation::Unused { .. } => self
                    .find_in_unused(location, dragged_position, ctx)
                    .or_else(|| self.find_in_used(location, dragged_position, ctx)),
                _ => self
                    .find_in_used(location, dragged_position, ctx)
                    .or_else(|| self.find_in_unused(location, dragged_position, ctx)),
            },
            ChipConfiguratorLayout::LeftRightZones => match location {
                ChipLocation::Right { .. } => self
                    .find_in_right(location, dragged_position, ctx)
                    .or_else(|| self.find_in_left(location, dragged_position, ctx))
                    .or_else(|| self.find_in_unused(location, dragged_position, ctx)),
                ChipLocation::Unused { .. } => self
                    .find_in_unused(location, dragged_position, ctx)
                    .or_else(|| self.find_in_left(location, dragged_position, ctx))
                    .or_else(|| self.find_in_right(location, dragged_position, ctx)),
                _ => self
                    .find_in_left(location, dragged_position, ctx)
                    .or_else(|| self.find_in_right(location, dragged_position, ctx))
                    .or_else(|| self.find_in_unused(location, dragged_position, ctx)),
            },
        };

        if let Some(new_location) = new_location {
            // Don't allow non-removable items to be dragged to the unused zone.
            if matches!(new_location, ChipLocation::Unused { .. })
                && !self.is_chip_at_location_removable(location)
            {
                return;
            }

            // Don't allow left-pinned items (e.g. the tabs panel, locked to the
            // left in Clinch) to be dragged into the right zone.
            if matches!(new_location, ChipLocation::Right { .. })
                && self.is_chip_at_location_left_pinned(location)
            {
                return;
            }

            // Suppress cross-zone moves when the dragged center is still inside
            // the current zone's drop target. This prevents oscillation when
            // adding a chip to a zone causes a row wrap: the chip's layout
            // position lands on row 2, fails the height check on the next frame,
            // and the code erroneously moves it to an adjacent zone.
            //
            // We use center-point containment rather than rect intersection so
            // that horizontally-adjacent zones (left/right) allow moves once the
            // user's drag center crosses the boundary, even though the chip rect
            // still partially overlaps the old zone.
            if !new_location.in_same_area_as(&location) {
                let current_zone_id = Self::drop_target_position_id(location);
                if let Some(target_pos) = ctx.element_position_by_id(current_zone_id) {
                    let center = dragged_position.center();
                    let center_in_zone = center.x() >= target_pos.min_x()
                        && center.x() <= target_pos.max_x()
                        && center.y() >= target_pos.min_y()
                        && center.y() <= target_pos.max_y();
                    if center_in_zone {
                        return;
                    }
                }
            }

            self.remove_and_insert_chip_at_location(location, new_location);
            if let Some(state) = self.current_dragging_state.as_mut() {
                state.current_location = new_location;
            }
        }
    }

    /// The column under the pointer decides the list; the rows' middles decide the place in it.
    fn handle_list_row_dragged<V: View>(
        &mut self,
        location: ChipLocation,
        dragged_position: RectF,
        ctx: &mut ViewContext<V>,
    ) {
        let pointer = dragged_position.center();
        let column_holds_pointer = |id: &str| {
            ctx.element_position_by_id(id)
                .is_some_and(|rect| rect.min_x() <= pointer.x() && pointer.x() <= rect.max_x())
        };
        let to_location: fn(usize) -> ChipLocation = if column_holds_pointer(USED_CHIPS_POSITION_ID)
        {
            |index| ChipLocation::Used { index }
        } else if column_holds_pointer(UNUSED_CHIPS_POSITION_ID) {
            |index| ChipLocation::Unused { index }
        } else {
            return;
        };
        let target_list = to_location(0);
        let Some(slot_middles) = (0..self.chips_for_location(target_list).len())
            .map(|slot| {
                ctx.element_position_by_id(Self::chip_save_position_id(to_location(slot)))
                    .map(|rect| rect.center().y())
            })
            .collect::<Option<Vec<_>>>()
        else {
            // A row that was just added has not been laid out yet; the next move catches up.
            return;
        };
        let dragged_slot = location
            .in_same_area_as(&target_list)
            .then(|| location.index());
        let new_location = to_location(list_insertion_index(
            &slot_middles,
            dragged_slot,
            pointer.y(),
        ));
        if new_location == location
            || (matches!(new_location, ChipLocation::Unused { .. })
                && !self.is_chip_at_location_removable(location))
        {
            return;
        }
        self.remove_and_insert_chip_at_location(location, new_location);
        if let Some(state) = self.current_dragging_state.as_mut() {
            state.current_location = new_location;
        }
    }

    fn handle_chip_dropped<V: View>(&mut self, drop_position: RectF, ctx: &mut ViewContext<V>) {
        if let Some(state) = &self.current_dragging_state {
            let expected = state.current_location;
            let target_id = Self::drop_target_position_id(expected);
            if let Some(target_pos) = ctx.element_position_by_id(target_id) {
                if !target_pos.intersects(drop_position) {
                    self.remove_and_insert_chip_at_location(
                        state.current_location,
                        state.original_location,
                    );
                }
            }
        }
        self.current_dragging_state = None;
    }

    pub fn render_draggable_chip<A: Action + Clone + 'static>(
        &self,
        location: ChipLocation,
        chip: Box<dyn Element>,
        item: &ConfigurableItem,
        on_click_action: A,
        wrap_chip_action: fn(ChipConfiguratorAction) -> A,
    ) -> Box<dyn Element> {
        let clickable = EventHandler::new(chip)
            .on_left_mouse_down(move |ctx, _, _| {
                ctx.dispatch_typed_action(on_click_action.clone());
                DispatchEventResult::PropagateToParent
            })
            .finish();

        let start_loc = location;

        SavePosition::new(
            Draggable::new(item.draggable_state(), clickable)
                .on_drag_start(move |ctx, _, _| {
                    ctx.dispatch_typed_action(wrap_chip_action(
                        ChipConfiguratorAction::StartDraggingChip {
                            location: start_loc,
                        },
                    ));
                })
                .on_drag(move |ctx, _, rect, _| {
                    ctx.dispatch_typed_action(wrap_chip_action(ChipConfiguratorAction::DragChip {
                        current_position: rect,
                    }));
                })
                .on_drop(move |ctx, _, position, _| {
                    ctx.dispatch_typed_action(wrap_chip_action(ChipConfiguratorAction::DropChip {
                        position,
                    }));
                })
                .finish(),
            &Self::chip_save_position_id(location),
        )
        .finish()
    }

    fn interaction_mouse_state(
        &self,
        location: ChipLocation,
        purpose: &'static str,
    ) -> MouseStateHandle {
        self.interaction_mouse_states
            .borrow_mut()
            .entry(format!("{purpose}:{location:?}"))
            .or_default()
            .clone()
    }

    fn render_auxiliary_button<A: Action + Clone + 'static>(
        &self,
        label: String,
        location: ChipLocation,
        purpose: &'static str,
        action: A,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        appearance
            .ui_builder()
            .button(
                ButtonVariant::Secondary,
                self.interaction_mouse_state(location, purpose),
            )
            .with_text_label(label)
            .with_style(UiComponentStyles {
                font_size: Some(udi_font_size(appearance)),
                padding: Some(Coords::default().top(3.).bottom(3.).left(7.).right(7.)),
                ..Default::default()
            })
            .build()
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
            .finish()
    }

    fn render_item_with_controls<A: Action + Clone + 'static>(
        &self,
        location: ChipLocation,
        rendered_chip: Box<dyn Element>,
        is_used: bool,
        wrap_chip_action: fn(ChipConfiguratorAction) -> A,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(rendered_chip)
            .with_spacing(4.);

        if !is_used {
            row.add_child(self.render_auxiliary_button(
                "+".to_owned(),
                location,
                "add",
                wrap_chip_action(ChipConfiguratorAction::AddFromUnused {
                    index: location.index(),
                }),
                appearance,
            ));
        }

        row.finish()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render_wrappable_chips<'a, A: Action + Clone + 'static>(
        &self,
        chips: impl Iterator<Item = &'a ConfigurableItem>,
        location_fn: impl Fn(usize) -> ChipLocation,
        is_used: bool,
        on_click_action: A,
        on_remove_action_fn: impl Fn(usize) -> A,
        wrap_chip_action: fn(ChipConfiguratorAction) -> A,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let drag_state = ChipDragState::Draggable {
            is_dragging: self.current_dragging_state.is_some(),
        };
        Wrap::row()
            .with_children(chips.enumerate().map(|(index, item)| {
                let location = location_fn(index);
                let rendered_chip = if is_used {
                    item.render_used(drag_state, on_remove_action_fn(index), appearance)
                } else {
                    item.render_unused(drag_state, appearance)
                };
                let draggable = self.render_draggable_chip(
                    location,
                    rendered_chip,
                    item,
                    on_click_action.clone(),
                    wrap_chip_action,
                );
                Container::new(self.render_item_with_controls(
                    location,
                    draggable,
                    is_used,
                    wrap_chip_action,
                    appearance,
                ))
                .with_horizontal_margin(4.)
                .finish()
            }))
            .with_run_spacing(8.)
            .finish()
    }

    /// The footer's buttons in order beside the ones that can be added. A line marks where the
    /// footer runs out of room and the rest open from More.
    pub fn render_list_columns<A: Action + Clone + 'static>(
        &self,
        presentation: ListPresentation,
        on_click_action: A,
        wrap_chip_action: fn(ChipConfiguratorAction) -> A,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let tokens = *appearance.tokens();
        let font_family = appearance.ui_font_family();

        let mut footer = Flex::column()
            .with_spacing(LIST_ROW_GAP)
            .with_child(render_list_caption("On the footer", font_family, &tokens));
        for (index, item) in self.used_chips.iter().enumerate() {
            if index == presentation.visible_count {
                footer.add_child(render_more_line(font_family, &tokens));
            }
            let location = ChipLocation::Used { index };
            footer.add_child(self.render_draggable_chip(
                location,
                self.render_list_row(
                    location,
                    item,
                    index >= presentation.visible_count,
                    wrap_chip_action,
                    appearance,
                ),
                item,
                on_click_action.clone(),
                wrap_chip_action,
            ));
        }
        if self.used_chips.is_empty() {
            footer.add_child(render_list_hint(
                "Drag buttons here, or press + beside one.",
                font_family,
                &tokens,
            ));
        }

        let mut available = Flex::column()
            .with_spacing(LIST_ROW_GAP)
            .with_child(render_list_caption("Available", font_family, &tokens));
        for (index, item) in self.unused_chips.iter().enumerate() {
            let location = ChipLocation::Unused { index };
            available.add_child(self.render_draggable_chip(
                location,
                self.render_list_row(location, item, false, wrap_chip_action, appearance),
                item,
                on_click_action.clone(),
                wrap_chip_action,
            ));
        }
        if self.unused_chips.is_empty() {
            available.add_child(render_list_hint(
                "Everything is on the footer.",
                font_family,
                &tokens,
            ));
        }

        let columns = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(16.)
            .with_child(
                Expanded::new(
                    3.,
                    SavePosition::new(footer.finish(), USED_CHIPS_POSITION_ID).finish(),
                )
                .finish(),
            )
            .with_child(
                Expanded::new(
                    2.,
                    SavePosition::new(available.finish(), UNUSED_CHIPS_POSITION_ID).finish(),
                )
                .finish(),
            )
            .finish();

        ConstrainedBox::new(
            ClippedScrollable::vertical(
                self.list_scroll_state.clone(),
                Container::new(columns)
                    .with_padding_right(LIST_SCROLLBAR_GUTTER)
                    .finish(),
                ScrollbarWidth::Auto,
                warpui::elements::Fill::Solid(tokens.control_track),
                warpui::elements::Fill::Solid(tokens.text_3),
                warpui::elements::Fill::None,
            )
            .finish(),
        )
        .with_max_height(LIST_MAX_HEIGHT)
        .finish()
    }

    /// One button per row: grip, icon and label, then whatever applies to it. A footer row can
    /// switch between sending and only inserting, and be removed; an available row can be added.
    fn render_list_row<A: Action + Clone + 'static>(
        &self,
        location: ChipLocation,
        item: &ConfigurableItem,
        in_more_menu: bool,
        wrap_chip_action: fn(ChipConfiguratorAction) -> A,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let tokens = *appearance.tokens();
        let font_family = appearance.ui_font_family();
        let ui_builder = appearance.ui_builder().clone();
        let is_used = matches!(location, ChipLocation::Used { .. });
        let label = item.display_label();
        let icon = item.display_icon();
        let auto_send = is_used
            .then(|| item.item_kind())
            .flatten()
            .and_then(|kind| kind.auto_send_behavior());
        let removable = is_used && item.is_removable();
        let is_dragging = self.current_dragging_state.is_some();
        let send_state = self.interaction_mouse_state(location, "auto_send");
        let trailing_state = self.interaction_mouse_state(location, "trailing");
        let toggle_action = wrap_chip_action(ChipConfiguratorAction::ToggleAutoSend { location });
        let trailing_action = if is_used {
            wrap_chip_action(ChipConfiguratorAction::RemoveFromUsed { location })
        } else {
            wrap_chip_action(ChipConfiguratorAction::AddFromUnused {
                index: location.index(),
            })
        };

        Hoverable::new(
            self.interaction_mouse_state(location, "row"),
            move |row_state| {
                let hovered = row_state.is_hovered() && !is_dragging;
                let label_color = if in_more_menu {
                    tokens.text_2
                } else {
                    tokens.text_1
                };
                let mut row = Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(8.)
                    .with_child(render_list_icon(
                        icons::Icon::DragIndicatorVertical,
                        tokens.text_3,
                    ));
                if let Some(icon) = icon {
                    row.add_child(render_list_icon(icon, tokens.text_2));
                }
                row.add_child(
                    Expanded::new(
                        1.,
                        Text::new_inline(label.clone(), font_family, LIST_FONT_SIZE)
                            .with_color(label_color)
                            .with_clip(ClipConfig::ellipsis())
                            .finish(),
                    )
                    .finish(),
                );
                if let Some(auto_send) = auto_send {
                    row.add_child(render_send_toggle(
                        auto_send,
                        send_state.clone(),
                        toggle_action.clone(),
                        &tokens,
                        font_family,
                        &ui_builder,
                    ));
                }
                if !is_used || removable {
                    let icon = if is_used {
                        icons::Icon::X
                    } else {
                        icons::Icon::Plus
                    };
                    let action = trailing_action.clone();
                    row.add_child(
                        Hoverable::new(trailing_state.clone(), move |state| {
                            let hovered = state.is_hovered();
                            Container::new(render_list_icon(
                                icon,
                                if hovered {
                                    tokens.text_1
                                } else {
                                    tokens.text_3
                                },
                            ))
                            .with_uniform_padding(4.)
                            .with_background(Fill::Solid(if hovered {
                                tokens.surface_hover
                            } else {
                                ColorU::transparent_black()
                            }))
                            .with_corner_radius(corners_sm())
                            .finish()
                        })
                        .with_cursor(Cursor::PointingHand)
                        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
                        .finish(),
                    );
                }
                Container::new(
                    ConstrainedBox::new(row.finish())
                        .with_height(LIST_ROW_HEIGHT)
                        .finish(),
                )
                .with_horizontal_padding(8.)
                .with_background(Fill::Solid(if hovered {
                    tokens.surface_raised
                } else {
                    tokens.surface_field
                }))
                .with_corner_radius(corners_sm())
                .finish()
            },
        )
        .with_cursor(Cursor::OpenHand)
        .finish()
    }

    pub fn render_unused_chips_bank<A: Action + Clone + 'static>(
        &self,
        on_click_action: A,
        wrap_chip_action: fn(ChipConfiguratorAction) -> A,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let dummy = on_click_action.clone();
        SavePosition::new(
            self.render_wrappable_chips(
                self.unused_chips.iter(),
                |i| ChipLocation::Unused { index: i },
                false,
                on_click_action,
                move |_| dummy.clone(),
                wrap_chip_action,
                appearance,
            ),
            UNUSED_CHIPS_POSITION_ID,
        )
        .finish()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render_drop_zone<A: Action + Clone + 'static>(
        &self,
        chips: &[ConfigurableItem],
        location_fn: impl Fn(usize) -> ChipLocation,
        remove_location_fn: impl Fn(usize) -> ChipLocation,
        position_id: &str,
        on_click_action: A,
        wrap_chip_action: fn(ChipConfiguratorAction) -> A,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let drag_state = ChipDragState::Draggable {
            is_dragging: self.current_dragging_state.is_some(),
        };

        let inner = if chips.is_empty() {
            Empty::new().finish()
        } else {
            Wrap::row()
                .with_children(chips.iter().enumerate().map(|(index, item)| {
                    let location = location_fn(index);
                    let remove_loc = remove_location_fn(index);
                    let rendered = item.render_used(
                        drag_state,
                        wrap_chip_action(ChipConfiguratorAction::RemoveFromUsed {
                            location: remove_loc,
                        }),
                        appearance,
                    );
                    let draggable = self.render_draggable_chip(
                        location,
                        rendered,
                        item,
                        on_click_action.clone(),
                        wrap_chip_action,
                    );
                    Container::new(self.render_item_with_controls(
                        location,
                        draggable,
                        true,
                        wrap_chip_action,
                        appearance,
                    ))
                    .with_horizontal_margin(4.)
                    .finish()
                }))
                .with_run_spacing(8.)
                .finish()
        };

        SavePosition::new(
            Container::new(
                ConstrainedBox::new(inner)
                    .with_min_height(appearance.monospace_font_size() * 2.5)
                    .finish(),
            )
            .with_uniform_padding(8.)
            .with_background(appearance.theme().surface_1())
            .with_border(
                Border::all(2.)
                    .with_border_fill(appearance.theme().outline())
                    .with_dashed_border(Dash {
                        dash_length: 8.,
                        gap_length: 8.,
                        ..Default::default()
                    }),
            )
            .finish(),
            position_id,
        )
        .finish()
    }

    pub fn render_used_drop_zone<A: Action + Clone + 'static>(
        &self,
        on_click_action: A,
        wrap_chip_action: fn(ChipConfiguratorAction) -> A,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        self.render_drop_zone(
            &self.used_chips,
            |i| ChipLocation::Used { index: i },
            |i| ChipLocation::Used { index: i },
            USED_CHIPS_POSITION_ID,
            on_click_action,
            wrap_chip_action,
            appearance,
        )
    }

    pub fn render_left_drop_zone<A: Action + Clone + 'static>(
        &self,
        on_click_action: A,
        wrap_chip_action: fn(ChipConfiguratorAction) -> A,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        self.render_drop_zone(
            &self.left_chips,
            |i| ChipLocation::Left { index: i },
            |i| ChipLocation::Left { index: i },
            LEFT_CHIPS_POSITION_ID,
            on_click_action,
            wrap_chip_action,
            appearance,
        )
    }

    pub fn render_right_drop_zone<A: Action + Clone + 'static>(
        &self,
        on_click_action: A,
        wrap_chip_action: fn(ChipConfiguratorAction) -> A,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        self.render_drop_zone(
            &self.right_chips,
            |i| ChipLocation::Right { index: i },
            |i| ChipLocation::Right { index: i },
            RIGHT_CHIPS_POSITION_ID,
            on_click_action,
            wrap_chip_action,
            appearance,
        )
    }
}

fn render_list_icon(icon: icons::Icon, color: ColorU) -> Box<dyn Element> {
    ConstrainedBox::new(icon.to_warpui_icon(Fill::Solid(color)).finish())
        .with_width(LIST_ICON_SIZE)
        .with_height(LIST_ICON_SIZE)
        .finish()
}

fn render_list_caption(label: &str, font_family: FamilyId, tokens: &Tokens) -> Box<dyn Element> {
    Container::new(
        Text::new_inline(label.to_owned(), font_family, LIST_CAPTION_FONT_SIZE)
            .with_color(tokens.text_3)
            .finish(),
    )
    .with_margin_bottom(2.)
    .finish()
}

fn render_list_hint(text: &str, font_family: FamilyId, tokens: &Tokens) -> Box<dyn Element> {
    Container::new(
        Text::new(text.to_owned(), font_family, LIST_CAPTION_FONT_SIZE)
            .with_color(tokens.text_3)
            .finish(),
    )
    .with_vertical_padding(8.)
    .finish()
}

/// The point past which buttons no longer fit on the footer and open from More instead.
fn render_more_line(font_family: FamilyId, tokens: &Tokens) -> Box<dyn Element> {
    let rule = || {
        Expanded::new(
            1.,
            ConstrainedBox::new(Rect::new().with_background_color(tokens.hairline).finish())
                .with_height(1.)
                .finish(),
        )
        .finish()
    };
    Container::new(
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.)
            .with_child(rule())
            .with_child(
                Text::new_inline("In More".to_owned(), font_family, LIST_CAPTION_FONT_SIZE)
                    .with_color(tokens.text_3)
                    .finish(),
            )
            .with_child(rule())
            .finish(),
    )
    .with_vertical_padding(4.)
    .finish()
}

/// Whether a prompt button sends right away or only fills the composer. Quiet until hovered:
/// most buttons send, so the switch is there when wanted rather than shouting on every row.
fn render_send_toggle<A: Action + Clone + 'static>(
    auto_send: bool,
    mouse_state: MouseStateHandle,
    action: A,
    tokens: &Tokens,
    font_family: FamilyId,
    ui_builder: &warp_core::ui::builder::UiBuilder,
) -> Box<dyn Element> {
    let tokens = *tokens;
    let ui_builder = ui_builder.clone();
    Hoverable::new(mouse_state, move |state| {
        let hovered = state.is_hovered();
        let color = if hovered {
            tokens.text_1
        } else {
            tokens.text_3
        };
        let mut content = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(4.);
        if auto_send {
            content.add_child(
                ConstrainedBox::new(
                    icons::Icon::CornerDownLeft
                        .to_warpui_icon(Fill::Solid(color))
                        .finish(),
                )
                .with_width(12.)
                .with_height(12.)
                .finish(),
            );
        }
        content.add_child(
            Text::new_inline(
                if auto_send { "Sends" } else { "Inserts" }.to_owned(),
                font_family,
                LIST_CAPTION_FONT_SIZE,
            )
            .with_color(color)
            .finish(),
        );
        let toggle = Container::new(content.finish())
            .with_horizontal_padding(6.)
            .with_vertical_padding(3.)
            .with_background(Fill::Solid(if hovered {
                tokens.surface_hover
            } else {
                ColorU::transparent_black()
            }))
            .with_corner_radius(corners_sm())
            .finish();
        if !hovered {
            return toggle;
        }
        let mut stack = Stack::new().with_child(toggle);
        stack.add_positioned_overlay_child(
            ui_builder
                .tool_tip(
                    if auto_send {
                        "Sends the prompt right away. Click to only insert it."
                    } else {
                        "Puts the prompt in the composer without sending. Click to send right away."
                    }
                    .to_owned(),
                )
                .build()
                .finish(),
            OffsetPositioning::offset_from_parent(
                vec2f(0., -6.),
                ParentOffsetBounds::WindowByPosition,
                ParentAnchor::TopMiddle,
                ChildAnchor::BottomMiddle,
            ),
        );
        stack.finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
    .finish()
}

#[cfg(test)]
mod list_tests {
    use super::list_insertion_index;

    const SLOTS: [f32; 4] = [15., 49., 83., 117.];

    #[test]
    fn moving_down_lands_after_the_rows_passed() {
        assert_eq!(list_insertion_index(&SLOTS, Some(0), 60.), 1);
        assert_eq!(list_insertion_index(&SLOTS, Some(0), 130.), 3);
    }

    #[test]
    fn moving_up_lands_before_the_rows_passed() {
        assert_eq!(list_insertion_index(&SLOTS, Some(3), 40.), 1);
        assert_eq!(list_insertion_index(&SLOTS, Some(3), 0.), 0);
    }

    #[test]
    fn staying_inside_its_own_slot_keeps_the_place() {
        assert_eq!(list_insertion_index(&SLOTS, Some(2), 83.), 2);
        assert_eq!(list_insertion_index(&SLOTS, Some(2), 70.), 2);
    }

    #[test]
    fn arriving_from_the_other_list_inserts_between_rows() {
        assert_eq!(list_insertion_index(&SLOTS, None, 60.), 2);
        assert_eq!(list_insertion_index(&SLOTS, None, 200.), 4);
        assert_eq!(list_insertion_index(&[], None, 10.), 0);
    }
}
