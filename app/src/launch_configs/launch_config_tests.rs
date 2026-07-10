use std::path::PathBuf;

use super::{LaunchConfig, PaneMode, PaneTemplateType, ProjectWindowTemplate, WindowTemplate};
use crate::app_state::{
    AppState, BranchSnapshot, LeafContents, LeafSnapshot, NotebookPaneSnapshot, PaneFlex,
    PaneNodeSnapshot, ProjectWindowSnapshot, SplitDirection, TabSnapshot, TerminalPaneSnapshot,
    WindowSnapshot,
};
use crate::drive::OpenWarpDriveObjectSettings;
use crate::tab::SelectedTabColor;

fn single_tab_snapshot(root: PaneNodeSnapshot) -> AppState {
    AppState {
        windows: vec![ProjectWindowSnapshot::singleton(WindowSnapshot {
            tabs: vec![TabSnapshot {
                custom_title: None,
                default_directory_color: None,
                selected_color: SelectedTabColor::default(),
                root,
                left_panel: None,
                right_panel: None,
                group_id: None,
                pinned: false,
            }],
            active_tab_index: 0,
            bounds: None,
            quake_mode: false,
            universal_search_width: None,
            warp_ai_width: None,
            voltron_width: None,
            warp_drive_index_width: None,
            left_panel_open: false,
            vertical_tabs_panel_open: false,
            fullscreen_state: Default::default(),
            left_panel_width: None,
            right_panel_width: None,
            agent_management_filters: None,
            tab_groups: vec![],
        })],
        active_window_index: Some(0),
        block_lists: Default::default(),
        running_mcp_servers: Default::default(),
    }
}

fn multi_tab_snapshot(active_tab_index: usize, tabs: Vec<TabSnapshot>) -> AppState {
    AppState {
        windows: vec![ProjectWindowSnapshot::singleton(WindowSnapshot {
            tabs,
            active_tab_index,
            bounds: None,
            quake_mode: false,
            universal_search_width: None,
            warp_ai_width: None,
            voltron_width: None,
            warp_drive_index_width: None,
            left_panel_open: false,
            vertical_tabs_panel_open: false,
            fullscreen_state: Default::default(),
            left_panel_width: None,
            right_panel_width: None,
            agent_management_filters: None,
            tab_groups: vec![],
        })],
        active_window_index: Some(0),
        block_lists: Default::default(),
        running_mcp_servers: Default::default(),
    }
}

fn first_active_project(config: &LaunchConfig) -> &WindowTemplate {
    config.windows[0]
        .active_project()
        .expect("launch config window should contain a project")
}

#[test]
fn test_config_from_snapshot_flattens_single_pane() {
    // If only one pane of the branch can be saved into a launch configuration, it should
    // be flattened to a single leaf.

    let state = single_tab_snapshot(PaneNodeSnapshot::Branch(BranchSnapshot {
        direction: SplitDirection::Vertical,
        children: vec![
            (
                PaneFlex(1.),
                PaneNodeSnapshot::Leaf(LeafSnapshot {
                    is_focused: true,
                    custom_vertical_tabs_title: None,
                    contents: LeafContents::Notebook(NotebookPaneSnapshot::CloudNotebook {
                        notebook_id: None,
                        settings: OpenWarpDriveObjectSettings::default(),
                    }),
                }),
            ),
            (
                PaneFlex(1.),
                PaneNodeSnapshot::Leaf(LeafSnapshot {
                    is_focused: true,
                    custom_vertical_tabs_title: None,
                    contents: LeafContents::Terminal(TerminalPaneSnapshot {
                        uuid: vec![],
                        cwd: Some("/some/dir".into()),
                        is_active: true,
                        is_read_only: false,
                        shell_launch_data: None,
                        input_config: None,
                        llm_model_override: None,
                        active_profile_id: None,
                        conversation_ids_to_restore: vec![],
                        active_conversation_id: None,
                        on_restore_command: None,
                    }),
                }),
            ),
        ],
    }));

    let template = LaunchConfig::from_snapshot("Test".into(), &state);
    assert_eq!(
        first_active_project(&template).tabs[0].layout,
        PaneTemplateType::PaneTemplate {
            is_focused: Some(true),
            cwd: PathBuf::from("/some/dir"),
            commands: vec![],
            pane_mode: PaneMode::Terminal,
            shell: None,
        },
    )
}

#[test]
fn test_config_from_snapshot_filters_panes() {
    let state = single_tab_snapshot(PaneNodeSnapshot::Branch(BranchSnapshot {
        direction: SplitDirection::Vertical,
        children: vec![
            (
                PaneFlex(1.),
                PaneNodeSnapshot::Leaf(LeafSnapshot {
                    is_focused: true,
                    custom_vertical_tabs_title: None,
                    contents: LeafContents::Terminal(TerminalPaneSnapshot {
                        uuid: vec![],
                        cwd: Some("/path/to/dir".into()),
                        is_active: true,
                        is_read_only: false,
                        shell_launch_data: None,
                        input_config: None,
                        llm_model_override: None,
                        active_profile_id: None,
                        conversation_ids_to_restore: vec![],
                        active_conversation_id: None,
                        on_restore_command: None,
                    }),
                }),
            ),
            (
                PaneFlex(1.),
                PaneNodeSnapshot::Leaf(LeafSnapshot {
                    is_focused: false,
                    custom_vertical_tabs_title: None,
                    contents: LeafContents::Notebook(NotebookPaneSnapshot::CloudNotebook {
                        notebook_id: None,
                        settings: OpenWarpDriveObjectSettings::default(),
                    }),
                }),
            ),
            (
                PaneFlex(1.),
                PaneNodeSnapshot::Leaf(LeafSnapshot {
                    is_focused: false,
                    custom_vertical_tabs_title: None,
                    contents: LeafContents::Terminal(TerminalPaneSnapshot {
                        uuid: vec![],
                        cwd: Some("/some/dir".into()),
                        is_active: true,
                        is_read_only: false,
                        shell_launch_data: None,
                        input_config: None,
                        llm_model_override: None,
                        active_profile_id: None,
                        conversation_ids_to_restore: vec![],
                        active_conversation_id: None,
                        on_restore_command: None,
                    }),
                }),
            ),
        ],
    }));

    let template = LaunchConfig::from_snapshot("Test".into(), &state);
    assert_eq!(
        first_active_project(&template).tabs[0].layout,
        PaneTemplateType::PaneBranchTemplate {
            split_direction: SplitDirection::Vertical.into(),
            panes: vec![
                PaneTemplateType::PaneTemplate {
                    is_focused: Some(true),
                    cwd: PathBuf::from("/path/to/dir"),
                    commands: vec![],
                    pane_mode: PaneMode::Terminal,
                    shell: None,
                },
                PaneTemplateType::PaneTemplate {
                    is_focused: Some(false),
                    cwd: PathBuf::from("/some/dir"),
                    commands: vec![],
                    pane_mode: PaneMode::Terminal,
                    shell: None,
                },
            ]
        }
    )
}

#[test]
fn test_config_from_snapshot_filters_tabs() {
    // If no panes of a tab are valid, it's filtered out entirely.

    let state = single_tab_snapshot(PaneNodeSnapshot::Branch(BranchSnapshot {
        direction: SplitDirection::Vertical,
        children: vec![(
            PaneFlex(1.),
            PaneNodeSnapshot::Leaf(LeafSnapshot {
                is_focused: true,
                custom_vertical_tabs_title: None,
                contents: LeafContents::Notebook(NotebookPaneSnapshot::CloudNotebook {
                    notebook_id: None,
                    settings: OpenWarpDriveObjectSettings::default(),
                }),
            }),
        )],
    }));

    let template = LaunchConfig::from_snapshot("Test".into(), &state);
    assert!(first_active_project(&template).tabs.is_empty())
}

#[test]
fn test_config_with_active_tab_index() {
    let state = multi_tab_snapshot(
        1,
        vec![
            TabSnapshot {
                custom_title: None,
                default_directory_color: None,
                selected_color: SelectedTabColor::default(),
                root: PaneNodeSnapshot::Branch(BranchSnapshot {
                    direction: SplitDirection::Vertical,
                    children: vec![(
                        PaneFlex(1.),
                        PaneNodeSnapshot::Leaf(LeafSnapshot {
                            is_focused: true,
                            custom_vertical_tabs_title: None,
                            contents: LeafContents::Terminal(TerminalPaneSnapshot {
                                uuid: vec![],
                                cwd: Some("/path/to/dir".into()),
                                is_active: true,
                                is_read_only: false,
                                shell_launch_data: None,
                                input_config: None,
                                llm_model_override: None,
                                active_profile_id: None,
                                conversation_ids_to_restore: vec![],
                                active_conversation_id: None,
                                on_restore_command: None,
                            }),
                        }),
                    )],
                }),
                left_panel: None,
                right_panel: None,
                group_id: None,
                pinned: false,
            };
            3
        ],
    );

    let template = LaunchConfig::from_snapshot("Test".into(), &state);
    assert_eq!(first_active_project(&template).active_tab_index, Some(1))
}

#[test]
fn test_config_with_active_tab_index_and_filtered_tabs() {
    let state = multi_tab_snapshot(
        1,
        vec![
            TabSnapshot {
                custom_title: None,
                default_directory_color: None,
                selected_color: SelectedTabColor::default(),
                root: PaneNodeSnapshot::Branch(BranchSnapshot {
                    direction: SplitDirection::Vertical,
                    children: vec![(
                        PaneFlex(1.),
                        PaneNodeSnapshot::Leaf(LeafSnapshot {
                            is_focused: true,
                            custom_vertical_tabs_title: None,
                            contents: LeafContents::Notebook(NotebookPaneSnapshot::CloudNotebook {
                                notebook_id: None,
                                settings: OpenWarpDriveObjectSettings::default(),
                            }),
                        }),
                    )],
                }),
                left_panel: None,
                right_panel: None,
                group_id: None,
                pinned: false,
            },
            TabSnapshot {
                custom_title: None,
                default_directory_color: None,
                selected_color: SelectedTabColor::default(),
                root: PaneNodeSnapshot::Branch(BranchSnapshot {
                    direction: SplitDirection::Vertical,
                    children: vec![(
                        PaneFlex(1.),
                        PaneNodeSnapshot::Leaf(LeafSnapshot {
                            is_focused: true,
                            custom_vertical_tabs_title: None,
                            contents: LeafContents::Terminal(TerminalPaneSnapshot {
                                uuid: vec![],
                                cwd: Some("/path/to/dir".into()),
                                is_active: true,
                                is_read_only: false,
                                shell_launch_data: None,
                                input_config: None,
                                llm_model_override: None,
                                active_profile_id: None,
                                conversation_ids_to_restore: vec![],
                                active_conversation_id: None,
                                on_restore_command: None,
                            }),
                        }),
                    )],
                }),
                left_panel: None,
                right_panel: None,
                group_id: None,
                pinned: false,
            },
        ],
    );

    let template = LaunchConfig::from_snapshot("Test".into(), &state);
    assert_eq!(first_active_project(&template).active_tab_index, Some(0))
}

#[test]
fn test_config_with_active_tab_being_filtered() {
    let state = multi_tab_snapshot(
        1,
        vec![
            TabSnapshot {
                custom_title: None,
                default_directory_color: None,
                selected_color: SelectedTabColor::default(),
                root: PaneNodeSnapshot::Branch(BranchSnapshot {
                    direction: SplitDirection::Vertical,
                    children: vec![(
                        PaneFlex(1.),
                        PaneNodeSnapshot::Leaf(LeafSnapshot {
                            is_focused: true,
                            custom_vertical_tabs_title: None,
                            contents: LeafContents::Terminal(TerminalPaneSnapshot {
                                uuid: vec![],
                                cwd: Some("/path/to/dir".into()),
                                is_active: true,
                                is_read_only: false,
                                shell_launch_data: None,
                                input_config: None,
                                llm_model_override: None,
                                active_profile_id: None,
                                conversation_ids_to_restore: vec![],
                                active_conversation_id: None,
                                on_restore_command: None,
                            }),
                        }),
                    )],
                }),
                left_panel: None,
                right_panel: None,
                group_id: None,
                pinned: false,
            },
            TabSnapshot {
                custom_title: None,
                default_directory_color: None,
                selected_color: SelectedTabColor::default(),
                root: PaneNodeSnapshot::Branch(BranchSnapshot {
                    direction: SplitDirection::Vertical,
                    children: vec![(
                        PaneFlex(1.),
                        PaneNodeSnapshot::Leaf(LeafSnapshot {
                            is_focused: true,
                            custom_vertical_tabs_title: None,
                            contents: LeafContents::Notebook(NotebookPaneSnapshot::CloudNotebook {
                                notebook_id: None,
                                settings: OpenWarpDriveObjectSettings::default(),
                            }),
                        }),
                    )],
                }),
                left_panel: None,
                right_panel: None,
                group_id: None,
                pinned: false,
            },
        ],
    );

    let template = LaunchConfig::from_snapshot("Test".into(), &state);
    assert_eq!(first_active_project(&template).active_tab_index, None)
}

fn terminal_project_snapshot(cwd: &str) -> WindowSnapshot {
    WindowSnapshot {
        tabs: vec![TabSnapshot {
            custom_title: None,
            default_directory_color: None,
            selected_color: SelectedTabColor::default(),
            root: PaneNodeSnapshot::Leaf(LeafSnapshot {
                is_focused: true,
                custom_vertical_tabs_title: None,
                contents: LeafContents::Terminal(TerminalPaneSnapshot {
                    uuid: vec![],
                    cwd: Some(cwd.to_string()),
                    is_active: true,
                    is_read_only: false,
                    shell_launch_data: None,
                    input_config: None,
                    llm_model_override: None,
                    active_profile_id: None,
                    conversation_ids_to_restore: vec![],
                    active_conversation_id: None,
                    on_restore_command: None,
                }),
            }),
            left_panel: None,
            right_panel: None,
            group_id: None,
            pinned: false,
        }],
        active_tab_index: 0,
        bounds: None,
        quake_mode: false,
        universal_search_width: None,
        warp_ai_width: None,
        voltron_width: None,
        warp_drive_index_width: None,
        left_panel_open: false,
        vertical_tabs_panel_open: false,
        fullscreen_state: Default::default(),
        left_panel_width: None,
        right_panel_width: None,
        agent_management_filters: None,
        tab_groups: vec![],
    }
}

#[test]
fn multi_project_window_round_trips_with_grouping() {
    let first_project = terminal_project_snapshot("/project/one");
    let second_project = terminal_project_snapshot("/project/two");
    let state = AppState {
        windows: vec![ProjectWindowSnapshot {
            projects: vec![first_project.clone(), second_project.clone()],
            active_project_index: 1,
        }],
        active_window_index: Some(0),
        block_lists: Default::default(),
        running_mcp_servers: Default::default(),
    };

    let saved = LaunchConfig::from_snapshot("Grouped".into(), &state);
    let serialized = serde_json::to_string(&saved).expect("launch config should serialize");
    let restored: LaunchConfig =
        serde_json::from_str(&serialized).expect("launch config should deserialize");

    assert_eq!(restored, saved);
    assert_eq!(restored.windows.len(), 1);
    assert_eq!(restored.windows[0].active_project_index(), 1);
    assert_eq!(
        restored.windows[0].projects(),
        &[
            WindowTemplate::from(first_project),
            WindowTemplate::from(second_project),
        ]
    );
}

#[test]
fn legacy_flat_windows_deserialize_as_single_project_windows() {
    let legacy_json = r#"{
        "name": "Legacy",
        "active_window_index": 1,
        "windows": [
            {
                "active_tab_index": 0,
                "tabs": [{ "layout": { "cwd": "/legacy/one" } }]
            },
            {
                "tabs": [{ "layout": { "cwd": "/legacy/two" } }]
            }
        ]
    }"#;

    let config: LaunchConfig =
        serde_json::from_str(legacy_json).expect("legacy launch config should deserialize");

    assert_eq!(config.active_window_index, Some(1));
    assert_eq!(config.windows.len(), 2);
    assert!(config
        .windows
        .iter()
        .all(|window| matches!(window, ProjectWindowTemplate::Legacy(_))));
    assert!(config
        .windows
        .iter()
        .all(|window| window.projects().len() == 1));
}
