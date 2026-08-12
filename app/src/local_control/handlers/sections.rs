//! Typed sidebar-section management backed by the workspace tab-group model.

use std::collections::HashSet;
use std::str::FromStr as _;

use ::local_control::protocol::{
    Direction, SectionCreateParams, SectionIdParams, SectionMoveParams, SectionUpdateParams,
    TargetSelector,
};
use ::local_control::{ActionKind, ControlError, ErrorCode};
use serde_json::json;
use uuid::Uuid;
use warp_core::features::FeatureFlag;
use warp_core::ui::theme::AnsiColorIdentifier;
use warpui::{AppContext, ModelContext};

use crate::local_control::resolver::{
    reject_target_families, tab_index_from_target, target_workspace,
};
use crate::local_control::LocalControlBridge;
use crate::tab::SelectedTabColor;
use crate::workspace::tab_group::TabGroupId;
use crate::workspace::{TabMovement, Workspace};

const MAX_SECTION_NAME_BYTES: usize = 256;

pub(crate) fn handle(
    action: &::local_control::Action,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    if !FeatureFlag::GroupedTabs.is_enabled() {
        return Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            "sidebar sections are disabled in this Clinch build",
        ));
    }
    match action.kind {
        ActionKind::SectionList => list(target, ctx),
        ActionKind::SectionCreate => create(action, target, ctx),
        ActionKind::SectionUpdate => update(action, target, ctx),
        ActionKind::SectionDelete => delete(action, target, ctx),
        ActionKind::SectionMove => move_section(action, target, ctx),
        ActionKind::SectionTabAdd => add_tab(action, target, ctx),
        ActionKind::SectionTabRemove => remove_tab(target, ctx),
        _ => Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            format!("{} is not a section action", action.kind.as_str()),
        )),
    }
}

fn list(
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    reject_lower_targets(ActionKind::SectionList, target, false)?;
    let workspace = target_workspace(ActionKind::SectionList, target, ctx)?;
    Ok(workspace.read(ctx, section_state))
}

fn create(
    action: &::local_control::Action,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    reject_lower_targets(ActionKind::SectionCreate, target, true)?;
    let SectionCreateParams { name } = action.params_as()?;
    let name = validated_name(name)?;
    let workspace = target_workspace(ActionKind::SectionCreate, target, ctx)?;
    workspace.update(ctx, |workspace, ctx| {
        let tab_index = tab_index_from_target(target, workspace, ctx)?;
        workspace
            .create_named_tab_group_from_tab(tab_index, name, ctx)
            .ok_or_else(|| {
                ControlError::new(
                    ErrorCode::Internal,
                    "section.create could not create a section from the selected tab",
                )
            })?;
        Ok::<_, ControlError>(section_state(workspace, ctx))
    })
}

fn update(
    action: &::local_control::Action,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    reject_lower_targets(ActionKind::SectionUpdate, target, false)?;
    let params = action.params_as::<SectionUpdateParams>()?;
    if params.name.is_none() && params.collapsed.is_none() && params.color.is_none() {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            "section.update requires --name, --collapsed, or --color",
        ));
    }
    let section_id = parse_section_id(&params.section_id)?;
    let name = params.name.map(validated_name).transpose()?;
    let color = params.color.map(parse_color).transpose()?;
    let workspace = target_workspace(ActionKind::SectionUpdate, target, ctx)?;
    workspace.update(ctx, |workspace, ctx| {
        let group = workspace.tab_groups.get_mut(&section_id).ok_or_else(|| {
            ControlError::new(
                ErrorCode::StaleTarget,
                "section id is not present in this window",
            )
        })?;
        if let Some(name) = name {
            group.name = Some(name);
        }
        if let Some(collapsed) = params.collapsed {
            group.collapsed = collapsed;
        }
        if let Some(color) = color {
            group.color = color;
        }
        ctx.dispatch_global_action("workspace:save_app", ());
        ctx.notify();
        Ok::<_, ControlError>(section_state(workspace, ctx))
    })
}

fn delete(
    action: &::local_control::Action,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    reject_lower_targets(ActionKind::SectionDelete, target, false)?;
    let section_id = parse_section_id(&action.params_as::<SectionIdParams>()?.section_id)?;
    let workspace = target_workspace(ActionKind::SectionDelete, target, ctx)?;
    workspace.update(ctx, |workspace, ctx| {
        require_section(workspace, section_id)?;
        workspace.ungroup_tabs(section_id, ctx);
        Ok::<_, ControlError>(section_state(workspace, ctx))
    })
}

fn move_section(
    action: &::local_control::Action,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    reject_lower_targets(ActionKind::SectionMove, target, false)?;
    let params = action.params_as::<SectionMoveParams>()?;
    let section_id = parse_section_id(&params.section_id)?;
    let direction = match params.direction {
        Direction::Up | Direction::Previous | Direction::Left => TabMovement::Left,
        Direction::Down | Direction::Next | Direction::Right => TabMovement::Right,
    };
    let workspace = target_workspace(ActionKind::SectionMove, target, ctx)?;
    workspace.update(ctx, |workspace, ctx| {
        require_section(workspace, section_id)?;
        if !workspace.can_move_tab_group(section_id, direction) {
            return Err(ControlError::new(
                ErrorCode::InvalidParams,
                "section cannot move farther in that direction",
            ));
        }
        workspace.move_tab_group(section_id, direction, ctx);
        Ok::<_, ControlError>(section_state(workspace, ctx))
    })
}

fn add_tab(
    action: &::local_control::Action,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    reject_lower_targets(ActionKind::SectionTabAdd, target, true)?;
    let section_id = parse_section_id(&action.params_as::<SectionIdParams>()?.section_id)?;
    let workspace = target_workspace(ActionKind::SectionTabAdd, target, ctx)?;
    workspace.update(ctx, |workspace, ctx| {
        require_section(workspace, section_id)?;
        let tab_index = tab_index_from_target(target, workspace, ctx)?;
        workspace.move_tab_to_group(tab_index, section_id, ctx);
        Ok::<_, ControlError>(section_state(workspace, ctx))
    })
}

fn remove_tab(
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    reject_lower_targets(ActionKind::SectionTabRemove, target, true)?;
    let workspace = target_workspace(ActionKind::SectionTabRemove, target, ctx)?;
    workspace.update(ctx, |workspace, ctx| {
        let tab_index = tab_index_from_target(target, workspace, ctx)?;
        if workspace.tabs[tab_index].group_id.is_none() {
            return Err(ControlError::new(
                ErrorCode::InvalidParams,
                "selected tab is not in a sidebar section",
            ));
        }
        workspace.remove_tab_from_group(tab_index, ctx);
        Ok::<_, ControlError>(section_state(workspace, ctx))
    })
}

fn reject_lower_targets(
    action: ActionKind,
    target: &TargetSelector,
    allow_tab: bool,
) -> Result<(), ControlError> {
    reject_target_families(
        action,
        (!allow_tab && target.tab.is_some()) || target.pane.is_some() || target.session.is_some(),
        if allow_tab {
            "pane or session selectors"
        } else {
            "tab, pane, or session selectors"
        },
    )
}

fn parse_section_id(value: &str) -> Result<TabGroupId, ControlError> {
    Uuid::parse_str(value).map(TabGroupId).map_err(|_| {
        ControlError::new(
            ErrorCode::InvalidParams,
            format!("{value:?} is not a valid section id"),
        )
    })
}

fn validated_name(name: String) -> Result<String, ControlError> {
    let name = name.trim();
    if name.is_empty() || name.len() > MAX_SECTION_NAME_BYTES || name.chars().any(char::is_control)
    {
        Err(ControlError::new(
            ErrorCode::InvalidParams,
            "section name must be non-empty, short, and free of control characters",
        ))
    } else {
        Ok(name.to_owned())
    }
}

fn parse_color(color: String) -> Result<SelectedTabColor, ControlError> {
    if matches!(color.to_ascii_lowercase().as_str(), "default" | "unset") {
        return Ok(SelectedTabColor::Unset);
    }
    AnsiColorIdentifier::from_str(&color)
        .map(SelectedTabColor::Color)
        .map_err(|_| {
            ControlError::new(
                ErrorCode::InvalidParams,
                format!("{color:?} is not an ANSI color or `default`"),
            )
        })
}

fn require_section(workspace: &Workspace, section_id: TabGroupId) -> Result<(), ControlError> {
    if workspace.tab_groups.contains_key(&section_id) {
        Ok(())
    } else {
        Err(ControlError::new(
            ErrorCode::StaleTarget,
            "section id is not present in this window",
        ))
    }
}

fn section_state(workspace: &Workspace, _ctx: &AppContext) -> serde_json::Value {
    let mut seen = HashSet::new();
    let mut ordered_ids = workspace
        .tabs
        .iter()
        .filter_map(|tab| tab.group_id)
        .filter(|group_id| seen.insert(*group_id))
        .collect::<Vec<_>>();
    ordered_ids.extend(
        workspace
            .tab_groups
            .keys()
            .copied()
            .filter(|group_id| seen.insert(*group_id)),
    );
    let sections = ordered_ids
        .into_iter()
        .enumerate()
        .filter_map(|(position, section_id)| {
            let group = workspace.tab_groups.get(&section_id)?;
            let tab_ids = workspace
                .tabs
                .iter()
                .filter(|tab| tab.group_id == Some(section_id))
                .map(|tab| tab.pane_group.id().to_string())
                .collect::<Vec<_>>();
            let color = match group.color {
                SelectedTabColor::Unset => None,
                SelectedTabColor::Cleared => Some("none".to_owned()),
                SelectedTabColor::Color(color) => Some(color.to_string().to_ascii_lowercase()),
            };
            Some(json!({
                "section_id": section_id.0.to_string(),
                "position": position,
                "name": group.name.clone(),
                "collapsed": group.collapsed,
                "pinned": group.pinned,
                "color": color,
                "tab_ids": tab_ids,
            }))
        })
        .collect::<Vec<_>>();
    json!({ "sections": sections })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_names_are_trimmed_and_cannot_be_empty() {
        assert_eq!(validated_name("  Backend  ".to_owned()).unwrap(), "Backend");
        assert_eq!(
            validated_name("   ".to_owned()).unwrap_err().code,
            ErrorCode::InvalidParams
        );
    }

    #[test]
    fn default_color_clears_the_section_tint() {
        assert_eq!(
            parse_color("default".to_owned()).unwrap(),
            SelectedTabColor::Unset
        );
        assert_eq!(
            parse_color("magenta".to_owned()).unwrap(),
            SelectedTabColor::Color(AnsiColorIdentifier::Magenta)
        );
    }
}
