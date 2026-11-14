//! The agent harness picker: the current harness's logo and name on a button, opening a menu of
//! the harnesses it offers. Start screens show it in their box; a Coordinator's footer shows it in
//! place of the agent's logo, where it switches the agent the Coordinator runs.
use pathfinder_geometry::vector::vec2f;
use warp_core::ui::icons::Icon;
use warp_core::ui::theme::Fill as ThemeFill;
use warp_core::ui::tokens::SPACE_4;
use warpui::elements::{
    Border, ChildAnchor, ChildView, OffsetPositioning, ParentAnchor, ParentElement as _,
    ParentOffsetBounds, Stack,
};
use warpui::{
    AppContext, Element, Entity, SingletonEntity as _, TypedActionView, View, ViewContext,
    ViewHandle,
};

use crate::agent_resume::AgentResumeProvider;
use crate::ai::blocklist::agent_view::agent_input_footer::AgentInputButtonTheme;
use crate::appearance::Appearance;
use crate::menu::{Event as MenuEvent, Menu, MenuItem, MenuItemFields};
use crate::terminal::cli_agent_sessions::agent_for_provider;
use crate::view_components::action_button::{ActionButton, ButtonSize, TooltipAlignment};

const PICKER_MENU_WIDTH: f32 = 240.;

/// What a start screen's box starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartChoice {
    Agent(AgentResumeProvider),
    /// The box's text runs as a command in the tab's shell.
    Terminal,
}

/// The agents a Coordinator running `current` can switch to in its pickers, in order.
pub(crate) fn coordinator_choices(current: AgentResumeProvider) -> Vec<StartChoice> {
    use crate::workspace::view::coordinator::{coordinator_agents, devin_is_installed};
    coordinator_agents(current, devin_is_installed())
        .into_iter()
        .map(StartChoice::Agent)
        .collect()
}

/// The name Clinch shows for an agent, such as "Claude Code".
pub(crate) fn agent_display_name(provider: AgentResumeProvider) -> &'static str {
    agent_for_provider(provider).display_name()
}

impl StartChoice {
    pub(crate) fn name(self) -> &'static str {
        match self {
            StartChoice::Agent(provider) => agent_display_name(provider),
            StartChoice::Terminal => "Terminal",
        }
    }

    fn icon(self) -> Icon {
        match self {
            // An agent without a logo of its own, such as Devin, shows the terminal's.
            StartChoice::Agent(provider) => agent_for_provider(provider)
                .icon()
                .unwrap_or(Icon::Terminal),
            StartChoice::Terminal => Icon::Terminal,
        }
    }

    fn color(self, appearance: &Appearance) -> warpui::color::ColorU {
        let tokens = appearance.tokens();
        match self {
            StartChoice::Agent(AgentResumeProvider::Claude) => tokens.provider_claude,
            StartChoice::Agent(AgentResumeProvider::Codex) => tokens.provider_codex,
            StartChoice::Agent(provider) => agent_for_provider(provider)
                .brand_color()
                .unwrap_or(tokens.text_2),
            StartChoice::Terminal => tokens.text_2,
        }
    }
}

/// Where a picker sits, which decides which way its menu opens and what it says about itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PickerPlacement {
    /// In a start screen's box: the menu drops below it.
    StartScreen,
    /// At the left of a Coordinator's footer: the menu opens above it and says that switching
    /// starts a new conversation.
    CoordinatorFooter,
}

impl PickerPlacement {
    fn tooltip(self) -> &'static str {
        match self {
            PickerPlacement::StartScreen => "What this starts",
            PickerPlacement::CoordinatorFooter => "Switch the agent the coordinator runs",
        }
    }

    /// At the left edge of a footer, the tooltip runs right so it stays over the pane.
    fn tooltip_alignment(self) -> TooltipAlignment {
        match self {
            PickerPlacement::StartScreen => TooltipAlignment::default(),
            PickerPlacement::CoordinatorFooter => TooltipAlignment::Left,
        }
    }

    /// A line at the top of the menu saying what picking does.
    fn note(self) -> Option<&'static str> {
        match self {
            PickerPlacement::StartScreen => None,
            PickerPlacement::CoordinatorFooter => Some("Switching starts a new conversation"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum HarnessPickerAction {
    ToggleMenu,
    Select(StartChoice),
}

pub enum HarnessPickerEvent {
    /// The user picked something other than the current choice.
    Selected(StartChoice),
    Closed,
}

pub struct HarnessPicker {
    choice: StartChoice,
    choices: Vec<StartChoice>,
    placement: PickerPlacement,
    button: ViewHandle<ActionButton>,
    menu: ViewHandle<Menu<HarnessPickerAction>>,
    is_menu_open: bool,
}

impl HarnessPicker {
    pub(crate) fn new(
        choice: StartChoice,
        choices: Vec<StartChoice>,
        placement: PickerPlacement,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let button = ctx.add_typed_action_view(|_| {
            ActionButton::new("", AgentInputButtonTheme)
                .with_size(ButtonSize::AgentInputButton)
                .with_menu(true)
                .with_tooltip(placement.tooltip())
                .with_tooltip_alignment(placement.tooltip_alignment())
                .on_click(|ctx| ctx.dispatch_typed_action(HarnessPickerAction::ToggleMenu))
        });
        let menu = ctx.add_typed_action_view(|_| {
            Menu::new()
                .with_width(PICKER_MENU_WIDTH)
                .with_drop_shadow()
                .prevent_interaction_with_other_elements()
        });
        ctx.subscribe_to_view(&menu, |me, _, event, ctx| {
            if let MenuEvent::Close { .. } = event {
                me.set_menu_open(false, ctx);
            }
        });
        ctx.subscribe_to_model(&Appearance::handle(ctx), |me, _, _, ctx| me.refresh(ctx));
        let mut picker = Self {
            choice,
            choices,
            placement,
            button,
            menu,
            is_menu_open: false,
        };
        picker.refresh(ctx);
        picker
    }

    pub(crate) fn choice(&self) -> StartChoice {
        self.choice
    }

    pub(crate) fn set_choice(&mut self, choice: StartChoice, ctx: &mut ViewContext<Self>) {
        if self.choice != choice {
            self.choice = choice;
            self.refresh(ctx);
        }
    }

    fn refresh(&mut self, ctx: &mut ViewContext<Self>) {
        let appearance = Appearance::as_ref(ctx);
        let border = Border::all(1.).with_border_fill(appearance.theme().outline());
        let current = self.choice;
        let color = current.color(appearance);
        let note = self.placement.note().map(|note| MenuItem::Header {
            fields: MenuItemFields::new(note),
            clickable: false,
            right_side_fields: None,
        });
        let items: Vec<MenuItem<HarnessPickerAction>> = note
            .into_iter()
            .chain(self.choices.iter().map(|&choice| {
                let fields = MenuItemFields::new(choice.name())
                    .with_icon(choice.icon())
                    .with_icon_size_override(16.)
                    .with_override_icon_color(ThemeFill::Solid(choice.color(appearance)))
                    .with_on_select_action(HarnessPickerAction::Select(choice));
                MenuItem::Item(if choice == current {
                    fields.with_right_side_icon(Icon::Check)
                } else {
                    fields
                })
            }))
            .collect();
        self.button.update(ctx, |button, ctx| {
            button.set_label(current.name(), ctx);
            button.set_icon(Some(current.icon()), ctx);
            button.set_icon_color(Some(color), ctx);
        });
        self.menu.update(ctx, |menu, ctx| {
            menu.set_border(Some(border));
            menu.set_items(items, ctx);
        });
    }

    fn set_menu_open(&mut self, open: bool, ctx: &mut ViewContext<Self>) {
        if self.is_menu_open == open {
            return;
        }
        self.is_menu_open = open;
        if open {
            ctx.focus(&self.menu);
            let current = HarnessPickerAction::Select(self.choice);
            self.menu
                .update(ctx, |menu, ctx| menu.set_selected_by_action(&current, ctx));
        } else {
            ctx.emit(HarnessPickerEvent::Closed);
        }
        ctx.notify();
    }
}

impl Entity for HarnessPicker {
    type Event = HarnessPickerEvent;
}

impl TypedActionView for HarnessPicker {
    type Action = HarnessPickerAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            HarnessPickerAction::ToggleMenu => {
                self.set_menu_open(!self.is_menu_open, ctx);
            }
            HarnessPickerAction::Select(choice) => {
                self.set_menu_open(false, ctx);
                if *choice != self.choice {
                    ctx.emit(HarnessPickerEvent::Selected(*choice));
                }
            }
        }
    }
}

impl View for HarnessPicker {
    fn ui_name() -> &'static str {
        "HarnessPicker"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        let mut stack = Stack::new();
        stack.add_child(ChildView::new(&self.button).finish());
        if self.is_menu_open {
            let positioning = match self.placement {
                PickerPlacement::StartScreen => OffsetPositioning::offset_from_parent(
                    vec2f(0., SPACE_4),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::BottomRight,
                    ChildAnchor::TopRight,
                ),
                PickerPlacement::CoordinatorFooter => OffsetPositioning::offset_from_parent(
                    vec2f(0., -SPACE_4),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::TopLeft,
                    ChildAnchor::BottomLeft,
                ),
            };
            stack.add_positioned_overlay_child(ChildView::new(&self.menu).finish(), positioning);
        }
        stack.finish()
    }
}
