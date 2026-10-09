use warp_core::channel::{Channel, ChannelConfig, ChannelState};
use warp_core::AppId;
use warpui::elements::Fill;
use warpui::platform::WindowStyle;
use warpui::{App, EntityId, SingletonEntity};

use super::AgentAttentionPulse;
use crate::root_view::{NewWorkspaceSource, RootView};
use crate::terminal::cli_agent_sessions::tests::idle_test_session;
use crate::terminal::cli_agent_sessions::{CLIAgentSessionStatus, CLIAgentSessionsModel};
use crate::terminal::CLIAgent;
use crate::ui_components::CLINCH_ATTENTION_AMBER;
use crate::GlobalResourceHandles;

fn set_status(app: &mut App, terminal: EntityId, status: CLIAgentSessionStatus) {
    CLIAgentSessionsModel::handle(app).update(app, |model, ctx| {
        let mut session = idle_test_session(CLIAgent::Codex);
        session.has_observed_turn_activity = true;
        session.status = status;
        model.set_session(terminal, session, ctx);
    });
}

#[test]
fn pulse_stays_idle_and_lit_while_nothing_is_blocked() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let pulse = app.add_singleton_model(AgentAttentionPulse::new);

        set_status(&mut app, EntityId::new(), CLIAgentSessionStatus::InProgress);

        pulse.read(&app, |pulse, _| {
            assert!(!pulse.ticking);
            assert!(pulse.is_lit());
        });
    });
}

#[test]
fn pulse_blinks_while_a_session_is_blocked_and_settles_lit_afterwards() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let pulse = app.add_singleton_model(AgentAttentionPulse::new);
        let terminal = EntityId::new();

        set_status(
            &mut app,
            terminal,
            CLIAgentSessionStatus::Blocked {
                message: Some("Red or blue?".to_owned()),
            },
        );
        pulse.read(&app, |pulse, _| {
            assert!(pulse.ticking);
            assert!(pulse.is_lit(), "a new block starts visible");
        });

        pulse.update(&mut app, |pulse, ctx| pulse.tick(ctx));
        pulse.read(&app, |pulse, _| assert!(!pulse.is_lit()));
        pulse.update(&mut app, |pulse, ctx| pulse.tick(ctx));
        pulse.read(&app, |pulse, _| assert!(pulse.is_lit()));
        pulse.update(&mut app, |pulse, ctx| pulse.tick(ctx));
        pulse.read(&app, |pulse, _| assert!(!pulse.is_lit()));

        // Answering the question unblocks the session: the next tick stops the clock and
        // restores the lit phase so the next block does not start dimmed.
        set_status(&mut app, terminal, CLIAgentSessionStatus::InProgress);
        pulse.update(&mut app, |pulse, ctx| pulse.tick(ctx));
        pulse.read(&app, |pulse, _| {
            assert!(!pulse.ticking);
            assert!(pulse.is_lit());
        });
    });
}

#[test]
fn horizontal_project_badge_repaints_on_attention_pulse() {
    let _horizontal_tabs = warp_core::features::FeatureFlag::VerticalTabs.override_enabled(false);
    App::test((), |mut app| async move {
        crate::workspace::view::tests::initialize_app(&mut app);
        assert!(!crate::tab::uses_vertical_tabs());
        app.update(crate::root_view::init);
        ChannelState::set(ChannelState::new(
            Channel::Local,
            ChannelConfig::no_backend(AppId::new("test", "warp", "WarpTest"), "warp-test.log"),
        ));
        let resources = GlobalResourceHandles::mock(&mut app);
        let (window_id, root) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            RootView::new(
                resources,
                NewWorkspaceSource::Empty {
                    previous_active_window: None,
                    shell: None,
                },
                ctx,
            )
        });
        let terminal = root.read(&app, |root, ctx| {
            let project_window = root.project_window().unwrap();
            let workspace = project_window.as_ref(ctx).active_workspace();
            workspace
                .as_ref(ctx)
                .active_tab_pane_group()
                .as_ref(ctx)
                .focused_session_view(ctx)
                .unwrap()
                .id()
        });
        set_status(
            &mut app,
            terminal,
            CLIAgentSessionStatus::Blocked {
                message: Some("Choose an option".to_owned()),
            },
        );
        // Inspect the badge's border in the actual cached scene, without manually invalidating
        // the project view. The mock asset provider does not supply fonts for its text glyph.
        // A missing subscription leaves this border at full opacity after the pulse ticks.
        let badge_opacity = |app: &App| {
            let presenter = app.presenter(window_id).unwrap();
            let presenter = presenter.borrow();
            let opacity = presenter
                .scene()
                .unwrap()
                .layers()
                .flat_map(|layer| &layer.rects)
                .find_map(|rect| {
                    let Fill::Solid(color) = rect.border.color else {
                        return None;
                    };
                    (color.r == CLINCH_ATTENTION_AMBER.r
                        && color.g == CLINCH_ATTENTION_AMBER.g
                        && color.b == CLINCH_ATTENTION_AMBER.b)
                        .then_some(color.a)
                })
                .expect("horizontal project header should show an amber input badge");
            opacity
        };
        assert_eq!(badge_opacity(&app), 255);
        let pulse = AgentAttentionPulse::handle(&app);
        pulse.update(&mut app, |pulse, ctx| pulse.tick(ctx));
        assert_eq!(badge_opacity(&app), 64);
        pulse.update(&mut app, |pulse, ctx| pulse.tick(ctx));
        assert_eq!(badge_opacity(&app), 255);
    });
}
