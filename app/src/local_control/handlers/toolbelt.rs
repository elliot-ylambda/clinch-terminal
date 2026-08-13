//! Typed mutations for Claude Code, Codex, and terminal footer toolbelts.

use ::local_control::protocol::{
    ToolbeltButtonCreateParams, ToolbeltButtonDeleteParams, ToolbeltButtonMoveParams,
    ToolbeltFooter, ToolbeltListParams, ToolbeltSide, ToolbeltSuggestionListParams,
    ToolbeltSuggestionOutcome, ToolbeltSuggestionResolveParams,
};
use ::local_control::{ActionKind, ControlError, ErrorCode};
use serde_json::json;
use settings::Setting as _;
use warpui::{ModelContext, SingletonEntity as _};

use crate::ai::blocklist::agent_view::agent_input_footer::toolbar_item::AgentToolbarItemKind;
use crate::local_control::LocalControlBridge;
use crate::terminal::session_settings::{
    CLIAgentToolbarChipSelection, SessionSettings, TerminalToolbarChipSelection,
    ToolbarChipSelection,
};

const MAX_LABEL_BYTES: usize = 128;
const MAX_TEXT_BYTES: usize = 32 * 1024;

#[derive(Clone)]
enum Selection {
    Cli(CLIAgentToolbarChipSelection),
    Terminal(TerminalToolbarChipSelection),
}

impl Selection {
    fn items(&self) -> (Vec<AgentToolbarItemKind>, Vec<AgentToolbarItemKind>) {
        match self {
            Self::Cli(selection) => (selection.left_items(), selection.right_items()),
            Self::Terminal(selection) => (selection.left_items(), selection.right_items()),
        }
    }

    fn hidden_custom_inserts(&self) -> Vec<AgentToolbarItemKind> {
        match self {
            Self::Cli(selection) => selection.hidden_custom_inserts(),
            Self::Terminal(selection) => selection.hidden_custom_inserts(),
        }
    }

    fn rebuild(
        &self,
        left: Vec<AgentToolbarItemKind>,
        right: Vec<AgentToolbarItemKind>,
        hidden_custom_inserts: Vec<AgentToolbarItemKind>,
    ) -> Self {
        match self {
            Self::Cli(_) => Self::Cli(
                CLIAgentToolbarChipSelection::custom_from_effective_items_and_hidden_custom_inserts(
                    left,
                    right,
                    hidden_custom_inserts,
                ),
            ),
            Self::Terminal(_) => Self::Terminal(
                TerminalToolbarChipSelection::custom_from_effective_items_and_hidden_custom_inserts(
                    left,
                    right,
                    hidden_custom_inserts,
                ),
            ),
        }
    }

    fn default_items(&self) -> Vec<AgentToolbarItemKind> {
        let (left, right) = match self {
            Self::Cli(_) => (
                CLIAgentToolbarChipSelection::default_left_items(),
                CLIAgentToolbarChipSelection::default_right_items(),
            ),
            Self::Terminal(_) => (
                TerminalToolbarChipSelection::default_left_items(),
                TerminalToolbarChipSelection::default_right_items(),
            ),
        };
        left.into_iter().chain(right).collect()
    }

    fn reserved_items(&self) -> Vec<AgentToolbarItemKind> {
        match self {
            Self::Cli(_) => AgentToolbarItemKind::all_available_for_cli_input(),
            Self::Terminal(_) => AgentToolbarItemKind::all_available_for_terminal_input(),
        }
    }
}

pub(crate) fn handle(
    action: &::local_control::Action,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    match action.kind {
        ActionKind::ToolbeltList => {
            let params = action.params_as::<ToolbeltListParams>()?;
            Ok(state_result(
                params.footer,
                &read_selection(params.footer, ctx),
            ))
        }
        ActionKind::ToolbeltButtonCreate => create(action, ctx),
        ActionKind::ToolbeltButtonDelete => delete(action, ctx),
        ActionKind::ToolbeltButtonMove => move_button(action, ctx),
        ActionKind::ToolbeltSuggestionList => suggestion_list(action, ctx),
        ActionKind::ToolbeltSuggestionResolve => resolve_suggestion(action),
        _ => Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            format!("{} is not a toolbelt action", action.kind.as_str()),
        )),
    }
}

fn suggestion_list(
    action: &::local_control::Action,
    ctx: &ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let params = action.params_as::<ToolbeltSuggestionListParams>()?;
    if params.footer == ToolbeltFooter::Terminal {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            "learned suggestions are available only for Claude Code and Codex footers",
        ));
    }
    let enabled = crate::agent_resume::toolbelt_learning_enabled();
    let selection = read_selection(params.footer, ctx);
    let suggestions = crate::agent_resume::learned_toolbelt_suggestions()
        .into_iter()
        .filter(|suggestion| !selection_contains_text(&selection, &suggestion.text))
        .collect::<Vec<_>>();
    Ok(json!({
        "enabled": enabled,
        "footer": params.footer,
        "suggestions": suggestions,
    }))
}

fn selection_contains_text(selection: &Selection, candidate_text: &str) -> bool {
    let (left, right) = selection.items();
    left.iter().chain(&right).any(|item| {
        matches!(
            item,
            AgentToolbarItemKind::CustomInsert { text, .. } if text == candidate_text
        )
    })
}

fn resolve_suggestion(action: &::local_control::Action) -> Result<serde_json::Value, ControlError> {
    let params = action.params_as::<ToolbeltSuggestionResolveParams>()?;
    let accepted = params.outcome == ToolbeltSuggestionOutcome::Accepted;
    crate::agent_resume::resolve_toolbelt_suggestion(&params.suggestion_id, accepted).map_err(
        |error| {
            let code = match error.kind() {
                std::io::ErrorKind::NotFound => ErrorCode::MissingTarget,
                std::io::ErrorKind::PermissionDenied => ErrorCode::InvalidParams,
                _ => ErrorCode::Internal,
            };
            ControlError::with_details(
                code,
                "could not resolve toolbelt suggestion",
                error.to_string(),
            )
        },
    )?;
    Ok(json!({
        "suggestion_id": params.suggestion_id,
        "outcome": params.outcome,
        "resolved": true,
    }))
}

fn create(
    action: &::local_control::Action,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let params = action.params_as::<ToolbeltButtonCreateParams>()?;
    let label = params.label.trim();
    if label.is_empty() || label.len() > MAX_LABEL_BYTES || label.chars().any(char::is_control) {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            "toolbelt button label must be non-empty, short, and free of control characters",
        ));
    }
    if params.text.len() > MAX_TEXT_BYTES || params.text.chars().any(|c| c == '\0') {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            "toolbelt button text is too large or contains a null byte",
        ));
    }

    let current = read_selection(params.footer, ctx);
    let (mut left, mut right) = current.items();
    let hidden = current.hidden_custom_inserts();
    let candidate = AgentToolbarItemKind::CustomInsert {
        label: label.to_owned(),
        text: params.text,
        auto_send: params.auto_send,
    };
    if left
        .iter()
        .chain(&right)
        .chain(&hidden)
        .chain(current.reserved_items().iter())
        .any(|item| item.has_same_toolbar_identity(&candidate))
    {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            format!("a toolbelt button labeled {label:?} already exists"),
        ));
    }
    insert_at_side(
        &mut left,
        &mut right,
        candidate,
        params.side,
        params.position,
    )?;
    let next = current.rebuild(left, right, hidden);
    save_selection(params.footer, next.clone(), ctx)?;
    Ok(state_result(params.footer, &next))
}

fn delete(
    action: &::local_control::Action,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let params = action.params_as::<ToolbeltButtonDeleteParams>()?;
    let current = read_selection(params.footer, ctx);
    let (mut left, mut right) = current.items();
    let mut hidden = current.hidden_custom_inserts();
    let (side, index) = find_unique_button(&left, &right, &params.label)?;
    let removed = match side {
        ToolbeltSide::Left => left.remove(index),
        ToolbeltSide::Right => right.remove(index),
    };
    if matches!(removed, AgentToolbarItemKind::CustomInsert { .. })
        && !current
            .default_items()
            .iter()
            .any(|item| item.has_same_toolbar_identity(&removed))
    {
        hidden.retain(|item| !item.has_same_toolbar_identity(&removed));
    }
    let next = current.rebuild(left, right, hidden);
    save_selection(params.footer, next.clone(), ctx)?;
    Ok(state_result(params.footer, &next))
}

fn move_button(
    action: &::local_control::Action,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let params = action.params_as::<ToolbeltButtonMoveParams>()?;
    let current = read_selection(params.footer, ctx);
    let (mut left, mut right) = current.items();
    let hidden = current.hidden_custom_inserts();
    let (side, index) = find_unique_button(&left, &right, &params.label)?;
    let button = match side {
        ToolbeltSide::Left => left.remove(index),
        ToolbeltSide::Right => right.remove(index),
    };
    insert_at_side(
        &mut left,
        &mut right,
        button,
        params.side,
        Some(params.position),
    )?;
    let next = current.rebuild(left, right, hidden);
    save_selection(params.footer, next.clone(), ctx)?;
    Ok(state_result(params.footer, &next))
}

fn insert_at_side(
    left: &mut Vec<AgentToolbarItemKind>,
    right: &mut Vec<AgentToolbarItemKind>,
    button: AgentToolbarItemKind,
    side: ToolbeltSide,
    position: Option<u32>,
) -> Result<(), ControlError> {
    let items = match side {
        ToolbeltSide::Left => left,
        ToolbeltSide::Right => right,
    };
    let position = position
        .map(|position| usize::try_from(position).unwrap_or(usize::MAX))
        .unwrap_or(items.len());
    if position > items.len() {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            format!(
                "toolbelt position {position} is outside the selected side's 0..={} range",
                items.len()
            ),
        ));
    }
    items.insert(position, button);
    Ok(())
}

fn find_unique_button(
    left: &[AgentToolbarItemKind],
    right: &[AgentToolbarItemKind],
    label: &str,
) -> Result<(ToolbeltSide, usize), ControlError> {
    let matches = left
        .iter()
        .enumerate()
        .filter(|(_, item)| item.display_label() == label)
        .map(|(index, _)| (ToolbeltSide::Left, index))
        .chain(
            right
                .iter()
                .enumerate()
                .filter(|(_, item)| item.display_label() == label)
                .map(|(index, _)| (ToolbeltSide::Right, index)),
        )
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [target] => Ok(*target),
        [] => Err(ControlError::new(
            ErrorCode::MissingTarget,
            format!("no visible toolbelt button is labeled {label:?}"),
        )),
        _ => Err(ControlError::new(
            ErrorCode::AmbiguousTarget,
            format!("multiple visible toolbelt buttons are labeled {label:?}"),
        )),
    }
}

fn read_selection(footer: ToolbeltFooter, ctx: &ModelContext<LocalControlBridge>) -> Selection {
    let settings = SessionSettings::as_ref(ctx);
    match footer {
        ToolbeltFooter::ClaudeCode => {
            Selection::Cli(settings.claude_code_footer_chip_selection_value().clone())
        }
        ToolbeltFooter::Codex => {
            Selection::Cli(settings.codex_footer_chip_selection_value().clone())
        }
        ToolbeltFooter::Terminal => {
            Selection::Terminal(settings.terminal_footer_chip_selection.value().clone())
        }
    }
}

fn save_selection(
    footer: ToolbeltFooter,
    selection: Selection,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<(), ControlError> {
    SessionSettings::handle(ctx).update(ctx, |settings, ctx| {
        let result = match (footer, selection) {
            (ToolbeltFooter::ClaudeCode, Selection::Cli(selection)) => settings
                .claude_code_footer_chip_selection
                .set_value(Some(selection), ctx),
            (ToolbeltFooter::Codex, Selection::Cli(selection)) => settings
                .codex_footer_chip_selection
                .set_value(Some(selection), ctx),
            (ToolbeltFooter::Terminal, Selection::Terminal(selection)) => settings
                .terminal_footer_chip_selection
                .set_value(selection, ctx),
            _ => unreachable!("footer selection type must match its setting"),
        };
        result.map_err(|error| {
            ControlError::with_details(
                ErrorCode::Internal,
                "failed to persist toolbelt selection",
                error.to_string(),
            )
        })
    })
}

fn state_result(footer: ToolbeltFooter, selection: &Selection) -> serde_json::Value {
    let (left, right) = selection.items();
    let shipped = selection.reserved_items();
    json!({
        "footer": footer,
        "left": button_summaries(&left, &shipped),
        "right": button_summaries(&right, &shipped),
    })
}

fn button_summaries(
    items: &[AgentToolbarItemKind],
    shipped: &[AgentToolbarItemKind],
) -> Vec<serde_json::Value> {
    items
        .iter()
        .enumerate()
        .map(|(position, item)| {
            let source = if shipped
                .iter()
                .any(|shipped| shipped.has_same_toolbar_identity(item))
            {
                "shipped"
            } else {
                "custom"
            };
            match item {
                AgentToolbarItemKind::CustomInsert {
                    label,
                    text,
                    auto_send,
                } => json!({
                    "label": label,
                    "kind": "quick_insert",
                    "source": source,
                    "text": text,
                    "auto_send": auto_send,
                    "position": position,
                }),
                _ => json!({
                    "label": item.display_label(),
                    "kind": "built_in",
                    "source": source,
                    "position": position,
                }),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn button(label: &str) -> AgentToolbarItemKind {
        AgentToolbarItemKind::custom_insert(label, format!("{label} prompt"))
    }

    #[test]
    fn insert_uses_exact_side_position() {
        let mut left = vec![button("A"), button("C")];
        let mut right = vec![];

        insert_at_side(
            &mut left,
            &mut right,
            button("B"),
            ToolbeltSide::Left,
            Some(1),
        )
        .expect("position is valid");

        assert_eq!(
            left.iter()
                .map(|item| item.display_label().into_owned())
                .collect::<Vec<_>>(),
            ["A", "B", "C"]
        );
    }

    #[test]
    fn insertion_rejects_out_of_range_position() {
        let mut left = vec![button("A")];
        let mut right = vec![];

        let error = insert_at_side(
            &mut left,
            &mut right,
            button("B"),
            ToolbeltSide::Left,
            Some(2),
        )
        .expect_err("position past the end is rejected");

        assert_eq!(error.code, ErrorCode::InvalidParams);
    }

    #[test]
    fn deleting_shipped_button_rebuilds_as_hidden_default() {
        let current = Selection::Cli(CLIAgentToolbarChipSelection::Default);
        let (mut left, right) = current.items();
        let removed = left.remove(0);
        let next = current.rebuild(left, right, Vec::new());
        let (next_left, next_right) = next.items();

        assert!(!next_left
            .iter()
            .chain(&next_right)
            .any(|item| item.has_same_toolbar_identity(&removed)));
    }

    #[test]
    fn label_selector_rejects_ambiguous_buttons() {
        let left = vec![button("Review")];
        let right = vec![button("Review")];

        let error = find_unique_button(&left, &right, "Review")
            .expect_err("duplicate labels are ambiguous");

        assert_eq!(error.code, ErrorCode::AmbiguousTarget);
    }

    #[test]
    fn shipped_quick_insert_labels_are_reserved() {
        let selection = Selection::Cli(CLIAgentToolbarChipSelection::Default);
        let candidate = button("Create a Plan");

        assert!(selection
            .reserved_items()
            .iter()
            .any(|item| item.has_same_toolbar_identity(&candidate)));
    }

    #[test]
    fn existing_quick_insert_text_suppresses_learned_candidate() {
        let selection = Selection::Cli(
            CLIAgentToolbarChipSelection::custom_from_effective_items_and_hidden_custom_inserts(
                vec![AgentToolbarItemKind::CustomInsert {
                    label: "Serve".to_owned(),
                    text: "Run the local server".to_owned(),
                    auto_send: false,
                }],
                Vec::new(),
                Vec::new(),
            ),
        );

        assert!(selection_contains_text(&selection, "Run the local server"));
        assert!(!selection_contains_text(&selection, "Run the test suite"));
    }
}
