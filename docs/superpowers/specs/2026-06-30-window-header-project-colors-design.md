# Window header project colors — design

**Date:** 2026-06-30
**Status:** Approved design, pending spec review
**Feature flag:** `FeatureFlag::WindowHeaderColors`

## Summary

Tint the top header strip (the tab-bar strip) of each Clinch window with a
subtle, project-specific color, so multiple open windows are instantly
distinguishable at a glance. The color is derived **automatically** from the
active tab's git/project root — zero configuration — and is **stable**: the same
project always gets the same hue across restarts. The header "follows the active
tab," so switching to a tab in a different project re-tints the header.

This is an additive, self-isolated feature that reuses the existing tab-color
primitives (palette, opacity/blend helpers, directory→pwd plumbing). It does
**not** change the existing per-tab color dots.

## Goals

- Every window's top header gets a distinct, subtle color with **no per-project
  setup**.
- Same project → same color, every launch (deterministic and stable).
- Header color **follows the active tab's** project.
- Respect explicit user color choices (manual tab color, or a configured
  `DirectoryTabColors` mapping) — those win over the automatic hash.
- Subtle "hint of color," not a prominent bar.

## Non-goals

- Auto-coloring the per-tab dots (the existing `DirectoryTabColors` feature stays
  as-is — manual/configured only).
- A user-facing Settings toggle or Command Palette entry (not wanted for v1).
- Guaranteeing globally unique colors across arbitrarily many projects (a small
  fixed palette is acceptable — the goal is telling a handful of open windows
  apart).
- Remote-session tinting in v1 (local sessions only; see Edge cases).

## Decisions (locked)

| Question | Decision |
|---|---|
| Color source | Automatic, deterministic hash of the project directory |
| Project unit | **Git/project root** (stable as you `cd` into subfolders); fall back to the cwd when not in a repo |
| Which directory | **Follow the active tab** |
| Override behavior | Explicit tab color / configured directory color **wins** over the hash |
| Palette | Reuse the 6 `TAB_COLOR_OPTIONS` ANSI theme colors (red/green/yellow/blue/magenta/cyan) |
| Tint strength | **Subtle, ~18% opacity** solid wash over the theme background |
| Rollout | Behind `FeatureFlag::WindowHeaderColors`, **enabled for everyone on all channels** (flag retained as kill-switch); no user setting |

## Background: how things work today

- **The "top header" is the tab-bar strip**, rendered by the per-window
  `Workspace` view. Entry point `render_tab_bar` (`app/src/workspace/view.rs`,
  ~line 20660); the strip container's background is set at ~20683–20700. Today
  that container is transparent unless `FeatureFlag::NewTabStyling` is on, in
  which case it fills with `internal_colors::fg_overlay_1`. The visible
  background otherwise comes from `get_terminal_background_fill` (the theme
  background). This container is exactly where a per-window tint goes.
- **`Workspace` is per-window** and already stores `window_id`
  (`view.rs:978`), so no new per-window registry is needed.
- **Existing per-tab colors** (`DirectoryTabColors`): a **manual**,
  user-configured `HashMap<directory, color>` resolved by longest-prefix match
  in `DirectoryTabColors::color_for_directory` (`app/src/workspace/tab_settings.rs:204`).
  A tab's effective color is `TabData::color()` =
  `selected_color.resolve(default_directory_color)` (`app/src/tab.rs:187`),
  returning `Option<AnsiColorIdentifier>`. This is the "explicit override" we
  honor.
- **Active tab's directory (sync)**: `tab.pane_group.as_ref(ctx)
  .active_session_view(ctx).and_then(|tv| tv.as_ref(ctx)
  .canonical_session_pwd_if_local(ctx))` — the exact call
  `sync_codebase_tab_color` uses (`view.rs:5497`). Returns `None` for remote
  sessions.
- **Repo root (sync)**: `DetectedRepositories::get_root_for_path(&LocalOrRemotePath)
  -> Option<LocalOrRemotePath>` (`crates/repo_metadata/src/repositories.rs:182`),
  a singleton entity (`DetectedRepositories::as_ref(ctx)`). Returns the
  already-detected root, or `None` if not (yet) detected / not in a repo.
- **Color primitives**: palette `TAB_COLOR_OPTIONS`
  (`app/src/ui_components/color_dot.rs:18`); resolve an `AnsiColorIdentifier`
  to a `ColorU` via `identifier.to_ansi_color(&theme.terminal_colors().normal)`;
  subtle tint via `coloru_with_opacity` / the `Blend` trait
  (`crates/warp_core/src/ui/color/`). The tab tint uses opacity 25
  (`WARP_2_TAB_COLOR_OPACITY`, `app/src/tab.rs:70`) as a template.

## Design

### Components

1. **Feature flag** — `FeatureFlag::WindowHeaderColors` in
   `crates/warp_features/src/lib.rs`. All new behavior is gated on
   `FeatureFlag::WindowHeaderColors.is_enabled()`. Registered so it is enabled on
   dogfood, preview, and release (implementation follows the `add-feature-flag` /
   `promote-feature` skills; per the `RELEASE_FLAGS` note at lib.rs:970, launch
   is likely default-on via `app/Cargo.toml` rather than a `RELEASE_FLAGS` entry).

2. **Stable hash** — new module `app/src/workspace/header_color.rs`:
   ```
   fn header_color_for_path(path: &Path) -> AnsiColorIdentifier
   ```
   Uses an **explicit, stable** hash (FNV-1a over the canonicalized path string)
   → index into `TAB_COLOR_OPTIONS`. Explicitly *not* `DefaultHasher`, so the
   mapping is reproducible across versions and platforms. The path string is
   canonicalized the same way tab colors key directories
   (`canonical_directory_key`) so equivalent paths hash identically. Colocated
   unit tests.

3. **Resolution** — `Workspace::resolve_header_color(&self, ctx) ->
   Option<AnsiColorIdentifier>`:
   1. `active_tab = self.tabs.get(self.active_tab_index)?`
   2. If `active_tab.color()` is `Some(c)` → return `Some(c)` (explicit override
      wins; header matches the tab dot).
   3. Else `cwd = active_tab`'s `canonical_session_pwd_if_local(ctx)?`
      (`None` → no tint).
   4. `root = DetectedRepositories::as_ref(ctx).get_root_for_path(&cwd)
      .unwrap_or(cwd)`.
   5. Return `Some(header_color_for_path(root.as_path()))`.

4. **Apply** — in `render_tab_bar` (`view.rs` ~20683–20700), when
   `FeatureFlag::WindowHeaderColors.is_enabled()` and `resolve_header_color`
   returns `Some(id)`:
   - `color: ThemeFill = id.to_ansi_color(&theme.terminal_colors().normal).into()`
   - Blend it over the base background at ~18%, mirroring the existing
     `internal_colors::accent_bg` idiom (`crates/warp_core/src/ui/theme/color.rs:572`):
     `Fill::Solid(theme.background().into_solid()).blend(&color.with_opacity(WINDOW_HEADER_TINT_OPACITY))`
     — a solid, theme-aware wash over the base background.
   - Set the tab-bar container background to that fill (via `.into()`), taking
     precedence over the `NewTabStyling` `fg_overlay_1` fill so the two coexist
     cleanly.
   - Introduce a named constant `WINDOW_HEADER_TINT_OPACITY: Opacity = 18`.

### Data flow

```
active tab ──(pane_group → active_session_view → canonical_session_pwd_if_local)──▶ cwd
   │                                                                                 │
   │ tab.color() (manual/configured)                     DetectedRepositories        │
   ▼        │ Some → override wins                        .get_root_for_path(cwd)     ▼
resolve_header_color ◀──────────────────────────────────── unwrap_or(cwd) ──▶ header_color_for_path(root)
   │                                                                       (FNV-1a → TAB_COLOR_OPTIONS)
   ▼
AnsiColorIdentifier ─▶ to_ansi_color(theme.normal) ─▶ ColorU ─▶ theme.background().blend(color @18%)
   ▼
tab-bar container .with_background(fill)  (render_tab_bar, gated by flag)
```

Resolution happens at render time from the active tab, so "follow the active
tab" and "stays stable within a repo" both fall out for free — a tab switch or
cwd change simply re-renders and re-resolves. Cost per render is one path hash +
one map lookup — negligible.

### Edge cases

- **Remote sessions / no local cwd / WASM** → `resolve_header_color` returns
  `None` → no tint (graceful; matches the local-only behavior of existing tab
  colors, and `get_root_for_path` on WASM has no data).
- **Before repo detection completes** → `get_root_for_path` returns `None`, so we
  hash the cwd; once the root is detected a re-render settles to the repo-root
  hue. No flicker when the window opened at the repo root (cwd == root). A window
  opened in a subdirectory may briefly show the subdir hue before settling —
  acceptable.
- **Palette collisions** → 6 buckets means two projects can share a hue; fine for
  telling a handful of windows apart. Expanding the palette later is a one-line
  change to the hash target and is out of scope for v1.
- **Contrast/legibility** → header text and the traffic-light controls sit over a
  subtle ~18% wash; verify legibility on light themes during manual testing.
  Contrast helpers exist (`pick_best_foreground_color`) if adjustment is needed,
  but are not expected to be necessary at 18%.

## Testing

**Unit tests (`header_color_tests.rs`)** — the pure logic, where bugs live:
- `header_color_for_path` is deterministic (same path → same color across calls)
  and stable (assert specific path→color mappings so regressions are caught).
- Distinct representative paths spread across the palette (not all one bucket).
- Path canonicalization: equivalent paths (e.g. trailing slash) hash identically.

**Resolution priority** — a small test that an explicit `TabData.color()`
(`SelectedTabColor::Color(_)`) takes precedence over the directory hash.

**Manual / visual** (`cargo run`):
- Open two windows in different repos → visibly different, subtle header tints.
- `cd` into a subdirectory of the same repo → header color unchanged.
- Switch between tabs in different projects → header re-tints to the active tab.
- Flag off → header renders exactly as today (no tint).
- Light and dark themes → tint is subtle and text stays legible.

## Cleanup / dead code

- Reuse (do **not** duplicate) `TAB_COLOR_OPTIONS`, `coloru_with_opacity` /
  `Blend`, `AnsiColorIdentifier::to_ansi_color`, `canonical_directory_key`, and
  the `canonical_session_pwd_if_local` plumbing.
- The feature is additive; it introduces no dead branches beyond the flag gate.
- Post-launch, once stable, remove `FeatureFlag::WindowHeaderColors` and inline
  its gated branch (via the `remove-feature-flag` skill). Tracked as the flag's
  eventual cleanup.

## Files touched (anticipated)

- `crates/warp_features/src/lib.rs` — add `FeatureFlag::WindowHeaderColors`
  variant + register for all channels.
- `app/src/workspace/header_color.rs` — **new**: `header_color_for_path` + tests.
- `app/src/workspace/mod.rs` — declare the new module.
- `app/src/workspace/view.rs` — `resolve_header_color`; apply the tint in
  `render_tab_bar`; `WINDOW_HEADER_TINT_OPACITY` constant.
