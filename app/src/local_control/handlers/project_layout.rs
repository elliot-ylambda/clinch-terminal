//! Bounded, declarative project snapshots. Imported files never contain executable shell source.
use std::collections::{HashMap, HashSet};
use std::path::Path;

use ::local_control::projects::{
    AgentResume, PaneLayout, ProjectLayout, ResumeProvider, SectionLayout, SplitDirection,
    TabLayout, MAX_PROJECT_LAYOUT_BYTES, PROJECT_LAYOUT_VERSION,
};
use ::local_control::{ControlError, ErrorCode};
use uuid::Uuid;

use super::super::sections::{color_value, parse_color};
use super::{parse_tab_color, tab_color_value};
use crate::agent_resume::{
    agent_session_seed_from_restore_command, resolve_on_restore_command, AgentResumeProvider,
};
use crate::app_state::{
    self, BranchSnapshot, LeafContents, LeafSnapshot, PaneFlex, PaneNodeSnapshot, TabGroupSnapshot,
    TabSnapshot, TerminalPaneSnapshot, WindowSnapshot,
};
use crate::workspace::tab_group::{SelectedSectionColor, TabGroupId};
use crate::workspace::task::WorkspaceTask;

fn invalid(message: &str) -> ControlError {
    ControlError::new(ErrorCode::InvalidParams, message)
}

fn valid_cwd(cwd: &Option<String>) -> Result<(), ControlError> {
    if let Some(cwd) = cwd {
        if cwd.len() > 4096
            || cwd.chars().any(char::is_control)
            || !Path::new(cwd).is_absolute()
            || !Path::new(cwd).is_dir()
        {
            return Err(invalid("cwd must be an existing absolute local directory"));
        }
    }
    Ok(())
}

fn valid_title(title: &Option<String>) -> Result<(), ControlError> {
    if title
        .as_ref()
        .is_some_and(|title| title.len() > 4096 || title.chars().any(char::is_control))
    {
        return Err(invalid(
            "titles must be at most 4096 bytes and contain no control characters",
        ));
    }
    Ok(())
}

pub(super) fn new_project(cwd: String) -> Result<WindowSnapshot, ControlError> {
    restore(
        ProjectLayout {
            version: PROJECT_LAYOUT_VERSION,
            active_tab: 0,
            sections: vec![],
            tasks: vec![],
            tabs: vec![TabLayout {
                title: None,
                color: None,
                pinned: false,
                section: None,
                panes: PaneLayout::Terminal {
                    cwd: Some(cwd),
                    title: None,
                    focused: true,
                    resume: None,
                },
            }],
        },
        false,
    )
}

pub(super) fn export(snapshot: WindowSnapshot) -> Result<ProjectLayout, ControlError> {
    let mut seen = HashSet::new();
    let sections_by_id: HashMap<_, _> = snapshot
        .tab_groups
        .into_iter()
        .map(|group| (group.id, group))
        .collect();
    let mut sections = Vec::new();
    let mut tabs = Vec::new();
    for tab in snapshot.tabs {
        if let Some(id) = tab.group_id {
            if seen.insert(id) {
                let group = sections_by_id
                    .get(&id)
                    .ok_or_else(|| invalid("tab references a missing section"))?;
                sections.push(SectionLayout {
                    id: id.0.to_string(),
                    name: group.name.clone(),
                    color: color_value(group.color),
                    collapsed: group.collapsed,
                    pinned: group.pinned,
                });
            }
        }
        tabs.push(TabLayout {
            title: tab.custom_title,
            color: tab_color_value(tab.selected_color),
            pinned: tab.pinned,
            section: tab.group_id.map(|id| id.0.to_string()),
            panes: export_pane(tab.root)?,
        });
    }
    let layout = ProjectLayout {
        version: PROJECT_LAYOUT_VERSION,
        active_tab: snapshot.active_tab_index,
        sections,
        tabs,
        tasks: snapshot.tasks.into_iter().map(|task| task.text).collect(),
    };
    check_size(&layout)?;
    Ok(layout)
}

fn export_pane(node: PaneNodeSnapshot) -> Result<PaneLayout, ControlError> {
    match node {
        PaneNodeSnapshot::Branch(branch) => {
            let (weights, children): (Vec<_>, Vec<_>) = branch.children.into_iter()
                .map(|(flex, child)| (flex.0, child)).unzip();
            Ok(PaneLayout::Split {
                direction: match branch.direction {
                    app_state::SplitDirection::Horizontal => SplitDirection::Horizontal,
                    app_state::SplitDirection::Vertical => SplitDirection::Vertical,
                },
                children: children.into_iter().map(export_pane).collect::<Result<_, _>>()?,
                weights,
            })
        }
        PaneNodeSnapshot::Leaf(LeafSnapshot {
            is_focused,
            custom_vertical_tabs_title,
            contents: LeafContents::Terminal(terminal),
        }) => {
            let resume = resolve_on_restore_command(&terminal.uuid, terminal.on_restore_command)
                .and_then(|command| agent_session_seed_from_restore_command(&command))
                .map(|(provider, conversation_id)| AgentResume {
                    provider: match provider {
                        AgentResumeProvider::Claude => ResumeProvider::Claude,
                        AgentResumeProvider::Codex => ResumeProvider::Codex,
                    },
                    conversation_id,
                });
            Ok(PaneLayout::Terminal {
                cwd: terminal.cwd,
                title: custom_vertical_tabs_title,
                focused: is_focused,
                resume,
            })
        }
        PaneNodeSnapshot::Leaf(_) => Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            "portable project export currently supports terminal and Claude/Codex panes; this project also contains another pane type",
        )),
    }
}

fn check_size(layout: &ProjectLayout) -> Result<(), ControlError> {
    let size = serde_json::to_vec(layout)
        .map_err(|_| invalid("layout cannot be serialized"))?
        .len();
    if size > MAX_PROJECT_LAYOUT_BYTES {
        return Err(invalid("project layout exceeds 1 MiB"));
    }
    Ok(())
}

pub(super) fn restore(
    layout: ProjectLayout,
    resume_agents: bool,
) -> Result<WindowSnapshot, ControlError> {
    check_size(&layout)?;
    if layout.version != PROJECT_LAYOUT_VERSION {
        return Err(invalid("unsupported project layout version"));
    }
    if layout.tabs.is_empty()
        || layout.tabs.len() > 128
        || layout.active_tab >= layout.tabs.len()
        || layout.sections.len() > 128
        || layout.tasks.len() > 256
    {
        return Err(invalid(
            "layout requires 1–128 tabs, a valid active tab, at most 128 sections and 256 tasks",
        ));
    }
    let mut groups = HashMap::new();
    let mut tab_groups = Vec::new();
    for section in layout.sections {
        if section.id.is_empty() || section.id.len() > 128 || groups.contains_key(&section.id) {
            return Err(invalid("section IDs must be nonempty and unique"));
        }
        valid_title(&section.name)?;
        let id = TabGroupId(Uuid::new_v4());
        groups.insert(section.id, (id, section.pinned));
        tab_groups.push(TabGroupSnapshot {
            id,
            name: section.name,
            color: section
                .color
                .map(parse_color)
                .transpose()?
                .unwrap_or_default(),
            collapsed: section.collapsed,
            pinned: section.pinned,
        });
    }
    let mut tabs = Vec::new();
    let mut completed_sections = HashSet::new();
    let mut previous_section = None;
    let mut reached_unpinned = false;
    let mut total_panes = 0;
    for tab in layout.tabs {
        valid_title(&tab.title)?;
        let group = tab
            .section
            .as_ref()
            .map(|id| {
                groups
                    .get(id)
                    .copied()
                    .ok_or_else(|| invalid("tab references an unknown section"))
            })
            .transpose()?;
        if tab.pinned && group.is_some() {
            return Err(invalid(
                "a tab inside a section cannot be individually pinned",
            ));
        }
        let pinned = tab.pinned || group.is_some_and(|(_, pinned)| pinned);
        if reached_unpinned && pinned {
            return Err(invalid(
                "pinned tabs and sections must precede unpinned items",
            ));
        }
        reached_unpinned |= !pinned;
        let section = group.map(|(id, _)| id);
        if section != previous_section {
            if let Some(previous) = previous_section {
                completed_sections.insert(previous);
            }
            if section.is_some_and(|id| completed_sections.contains(&id)) {
                return Err(invalid("section members must be contiguous"));
            }
            previous_section = section;
        }
        let mut focused = 0;
        let root = restore_pane(tab.panes, resume_agents, 0, &mut total_panes, &mut focused)?;
        if focused > 1 {
            return Err(invalid("a tab can have at most one focused pane"));
        }
        tabs.push(TabSnapshot {
            origin: None,
            custom_title: tab.title,
            root,
            default_directory_color: None,
            selected_color: parse_tab_color(tab.color)?,
            left_panel: None,
            right_panel: None,
            group_id: section,
            pinned: tab.pinned,
        });
    }
    if tab_groups
        .iter()
        .any(|group| !tabs.iter().any(|tab| tab.group_id == Some(group.id)))
    {
        return Err(invalid("sections must contain at least one tab"));
    }
    let tasks = layout
        .tasks
        .into_iter()
        .map(|text| WorkspaceTask::new(text).ok_or_else(|| invalid("invalid project task text")))
        .collect::<Result<_, _>>()?;
    Ok(WindowSnapshot {
        tabs,
        active_tab_index: layout.active_tab,
        bounds: None,
        fullscreen_state: Default::default(),
        quake_mode: false,
        universal_search_width: None,
        warp_ai_width: None,
        voltron_width: None,
        warp_drive_index_width: None,
        left_panel_open: false,
        vertical_tabs_panel_open: true,
        vertical_tabs_panel_width: None,
        left_panel_width: None,
        right_panel_width: None,
        agent_management_filters: None,
        tab_groups,
        tasks,
        tasks_collapsed: false,
        bookmarked_sessions_color: SelectedSectionColor::Unset,
    })
}

fn restore_pane(
    node: PaneLayout,
    resume_agents: bool,
    depth: usize,
    total: &mut usize,
    focused: &mut usize,
) -> Result<PaneNodeSnapshot, ControlError> {
    *total += 1;
    if depth > 16 || *total > 512 {
        return Err(invalid("layout exceeds 16 split levels or 512 pane nodes"));
    }
    match node {
        PaneLayout::Split {
            direction,
            children,
            weights,
        } => {
            if children.len() < 2
                || children.len() != weights.len()
                || weights
                    .iter()
                    .any(|weight| !weight.is_finite() || *weight <= 0.0)
                || !weights.iter().sum::<f32>().is_finite()
            {
                return Err(invalid(
                    "splits require at least two children with matching positive finite weights",
                ));
            }
            Ok(PaneNodeSnapshot::Branch(BranchSnapshot {
                direction: match direction {
                    SplitDirection::Horizontal => app_state::SplitDirection::Horizontal,
                    SplitDirection::Vertical => app_state::SplitDirection::Vertical,
                },
                children: children
                    .into_iter()
                    .zip(weights)
                    .map(|(child, weight)| {
                        Ok((
                            PaneFlex(weight),
                            restore_pane(child, resume_agents, depth + 1, total, focused)?,
                        ))
                    })
                    .collect::<Result<_, ControlError>>()?,
            }))
        }
        PaneLayout::Terminal {
            cwd,
            title,
            focused: is_focused,
            resume,
        } => {
            valid_cwd(&cwd)?;
            valid_title(&title)?;
            *focused += usize::from(is_focused);
            let on_restore_command = resume
                .map(|resume| {
                    let id = resume.conversation_id;
                    if id.is_empty()
                        || id.len() > 256
                        || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
                    {
                        return Err(invalid("invalid agent conversation ID"));
                    }
                    Ok(format!(
                        "clinch_agent_resume_launch {} {}",
                        match resume.provider {
                            ResumeProvider::Claude => "claude",
                            ResumeProvider::Codex => "codex",
                        },
                        id
                    ))
                })
                .transpose()?
                .filter(|_| resume_agents);
            Ok(PaneNodeSnapshot::Leaf(LeafSnapshot {
                is_focused,
                custom_vertical_tabs_title: title,
                contents: LeafContents::Terminal(TerminalPaneSnapshot {
                    uuid: Uuid::new_v4().as_bytes().to_vec(),
                    cwd,
                    shell_launch_data: None,
                    is_active: true,
                    is_read_only: false,
                    input_config: None,
                    llm_model_override: None,
                    active_profile_id: None,
                    conversation_ids_to_restore: vec![],
                    active_conversation_id: None,
                    on_restore_command,
                }),
            }))
        }
    }
}
