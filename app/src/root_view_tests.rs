use warp_core::channel::{Channel, ChannelConfig, ChannelState};
use warp_core::user_preferences::GetUserPreferences as _;
use warp_core::AppId;
use warpui::platform::WindowStyle;
use warpui::{App, SingletonEntity};

use super::{
    has_completed_local_onboarding, AuthOnboardingState, GlobalResourceHandles, NewWorkspaceSource,
    RootView, HAS_COMPLETED_ONBOARDING_KEY,
};
use crate::auth::auth_manager::AuthManager;
use crate::auth::AuthStateProvider;
use crate::server::server_api::ServerApiProvider;

fn initialize_app(app: &mut App) {
    app.update(crate::settings::init_and_register_user_preferences);
    app.add_singleton_model(|_ctx| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AuthManager::new_for_test);
}

fn set_local_onboarding_completed(app: &mut App, completed: bool) {
    app.update(|ctx| {
        ctx.private_user_preferences()
            .write_value(
                HAS_COMPLETED_ONBOARDING_KEY,
                serde_json::to_string(&completed).unwrap(),
            )
            .unwrap();
    });
}

/// Regression test for the bug fixed by introducing
/// `RootView::sync_local_onboarding_to_server`: when a user completed onboarding
/// pre-login and later authenticated via a non-login-slide entrypoint (i.e. while
/// already in `Terminal` state), the server-side `is_onboarded` flag was never
/// flipped. The helper runs unconditionally on `AuthComplete` and must flip the
/// flag when all preconditions hold.
#[test]
fn test_sync_flips_server_is_onboarded_when_local_onboarding_completed() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Seed the "has_completed_local_onboarding" preference and make the user
        // appear not yet onboarded on the server. The default test user is
        // non-anonymous, so the guards in the helper won't short-circuit.
        set_local_onboarding_completed(&mut app, true);
        app.update(|ctx| {
            AuthStateProvider::as_ref(ctx).get().set_is_onboarded(false);
            assert!(has_completed_local_onboarding(ctx));
            assert_eq!(
                AuthStateProvider::as_ref(ctx).get().is_onboarded(),
                Some(false)
            );
        });

        app.update(|ctx| {
            let auth_state = AuthStateProvider::as_ref(ctx).get().clone();
            RootView::sync_local_onboarding_to_server(&auth_state, ctx);
        });

        app.read(|ctx| {
            assert_eq!(
                AuthStateProvider::as_ref(ctx).get().is_onboarded(),
                Some(true),
                "sync should have invoked AuthManager::set_user_onboarded"
            );
        });
    });
}

/// If the user hasn't completed local onboarding, the helper must leave the
/// server-side flag untouched — onboarding hasn't actually happened yet.
#[test]
fn test_sync_noop_when_local_onboarding_not_completed() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Do not set HAS_COMPLETED_ONBOARDING_KEY; it defaults to false.
        app.update(|ctx| {
            AuthStateProvider::as_ref(ctx).get().set_is_onboarded(false);
        });

        app.update(|ctx| {
            let auth_state = AuthStateProvider::as_ref(ctx).get().clone();
            RootView::sync_local_onboarding_to_server(&auth_state, ctx);
        });

        app.read(|ctx| {
            assert_eq!(
                AuthStateProvider::as_ref(ctx).get().is_onboarded(),
                Some(false),
                "sync should not have changed is_onboarded when local onboarding is incomplete"
            );
        });
    });
}

/// The server-side flag should also be left untouched when it is already set,
/// even if local onboarding is complete — avoids redundant server calls.
#[test]
fn test_sync_noop_when_already_onboarded_on_server() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        set_local_onboarding_completed(&mut app, true);
        app.update(|ctx| {
            // User::test() defaults to is_onboarded = true; assert that and
            // leave it in place.
            assert_eq!(
                AuthStateProvider::as_ref(ctx).get().is_onboarded(),
                Some(true)
            );
        });

        app.update(|ctx| {
            let auth_state = AuthStateProvider::as_ref(ctx).get().clone();
            RootView::sync_local_onboarding_to_server(&auth_state, ctx);
        });

        app.read(|ctx| {
            assert_eq!(
                AuthStateProvider::as_ref(ctx).get().is_onboarded(),
                Some(true)
            );
        });
    });
}

/// Backendless fork builds (Clinch stable/local, constructed via
/// `ChannelConfig::no_backend`) must launch straight into a blank terminal in
/// every window: no onboarding slides, no login slide, no auth screen. This
/// must hold even if stale `AuthState` would otherwise report the user as
/// logged in, since the backendless branch is checked first in
/// `RootView::new`. It also must hold across `log_out` (e.g. triggered by a
/// stale-auth-state CLI or Settings logout): `AuthOnboardingState::log_out`'s
/// `Terminal` arm is gated the same way, so it stays in `Terminal` instead of
/// re-entering the `Auth` screen.
///
/// Note: this test mutates the process-global `ChannelState` via
/// `ChannelState::set`, relying on `cargo nextest`'s process-per-test
/// isolation (the test runner mandated by this repo) to avoid bleeding into
/// other tests. Do not run this test under plain `cargo test` alongside other
/// tests that read `ChannelState`.
#[test]
fn test_backendless_build_launches_directly_into_terminal() {
    App::test((), |mut app| async move {
        // Reuse the full Workspace test scaffolding: RootView::new creates a
        // Workspace immediately in the backendless branch, which needs all of
        // the singleton models registered there.
        crate::workspace::view::tests::initialize_app(&mut app);

        ChannelState::set(ChannelState::new(
            Channel::Local,
            ChannelConfig::no_backend(AppId::new("test", "warp", "WarpTest"), "warp-test.log"),
        ));

        // `AuthStateProvider::new_for_test` (registered above) seeds
        // credentials, so `auth_state.is_logged_in()` is already `true` here —
        // exactly the "stale auth state" case the backendless branch must
        // still win against, since it's checked first in `RootView::new`.
        app.read(|ctx| {
            assert!(AuthStateProvider::as_ref(ctx).get().is_logged_in());
        });

        let global_resource_handles = GlobalResourceHandles::mock(&mut app);
        let active_window_id = app.read(|ctx| ctx.windows().active_window());
        let (_, root_view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            RootView::new(
                global_resource_handles,
                NewWorkspaceSource::Empty {
                    previous_active_window: active_window_id,
                    shell: None,
                },
                ctx,
            )
        });

        app.read(|ctx| {
            root_view.read(ctx, |view, _ctx| {
                assert!(
                    matches!(view.auth_onboarding_state, AuthOnboardingState::Terminal(_)),
                    "backendless builds must launch straight into the workspace"
                );
            });
        });

        // Simulate a logout (e.g. from Settings or the CLI, possibly triggered
        // by stale AuthState) and verify it does not re-enter the Auth screen.
        app.update(|ctx| {
            root_view.update(ctx, |view, ctx| {
                view.log_out(&(), ctx);
            });
        });

        app.read(|ctx| {
            root_view.read(ctx, |view, _ctx| {
                assert!(
                    matches!(view.auth_onboarding_state, AuthOnboardingState::Terminal(_)),
                    "backendless builds must stay in the workspace after log_out"
                );
            });
        });
    });
}
