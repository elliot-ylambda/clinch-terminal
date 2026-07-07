//! "Create quick-insert button" modal for the CLI-agent footer.
//!
//! Lets the user save a piece of text as a footer button that inserts-and-sends
//! it to the active CLI agent. The modal has two single-line editors (the text to
//! insert and an auto-derived, editable label) plus a scrollable pick list of the
//! user's discovered slash commands and skills (home + project scope). Picking a
//! row pre-fills the text field with a sensible default the user can edit.
//!
//! Follows the `ProviderKeysModalView` template (`app/src/auth/`): a self-contained
//! WarpUI `View` with an `Entity::Event`, a `TypedActionView`, `EditorView`
//! single-line fields read via `.buffer_text(ctx)`, and an escape-to-cancel
//! `FixedBinding`. Workspace wiring (build/store/open/render-stack) lives in
//! `app/src/workspace/view.rs`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

use ai::skills::SkillScope;
use pathfinder_color::ColorU;
use ui_components::{button, Component as _, Options as _};
use warp_core::ui::theme::color::internal_colors;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::elements::new_scrollable::{NewScrollable, ScrollableAppearance, SingleAxisConfig};
use warpui::elements::{
    Align, Border, ClippedScrollStateHandle, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, Dismiss, Fill, Flex, FormattedTextElement, MainAxisAlignment, MainAxisSize,
    MouseStateHandle, ParentElement, Radius, ScrollbarWidth, Shrinkable, Stack,
};
use warpui::fonts::Weight;
use warpui::keymap::FixedBinding;
use warpui::text_layout::TextAlignment;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Element, Entity, FocusContext, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use crate::ai::blocklist::view_util::render_provider_icon_button;
use crate::ai::cli_commands::{
    discover_commands, CommandProvider, CommandScope, DiscoveredCommand,
};
use crate::ai::skills::{render_skill_button, SkillDescriptor, SkillManager};
use crate::appearance::Appearance;
use crate::editor::{
    EditorView, PropagateAndNoOpNavigationKeys, SingleLineEditorOptions, TextOptions,
};
use crate::ui_components::icons::Icon;
use crate::workspace::view::skills_panel::group_skills_by_scope;

const MODAL_WIDTH: f32 = 520.;
const INPUT_BORDER_RADIUS: Radius = Radius::Pixels(4.);
const PICK_LIST_MAX_HEIGHT: f32 = 220.;
/// The auto-derived label is trimmed to this many characters.
const MAX_LABEL_LEN: usize = 24;

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;
    app.register_fixed_bindings([FixedBinding::new(
        "escape",
        QuickInsertModalAction::Cancel,
        id!(QuickInsertModal::ui_name()),
    )]);
}

/// Derives a button label from the insert text: the first non-empty line, trimmed
/// to [`MAX_LABEL_LEN`] characters, falling back to "Custom" when there is no text.
fn derive_label(text: &str) -> String {
    let first = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    if first.is_empty() {
        return "Custom".to_string();
    }
    first.chars().take(MAX_LABEL_LEN).collect()
}

#[derive(Debug)]
pub enum QuickInsertModalAction {
    Save,
    Cancel,
    /// A pick-list row was clicked; pre-fill the text field with this default.
    SetText(String),
}

#[derive(Debug)]
pub enum QuickInsertModalEvent {
    Save { label: String, text: String },
    Cancel,
}

pub struct QuickInsertModal {
    text_input: ViewHandle<EditorView>,
    label_input: ViewHandle<EditorView>,
    /// The last label value this modal auto-filled. While the label field still
    /// equals it, we keep syncing the derived label as the text changes; once the
    /// user overrides the label, they diverge and auto-fill stops.
    last_auto_label: String,
    /// Active working directory, used to scope command + skill discovery.
    cwd: Option<PathBuf>,
    commands: Vec<DiscoveredCommand>,
    skills: Vec<SkillDescriptor>,
    /// Per-row hover state, keyed by a stable row key so hover feedback survives
    /// re-renders. Created once and reused (never `MouseStateHandle::default()`
    /// inline in render).
    row_states: RefCell<HashMap<String, MouseStateHandle>>,
    scroll_state: ClippedScrollStateHandle,
    close_mouse_state: MouseStateHandle,
    cancel_button: button::Button,
    save_button: button::Button,
}

impl QuickInsertModal {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let text_input = Self::make_editor("Text to insert and send", ctx);
        let label_input = Self::make_editor("Button label", ctx);

        // As the insert text changes, keep the label in sync until the user
        // takes over the label field.
        ctx.subscribe_to_view(&text_input, |me, _, event, ctx| {
            if let crate::editor::Event::Edited(_) = event {
                me.maybe_autofill_label(ctx);
            }
        });

        Self {
            text_input,
            label_input,
            last_auto_label: String::new(),
            cwd: None,
            commands: Vec::new(),
            skills: Vec::new(),
            row_states: RefCell::new(HashMap::new()),
            scroll_state: ClippedScrollStateHandle::default(),
            close_mouse_state: MouseStateHandle::default(),
            cancel_button: button::Button::default(),
            save_button: button::Button::default(),
        }
    }

    fn make_editor(placeholder: &str, ctx: &mut ViewContext<Self>) -> ViewHandle<EditorView> {
        let placeholder = placeholder.to_string();
        ctx.add_typed_action_view(move |ctx| {
            let appearance = Appearance::as_ref(ctx);
            let text_colors = crate::settings_view::editor_text_colors(appearance);
            let options = SingleLineEditorOptions {
                text: TextOptions {
                    font_family_override: Some(appearance.ui_font_family()),
                    text_colors_override: Some(text_colors),
                    ..Default::default()
                },
                propagate_and_no_op_vertical_navigation_keys:
                    PropagateAndNoOpNavigationKeys::Always,
                ..Default::default()
            };
            let mut editor = EditorView::single_line(options, ctx);
            editor.set_placeholder_text(&placeholder, ctx);
            editor
        })
    }

    /// Seeds the modal for a new button and (re)scans commands + skills for `cwd`.
    pub fn open(&mut self, cwd: PathBuf, ctx: &mut ViewContext<Self>) {
        let commands = discover_commands(&cwd, ctx);
        let lor = LocalOrRemotePath::Local(cwd.clone());
        let skills = SkillManager::as_ref(ctx).get_skills_for_working_directory(Some(&lor), ctx);

        self.cwd = Some(cwd);
        self.commands = commands;
        self.skills = skills;
        self.last_auto_label = String::new();
        self.row_states.borrow_mut().clear();

        self.text_input
            .update(ctx, |editor, ctx| editor.set_buffer_text("", ctx));
        self.label_input
            .update(ctx, |editor, ctx| editor.set_buffer_text("", ctx));
        ctx.notify();
    }

    /// Auto-fills the label from the insert text unless the user has edited the
    /// label themselves (detected by the label no longer matching what we last
    /// wrote). Empty text clears the label so its placeholder shows; the "Custom"
    /// fallback is applied only at save time.
    fn maybe_autofill_label(&mut self, ctx: &mut ViewContext<Self>) {
        let current_label = self.label_input.as_ref(ctx).buffer_text(ctx);
        if current_label != self.last_auto_label {
            return;
        }
        let text = self.text_input.as_ref(ctx).buffer_text(ctx);
        let derived = if text.trim().is_empty() {
            String::new()
        } else {
            derive_label(&text)
        };
        if derived == current_label {
            return;
        }
        self.last_auto_label = derived.clone();
        self.label_input
            .update(ctx, |editor, ctx| editor.set_buffer_text(&derived, ctx));
        ctx.notify();
    }

    fn submit(&mut self, ctx: &mut ViewContext<Self>) {
        let text = self.text_input.as_ref(ctx).buffer_text(ctx);
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let label = self.label_input.as_ref(ctx).buffer_text(ctx);
        let label = label.trim();
        let label = if label.is_empty() {
            derive_label(text)
        } else {
            label.to_string()
        };
        ctx.emit(QuickInsertModalEvent::Save {
            label,
            text: text.to_string(),
        });
    }

    fn row_state(&self, key: &str) -> MouseStateHandle {
        self.row_states
            .borrow_mut()
            .entry(key.to_string())
            .or_default()
            .clone()
    }

    fn render_field(
        &self,
        appearance: &Appearance,
        label: &'static str,
        editor: ViewHandle<EditorView>,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let dialog_surface_solid = theme.surface_1().into_solid();
        let input_bg = theme.surface_2();
        let input_bg_solid = input_bg.into_solid();
        let input_text_color: ColorU = internal_colors::text_main(theme, input_bg_solid);
        let border_color = internal_colors::neutral_4(theme);

        let label_el = FormattedTextElement::from_str(label, appearance.ui_font_family(), 12.)
            .with_color(internal_colors::text_main(theme, dialog_surface_solid))
            .with_weight(Weight::Normal)
            .with_alignment(TextAlignment::Left)
            .with_line_height_ratio(1.0)
            .finish();

        let input = appearance
            .ui_builder()
            .text_input(editor)
            .with_style(UiComponentStyles {
                background: Some(input_bg.into()),
                border_width: Some(1.),
                border_color: Some(Fill::Solid(border_color)),
                border_radius: Some(CornerRadius::with_all(INPUT_BORDER_RADIUS)),
                font_color: Some(input_text_color),
                padding: Some(Coords {
                    top: 10.,
                    bottom: 10.,
                    left: 16.,
                    right: 16.,
                }),
                ..Default::default()
            })
            .build()
            .finish();

        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(label_el)
            .with_child(Container::new(input).with_margin_top(8.).finish())
            .finish()
    }

    fn render_pick_list(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let grouped_skills = group_skills_by_scope(self.skills.clone());

        let mut list_column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        let mut any_rows = false;

        for (command_scope, skill_scope, header) in [
            (CommandScope::Home, SkillScope::Home, "Home"),
            (CommandScope::Project, SkillScope::Project, "Project"),
        ] {
            let commands: Vec<&DiscoveredCommand> = self
                .commands
                .iter()
                .filter(|command| command.scope == command_scope)
                .collect();
            let skills: &[SkillDescriptor] = grouped_skills
                .iter()
                .find(|(scope, _)| *scope == skill_scope)
                .map(|(_, group)| group.as_slice())
                .unwrap_or(&[]);

            if commands.is_empty() && skills.is_empty() {
                continue;
            }
            any_rows = true;

            list_column = list_column.with_child(self.render_group_header(header, appearance));
            for command in commands {
                list_column = list_column.with_child(self.render_command_row(command, appearance));
            }
            for skill in skills {
                list_column = list_column.with_child(self.render_skill_row(skill, appearance));
            }
        }

        if !any_rows {
            list_column = list_column.with_child(
                Container::new(
                    FormattedTextElement::from_str(
                        "No slash commands or skills found for this directory.",
                        appearance.ui_font_family(),
                        13.,
                    )
                    .with_color(internal_colors::text_sub(
                        theme,
                        theme.surface_1().into_solid(),
                    ))
                    .with_alignment(TextAlignment::Left)
                    .finish(),
                )
                .with_vertical_padding(8.)
                .finish(),
            );
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

        ConstrainedBox::new(scrollable)
            .with_max_height(PICK_LIST_MAX_HEIGHT)
            .finish()
    }

    fn render_group_header(
        &self,
        label: &'static str,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        Container::new(
            FormattedTextElement::from_str(label, appearance.ui_font_family(), 11.)
                .with_color(internal_colors::text_sub(
                    theme,
                    theme.surface_1().into_solid(),
                ))
                .with_weight(Weight::Semibold)
                .with_alignment(TextAlignment::Left)
                .finish(),
        )
        .with_padding_top(8.)
        .with_padding_bottom(4.)
        .finish()
    }

    fn render_command_row(
        &self,
        command: &DiscoveredCommand,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let key = format!(
            "cmd:{command_scope:?}:{name}",
            command_scope = command.scope,
            name = command.name
        );
        let handle = self.row_state(&key);
        let icon = match command.provider {
            CommandProvider::Claude => Icon::ClaudeLogo,
            CommandProvider::Codex => Icon::OpenAILogo,
        };
        let label = match &command.description {
            Some(description) => format!("{name} — {description}", name = command.name),
            None => command.name.clone(),
        };
        let invocation = command.invocation.clone();
        let row = render_provider_icon_button(
            &label,
            handle,
            appearance,
            icon,
            internal_colors::fg_overlay_6(theme),
            move |ctx| {
                ctx.dispatch_typed_action(QuickInsertModalAction::SetText(invocation.clone()));
            },
        );
        Container::new(row).with_margin_bottom(4.).finish()
    }

    fn render_skill_row(
        &self,
        skill: &SkillDescriptor,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let key = format!("skill:{name}", name = skill.name);
        let handle = self.row_state(&key);
        let insert = format!("/{name}", name = skill.name);
        let row = render_skill_button(
            &skill.name,
            handle,
            appearance,
            skill.provider,
            skill.icon_override,
            move |ctx| {
                ctx.dispatch_typed_action(QuickInsertModalAction::SetText(insert.clone()));
            },
        );
        Container::new(row).with_margin_bottom(4.).finish()
    }
}

impl Entity for QuickInsertModal {
    type Event = QuickInsertModalEvent;
}

impl View for QuickInsertModal {
    fn ui_name() -> &'static str {
        "QuickInsertModal"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            ctx.focus(&self.text_input);
            ctx.notify();
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let dialog_surface = theme.surface_1();
        let dialog_surface_solid = dialog_surface.into_solid();
        let border_color = internal_colors::neutral_4(theme);
        let ui_builder = appearance.ui_builder();

        let title = FormattedTextElement::from_str(
            "Create quick-insert button",
            appearance.ui_font_family(),
            16.,
        )
        .with_color(internal_colors::text_main(theme, dialog_surface_solid))
        .with_weight(Weight::Bold)
        .with_line_height_ratio(1.25)
        .finish();

        let close_button = ui_builder
            .close_button(24., self.close_mouse_state.clone())
            .build()
            .on_click(|ctx: &mut warpui::EventContext, _, _| {
                ctx.dispatch_typed_action(QuickInsertModalAction::Cancel);
            })
            .finish();

        let title_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(Shrinkable::new(1., title).finish())
            .with_child(close_button)
            .finish();

        let subtitle = FormattedTextElement::from_str(
            "The button inserts and sends this text to the active CLI agent. Pick a command or skill below to pre-fill it.",
            appearance.ui_font_family(),
            14.,
        )
        .with_color(internal_colors::text_sub(theme, dialog_surface_solid))
        .with_weight(Weight::Normal)
        .with_alignment(TextAlignment::Left)
        .with_line_height_ratio(1.2)
        .finish();

        let body = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(Container::new(subtitle).with_margin_bottom(16.).finish())
            .with_child(self.render_field(appearance, "Text to insert", self.text_input.clone()))
            .with_child(
                Container::new(self.render_field(
                    appearance,
                    "Button label",
                    self.label_input.clone(),
                ))
                .with_margin_top(16.)
                .finish(),
            )
            .with_child(
                Container::new(self.render_pick_list(appearance))
                    .with_margin_top(16.)
                    .finish(),
            )
            .finish();

        let cancel_button = self.cancel_button.render(
            appearance,
            button::Params {
                content: button::Content::Label("Cancel".into()),
                theme: &button::themes::Naked,
                options: button::Options {
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(QuickInsertModalAction::Cancel);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );

        let save_button = self.save_button.render(
            appearance,
            button::Params {
                content: button::Content::Label("Add button".into()),
                theme: &button::themes::Primary,
                options: button::Options {
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(QuickInsertModalAction::Save);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );

        let footer = Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::End)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(cancel_button)
                .with_child(Container::new(save_button).with_margin_left(8.).finish())
                .finish(),
        )
        .with_border(Border::top(1.).with_border_color(border_color))
        .with_horizontal_padding(24.)
        .with_vertical_padding(12.)
        .finish();

        let dialog = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(
                Container::new(title_row)
                    .with_horizontal_padding(24.)
                    .with_padding_top(24.)
                    .with_padding_bottom(12.)
                    .finish(),
            )
            .with_child(
                Container::new(body)
                    .with_horizontal_padding(24.)
                    .with_padding_bottom(16.)
                    .finish(),
            )
            .with_child(footer)
            .finish();

        let modal = ConstrainedBox::new(
            Container::new(dialog)
                .with_background(dialog_surface)
                .with_border(Border::all(1.).with_border_color(border_color))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                .finish(),
        )
        .with_width(MODAL_WIDTH)
        .finish();

        let mut stack = Stack::new();
        stack.add_child(
            Container::new(warpui::elements::Empty::new().finish())
                .with_background_color(ColorU::new(0, 0, 0, 179))
                .finish(),
        );
        stack.add_child(
            Dismiss::new(Align::new(modal).finish())
                .on_dismiss(|ctx, _app| {
                    ctx.dispatch_typed_action(QuickInsertModalAction::Cancel);
                })
                .finish(),
        );
        stack.finish()
    }
}

impl TypedActionView for QuickInsertModal {
    type Action = QuickInsertModalAction;

    fn handle_action(&mut self, action: &QuickInsertModalAction, ctx: &mut ViewContext<Self>) {
        match action {
            QuickInsertModalAction::Save => {
                self.submit(ctx);
            }
            QuickInsertModalAction::Cancel => {
                ctx.emit(QuickInsertModalEvent::Cancel);
            }
            QuickInsertModalAction::SetText(text) => {
                self.text_input
                    .update(ctx, |editor, ctx| editor.set_buffer_text(text, ctx));
                ctx.focus(&self.text_input);
                ctx.notify();
            }
        }
    }
}

#[cfg(test)]
#[path = "quick_insert_modal_tests.rs"]
mod tests;
