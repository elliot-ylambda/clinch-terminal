mod grouping;
pub(crate) use grouping::{group_skills_by_scope, providers_for_subtab, SkillsSubtab};

use std::collections::{HashMap, HashSet};

use ai::skills::SkillScope;
use warp_core::ui::Icon;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::elements::new_scrollable::{NewScrollable, ScrollableAppearance, SingleAxisConfig};
use warpui::elements::{
    ChildView, ClippedScrollStateHandle, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, Element, Fill, Flex, Hoverable, MainAxisSize, MouseStateHandle,
    ParentElement, Radius, ScrollbarWidth, Shrinkable, Text,
};
use warpui::platform::Cursor;
use warpui::ui_components::components::{Coords, UiComponentStyles};
use warpui::ui_components::segmented_control::{
    LabelConfig, RenderableOptionConfig, SegmentedControl, SegmentedControlEvent,
};
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle};

use crate::ai::skills::{
    render_skill_button, SkillDescriptor, SkillManager, SkillManagerEvent, SkillReference,
};
use crate::appearance::Appearance;

/// Read-only skills inspector shown as a left-panel tool view.
pub struct SkillsPanel {
    /// The All/Claude/Codex subtab selector; owns its own selection state.
    subtab_control: ViewHandle<SegmentedControl<SkillsSubtab>>,
    /// Working directory of the active session; drives project-scope skills.
    working_directory: Option<LocalOrRemotePath>,
    /// Scope groups (Home/Project/Bundled) the user has collapsed.
    collapsed_scopes: HashSet<SkillScope>,
    scroll_state: ClippedScrollStateHandle,
    /// Per-skill row mouse states, keyed by skill name, so hover feedback survives
    /// re-renders triggered by unrelated state changes (e.g. a sibling skill's row).
    row_states: HashMap<String, MouseStateHandle>,
    /// Per-scope-group header mouse states, keyed by scope, so hover feedback on a
    /// group header survives re-renders triggered by unrelated state changes (e.g. a
    /// subtab switch or a `HomeSkillsChanged` event) while the mouse sits stationary.
    header_states: HashMap<SkillScope, MouseStateHandle>,
}

#[derive(Clone, Debug)]
pub enum SkillsPanelEvent {
    /// User clicked a skill row's "open" affordance; workspace opens the SKILL.md file.
    OpenSkillFile(LocalOrRemotePath),
}

/// Actions bridging click callbacks (which only ever get a `&mut EventContext` — see
/// `render_skill_button`'s `F: FnMut(&mut EventContext)` bound in `skill_utils.rs`, and
/// `Hoverable::on_click`'s equivalent bound) back to view methods that can mutate state or
/// emit events via `ViewContext`. No keyboard actions in Phase 1.
#[derive(Clone, Debug)]
pub enum SkillsPanelAction {
    /// Toggles whether a scope group (Home/Project/Bundled) is collapsed.
    ToggleScope(SkillScope),
    /// A skill row's open affordance was clicked.
    OpenSkillFile {
        path: LocalOrRemotePath,
        mouse_state: MouseStateHandle,
    },
}

impl SkillsPanel {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let subtab_control = ctx.add_typed_action_view(|ctx| {
            SegmentedControl::new(
                vec![SkillsSubtab::All, SkillsSubtab::Claude, SkillsSubtab::Codex],
                |subtab: SkillsSubtab, is_selected, app| {
                    let appearance = Appearance::as_ref(app);
                    let theme = appearance.theme();
                    Some(RenderableOptionConfig {
                        icon_path: "",
                        icon_color: theme.main_text_color(theme.background()).into(),
                        label: Some(LabelConfig {
                            label: subtab.label().into(),
                            width_override: None,
                            color: if is_selected {
                                theme.accent().into()
                            } else {
                                theme.main_text_color(theme.background()).into()
                            },
                        }),
                        tooltip: None,
                        background: if is_selected {
                            Fill::Solid(theme.surface_3().into())
                        } else {
                            Fill::None
                        },
                    })
                },
                SkillsSubtab::All,
                subtab_control_styles(ctx),
            )
        });

        // The subtab selection lives inside `subtab_control`; when it changes, re-sync the
        // row mouse-state cache (the visible skill set changed) and re-render.
        ctx.subscribe_to_view(&subtab_control, |me, _, event, ctx| {
            let SegmentedControlEvent::OptionSelected(_) = event;
            me.sync_row_states(ctx);
            ctx.notify();
        });

        // Refresh when home skills change on disk. Project skills are re-read on
        // `set_working_directory` and on every render (cheap — a HashMap walk).
        ctx.subscribe_to_model(&SkillManager::handle(ctx), |me, _model, event, ctx| {
            match event {
                SkillManagerEvent::HomeSkillsChanged => {
                    me.sync_row_states(ctx);
                    ctx.notify();
                }
            }
        });

        let mut this = Self {
            subtab_control,
            working_directory: None,
            collapsed_scopes: HashSet::new(),
            scroll_state: ClippedScrollStateHandle::default(),
            row_states: HashMap::new(),
            header_states: HashMap::new(),
        };
        this.sync_row_states(ctx);
        this
    }

    pub fn set_working_directory(
        &mut self,
        cwd: Option<LocalOrRemotePath>,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.working_directory != cwd {
            self.working_directory = cwd;
            self.sync_row_states(ctx);
            ctx.notify();
        }
    }

    /// The SkillsPanel has no focusable inner widget in Phase 1.
    pub fn on_left_panel_focused(&mut self, _ctx: &mut ViewContext<Self>) {}

    fn active_subtab(&self, app: &AppContext) -> SkillsSubtab {
        self.subtab_control.as_ref(app).selected_option()
    }

    /// Fetch + filter skills for the active subtab and working directory.
    fn current_skills(&self, app: &AppContext) -> Vec<SkillDescriptor> {
        let manager = SkillManager::as_ref(app);
        let cwd = self.working_directory.as_ref();
        match providers_for_subtab(self.active_subtab(app)) {
            None => manager.get_skills_for_working_directory(cwd, app),
            Some(providers) => manager.skills_for_providers(cwd, providers, app),
        }
    }

    /// Ensures every skill currently in scope has a persistent mouse-state handle, so hover
    /// feedback on a row survives a re-render triggered by something else changing.
    fn sync_row_states(&mut self, ctx: &mut ViewContext<Self>) {
        let skills = self.current_skills(ctx);
        for skill in &skills {
            self.row_states.entry(skill.name.clone()).or_default();
            self.header_states.entry(skill.scope).or_default();
        }
    }

    fn row_state(&self, name: &str) -> MouseStateHandle {
        self.row_states.get(name).cloned().unwrap_or_default()
    }

    fn header_state(&self, scope: SkillScope) -> MouseStateHandle {
        self.header_states.get(&scope).cloned().unwrap_or_default()
    }

    fn render_subtab_bar(&self) -> Box<dyn Element> {
        Container::new(ChildView::new(&self.subtab_control).finish())
            .with_horizontal_padding(8.)
            .with_vertical_padding(6.)
            .finish()
    }

    fn render_group(
        &self,
        scope: SkillScope,
        skills: &[SkillDescriptor],
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let is_collapsed = self.collapsed_scopes.contains(&scope);

        let chevron_icon = if is_collapsed {
            Icon::ChevronRight
        } else {
            Icon::ChevronDown
        };
        let chevron = ConstrainedBox::new(
            chevron_icon
                .to_warpui_icon(theme.sub_text_color(theme.background()))
                .finish(),
        )
        .with_width(12.)
        .with_height(12.);

        let title = Text::new_inline(scope_header_label(scope), appearance.ui_font_family(), 11.)
            .with_color(theme.sub_text_color(theme.background()).into());

        let header_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(4.)
            .with_child(chevron.finish())
            .with_child(title.finish())
            .finish();

        let header = Hoverable::new(self.header_state(scope), move |mouse_state| {
            let mut container = Container::new(header_row)
                .with_horizontal_padding(8.)
                .with_vertical_padding(4.);
            if mouse_state.is_hovered() {
                container = container.with_background(theme.surface_overlay_1());
            }
            container.finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(SkillsPanelAction::ToggleScope(scope));
        })
        .finish();

        let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        column = column.with_child(header);

        if !is_collapsed {
            for skill in skills {
                let button_handle = self.row_state(&skill.name);
                let action_mouse_state = button_handle.clone();
                let path = match &skill.reference {
                    SkillReference::Path(p) => Some(p.clone()),
                    SkillReference::BundledSkillId(_) => None,
                };
                let row = render_skill_button(
                    &skill.name,
                    button_handle,
                    appearance,
                    skill.provider,
                    skill.icon_override,
                    move |ctx| {
                        if let Some(path) = path.clone() {
                            ctx.dispatch_typed_action(SkillsPanelAction::OpenSkillFile {
                                path,
                                mouse_state: action_mouse_state.clone(),
                            });
                        }
                    },
                );
                column = column.with_child(Container::new(row).with_horizontal_padding(8.).finish());
            }
        }

        column.finish()
    }
}

fn scope_header_label(scope: SkillScope) -> &'static str {
    match scope {
        SkillScope::Home => "Home",
        SkillScope::Project => "Project",
        SkillScope::Bundled => "Bundled",
    }
}

fn subtab_control_styles(app: &AppContext) -> UiComponentStyles {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    UiComponentStyles {
        font_family_id: Some(appearance.ui_font_family()),
        font_size: Some(appearance.ui_font_size()),
        border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.0))),
        border_width: Some(1.0),
        border_color: Some(Fill::Solid(theme.surface_3().into())),
        background: Some(Fill::Solid(theme.background().into())),
        height: Some(20.0),
        padding: Some(Coords::uniform(0.0)),
        ..Default::default()
    }
}

impl Entity for SkillsPanel {
    type Event = SkillsPanelEvent;
}

impl TypedActionView for SkillsPanel {
    type Action = SkillsPanelAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            SkillsPanelAction::ToggleScope(scope) => {
                if !self.collapsed_scopes.remove(scope) {
                    self.collapsed_scopes.insert(*scope);
                }
                ctx.notify();
            }
            SkillsPanelAction::OpenSkillFile { path, mouse_state } => {
                // Resets the interaction state of the row's mouse state to avoid an
                // immediate re-hover once the panel re-renders.
                if let Ok(mut state) = mouse_state.lock() {
                    state.reset_interaction_state();
                }
                ctx.emit(SkillsPanelEvent::OpenSkillFile(path.clone()));
            }
        }
    }
}

impl View for SkillsPanel {
    fn ui_name() -> &'static str {
        "SkillsPanel"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let active_subtab = self.active_subtab(app);

        let skills = self.current_skills(app);
        let grouped = group_skills_by_scope(skills);

        let mut list_column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        if grouped.is_empty() {
            let empty_msg = match active_subtab {
                SkillsSubtab::All => "No skills found for this directory.".to_string(),
                other => format!("No skills {} can read in this directory.", other.label()),
            };
            list_column = list_column.with_child(
                Container::new(
                    Text::new_inline(empty_msg, appearance.ui_font_family(), appearance.ui_font_size())
                        .with_color(theme.sub_text_color(theme.background()).into())
                        .finish(),
                )
                .with_horizontal_padding(12.)
                .with_vertical_padding(8.)
                .finish(),
            );
        } else {
            for (scope, group) in &grouped {
                list_column = list_column.with_child(self.render_group(*scope, group, appearance));
            }
        }

        let scrollable = NewScrollable::vertical(
            SingleAxisConfig::Clipped {
                handle: self.scroll_state.clone(),
                child: list_column.finish(),
            },
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            Fill::None,
        )
        .with_vertical_scrollbar(ScrollableAppearance::new(ScrollbarWidth::Auto, false))
        .with_propagate_mousewheel_if_not_handled(true)
        .finish();

        Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(self.render_subtab_bar())
            .with_child(Shrinkable::new(1.0, scrollable).finish())
            .finish()
    }
}
