use ::local_control::projects::{
    AgentResume, PaneLayout, ProjectLayout, ResumeProvider, SectionLayout, SplitDirection,
    TabLayout, PROJECT_LAYOUT_VERSION,
};
use ::local_control::protocol::{TabSelector, TabTarget};
use warp_core::channel::{Channel, ChannelConfig, ChannelState};
use warp_core::AppId;
use warpui::platform::WindowStyle;
use warpui::App;

use super::*;
use crate::app_state::{LeafContents, PaneNodeSnapshot};
use crate::GlobalResourceHandles;

fn sample_layout() -> ProjectLayout {
    ProjectLayout {
        version: PROJECT_LAYOUT_VERSION,
        active_tab: 0,
        sections: vec![SectionLayout {
            id: "work".into(),
            name: Some("In Progress".into()),
            color: Some("clinch-green".into()),
            collapsed: true,
            pinned: true,
        }],
        tabs: vec![TabLayout {
            title: Some("Backend".into()),
            color: Some("none".into()),
            pinned: false,
            section: Some("work".into()),
            panes: PaneLayout::Split {
                direction: SplitDirection::Horizontal,
                children: vec![
                    PaneLayout::Terminal {
                        cwd: Some("/tmp".into()),
                        title: Some("Agent".into()),
                        focused: true,
                        resume: Some(AgentResume {
                            provider: ResumeProvider::Codex,
                            conversation_id: "abc-123".into(),
                        }),
                    },
                    PaneLayout::Terminal {
                        cwd: Some("/tmp".into()),
                        title: None,
                        focused: false,
                        resume: None,
                    },
                ],
                weights: vec![0.3, 0.7],
            },
        }],
        tasks: vec!["Review the deployment".into()],
    }
}

#[test]
fn layout_roundtrip_preserves_sections_splits_titles_colors_and_resume() {
    let original = sample_layout();
    let snapshot = layout::restore(original.clone(), true).unwrap();
    assert_eq!(snapshot.tab_groups[0].name.as_deref(), Some("In Progress"));
    let mut exported = layout::export(snapshot).unwrap();
    // Persistent section IDs and pane UUIDs are regenerated for the new project.
    assert_ne!(exported.sections[0].id, "work");
    exported.tabs[0].section = Some("work".into());
    exported.sections[0].id = "work".into();
    assert_eq!(exported, original);
}

#[test]
fn restore_uses_fresh_terminal_ids_and_requires_explicit_resume() {
    let a = layout::restore(sample_layout(), false).unwrap();
    let b = layout::restore(sample_layout(), true).unwrap();
    let leaf = |node: PaneNodeSnapshot| {
        let PaneNodeSnapshot::Branch(branch) = node else {
            panic!("split")
        };
        let PaneNodeSnapshot::Leaf(leaf) = branch.children[0].1.clone() else {
            panic!("leaf")
        };
        let LeafContents::Terminal(terminal) = leaf.contents else {
            panic!("terminal")
        };
        terminal
    };
    let a = leaf(a.tabs[0].root.clone());
    let b = leaf(b.tabs[0].root.clone());
    assert_ne!(a.uuid, b.uuid);
    assert!(a.on_restore_command.is_none());
    assert_eq!(
        b.on_restore_command.as_deref(),
        Some("clinch_agent_resume_launch codex abc-123")
    );
}

#[test]
fn restore_rejects_invalid_structure_before_opening_any_project() {
    let mut invalid = sample_layout();
    invalid.version += 1;
    assert!(layout::restore(invalid, false).is_err());
    let mut invalid = sample_layout();
    invalid.tabs[0].section = Some("unknown".into());
    assert!(layout::restore(invalid, false).is_err());
    let mut invalid = sample_layout();
    invalid.tabs[0].panes = PaneLayout::Terminal {
        cwd: Some("relative/path".into()),
        title: None,
        focused: true,
        resume: None,
    };
    assert!(layout::restore(invalid, false).is_err());
    let mut invalid = sample_layout();
    invalid.tabs[0].panes = PaneLayout::Terminal {
        cwd: None,
        title: None,
        focused: true,
        resume: Some(AgentResume {
            provider: ResumeProvider::Claude,
            conversation_id: "id; touch /tmp/unwanted".into(),
        }),
    };
    assert!(layout::restore(invalid, true).is_err());
    let mut invalid = sample_layout();
    if let PaneLayout::Split { weights, .. } = &mut invalid.tabs[0].panes {
        weights[0] = 0.;
    }
    assert!(layout::restore(invalid, false).is_err());
    let mut invalid = sample_layout();
    let mut ungrouped = invalid.tabs[0].clone();
    ungrouped.section = None;
    invalid.tabs = vec![invalid.tabs[0].clone(), ungrouped, invalid.tabs[0].clone()];
    assert!(layout::restore(invalid, false).is_err());
}

fn mock_projects(app: &mut App) -> ViewHandle<ProjectWindow> {
    crate::workspace::view::tests::initialize_app(app);
    app.update(crate::root_view::init);
    ChannelState::set(ChannelState::new(
        Channel::Local,
        ChannelConfig::no_backend(AppId::new("test", "warp", "WarpTest"), "warp-test.log"),
    ));
    let resources = GlobalResourceHandles::mock(app);
    let (_, root) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
        RootView::new(
            resources,
            NewWorkspaceSource::Empty {
                previous_active_window: None,
                shell: None,
            },
            ctx,
        )
    });
    root.read(app, |root, _| root.project_window()).unwrap()
}

#[test]
fn cli_creates_inspects_inactive_projects_and_transfers_the_same_live_session() {
    let _groups = warp_core::features::FeatureFlag::GroupedTabs.override_enabled(true);
    App::test((), |mut app| async move {
        let parent = mock_projects(&mut app);
        let bridge = app.add_singleton_model(LocalControlBridge::new);
        let instance = InstanceId("test-project-control".into());
        let (source_id, source) = parent.read(&app, |parent, _| {
            let (id, workspace) = parent.projects().next().unwrap();
            (id.opaque_id(), workspace.clone())
        });
        let pane = source.read(&app, |workspace, _| {
            workspace.active_tab_pane_group().clone()
        });
        let terminal = pane.read(&app, |pane, ctx| pane.terminal_views(ctx)[0].clone());
        source.update(&mut app, |workspace, _| {
            workspace.tabs[0].selected_color = SelectedTabColor::Cleared
        });
        let created = bridge.update(&mut app, |_, ctx| {
            handle(
                &instance,
                &Action::with_params(
                    ActionKind::ProjectCreate,
                    ProjectCreateParams {
                        cwd: Some("/tmp".into()),
                    },
                )
                .unwrap(),
                &TargetSelector::default(),
                ctx,
            )
            .unwrap()
        });
        let destination_id = created["project"]["project_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let destination = app.read(|ctx| resolve_project(&destination_id, ctx).unwrap().workspace);
        let section = destination.update(&mut app, |workspace, ctx| {
            let group = workspace
                .create_named_tab_group_from_tab(0, "Review".into(), ctx)
                .unwrap();
            workspace.tab_groups.get_mut(&group).unwrap().collapsed = true;
            group
        });
        let target = TargetSelector {
            project: Some(source_id.clone()),
            tab: Some(TabTarget::Id {
                id: TabSelector(pane.id().to_string()),
            }),
            ..Default::default()
        };
        bridge.update(&mut app, |_, ctx| {
            let (_, origin_override) = crate::local_control::resolver::workspace_for_tab_create(
                &TargetSelector {
                    project: target.project.clone(),
                    ..Default::default()
                },
                Some(&Uuid::nil()),
                ctx,
            )
            .unwrap();
            assert_eq!(origin_override.id(), source.id());
            let metadata = super::super::metadata::tab_list(&target, ctx).unwrap();
            assert!(metadata.to_string().contains(&pane.id().to_string()));
            let read = handle(
                &instance,
                &Action::new(ActionKind::ProjectInspect),
                &TargetSelector {
                    project: Some(source_id),
                    ..Default::default()
                },
                ctx,
            )
            .unwrap();
            assert_eq!(read["project"]["tabs"][0]["color"], "none");
            assert_eq!(parent.as_ref(ctx).active_project_index(), 1);
            let params = TabTransferParams {
                destination_project: destination_id.clone(),
                section_id: Some(section.0.to_string()),
                index: Some(2),
            };
            assert!(handle(
                &instance,
                &Action::with_params(ActionKind::TabTransfer, params).unwrap(),
                &target,
                ctx
            )
            .is_err());
            assert_eq!(source.as_ref(ctx).tab_count(), 1);
            let params = TabTransferParams {
                destination_project: destination_id.clone(),
                section_id: Some(section.0.to_string()),
                index: Some(0),
            };
            let moved = handle(
                &instance,
                &Action::with_params(ActionKind::TabTransfer, params).unwrap(),
                &target,
                ctx,
            )
            .unwrap();
            assert_eq!(moved["tab_id"], pane.id().to_string());
        });
        destination.read(&app, |workspace, ctx| {
            assert_eq!(workspace.tabs[0].pane_group.id(), pane.id());
            assert_eq!(workspace.tabs[0].group_id, Some(section));
            assert_eq!(workspace.tabs[0].selected_color, SelectedTabColor::Cleared);
            assert!(!workspace.tab_groups[&section].collapsed);
            assert_eq!(
                workspace.tabs[0].pane_group.as_ref(ctx).terminal_views(ctx)[0].id(),
                terminal.id()
            );
        });
        assert_eq!(parent.read(&app, |parent, _| parent.projects().count()), 1);
        bridge.update(&mut app, |_, ctx| {
            assert_eq!(
                target_workspace(ActionKind::TabInspect, &target, ctx)
                    .unwrap_err()
                    .code,
                ErrorCode::StaleTarget
            );
            let target = TargetSelector {
                project: Some(destination_id),
                ..Default::default()
            };
            let exported = handle(
                &instance,
                &Action::new(ActionKind::ProjectExport),
                &target,
                ctx,
            )
            .unwrap();
            let layout: ProjectLayout = serde_json::from_value(exported["layout"].clone()).unwrap();
            let restored = handle(
                &instance,
                &Action::with_params(
                    ActionKind::ProjectRestore,
                    ProjectRestoreParams {
                        layout,
                        resume_agents: false,
                    },
                )
                .unwrap(),
                &TargetSelector::default(),
                ctx,
            )
            .unwrap();
            assert_eq!(restored["project"]["tabs"].as_array().unwrap().len(), 2);
            assert_ne!(
                restored["project"]["tabs"][0]["tab_id"],
                pane.id().to_string()
            );
            assert_eq!(parent.as_ref(ctx).projects().count(), 2);
        });
    });
}

#[test]
fn mismatched_window_and_project_fail_without_changing_focus() {
    use ::local_control::protocol::{WindowSelector, WindowTarget};
    App::test((), |mut app| async move {
        let parent = mock_projects(&mut app);
        let resources = GlobalResourceHandles::mock(&mut app);
        let (other_window, _) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            RootView::new(
                resources,
                NewWorkspaceSource::Empty {
                    previous_active_window: None,
                    shell: None,
                },
                ctx,
            )
        });
        let id = parent.read(&app, |parent, _| {
            parent.projects().next().unwrap().0.opaque_id()
        });
        let bridge = app.add_singleton_model(LocalControlBridge::new);
        bridge.update(&mut app, |_, ctx| {
            let target = TargetSelector {
                project: Some(id),
                window: Some(WindowTarget::Id {
                    id: WindowSelector(other_window.to_string()),
                }),
                ..Default::default()
            };
            assert_eq!(
                target_workspace(ActionKind::TabCreate, &target, ctx)
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidSelector
            );
        });
        assert_eq!(parent.read(&app, |parent, _| parent.projects().count()), 1);
    });
}

#[test]
fn local_control_background_launch_preserves_inactive_project_selection_and_section_state() {
    let _drag = warp_core::features::FeatureFlag::DragTabsToWindows.override_enabled(true);
    let _groups = warp_core::features::FeatureFlag::GroupedTabs.override_enabled(true);
    App::test((), |mut app| async move {
        let parent = mock_projects(&mut app);
        let bridge = app.add_singleton_model(LocalControlBridge::new);
        let (id, workspace) = parent.read(&app, |parent, _| {
            let (id, ws) = parent.projects().next().unwrap();
            (id.opaque_id(), ws.clone())
        });
        let section = workspace.update(&mut app, |workspace, ctx| {
            let section = workspace
                .create_named_tab_group_from_tab(0, "Background".into(), ctx)
                .unwrap();
            workspace.tab_groups.get_mut(&section).unwrap().collapsed = true;
            section
        });
        let selected = workspace.read(&app, |workspace, _| workspace.active_tab_pane_group().id());
        parent.update(&mut app, |parent, ctx| parent.add_project(ctx));
        let window_id = parent.update(&mut app, |_, ctx| ctx.window_id());
        let focus = app.focused_view_id(window_id);
        assert!(focus.is_some());
        let active = parent.read(&app, |parent, _| parent.active_project_index());
        bridge.update(&mut app, |_, ctx| {
            let params = ::local_control::agents::AgentLaunchParams {
                provider: ::local_control::agents::AgentProvider::Claude,
                project_id: id,
                section_id: Some(section.0.to_string()),
                cwd: Some("/tmp".into()),
                title: Some("Worker".into()),
                prompt: Some("--literal prompt".into()),
                background: true,
            };
            let created = crate::local_control::agents::launch(
                &InstanceId("launch-test".into()),
                params.clone(),
                ctx,
            )
            .unwrap();
            assert_eq!(created["created"], true);
            assert!(created["pane_id"].is_string());
            assert_eq!(parent.as_ref(ctx).active_project_index(), active);
            assert_eq!(workspace.as_ref(ctx).active_tab_pane_group().id(), selected);
            assert!(workspace.as_ref(ctx).tab_groups[&section].collapsed);
            assert_eq!(workspace.as_ref(ctx).tabs[1].group_id, Some(section));
            let mut invalid = params;
            invalid.section_id = Some(Uuid::new_v4().to_string());
            assert!(crate::local_control::agents::launch(
                &InstanceId("launch-test".into()),
                invalid,
                ctx
            )
            .is_err());
            assert_eq!(workspace.as_ref(ctx).tab_count(), 2);
        });
        app.update(|_| ());
        assert_eq!(app.focused_view_id(window_id), focus);
        async_io::Timer::after(std::time::Duration::from_millis(30)).await;
        assert_eq!(app.focused_view_id(window_id), focus);
    });
}

#[test]
fn local_control_tasks_edit_stable_ids_and_complete_pending_items() {
    App::test((), |mut app| async move {
        let parent = mock_projects(&mut app);
        let bridge = app.add_singleton_model(LocalControlBridge::new);
        let id = parent.read(&app, |parent, _| {
            parent.projects().next().unwrap().0.opaque_id()
        });
        bridge.update(&mut app, |_, ctx| {
            let instance = InstanceId("tasks-test".into());
            let target = TargetSelector {
                project: Some(id),
                ..Default::default()
            };
            let result = handle(
                &instance,
                &Action::with_params(
                    ActionKind::ProjectTaskCreate,
                    ::local_control::projects::ProjectTaskCreateParams {
                        text: " review ".into(),
                    },
                )
                .unwrap(),
                &target,
                ctx,
            )
            .unwrap();
            let id = result["task_id"].as_str().unwrap().to_owned();
            assert_eq!(result["tasks"][0]["text"], "review");
            let updated = handle(
                &instance,
                &Action::with_params(
                    ActionKind::ProjectTaskUpdate,
                    ::local_control::projects::ProjectTaskUpdateParams {
                        task_id: id.clone(),
                        text: " ship ".into(),
                    },
                )
                .unwrap(),
                &target,
                ctx,
            )
            .unwrap();
            assert_eq!(updated["tasks"][0]["id"], id);
            assert_eq!(updated["tasks"][0]["text"], "ship");
            let result = handle(
                &instance,
                &Action::with_params(
                    ActionKind::ProjectTaskComplete,
                    ::local_control::projects::ProjectTaskIdParams {
                        task_id: id.clone(),
                    },
                )
                .unwrap(),
                &target,
                ctx,
            )
            .unwrap();
            assert_eq!(result["completed"], true);
            assert!(result["tasks"].as_array().unwrap().is_empty());
            assert!(handle(
                &instance,
                &Action::with_params(
                    ActionKind::ProjectTaskDelete,
                    ::local_control::projects::ProjectTaskIdParams { task_id: id }
                )
                .unwrap(),
                &target,
                ctx
            )
            .is_err());
        });
    });
}

fn exercise_cross_window_transfer(keep_tasks: bool) {
    let _groups = warp_core::features::FeatureFlag::GroupedTabs.override_enabled(true);
    let _pins = warp_core::features::FeatureFlag::PinnedTabs.override_enabled(true);
    App::test((), move |mut app| async move {
        let source_parent = mock_projects(&mut app);
        let resources = GlobalResourceHandles::mock(&mut app);
        let (_, root) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            RootView::new(
                resources,
                NewWorkspaceSource::Empty {
                    previous_active_window: None,
                    shell: None,
                },
                ctx,
            )
        });
        let destination_parent = root.read(&app, |root, _| root.project_window()).unwrap();
        let (source_id, source) = source_parent.read(&app, |parent, _| {
            let (id, ws) = parent.projects().next().unwrap();
            (id.opaque_id(), ws.clone())
        });
        let (destination_id, destination) = destination_parent.read(&app, |parent, _| {
            let (id, ws) = parent.projects().next().unwrap();
            (id.opaque_id(), ws.clone())
        });
        let destination_window = destination.update(&mut app, |_, ctx| ctx.window_id());
        let section = destination.update(&mut app, |workspace, ctx| {
            let id = workspace
                .create_named_tab_group_from_tab(0, "Pinned".into(), ctx)
                .unwrap();
            workspace.pin_tab_group(id, ctx);
            id
        });
        let pane = source.read(&app, |workspace, _| {
            workspace.active_tab_pane_group().clone()
        });
        let terminal = pane.read(&app, |pane, ctx| pane.terminal_views(ctx)[0].clone());
        source.update(&mut app, |workspace, ctx| {
            workspace.tabs[0].selected_color = SelectedTabColor::Cleared;
            if keep_tasks {
                workspace.add_workspace_task("keep this task".into(), ctx);
            }
        });
        let bridge = app.add_singleton_model(LocalControlBridge::new);
        bridge.update(&mut app, |_, ctx| {
            let target = TargetSelector {
                project: Some(source_id),
                tab: Some(TabTarget::Id {
                    id: TabSelector(pane.id().to_string()),
                }),
                ..Default::default()
            };
            let params = TabTransferParams {
                destination_project: destination_id,
                section_id: Some(section.0.to_string()),
                index: Some(0),
            };
            let moved = handle(
                &InstanceId("cross-window".into()),
                &Action::with_params(ActionKind::TabTransfer, params).unwrap(),
                &target,
                ctx,
            )
            .unwrap();
            assert_eq!(moved["tab_id"], pane.id().to_string());
            assert_eq!(source.as_ref(ctx).tab_count(), usize::from(keep_tasks));
            assert_eq!(
                source.as_ref(ctx).workspace_tasks().len(),
                usize::from(keep_tasks)
            );
            let tab = destination
                .as_ref(ctx)
                .tabs
                .iter()
                .find(|tab| tab.pane_group.id() == pane.id())
                .unwrap();
            assert_eq!(tab.selected_color, SelectedTabColor::Cleared);
            assert_eq!(tab.group_id, Some(section));
            assert!(destination.as_ref(ctx).tab_groups[&section].pinned);
            assert_eq!(destination.as_ref(ctx).tabs[0].pane_group.id(), pane.id());
            assert_eq!(
                tab.pane_group.as_ref(ctx).terminal_views(ctx)[0].id(),
                terminal.id()
            );
        });
        assert_eq!(
            terminal.update(&mut app, |_, ctx| ctx.window_id()),
            destination_window
        );
        terminal.update(&mut app, |view, ctx| {
            assert_eq!(
                view.input().update(ctx, |_, ctx| ctx.window_id()),
                destination_window
            );
        });
        // The test platform's close_window_async is a no-op; assert the project was
        // removed before the native close request, while task-owning projects remain.
        assert_eq!(
            source_parent.read(&app, |parent, _| parent.projects().count()),
            usize::from(keep_tasks)
        );
    });
}

#[test]
fn local_control_cross_window_transfer_retains_project_tasks_and_live_terminal() {
    exercise_cross_window_transfer(true);
}
#[test]
fn local_control_cross_window_transfer_removes_empty_source_without_killing_terminal() {
    exercise_cross_window_transfer(false);
}
