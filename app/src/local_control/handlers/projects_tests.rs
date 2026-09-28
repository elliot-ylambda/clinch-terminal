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
