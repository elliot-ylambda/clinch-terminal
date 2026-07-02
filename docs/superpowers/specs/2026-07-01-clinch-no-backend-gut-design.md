# Clinch: gut onboarding/login, backendless UI, new defaults & icon

**Date:** 2026-07-01
**Status:** Approved (design reviewed with owner)

## Problem

A fresh install of Clinch (stable channel, `ChannelConfig::no_backend()`) still
behaves like retail Warp on first launch:

1. Warp's welcome/onboarding slides appear — and appear **per window** when a
   session is restored, because each window's `RootView` reads the
   `HasCompletedOnboarding` preference before any window has written it.
2. Onboarding offers login/signup. Clinch must never let a user create a
   login or accidentally talk to Warp's backend.
3. Backend-dependent UI (AI/Agent Mode, Warp Drive, account/billing settings)
   is visible but can never work.
4. The app icon is still Warp's.
5. Desired defaults: vertical tabs (left panel) on; dark theme (already the
   compiled default — needs no change, only the removal of the onboarding
   theme picker).

## Goals

- First launch (and every logged-out launch) lands directly in a blank
  terminal. No onboarding, no login slide, no auth screen, in any window.
- No login/signup entry point anywhere: menus, command palette, settings,
  banners, tab bar, sharing UI.
- Backend-dependent UI is hidden: AI/Agent Mode, Warp Drive, Account, Teams,
  Billing, Referrals, Shared blocks settings pages.
- Vertical tabs enabled by default (feature flag is already on; only the user
  setting default flips).
- Clinch app icon replaces Warp's (direction: dark with a glowing accent,
  color + depth, not flat monochrome). Owner approves a preview before it
  ships.
- WarpLocal (local channel, also `no_backend()`) gets identical behavior.

## Non-goals

- Deleting Warp's AI/auth/drive code from the tree (stay upstream-mergeable).
- Compile-time feature trimming for a smaller binary (possible follow-up).
- BYOK ("bring your own API key") AI support — possible future follow-up; the
  upstream `SoloUserByok` path exists but is out of scope.
- Renaming the `oz` CLI, copyright string (tracked separately in CLAUDE.md).

## Design

### 1. The gate: `ChannelState::has_backend()`

`ChannelConfig` (crates/warp_core/src/channel/config.rs) gains:

```rust
/// Whether this build talks to a real Warp backend. `false` for backend-free
/// fork channels built via [`ChannelConfig::no_backend`].
#[serde(default = "default_true")]
pub has_backend: bool,
```

- `no_backend()` sets `has_backend: false`.
- The serde default (`true`) keeps upstream's generated dev/preview JSON
  configs valid without regeneration.
- New static helper `ChannelState::has_backend()` alongside the existing
  `channel()` / `is_release_bundle()` helpers.
- Rationale: gating flows from the *fact* that the build has no backend, not
  from `Channel::Stable` (upstream Stable is real Warp; Clinch local + stable
  are both backendless).

### 2. Launch flow: straight to the workspace

`RootView::new` (app/src/root_view.rs, ~line 1671): when
`!ChannelState::has_backend()`, construct
`AuthOnboardingState::Terminal(workspace_args.create_workspace(ctx))`
directly — before the `is_logged_in()` / onboarding / ForceLogin /
SkipFirebaseAnonymousUser decision tree. Every window takes this
short-circuit, which also fixes the per-window onboarding bug at the root.

Same gate applied to re-entry points:

- OAuth deep-link handling (`clinch://` auth callback via `AuthManager`) — a
  received token must be ignored when backendless.
- `Workspace::should_show_agent_onboarding`,
  `start_agent_onboarding_tutorial`, `dispatch_tutorial_when_bootstrapped`
  (app/src/workspace/view/onboarding.rs) — belt-and-suspenders; most are
  already unreachable once RootView never enters onboarding.
- Any "restart onboarding" debug/palette action.

Session restore is unchanged: `restore_session` stays default-`true`; first
launch is blank because there is nothing to restore, returning users get
their tabs back.

### 3. Hiding backend surfaces

Two choke points plus an enumerated sweep, all keyed on `has_backend()`:

- **AI master switch:** `AISettings::is_any_ai_enabled()`
  (app/src/settings/ai.rs:1657) returns `false` when backendless. Call sites
  that read the raw `*settings.is_any_ai_enabled` setting directly (e.g.
  app/src/workspace/view/onboarding.rs:163) are switched to the method so the
  gate is authoritative. This hides Agent Mode entry points, AI banners
  (incl. the AWS Bedrock login banner), AI command-palette actions, and the
  AI settings pages that already condition on it.
- **Warp Drive:** already requires a logged-in user
  (app/src/drive/settings.rs:48) — hidden automatically. Verified in the
  sweep.
- **Settings sidebar** (app/src/settings_view/mod.rs `build_nav_stops`
  `is_visible` filter, ~line 1307): when backendless, drop Account, Teams,
  BillingAndUsage, Referrals, SharedBlocks, WarpDrive, and the AI section
  tree. Keep Appearance, Features, Keybindings, Privacy (local toggles),
  About, platform/scripting pages.
- **Sweep** (each gated or verified already-hidden when logged out):
  - app menus (app/src/app_menus.rs): Sign in/out, account items.
  - command palette (app/src/search/command_palette/data_sources.rs):
    Sign In action; AI actions covered by the master switch.
  - tab-bar avatar (`avatar_in_tab_bar` surfaces).
  - pane header sharing UI (app/src/pane_group/pane/view/header/sharing.rs) —
    session sharing already has `session_sharing_server_url: None`.
  - stray sign-in CTAs: notebooks details bar, workflow view,
    one-time modals.
- Telemetry, autoupdate, crash reporting: already `None` in
  `ChannelConfig::no_backend()`; no work beyond verification.

### 4. Defaults

- `use_vertical_tabs` (app/src/workspace/tab_settings.rs:488):
  `default: false` → `true`. The `vertical_tabs` cargo feature/flag is
  already enabled for all fork builds.
- `show_vertical_tab_panel_in_restored_windows`
  (app/src/workspace/tab_settings.rs:495): `false` → `true` so restored
  windows show the panel too.
- Theme: no change — `ThemeKind::Dark` is already the compiled default and
  `use_system_theme` defaults to `false`. The onboarding theme picker (the
  only thing that overrode it) no longer runs.
- These default flips are fork-global (all bins), which is desirable —
  WarpLocal should match Clinch.

### 5. Icon

Replace Warp assets with a generated Clinch icon (dark base, glowing accent,
color + depth — owner-approved preview before shipping):

- `app/channels/stable/icon/AppIcon.icon/` — Xcode 26 `.icon` bundle
  (Assets PNG + SVG glyph + icon.json), compiled by `script/compile_icon`
  via `actool` (pipeline already runs for stable; no build changes).
- `app/channels/stable/icon/no-padding/` — 16–512px PNG set + `icon.ico`.
- Any `.icns`/icon path referenced by
  `[package.metadata.bundle.bin.stable]` in app/Cargo.toml.

WarpLocal keeps its current icon (separate channel dir); can adopt the
Clinch icon later.

## Error handling & edge cases

- **Stale auth state:** if keychain/preferences contain an old session,
  backendless builds still go straight to `Terminal`; `AuthManager` must not
  fire requests (URLs are unroutable regardless — TEST-NET black hole).
- **Serde compatibility:** `has_backend` defaults `true` on deserialize, so
  upstream generated configs and any persisted config snapshots keep working.
- **Upstream channels:** dev/preview/oss behavior is unchanged
  (`has_backend == true` — oss uses `WarpServerConfig::production()`).
- **Settings written by old onboarding:** existing Clinch/WarpLocal profiles
  that already completed onboarding keep their chosen settings; default
  flips only affect unset values.

## Testing & verification

- Unit tests: `has_backend` serde default + `no_backend()` (extend
  crates/warp_core/src/channel/config_tests.rs); RootView state selection
  gate (extend app/src/root_view_tests.rs) if the harness allows; settings
  nav filtering.
- `./script/presubmit`-equivalent: format + the two clippy invocations it
  specifies + nextest.
- Manual smoke (WarpLocal first, then Clinch): launch with a fresh storage
  dir → no onboarding; blank dark terminal; vertical tabs panel visible;
  Settings sidebar shows no Account/Teams/Billing/Referrals/AI/Drive; no
  Sign In in menus or command palette; icon shows Clinch art.
- Ship via `make ship` after merge; releases must keep attaching
  `Clinch.app.zip` (install one-liner depends on it).

## Documentation updates (same change)

- CLAUDE.md: the "Verify on first login / ?scheme=clinch" block becomes
  obsolete (login is gutted) — replace with a note that backendless builds
  have no login; remove the "Icon is still Warp's" follow-up line once the
  icon lands.
- WARP.md: no changes expected.

## Follow-ups (explicitly deferred)

- BYOK AI re-enablement behind `SoloUserByok`.
- Compile-out feature trimming for binary size.
- WarpLocal icon adoption.
- `oz` CLI rename; bundle copyright string.
