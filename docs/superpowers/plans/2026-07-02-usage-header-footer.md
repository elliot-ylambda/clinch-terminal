# Prominent CLI-agent Footer + Usage Status in Tab Bar — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enlarge the CLI-agent input footer (buttons incl. Fork/Compact) and relocate the Claude Code + Codex usage status into the window tab bar with 5h + weekly limits, reset countdowns, and graceful width degradation.

**Architecture:** Add a dedicated larger `ButtonSize::AgentInputButtonLarge` used only by the CLI footer. Build a new `cli_agent_usage_header` render module (three width variants inside a `SizeConstraintSwitch`, reusing the existing pure format helpers and severity coloring). Wire it into the tab bar's shared right-side-controls method in `Workspace`, with click-to-open reusing the existing detail panel. Remove the now-dead footer usage-chip code.

**Tech Stack:** Rust, WarpUI custom element framework (`warpui`/`warpui_core`), `chrono`, the `cli_agent_usage` crate (pure format/aggregate logic).

## Global Constraints

- **No wildcard `_` match arms** when adding/editing `match` on enums (repo rule). `WorkspaceAction`'s `should_save_app_state_on_action` (`app/src/workspace/action.rs:868`) and the `Workspace::handle_action` match (`app/src/workspace/view.rs:23313`) are **exhaustive** — a new variant needs an explicit arm in each.
- **Lint gate:** `./script/format` and the `cargo clippy` invocation in `./script/presubmit` must pass with zero warnings before any PR/push. Clippy runs with `-D warnings`, so unused imports are hard errors — remove them.
- **Inline format args** in `format!`/`println!` etc. (`format!("{x}")`, not `format!("{}", x)`).
- **Context param** named `ctx`, placed last (except when a closure is last).
- **Remove unused params entirely** (don't `_`-prefix); remove dead code rather than leaving it.
- **Tests** live in a sibling file `${filename}_tests.rs`, included at the module end via `#[cfg(test)] #[path = "..."] mod tests;`.
- **MouseStateHandle** must be created once (in the constructor) and cloned at render time — never `Default::default()` inline while rendering.
- **Terminal model locking:** none of these tasks call `TerminalModel::lock()`; don't add any.

## File Structure

- **Modify** `app/src/view_components/action_button.rs` — add `ButtonSize::AgentInputButtonLarge` variant + all its `match` arms.
- **Create** `app/src/view_components/action_button_tests.rs` — sizing unit test (only if no test module exists there; confirmed none does).
- **Modify** `app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs` — use the large size for CLI buttons; bump footer padding; later remove dead usage-chip code.
- **Modify** `app/src/ai/blocklist/usage/cli_agent_usage_chip.rs` — promote `span` + `severity_fill` to `pub(super)` for reuse.
- **Create** `app/src/ai/blocklist/usage/cli_agent_usage_header.rs` — the tab-bar usage widget (three variants + pure `limit_texts` helper).
- **Create** `app/src/ai/blocklist/usage/cli_agent_usage_header_tests.rs` — unit tests for `limit_texts`.
- **Modify** `app/src/ai/blocklist/usage/mod.rs` — declare + re-export the new module.
- **Modify** `app/src/workspace/action.rs` — add `WorkspaceAction::ToggleCliAgentUsagePanel` + its `should_save_app_state_on_action` arm.
- **Modify** `app/src/workspace/view.rs` — `Workspace` fields + init + `CliAgentUsageModel` subscription + render widget in `add_configurable_right_side_tab_bar_controls` + `handle_action` arm.

---

### Task 1: Add `ButtonSize::AgentInputButtonLarge`

A new button size that mirrors `AgentInputButton` but is taller, with a larger icon/font and more horizontal padding. Isolated: nothing uses it yet after this task.

**Files:**
- Modify: `app/src/view_components/action_button.rs` (enum ~line 171; `impl ButtonSize` methods 1313–1535)
- Create: `app/src/view_components/action_button_tests.rs`

**Interfaces:**
- Produces: `ButtonSize::AgentInputButtonLarge` — a new enum variant. Its private sizing methods (`button_horizontal_padding(&self) -> f32`, `keystroke_left_spacing(&self) -> f32`, `font_size(&self, &Appearance) -> f32`, `icon_size(&self, &Appearance, &AppContext) -> f32`, `button_height(&self, &Appearance, &AppContext) -> f32`) return values `>=` the corresponding `AgentInputButton` values, with `button_horizontal_padding` == `9.0` and the `button_height` vertical-padding term == `6.0`.

- [ ] **Step 1: Write the failing test**

Create `app/src/view_components/action_button_tests.rs`:

```rust
use super::ButtonSize;

// The large CLI-footer button must be visibly bigger than the standard
// agent-input button in the size dimensions that don't require a live
// Appearance/AppContext (those are exercised via manual/visual testing).
#[test]
fn agent_input_button_large_is_bigger() {
    assert_eq!(
        ButtonSize::AgentInputButtonLarge.button_horizontal_padding(),
        9.0,
        "large variant should use 9px horizontal padding"
    );
    assert!(
        ButtonSize::AgentInputButtonLarge.button_horizontal_padding()
            > ButtonSize::AgentInputButton.button_horizontal_padding(),
        "large horizontal padding must exceed the standard AgentInputButton (4.0)"
    );
    assert!(
        ButtonSize::AgentInputButtonLarge.keystroke_left_spacing()
            >= ButtonSize::AgentInputButton.keystroke_left_spacing()
    );
}
```

Then add the include at the very end of `app/src/view_components/action_button.rs`:

```rust
#[cfg(test)]
#[path = "action_button_tests.rs"]
mod tests;
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p warp --lib view_components::action_button::tests::agent_input_button_large_is_bigger 2>&1 | tail -20`
Expected: FAIL to compile — `no variant named AgentInputButtonLarge`.

(If the app crate name differs, use `cargo test -p app ...`; discover with `grep -m1 '^name' app/Cargo.toml`.)

- [ ] **Step 3: Add the enum variant**

In `app/src/view_components/action_button.rs`, add the variant after `AgentInputButton` (line ~183):

```rust
    /// Sizing for the CLI-agent input footer buttons — a larger, more prominent
    /// variant of [`ButtonSize::AgentInputButton`].
    AgentInputButtonLarge,
```

- [ ] **Step 4: Add all `match` arms for the new variant**

Add an `AgentInputButtonLarge` arm to every `match self` in `impl ButtonSize` (the compiler will list each non-exhaustive match). Values:

`icon_size` (after the `AgentInputButton` arm, ~1323):

```rust
            ButtonSize::AgentInputButtonLarge => app.font_cache().line_height(
                appearance.monospace_font_size() + 2.0,
                DEFAULT_UI_LINE_HEIGHT_RATIO / 1.4,
            ),
```

`font_size` (~1339):

```rust
            ButtonSize::AgentInputButtonLarge => appearance.monospace_font_size(),
```

`font_properties` (~1352):

```rust
            ButtonSize::AgentInputButtonLarge => Properties::default(),
```

`keystroke_left_spacing` (~1366):

```rust
            ButtonSize::AgentInputButtonLarge => 4.,
```

`keystroke_sizing` (~1436) — copy the `AgentInputButton` body:

```rust
            ButtonSize::AgentInputButtonLarge => UiComponentStyles {
                font_size: Some(appearance.monospace_font_size() - 4.),
                width: Some(appearance.monospace_font_size()),
                height: Some(appearance.monospace_font_size()),
                padding: Some(Coords::default()),
                ..Default::default()
            },
```

`button_height` (~1466) — larger vertical padding term (6.0 vs the standard 3.0):

```rust
            ButtonSize::AgentInputButtonLarge => {
                // Larger vertical padding than AgentInputButton for a more
                // prominent footer bar.
                let vertical_padding = 6.;
                let line_height = app
                    .font_cache()
                    .line_height(self.font_size(appearance), appearance.line_height_ratio());
                2. * vertical_padding + line_height
            }
```

`negative_vertical_margin` (~1488):

```rust
            ButtonSize::AgentInputButtonLarge => None,
```

`button_horizontal_padding` (~1501) — add its own arm (do NOT fold into the `UDIPromptChip | AgentInputButton` arm):

```rust
            ButtonSize::AgentInputButtonLarge => 9.,
```

`tooltip_offset` (~1517):

```rust
            ButtonSize::AgentInputButtonLarge => -8.,
```

`callout_padding` (~1530):

```rust
            ButtonSize::AgentInputButtonLarge => {
                Padding::default().with_vertical(1.).with_horizontal(2.)
            }
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p warp --lib view_components::action_button::tests::agent_input_button_large_is_bigger 2>&1 | tail -20`
Expected: PASS (1 passed).

- [ ] **Step 6: Clippy + commit**

Run: `cargo clippy -p warp --lib 2>&1 | tail -20` — expect no warnings for `action_button.rs`.

```bash
git add app/src/view_components/action_button.rs app/src/view_components/action_button_tests.rs
git commit -m "feat(action-button): add larger AgentInputButtonLarge size"
```

---

### Task 2: Apply the large size to the CLI footer + make the bar more prominent

Switch the CLI-agent footer buttons to the new size and increase the footer's vertical padding. Fork, Compact, File explorer, Rich Input, Settings, and the install/update chips all grow together (they share `cli_button_size`).

**Files:**
- Modify: `app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs` (`cli_button_size` at line 399; `render_cli_mode_footer` `cli_icon_size` at 1583 and wrapping `Container` at 1718)

**Interfaces:**
- Consumes: `ButtonSize::AgentInputButtonLarge` (Task 1).

- [ ] **Step 1: Point `cli_button_size` at the large variant**

At line 399:

```rust
        // CLI agent-specific buttons (only rendered when a CLI agent session is active).
        let cli_button_size = ButtonSize::AgentInputButtonLarge;
```

- [ ] **Step 2: Match the footer brand icon + bar padding**

In `render_cli_mode_footer`, line 1583:

```rust
        let cli_icon_size = ButtonSize::AgentInputButtonLarge.icon_size(appearance, app);
```

At the end of the same function, line 1718, bump the wrapping container's vertical padding:

```rust
        Container::new(content).with_vertical_padding(8.).finish()
```

- [ ] **Step 3: Build to verify it compiles**

Run: `cargo build -p warp 2>&1 | tail -20`
Expected: builds clean (warnings about the not-yet-removed usage chip are fine; they're addressed in Task 6).

- [ ] **Step 4: Manual visual check**

Run: `cargo run` — start a CLI agent session (e.g. run `claude`/`codex` in a pane). Confirm the footer bar is taller and File explorer / Compact / Fork / Rich Input / Settings are visibly larger with even heights. Confirm the agent-view (non-CLI) footer is unchanged.

- [ ] **Step 5: Clippy + commit**

Run: `cargo clippy -p warp --lib 2>&1 | tail -20` — no new warnings from this change.

```bash
git add app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs
git commit -m "feat(cli-footer): enlarge CLI-agent footer buttons and bar"
```

---

### Task 3: New `cli_agent_usage_header` render module

The always-visible tab-bar widget: three width variants (Full / Medium / Narrow) wrapped in a `SizeConstraintSwitch`, reusing the chip module's `span`/`severity_fill` and the crate's pure formatters. Includes a pure `limit_texts` helper with unit tests.

**Files:**
- Modify: `app/src/ai/blocklist/usage/cli_agent_usage_chip.rs` (promote helpers, lines 17 & 26)
- Create: `app/src/ai/blocklist/usage/cli_agent_usage_header.rs`
- Create: `app/src/ai/blocklist/usage/cli_agent_usage_header_tests.rs`
- Modify: `app/src/ai/blocklist/usage/mod.rs` (lines 5–11)

**Interfaces:**
- Consumes: `render_cli_agent_usage_chip(&UsageSnapshot, &Appearance, Fill) -> Option<Box<dyn Element>>` (existing, `pub`); `span(impl Into<String>, Fill, &Appearance) -> Box<dyn Element>` and `severity_fill(Severity, &WarpTheme, Fill) -> Fill` (promoted to `pub(super)` here); `cli_agent_usage::format::{fmt_pct, fmt_reset, chip_halves}`; `cli_agent_usage::{LimitWindow, Provider, Severity, UsageSnapshot}`.
- Produces: `pub fn render_cli_agent_usage_header(snapshot: &UsageSnapshot, appearance: &Appearance, bg: Fill) -> Option<Box<dyn Element>>` (re-exported from `usage`); private `fn limit_texts(Option<LimitWindow>, DateTime<Utc>, bool) -> (String, Option<String>, Severity)`.

- [ ] **Step 1: Write the failing test**

Create `app/src/ai/blocklist/usage/cli_agent_usage_header_tests.rs`:

```rust
use chrono::{Duration, TimeZone, Utc};
use cli_agent_usage::{LimitWindow, Severity};

use super::limit_texts;

#[test]
fn limit_texts_none_is_dash() {
    let now = Utc.with_ymd_and_hms(2026, 7, 2, 12, 0, 0).unwrap();
    let (pct, reset, sev) = limit_texts(None, now, true);
    assert_eq!(pct, "—");
    assert_eq!(reset, None);
    assert_eq!(sev, Severity::Normal);
}

#[test]
fn limit_texts_with_reset() {
    let now = Utc.with_ymd_and_hms(2026, 7, 2, 12, 0, 0).unwrap();
    let w = LimitWindow {
        percent: 47.0,
        resets_at: Some(now + Duration::hours(2)),
        severity: Severity::Warning,
    };
    let (pct, reset, sev) = limit_texts(Some(w), now, true);
    assert_eq!(pct, "47%");
    assert_eq!(reset.as_deref(), Some("in 2h"));
    assert_eq!(sev, Severity::Warning);
}

#[test]
fn limit_texts_without_reset_omits_countdown() {
    let now = Utc.with_ymd_and_hms(2026, 7, 2, 12, 0, 0).unwrap();
    let w = LimitWindow {
        percent: 8.0,
        resets_at: Some(now + Duration::days(6)),
        severity: Severity::Normal,
    };
    let (pct, reset, _sev) = limit_texts(Some(w), now, false);
    assert_eq!(pct, "8%");
    assert_eq!(reset, None);
}
```

- [ ] **Step 2: Promote the shared helpers**

In `app/src/ai/blocklist/usage/cli_agent_usage_chip.rs`, change the two private helpers to `pub(super)` (so the sibling header module can reuse them):

Line 17:

```rust
pub(super) fn severity_fill(severity: Severity, theme: &WarpTheme, bg: Fill) -> Fill {
```

Line 26:

```rust
pub(super) fn span(text: impl Into<String>, color: Fill, appearance: &Appearance) -> Box<dyn Element> {
```

- [ ] **Step 3: Create the header module**

Create `app/src/ai/blocklist/usage/cli_agent_usage_header.rs`:

```rust
use chrono::{DateTime, Utc};
use cli_agent_usage::format::{chip_halves, fmt_pct, fmt_reset};
use cli_agent_usage::{LimitWindow, Provider, Severity, UsageSnapshot};

use warp_core::ui::theme::Fill;
use warp_core::ui::Icon;
use warpui::elements::{
    ConstrainedBox, Container, CrossAxisAlignment, Empty, Flex, ParentElement,
    SizeConstraintCondition, SizeConstraintSwitch,
};
use warpui::Element;

use super::cli_agent_usage_chip::{render_cli_agent_usage_chip, severity_fill, span};
use crate::appearance::Appearance;

/// Below this available width (px) the widget collapses to the compact chip.
const NARROW_MAX: f32 = 340.;
/// Below this available width (px) the widget drops the reset countdowns.
const MEDIUM_MAX: f32 = 560.;

/// Text pieces for one limit window: the percent (colored by the returned
/// severity), an optional dimmed reset countdown, and the severity. A `None`
/// window renders as `"—"` with `Normal` severity and no reset.
fn limit_texts(
    window: Option<LimitWindow>,
    now: DateTime<Utc>,
    include_reset: bool,
) -> (String, Option<String>, Severity) {
    match window {
        None => ("—".to_string(), None, Severity::Normal),
        Some(w) => {
            let pct = fmt_pct(w.percent);
            let reset = include_reset.then(|| fmt_reset(w.resets_at, now));
            (pct, reset, w.severity)
        }
    }
}

/// One provider's inline segment: `{name} 5h {pct}[· {reset}] │ wk {pct}[· {reset}]`.
/// Percents are severity-colored; labels and resets are dimmed.
fn provider_segment(
    name: &str,
    provider: &Provider,
    now: DateTime<Utc>,
    include_resets: bool,
    appearance: &Appearance,
    bg: Fill,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let main = theme.main_text_color(bg);
    let sub = theme.sub_text_color(bg);
    let session = provider.plan.and_then(|p| p.session);
    let weekly = provider.plan.and_then(|p| p.weekly);

    let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
    row.add_child(span(format!("{name} "), main, appearance));

    for (idx, (label, window)) in [("5h ", session), ("wk ", weekly)].into_iter().enumerate() {
        if idx > 0 {
            row.add_child(span(" │ ", sub, appearance));
        }
        let (pct, reset, severity) = limit_texts(window, now, include_resets);
        row.add_child(span(label, sub, appearance));
        row.add_child(span(pct, severity_fill(severity, theme, bg), appearance));
        if let Some(reset) = reset {
            row.add_child(span(format!(" · {reset}"), sub, appearance));
        }
    }
    row.finish()
}

/// The full/medium inline layout: clock icon + Claude segment + gap + Codex segment.
fn inline_row(
    snapshot: &UsageSnapshot,
    now: DateTime<Utc>,
    include_resets: bool,
    appearance: &Appearance,
    bg: Fill,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let icon_size = appearance.monospace_font_size();
    let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
    row.add_child(
        Container::new(
            ConstrainedBox::new(Icon::Clock.to_warpui_icon(theme.main_text_color(bg)).finish())
                .with_width(icon_size)
                .with_height(icon_size)
                .finish(),
        )
        .with_margin_right(4.)
        .finish(),
    );
    row.add_child(provider_segment("Claude", &snapshot.claude, now, include_resets, appearance, bg));
    row.add_child(Container::new(Empty::new().finish()).with_margin_right(12.).finish());
    row.add_child(provider_segment("Codex", &snapshot.codex, now, include_resets, appearance, bg));
    row.finish()
}

/// The always-visible tab-bar usage widget. Three width variants inside a
/// `SizeConstraintSwitch`; `None` when neither provider has data (widget hidden).
pub fn render_cli_agent_usage_header(
    snapshot: &UsageSnapshot,
    appearance: &Appearance,
    bg: Fill,
) -> Option<Box<dyn Element>> {
    // Hidden when neither tool has data — same rule as the footer chip.
    chip_halves(snapshot)?;
    let now = Utc::now();

    let full = inline_row(snapshot, now, true, appearance, bg);
    let medium = inline_row(snapshot, now, false, appearance, bg);
    let narrow =
        render_cli_agent_usage_chip(snapshot, appearance, bg).unwrap_or_else(|| Empty::new().finish());

    // Conditions are checked in order; the narrower condition must come first so
    // it wins when both are satisfied. `full` is the default (widest) child.
    Some(
        SizeConstraintSwitch::new(
            full,
            vec![
                (SizeConstraintCondition::WidthLessThan(NARROW_MAX), narrow),
                (SizeConstraintCondition::WidthLessThan(MEDIUM_MAX), medium),
            ],
        )
        .finish(),
    )
}

#[cfg(test)]
#[path = "cli_agent_usage_header_tests.rs"]
mod tests;
```

Note on `.finish()`: if `SizeConstraintSwitch` has no `.finish()` builder, box it directly with `Box::new(SizeConstraintSwitch::new(...))`. Verify against its definition in `crates/warpui_core/src/elements/gui/size_constraint_switch.rs` (it implements `Element`); adjust the final expression accordingly.

- [ ] **Step 4: Register the module**

In `app/src/ai/blocklist/usage/mod.rs`, add the module declaration (next to line 5–6) and re-export (next to line 10–11):

```rust
mod cli_agent_usage_chip;
mod cli_agent_usage_header;
mod cli_agent_usage_model;
```

```rust
pub use cli_agent_usage_chip::{render_cli_agent_usage_chip, render_cli_agent_usage_panel};
pub use cli_agent_usage_header::render_cli_agent_usage_header;
pub use cli_agent_usage_model::CliAgentUsageModel;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p warp --lib ai::blocklist::usage::cli_agent_usage_header::tests 2>&1 | tail -20`
Expected: PASS (3 passed).

- [ ] **Step 6: Clippy + commit**

Run: `cargo clippy -p warp --lib 2>&1 | tail -30` — resolve any warnings (e.g. an unused import if `SizeConstraintSwitch` boxing differs).

```bash
git add app/src/ai/blocklist/usage/cli_agent_usage_chip.rs \
        app/src/ai/blocklist/usage/cli_agent_usage_header.rs \
        app/src/ai/blocklist/usage/cli_agent_usage_header_tests.rs \
        app/src/ai/blocklist/usage/mod.rs
git commit -m "feat(usage): add tab-bar usage header widget with responsive variants"
```

---

### Task 4: Add `WorkspaceAction::ToggleCliAgentUsagePanel`

Plumb a new workspace action that toggles the detail panel. Exhaustive matches require explicit arms.

**Files:**
- Modify: `app/src/workspace/action.rs` (enum ~line 413; `should_save_app_state_on_action` false-arm list, near line 1060)

**Interfaces:**
- Produces: `WorkspaceAction::ToggleCliAgentUsagePanel` (unit variant), consumed by Task 5.

- [ ] **Step 1: Add the variant**

In `app/src/workspace/action.rs`, add near the other header-toolbar actions (~line 413, after `ShowHeaderToolbarContextMenu { position }`):

```rust
    /// Toggle the expanded Claude Code + Codex usage panel anchored under the
    /// tab-bar usage status widget.
    ToggleCliAgentUsagePanel,
```

- [ ] **Step 2: Add its `should_save_app_state_on_action` arm**

This match is exhaustive with no wildcard. Toggling a transient UI panel does not need an app-state save, so add it to the `false` group (the list that includes `ShowHeaderToolbarContextMenu { .. }` around line 1060):

```rust
            | ShowHeaderToolbarContextMenu { .. }
            | ToggleCliAgentUsagePanel
```

- [ ] **Step 3: Build to verify exhaustiveness is satisfied**

Run: `cargo build -p warp 2>&1 | tail -20`
Expected: compile error is now about `Workspace::handle_action` missing the arm (added in Task 5) — that's expected. If instead it complains about `should_save_app_state_on_action` being non-exhaustive, fix the arm here.

Because Task 5 supplies the `handle_action` arm, commit Tasks 4 and 5 together if you prefer a green build at each commit; otherwise commit now and expect a transient non-exhaustive error until Task 5. **Recommended:** do not commit here — proceed to Task 5 and commit them together.

---

### Task 5: Render the usage widget in the tab bar + click-to-open panel

Add `Workspace` state (panel-open + mouse handle), subscribe to `CliAgentUsageModel`, render the widget in the shared right-side-controls method wrapped with click + overlay, and handle the toggle action.

**Files:**
- Modify: `app/src/workspace/view.rs` — imports (blocks at 55–120 area); `pub struct Workspace` (line 982); constructor `Self { ... }` (line 3279) and its subscription section (~2585–4455); `add_configurable_right_side_tab_bar_controls` (line 20671, insert before 20722); `handle_action` (before the closing `};` at 25591).

**Interfaces:**
- Consumes: `render_cli_agent_usage_header` + `render_cli_agent_usage_panel` + `CliAgentUsageModel` (from `crate::ai::blocklist::usage`); `WorkspaceAction::ToggleCliAgentUsagePanel` (Task 4).

- [ ] **Step 1: Add imports**

In `app/src/workspace/view.rs`, add to the existing `warpui::elements::{...}` block (lines ~91–99) the two responsive types:

```rust
    SizeConstraintCondition, SizeConstraintSwitch,
```

Add a `use` for the usage module near the other `crate::ai::blocklist` imports:

```rust
use crate::ai::blocklist::usage::{
    render_cli_agent_usage_header, render_cli_agent_usage_panel, CliAgentUsageModel,
};
```

(`Hoverable`, `Stack`, `OffsetPositioning`, `ChildAnchor`, `ParentAnchor`, `ParentOffsetBounds`, `MouseStateHandle`, `Cursor`, `Appearance` are already imported.)

- [ ] **Step 2: Add the `Workspace` fields**

In `pub struct Workspace {` (line 982), add near the other `MouseStateHandle` fields:

```rust
    /// Whether the tab-bar Claude Code + Codex usage detail panel is expanded.
    cli_agent_usage_panel_open: bool,
    /// Mouse-tracking handle for the tab-bar usage widget's `Hoverable`
    /// (created once here, cloned at render time).
    cli_agent_usage_mouse_state: MouseStateHandle,
```

- [ ] **Step 3: Initialize the fields**

In the constructor's `let mut ws = Self {` (line 3279), add:

```rust
            cli_agent_usage_panel_open: false,
            cli_agent_usage_mouse_state: Default::default(),
```

- [ ] **Step 4: Subscribe to `CliAgentUsageModel`**

In the constructor's subscription section (mirror the pattern at line 2807), add:

```rust
        ctx.subscribe_to_model(&CliAgentUsageModel::handle(ctx), |_, _, _, ctx| {
            // Re-render the tab bar when a usage scan completes.
            ctx.notify();
        });
```

- [ ] **Step 5: Handle the toggle action**

In `handle_action` (before the closing `};` at line 25591), add an arm:

```rust
            ToggleCliAgentUsagePanel => {
                self.cli_agent_usage_panel_open = !self.cli_agent_usage_panel_open;
                ctx.notify();
            }
```

- [ ] **Step 6: Render the widget in the tab bar**

In `add_configurable_right_side_tab_bar_controls` (line 20671), insert BEFORE the avatar block at line 20722:

```rust
        // Claude Code + Codex usage status. Window-global: shown whenever usage
        // data exists (not tied to a focused pane). Click toggles the detail panel.
        {
            let snapshot = CliAgentUsageModel::as_ref(ctx).latest().clone();
            let bg = appearance.theme().surface_1();
            if let Some(widget) = render_cli_agent_usage_header(&snapshot, appearance, bg) {
                let hover = Hoverable::new(
                    self.cli_agent_usage_mouse_state.clone(),
                    move |_state| widget,
                )
                .on_click(|ctx, _app, _position| {
                    ctx.dispatch_typed_action(WorkspaceAction::ToggleCliAgentUsagePanel);
                })
                .with_cursor(Cursor::PointingHand)
                .finish();

                let mut stack = Stack::new().with_child(hover);
                if self.cli_agent_usage_panel_open {
                    stack.add_positioned_overlay_child(
                        render_cli_agent_usage_panel(&snapshot, appearance),
                        OffsetPositioning::offset_from_parent(
                            vec2f(0., 4.),
                            ParentOffsetBounds::WindowByPosition,
                            ParentAnchor::BottomRight,
                            ChildAnchor::TopRight,
                        ),
                    );
                }

                // Shrinkable so the widget yields width to tabs; the
                // SizeConstraintSwitch inside picks Full/Medium/Narrow to fit.
                target.add_child(
                    Container::new(Shrinkable::new(1., stack.finish()).finish())
                        .with_margin_left(TAB_BAR_PADDING_LEFT)
                        .finish(),
                );
            }
        }

```

Notes:
- `Hoverable::new(mouse_state, closure)` — the closure captures `widget` by move and returns it; mirror the footer call at `agent_input_footer/mod.rs:1555`. If `Hoverable`'s closure must return a fresh element each call (borrow issues), build the widget inside the closure instead by capturing `snapshot`/`appearance`/`bg` and calling `render_cli_agent_usage_header` there.
- `vec2f`, `Shrinkable`, `TAB_BAR_PADDING_LEFT`, `Container` are already used in this function/file.
- Anchor `BottomRight → TopRight` opens the 320px panel downward, right-aligned to the widget, so it doesn't overflow the window's right edge.

- [ ] **Step 7: Build**

Run: `cargo build -p warp 2>&1 | tail -30`
Expected: clean build. Fix any `Hoverable` closure/borrow issue per the note in Step 6.

- [ ] **Step 8: Manual verification (the responsive behavior is the risk area)**

Run: `cargo run`. Verify:
1. With Claude/Codex usage data present, the tab bar shows `⏱ Claude 5h ..% · in .. │ wk ..% · in ..   Codex ...` on a wide window.
2. Narrow the window: reset countdowns drop (Medium), then it collapses to the compact `⏱ cc ..%w · cx ..%w` (Narrow). Tabs are never pushed off-screen.
3. Click the widget → the detail panel (5h/weekly + session/today/week/month tokens) opens below it; click again → closes.
4. Severity colors show on percents (normal/warning/critical).
5. With no `~/.claude` or `~/.codex` data, the widget is absent and the tab bar looks normal.

If degradation doesn't trigger, tune `NARROW_MAX`/`MEDIUM_MAX` in `cli_agent_usage_header.rs` and/or the `Shrinkable` weight, then rebuild.

- [ ] **Step 9: Clippy + commit (together with Task 4)**

Run: `./script/format && cargo clippy -p warp --lib 2>&1 | tail -30` — zero warnings.

```bash
git add app/src/workspace/action.rs app/src/workspace/view.rs
git commit -m "feat(tab-bar): show Claude Code + Codex usage status with detail panel"
```

---

### Task 6: Remove the dead footer usage-chip code

The usage chip now lives in the tab bar, so remove its footer implementation, state, action, subscription, and any imports clippy flags as unused.

**Files:**
- Modify: `app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs`

**Interfaces:**
- Removes: `AgentInputFooter::render_cli_agent_usage_chip_item`, fields `cli_agent_usage_panel_open`/`cli_agent_usage_mouse_state`, `AgentInputFooterAction::ToggleCliAgentUsagePanel`. `render_cli_agent_usage_chip`/`render_cli_agent_usage_panel` remain `pub` (used by the header/panel) — only the footer's *imports* of them go away.

- [ ] **Step 1: Remove the chip render method and its call site**

Delete the method `render_cli_agent_usage_chip_item` (lines 1538–1579). Delete its call site in `render_cli_mode_footer` (lines 1695–1701):

```rust
        if let Some(chip) = self.render_cli_agent_usage_chip_item(
            &shared_status,
            is_conversation_transcript_context,
            app,
        ) {
            right_buttons.add_child(chip);
        }
```

- [ ] **Step 2: Remove the fields**

Delete the struct fields (lines 262–266):

```rust
    // Always-on CLI-agent (Claude Code + Codex) usage chip: whether its
    // click-to-open panel is currently expanded, and the mouse-tracking handle
    // for its `Hoverable` (created once here, cloned at render time).
    cli_agent_usage_panel_open: bool,
    cli_agent_usage_mouse_state: MouseStateHandle,
```

Delete their initializers in `AgentInputFooter::new` (lines 919–920):

```rust
            cli_agent_usage_panel_open: false,
            cli_agent_usage_mouse_state: Default::default(),
```

- [ ] **Step 3: Remove the action variant, its dispatch, and its handler**

Delete the enum variant (lines 2547–2548):

```rust
    /// Toggle the expanded CLI-agent usage panel above the footer chip.
    ToggleCliAgentUsagePanel,
```

Delete the handler arm (lines 2638–2641):

```rust
            AgentInputFooterAction::ToggleCliAgentUsagePanel => {
                self.cli_agent_usage_panel_open = !self.cli_agent_usage_panel_open;
                ctx.notify();
            }
```

(The dispatch call at line 1560 is inside `render_cli_agent_usage_chip_item`, already deleted in Step 1.)

- [ ] **Step 4: Remove the now-dead usage-model subscription**

Delete the footer's usage subscription (lines 747–749), since the footer no longer displays usage:

```rust
        ctx.subscribe_to_model(&CliAgentUsageModel::handle(ctx), |_, _, _, ctx| {
            ctx.notify()
        });
```

- [ ] **Step 5: Remove newly-unused imports**

Edit the import block at lines 56–59 to drop the three usage items:

```rust
use crate::ai::blocklist::usage::icon_for_context_window_usage;
```

(Only `icon_for_context_window_usage` remains used — verify with `grep -n "render_cli_agent_usage_chip\|render_cli_agent_usage_panel\|CliAgentUsageModel" app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs`, which should return nothing after this task.)

- [ ] **Step 6: Build — let clippy name any other unused imports**

Run: `cargo clippy -p warp --lib 2>&1 | tail -40`
Expected: it compiles. If clippy reports further unused imports now that the chip method is gone (candidates that the method used exclusively: `Empty`, `Cursor`, possibly `Hoverable`/`Stack`/`OffsetPositioning` if not used elsewhere in the file), remove exactly the imports clippy names. Do NOT remove imports still used elsewhere — clippy is authoritative. Re-run until zero warnings.

- [ ] **Step 7: Confirm no lingering references + format**

Run: `grep -rn "render_cli_agent_usage_chip_item\|ToggleCliAgentUsagePanel\|cli_agent_usage_panel_open" app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs`
Expected: no output.

Run: `./script/format`

- [ ] **Step 8: Manual check + commit**

Run: `cargo run` — start a CLI agent session; confirm the footer no longer shows the usage chip, and the tab bar still shows usage (from Task 5).

```bash
git add app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs
git commit -m "refactor(cli-footer): remove usage chip now shown in the tab bar"
```

---

### Task 7: Full presubmit + branch wrap-up

**Files:** none (verification only).

- [ ] **Step 1: Format + clippy (presubmit versions)**

Run: `./script/format`
Run: `cargo clippy --workspace --all-targets --all-features --tests -- -D warnings 2>&1 | tail -40`
Expected: zero warnings.

- [ ] **Step 2: Run the touched crates' tests**

Run: `cargo nextest run -p warp 2>&1 | tail -30` (adjust crate name to match `app/Cargo.toml`).
Expected: the new `action_button` and `cli_agent_usage_header` tests pass; no regressions.

- [ ] **Step 3: Final manual smoke test**

Run: `cargo run`. Re-verify all five checks from Task 5 Step 8 plus the enlarged footer from Task 2 Step 4, together in one session.

- [ ] **Step 4: Decide integration**

Use `superpowers:finishing-a-development-branch` to choose merge/PR/cleanup. If opening a PR, use `.github/pull_request_template.md` and add a `CHANGELOG-IMPROVEMENT:` line (e.g. "Moved Claude Code + Codex usage status into the tab bar with 5-hour and weekly limits; enlarged the CLI-agent footer").

---

## Self-Review

**Spec coverage:**
- Enlarge footer / Fork+Compact bigger → Tasks 1–2. ✓
- New `ButtonSize::AgentInputButtonLarge`, CLI-footer-only → Task 1 (variant) + Task 2 (application); agent-view footer untouched (only `cli_button_size` changed). ✓
- Footer bar more prominent (vertical padding 4→8) → Task 2 Step 2. ✓
- Usage moved to window tab bar via `add_configurable_right_side_tab_bar_controls` (both layouts) → Task 5 Step 6. ✓
- Inline 5h + weekly + resets, severity-colored, dimmed resets → Task 3 `provider_segment` + `limit_texts`. ✓
- Graceful degradation Full→Medium→Narrow via `SizeConstraintSwitch`, narrower condition first → Task 3 `render_cli_agent_usage_header`. ✓
- Click-to-expand reuses `render_cli_agent_usage_panel`; state on `Workspace` → Task 5 Steps 2–6. ✓
- `WorkspaceAction::ToggleCliAgentUsagePanel` + exhaustive arms → Task 4 + Task 5 Step 5. ✓
- Re-render on data update (subscription) → Task 5 Step 4. ✓
- Window-global visibility (guard `chip_halves` Some; no viewer/transcript guard) → Task 3 (`chip_halves(snapshot)?`), guard intentionally not ported. ✓
- Dead code removal (method, fields, action, subscription, imports); keep `render_cli_agent_usage_chip`/`_panel` pub → Task 6. ✓
- No data → widget absent → Task 3 returns `None`; Task 5 `if let Some(..)`. ✓
- Testing: format tests already in crate; pure `limit_texts` tested (Task 3); sizing tested (Task 1); manual checks (Tasks 2,5,7). ✓

**Placeholder scan:** No TBD/TODO. The only deferred values are `NARROW_MAX`/`MEDIUM_MAX` and the `Shrinkable` weight, which have concrete starting values (340./560./1.) plus an explicit empirical-tuning step — not placeholders. The `.finish()`-vs-`Box::new` and `Hoverable` closure notes give concrete fallbacks, not vague instructions.

**Type consistency:** `render_cli_agent_usage_header(&UsageSnapshot, &Appearance, Fill) -> Option<Box<dyn Element>>` is defined in Task 3 and called with those exact args in Task 5. `limit_texts(Option<LimitWindow>, DateTime<Utc>, bool) -> (String, Option<String>, Severity)` matches its tests. `WorkspaceAction::ToggleCliAgentUsagePanel` (unit variant) is added in Task 4 and matched in Task 5. `severity_fill`/`span` signatures unchanged, only visibility widened to `pub(super)`. `CliAgentUsageModel::as_ref(app).latest()` / `::handle(ctx)` match existing footer usage. Panel/chip functions keep their existing public signatures.
