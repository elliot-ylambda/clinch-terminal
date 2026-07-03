# Design: Prominent CLI-agent footer + usage status in the tab bar

**Date:** 2026-07-02
**Status:** Approved (design), pending implementation plan

## Problem

The CLI-agent footer at the bottom of the agent input holds action buttons
(File explorer, Compact, Fork, Rich Input, Settings) plus a small
Claude Code + Codex usage chip. Two issues:

1. The footer bar and its buttons — especially **Fork** and **Compact** — are
   too small / not prominent enough.
2. The usage chip is cramped and only shows weekly-% per tool. There's room to
   show much more (the 5-hour session limit **and** the weekly limit, with
   reset countdowns) if it lives somewhere with more horizontal space.

## Goals

- Make the CLI-agent footer bar more prominent and its buttons bigger (Fork and
  Compact included), as a cohesive row.
- Move the Claude Code + Codex usage status out of the footer and into the
  **window tab bar** (a header surface), where it is always visible for the
  whole window.
- Show richer info inline: for each provider, both the **5-hour session** and
  **weekly** limits with percent and time-until-reset, colored by severity.
- Degrade gracefully when the tab bar runs low on horizontal space, so tabs are
  never pushed off-screen.
- Keep the click-to-expand detail panel (token counts) reachable from the new
  location.

## Non-goals

- No changes to how usage data is scanned/aggregated (`crates/cli_agent_usage`).
- No new user setting to show/hide the header widget (it follows the existing
  "show when there is data" rule). Making it a configurable
  `HeaderToolbarItemKind` is a possible future follow-up, explicitly out of
  scope here.
- No changes to the agent-view (non-CLI) footer button sizing.

## Background: relevant code

- **Footer:** `app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs`
  - `render_cli_mode_footer` renders the CLI-agent footer; wraps content in a
    `Container` with `with_vertical_padding(4.)`.
  - CLI buttons are built in `AgentInputFooter::new` using
    `ButtonSize::AgentInputButton` (via `cli_button_size`).
  - `render_cli_agent_usage_chip_item` builds the current footer usage chip +
    its click-to-open panel overlay. State lives in the footer:
    `cli_agent_usage_panel_open: bool` and `cli_agent_usage_mouse_state:
    MouseStateHandle`. Toggled via
    `AgentInputFooterAction::ToggleCliAgentUsagePanel`.
- **Usage rendering:** `app/src/ai/blocklist/usage/cli_agent_usage_chip.rs`
  - `render_cli_agent_usage_chip(snapshot, appearance, bg) -> Option<Element>`
    — compact chip: `⏱ cc 47%w · cx 55%w`.
  - `render_cli_agent_usage_panel(snapshot, appearance) -> Element` — the
    detailed dropdown (5h %, weekly %, session/today/week/month tokens).
  - `severity_fill(...)` maps `Severity` → themed `Fill`.
- **Usage format helpers (pure, no toolkit):**
  `crates/cli_agent_usage/src/format.rs` — `fmt_pct`, `fmt_reset`,
  `fmt_tokens`, `chip_halves`.
- **Usage data model:** `crates/cli_agent_usage/src/lib.rs` —
  `UsageSnapshot { claude: Provider, codex: Provider }`,
  `Provider { session, today, week, month, plan: Option<PlanLimits> }`,
  `PlanLimits { session: Option<LimitWindow>, weekly: Option<LimitWindow> }`,
  `LimitWindow { percent: f64, resets_at: Option<DateTime<Utc>>, severity }`.
- **Usage model (singleton):** `CliAgentUsageModel` — `::as_ref(app).latest()`
  returns the current `UsageSnapshot`; `::handle(ctx)` is subscribable.
- **Tab bar:** `app/src/workspace/view.rs`
  - `add_configurable_right_side_tab_bar_controls(target, config, ...)` is the
    single shared method that both the **horizontal** and **vertical** tab-bar
    layouts call to populate right-side controls. A `Shrinkable` empty
    placeholder before it lets the right cluster yield space to tabs.
- **Sizing:** `app/src/view_components/action_button.rs` — `enum ButtonSize`
  with per-variant `icon_size`, `font_size`, `button_height`,
  `button_horizontal_padding`, `keystroke_*`, `tooltip_offset`,
  `callout_padding`, `negative_vertical_margin`, `keystroke_sizing`.
- **Responsive primitive:** `crates/warpui_core/src/elements/gui/size_constraint_switch.rs`
  — `SizeConstraintSwitch::new(default_child, [(SizeConstraintCondition, child), ...])`
  picks a child by the incoming max-width `SizeConstraint` at layout time.
  `SizeConstraintCondition::WidthLessThan(f32)`.

## Design

### A. Footer: bigger and more prominent

**New button size.** Add a variant `ButtonSize::AgentInputButtonLarge` to
`action_button.rs`. It mirrors `AgentInputButton` in every `match` arm across
the impl (the codebase forbids wildcard `_` arms, so every method that matches
on `ButtonSize` must gain an explicit arm), with these differences:

- `font_size`: `appearance.monospace_font_size()` (was `monospace − 1.0`).
- `icon_size`: `AgentInputButton`'s line-height computation, bumped ~2–3px
  (e.g. use a larger line-height ratio or add a fixed delta — tuned during
  implementation).
- `button_height`: larger vertical padding term (≈ `UDI_CHIP_VERTICAL_PADDING`
  → a larger constant) so the row reads as a taller, more prominent bar.
- `button_horizontal_padding`: ≈ 6 → 9.
- `keystroke_sizing`, `keystroke_left_spacing`, `tooltip_offset`,
  `callout_padding`, `negative_vertical_margin`, `font_properties`: copy
  `AgentInputButton`'s values (adjust keystroke sizing only if the larger font
  visibly misaligns the keybinding pill).

**Apply it to the CLI footer.** In `AgentInputFooter::new`, change
`cli_button_size` from `ButtonSize::AgentInputButton` to
`ButtonSize::AgentInputButtonLarge`. This covers File explorer, Compact, Fork,
Rich Input, Settings, and the install/update plugin chips (they use
`cli_button_size`). Fork and Compact therefore grow **because the whole CLI
footer row grows together**, keeping heights even. The agent-view footer and
all other `AgentInputButton` call sites are untouched.

**More prominent bar.** In `render_cli_mode_footer`, change the wrapping
`Container`'s `with_vertical_padding(4.)` to `with_vertical_padding(8.)`. Update
the CLI brand-icon sizing to use `ButtonSize::AgentInputButtonLarge.icon_size`
so the leading agent icon matches the enlarged buttons.

### B. Header: usage status in the tab bar

**New render module** `app/src/ai/blocklist/usage/cli_agent_usage_header.rs`,
re-exported from `usage/mod.rs`. It exposes:

```rust
/// The always-visible tab-bar usage widget: three width variants wrapped in a
/// SizeConstraintSwitch. Returns None when neither provider has data.
pub fn render_cli_agent_usage_header(
    snapshot: &UsageSnapshot,
    appearance: &Appearance,
    bg: Fill,
) -> Option<Box<dyn Element>>;
```

Internals:

- Guard: if `chip_halves(snapshot).is_none()` → return `None` (hidden when no
  data), matching current footer behavior.
- Build three children, then wrap with `full` as the default child:
  `SizeConstraintSwitch::new(full, [(WidthLessThan(NARROW_MAX), narrow),
  (WidthLessThan(MEDIUM_MAX), medium)])`.
  `SizeConstraintSwitch` checks conditions in order and picks the **first**
  match, so the narrower condition must be listed **first** (at a narrow width,
  both `WidthLessThan(NARROW_MAX)` and `WidthLessThan(MEDIUM_MAX)` are true and
  we want `narrow` to win). Thresholds `NARROW_MAX < MEDIUM_MAX` (px) tuned
  empirically during implementation.
- **Full** variant, per provider `(label, Provider)`:
  `⏱ {name} 5h {pct}·{reset} │ wk {pct}·{reset}` with:
  - `pct` = `fmt_pct(window.percent)`, colored via `severity_fill(window.severity, ...)`.
  - `reset` = `fmt_reset(window.resets_at, now)`, dimmed (`sub_text_color`).
  - Missing `LimitWindow` (e.g. `plan.and_then(|p| p.session)` is `None`) → `—`.
  - Two providers separated by spacing; a leading clock icon (reuse
    `Icon::Clock`, as the chip does).
- **Medium** variant: same but omit the `·{reset}` fragments.
- **Narrow** variant: reuse `render_cli_agent_usage_chip(snapshot, appearance, bg)`
  (the existing compact `⏱ cc 47%w · cx 55%w`).

Text spans use the monospace font, mirroring `cli_agent_usage_chip.rs`'s `span`
helper (factor the shared helper into the header module or a shared spot rather
than duplicating).

**Placement.** In `add_configurable_right_side_tab_bar_controls`
(`workspace/view.rs`), before the avatar / resource-center controls, add the
header widget wrapped so it can shrink:

- Read `let snapshot = CliAgentUsageModel::as_ref(ctx).latest().clone();`
- `let bg = appearance.theme().surface_1();` (same background the footer chip
  used, so severity/text contrast matches).
- `if let Some(widget) = render_cli_agent_usage_header(&snapshot, appearance, bg)`
  → wrap in a click `Hoverable`/`EventHandler` that dispatches
  `WorkspaceAction::ToggleCliAgentUsagePanel`, and add to `target` with
  appropriate margin. Because both tab layouts call this method, the widget
  appears in both with one edit.

**Click-to-expand panel.** State moves to `Workspace`:

- Add fields `cli_agent_usage_panel_open: bool` and
  `cli_agent_usage_mouse_state: MouseStateHandle` to the `Workspace`-owning view
  struct in `workspace/view.rs`, initialized in its constructor
  (`MouseStateHandle` created once, per the WarpUI mouse-state rule).
- Add `WorkspaceAction::ToggleCliAgentUsagePanel` and handle it by flipping
  `cli_agent_usage_panel_open` + `ctx.notify()`.
- When open, render `render_cli_agent_usage_panel(&snapshot, appearance)` as a
  positioned overlay anchored under the header widget (a `Stack` with a
  positioned overlay child, mirroring the current footer overlay in
  `render_cli_agent_usage_chip_item`). Clicking again (or the action firing)
  closes it.

**Re-render on data updates.** Ensure the `Workspace` view subscribes to
`CliAgentUsageModel::handle(ctx)` with a `ctx.notify()` (the footer already does
this for itself; the tab bar needs its own subscription). If a broader workspace
subscription already forces tab-bar re-render on model changes, reuse it;
otherwise add the subscription in the constructor.

### C. Cleanup, data flow, and edge cases

**Behavior change (intentional):** usage was previously shown only in panes
running a CLI-agent session (inside `render_cli_mode_footer`). It is now shown
**window-wide** whenever usage data exists. This is the desired "more prominent"
outcome and is called out explicitly.

**Dead code to remove** (per repo convention — clean up what this change
obsoletes):

- Delete `AgentInputFooter::render_cli_agent_usage_chip_item` and its call site
  in `render_cli_mode_footer` (the block that appends the chip to
  `right_buttons`).
- Remove footer fields `cli_agent_usage_panel_open` and
  `cli_agent_usage_mouse_state` (initializers in `AgentInputFooter::new` too).
- Remove `AgentInputFooterAction::ToggleCliAgentUsagePanel` and its match arm /
  handler.
- **Keep** `render_cli_agent_usage_chip` (reused by the Narrow header variant)
  and `render_cli_agent_usage_panel` (reused by the header click panel). Their
  `pub use` in `usage/mod.rs` stays; add the new
  `render_cli_agent_usage_header` export.

**Edge cases:**

- The footer chip suppressed itself for viewer / conversation-transcript
  contexts (`shared_status.is_viewer()`, `is_conversation_transcript_context`).
  Those are pane-level concerns and do **not** carry over to a window-level tab
  bar widget — the guard is simply dropped, not ported.
- No usage data → `render_cli_agent_usage_header` returns `None`; the tab bar is
  unaffected (no empty slot, no click target).
- Narrow windows → `SizeConstraintSwitch` selects Medium then Narrow; the
  `Shrinkable` placeholder guarantees tabs keep priority for width.

## Testing

- **Unit (existing crate):** `crates/cli_agent_usage` format tests already cover
  `fmt_pct`/`fmt_reset`/`chip_halves`; no new pure logic is introduced there.
- **Rendering variants:** if the header module has any branch-selection logic
  beyond `SizeConstraintSwitch` (e.g. a helper that formats a provider's
  full/medium string), add a small unit test for that pure string builder
  (place in `cli_agent_usage_header_tests.rs` per repo test convention). The
  `SizeConstraintSwitch` selection itself is framework-tested.
- **Manual:** run the app locally (`cargo run`) and verify:
  1. CLI footer buttons (incl. Fork/Compact) are visibly larger; row heights
     even; agent-view footer unchanged.
  2. Tab bar shows the full usage string with resets when the window is wide;
     dropping reset text at medium width; compact chip when narrow; tabs never
     pushed off-screen.
  3. Clicking the widget opens the detail panel; clicking again closes it.
  4. Severity colors: normal/warning/critical reflected on percents.
  5. With no `~/.claude` or `~/.codex` data, the widget is absent and the tab
     bar looks normal.

## Rollout

Single change, no feature flag (consistent with how the existing usage chip
shipped). The enlarged footer and relocated usage are always on.

## Open items (resolved)

- Header location: **window tab bar** (via
  `add_configurable_right_side_tab_bar_controls`).
- Inline detail: **5h + weekly + resets**, per provider, severity-colored.
- Tight-space behavior: **graceful degradation** (Full → Medium → Narrow).
- Footer sizing: **new `ButtonSize::AgentInputButtonLarge`** applied to the CLI
  footer only, plus increased footer vertical padding.
