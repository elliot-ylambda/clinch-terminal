# Agent attention badges + notifications — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Claude/Codex panes in Clinch show **done** (✓) and **asking** (❗) status on the horizontal tab strip, sidebar, and pane header, and push a macOS notification when an agent asks for input on a pane the user isn't looking at — with settings to control it.

**Architecture:** The consuming pipeline (state machine → brand-circle status badge → desktop notification → in-app mailbox) already ships and runs without login. We (1) install Warp's marketplace plugins so the agents *emit* the OSC-777 events the pipeline consumes, (2) enable the `codex_plugin` build feature so Codex's rich events aren't dropped, (3) add the one missing render surface (horizontal tab), (4) narrow the notification focus gate from window-level to pane-level, and (5) add one settings toggle (reusing three existing ones).

**Tech Stack:** Rust (the bespoke `warpui` retained-mode UI framework), bash (the `tools/agent-resume` capture/replay layer), `cargo` features, macOS desktop notifications.

**Spec:** `docs/superpowers/specs/2026-06-30-agent-attention-badges-design.md`

## Global Constraints

- **macOS only.** Build the app via `./tools/agent-resume/build-app.sh` (OSS channel, compiles the `default` cargo feature set; no `--no-default-features`).
- **Do NOT gate any new behavior on login / `is_any_ai_enabled`.** Clinch is a logged-out skip_login build; the consumer/notification paths are intentionally not AI-gated.
- **No new dependencies.** `codex_plugin = []` is an empty cargo feature; nothing else may be added to `Cargo.toml` deps.
- **Plugin install is best-effort.** It must never abort `install.sh` and must run non-interactively (`</dev/null`); warn-and-continue on any failure (offline, missing CLI).
- **Keep the Codex OSC-9 fallback** (covers the no-plugin case). Do not remove it.
- **New settings default to `true`** and live under `NotificationsSettings`'s existing `#[serde(default)]` for backward compatibility.
- **The emitter wire format is authored by Warp's plugin, not by us.** We never print OSC-777 ourselves. (Reference only: title `warp://cli-agent`, body `{"v":1,"agent":"claude"|"codex","event":"stop"|"question_asked"|…}`.)
- **No dead code.** Reuse `is_navigated_away_from_window` (don't orphan it); reuse `render_icon_with_status` / `terminal_view_agent_icon_variant` (don't duplicate rendering).

---

### Task 1: Install the emitter plugins from `install.sh`

Adds the only thing missing for the existing pipeline to light up: the agents' Warp notification plugins. Extracted into a sourceable, source-guarded script so it is unit-testable with fake `claude`/`codex` binaries (mirrors `claude-session-start.sh`'s `BASH_SOURCE`-guard pattern).

**Files:**
- Create: `tools/agent-resume/install-agent-plugins.sh`
- Modify: `tools/agent-resume/install.sh` (call the new script before the final summary; install it into `$BIN`)
- Test: `tools/agent-resume/tests/test_agent_plugins_install.sh`
- Modify: `tools/agent-resume/README.md` (document the plugin install)

**Interfaces:**
- Produces: a shell function `warp_install_agent_notification_plugins` that, for each of `claude` and `codex` present on PATH, runs the agent's marketplace-add + plugin-install commands non-interactively, best-effort.

- [ ] **Step 1: Write the failing test**

Create `tools/agent-resume/tests/test_agent_plugins_install.sh`:

```bash
#!/usr/bin/env bash
# Verifies warp_install_agent_notification_plugins runs the right plugin CLI commands
# when claude/codex are present, skips cleanly when they're absent, and never aborts.
set -uo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
source "$HERE/install-agent-plugins.sh"   # source-guard: defines the fn, runs nothing

fail() { echo "FAIL: $1"; exit 1; }
TMP="$(mktemp -d)"; mkdir -p "$TMP/bin"; LOG="$TMP/calls.log"; : > "$LOG"

# Fake claude/codex that record their argv (one line per invocation), exit 0.
for tool in claude codex; do
  cat > "$TMP/bin/$tool" <<EOF
#!/usr/bin/env bash
echo "$tool \$*" >> "$LOG"
EOF
  chmod +x "$TMP/bin/$tool"
done

# Case A: both present -> all four commands recorded.
PATH="$TMP/bin:$PATH" warp_install_agent_notification_plugins >/dev/null 2>&1 \
  || fail "function returned non-zero with both tools present"
grep -qx "claude plugin marketplace add warpdotdev/claude-code-warp" "$LOG" || fail "missing claude marketplace add"
grep -qx "claude plugin install warp@claude-code-warp"               "$LOG" || fail "missing claude install"
grep -qx "codex plugin marketplace add warpdotdev/codex-warp"        "$LOG" || fail "missing codex marketplace add"
grep -qx "codex plugin add warp@codex-warp"                          "$LOG" || fail "missing codex add"

# Case B: a tool that fails must not abort the function (best-effort).
cat > "$TMP/bin/claude" <<'EOF'
#!/usr/bin/env bash
exit 3
EOF
chmod +x "$TMP/bin/claude"
PATH="$TMP/bin:$PATH" warp_install_agent_notification_plugins >/dev/null 2>&1 \
  || fail "function aborted when a plugin command failed"

# Case C: tools absent -> still exits 0, records nothing new.
# Use an ISOLATED empty PATH (NOT ":$PATH") so command -v cannot fall through to a
# real claude/codex on the host and run live plugin commands.
mkdir -p "$TMP/empty"
: > "$LOG"
PATH="$TMP/empty" warp_install_agent_notification_plugins >/dev/null 2>&1 \
  || fail "function aborted when tools absent"
[[ -s "$LOG" ]] && fail "recorded calls when no tools on PATH"

echo "PASS"
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bash tools/agent-resume/tests/test_agent_plugins_install.sh`
Expected: FAIL (`install-agent-plugins.sh` does not exist → `source` error).

- [ ] **Step 3: Create `tools/agent-resume/install-agent-plugins.sh`**

```bash
#!/usr/bin/env bash
# Installs Warp's CLI-agent notification plugins into Claude Code / Codex so that running
# them inside Clinch emits the OSC-777 `warp://cli-agent` status events Clinch already
# consumes (badge on tabs + desktop notifications). Best-effort: never aborts, never blocks.
#
# Sourcing this file only defines the function (so the tests can exercise it); the install
# runs only when the file is executed directly.

warp_install_agent_notification_plugins() {
  # Claude
  if command -v claude >/dev/null 2>&1; then
    echo "Installing Claude notification plugin (warp@claude-code-warp)..."
    claude plugin marketplace add warpdotdev/claude-code-warp </dev/null >/dev/null 2>&1 \
      || echo "  warn: 'claude plugin marketplace add' failed (offline?) -- skipping"
    claude plugin install warp@claude-code-warp </dev/null >/dev/null 2>&1 \
      || echo "  warn: 'claude plugin install' failed -- skipping"
  else
    echo "claude not on PATH -- skipping Claude notification plugin"
  fi
  # Codex
  if command -v codex >/dev/null 2>&1; then
    echo "Installing Codex notification plugin (warp@codex-warp)..."
    codex plugin marketplace add warpdotdev/codex-warp </dev/null >/dev/null 2>&1 \
      || echo "  warn: 'codex plugin marketplace add' failed (offline?) -- skipping"
    codex plugin add warp@codex-warp </dev/null >/dev/null 2>&1 \
      || echo "  warn: 'codex plugin add' failed -- skipping"
  else
    echo "codex not on PATH -- skipping Codex notification plugin"
  fi
  return 0
}

# Run only when executed directly; sourcing (tests) just loads the function.
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  warp_install_agent_notification_plugins
fi
```

Make it executable:

Run: `chmod +x tools/agent-resume/install-agent-plugins.sh`

- [ ] **Step 4: Run test to verify it passes**

Run: `bash tools/agent-resume/tests/test_agent_plugins_install.sh`
Expected: `PASS`

- [ ] **Step 5: Wire it into `install.sh`**

In `tools/agent-resume/install.sh`, add the new script to the `install -m 0755` list (line ~17, alongside the other scripts copied into `$BIN`):

```bash
install -m 0755 "$SRC/warp-agent-resume" "$SRC/claude-session-start.sh" \
  "$SRC/codex-session-start.sh" "$SRC/codex-session-end.sh" \
  "$SRC/install-agent-plugins.sh" "$BIN/"
```

Then, just before the final `echo ""` / `echo "Done. ..."` summary block (line ~80), source-and-run the function:

```bash
# Install the CLI-agent notification plugins (best-effort) so Claude/Codex emit the
# status events that drive tab badges + desktop notifications in Clinch.
source "$SRC/install-agent-plugins.sh" 2>/dev/null && warp_install_agent_notification_plugins || true
```

- [ ] **Step 6: Update README**

In `tools/agent-resume/README.md`, under "Install (capture layer)", add a short paragraph:

```markdown
The installer also runs `install-agent-plugins.sh`, which installs Warp's CLI-agent
notification plugins into Claude (`warp@claude-code-warp`) and Codex (`warp@codex-warp`).
These make the agents emit OSC-777 status events that Clinch turns into tab badges and
desktop notifications. It is best-effort (skips a missing CLI, warns and continues if
offline) and requires restarting the agent to take effect. Removing the plugins
(`claude plugin uninstall warp@claude-code-warp`) disables the badges/notifications;
everything else keeps working.
```

Add the file to the README "Files" table too:

```markdown
| `install-agent-plugins.sh` | install Warp's Claude/Codex notification plugins (emit the OSC-777 status events) |
```

- [ ] **Step 7: Commit**

```bash
git add tools/agent-resume/install-agent-plugins.sh tools/agent-resume/install.sh \
        tools/agent-resume/tests/test_agent_plugins_install.sh tools/agent-resume/README.md
git commit -m "agent-resume: install Warp CLI-agent notification plugins"
```

---

### Task 2: Enable structured Codex events (`codex_plugin` feature)

Without this, the consumer drops Codex's structured OSC-777 (`view.rs:13060-13066`), so Codex can only ever signal `Success` via OSC-9 — never `Blocked` — making "push when Codex asks a question" impossible.

**Files:**
- Modify: `app/Cargo.toml` (the `default = [ … ]` array, near line 666)

**Interfaces:**
- Produces: `FeatureFlag::CodexPlugin.is_enabled() == true` in the Clinch build (via the existing `#[cfg(feature = "codex_plugin")]` bridge at `app/src/features.rs:486`).

- [ ] **Step 1: Add the feature to the default set**

In `app/Cargo.toml`, in the `default = [` array, add `"codex_plugin"` immediately after the existing `"codex_notifications"` line (line ~666):

```toml
    "codex_notifications",
    "codex_plugin",
```

- [ ] **Step 2: Verify the feature resolves on**

Run: `cargo tree -e features -p warp 2>/dev/null | grep -c 'codex_plugin' || true`
(Optional sanity.) Primary check is the compile in the next step; the flag is wired purely through the existing cfg bridge, so no code change is needed beyond this line.

- [ ] **Step 3: Confirm it still builds (type-check only, fast)**

Run: `cargo check -p warp`
Expected: compiles (no errors). `codex_plugin = []` pulls in no deps and only flips already-flagged Codex plugin-manager branches.

- [ ] **Step 4: Commit**

```bash
git add app/Cargo.toml
git commit -m "build: enable codex_plugin feature so Codex emits structured status events"
```

---

### Task 3: Add the `show_agent_status_on_tabs` setting (end-to-end)

Reuses the three existing notification toggles (`is_agent_task_completed_enabled` → done notify, `is_needs_attention_enabled` → asking push, `play_notification_sound` → sound). Adds the one new toggle that gates the tab badge, wired exactly like the neighbors.

**Files:**
- Modify: `app/src/terminal/session_settings.rs` (`NotificationsSettings` struct + `Default`)
- Modify: `app/src/settings_view/mod.rs` (flag constant, ~line 451)
- Modify: `app/src/workspace/view.rs` (flag sync, ~line 22404)
- Modify: `app/src/settings_view/features_page.rs` (action enum, toggle row, handler, telemetry)

**Interfaces:**
- Produces: `SessionSettings::as_ref(app).notifications.show_agent_status_on_tabs: bool` (default `true`), read by Task 4.
- Produces: `flags::AGENT_STATUS_ON_TABS_FLAG: &str` and `FeaturesPageAction::ToggleAgentStatusOnTabs`.

- [ ] **Step 1: Add the struct field**

In `app/src/terminal/session_settings.rs`, in `struct NotificationsSettings` (after `play_notification_sound`, ~line 95):

```rust
    #[schemars(description = "Whether to play a sound with notifications.")]
    pub play_notification_sound: bool,

    #[schemars(description = "Whether to show CLI-agent (Claude/Codex) status badges on tabs.")]
    pub show_agent_status_on_tabs: bool,
```

- [ ] **Step 2: Default it to `true`**

In the same file, in `impl Default for NotificationsSettings` (after `play_notification_sound: true,`, ~line 107):

```rust
            play_notification_sound: true,
            show_agent_status_on_tabs: true,
```

- [ ] **Step 3: Confirm it compiles + backward-compat is handled**

Run: `cargo check -p warp`
Expected: compiles.

No field-level serde attribute is needed: `NotificationsSettings` already carries a
**container-level** `#[serde(default)]` (see its doc-comment: "Added [serde(default)] to ensure
that new notification settings are backwards compatible with old clients"). Container-level
`#[serde(default)]` fills *each* missing field from the struct's `Default` impl, so an old
config that lacks `show_agent_status_on_tabs` deserializes to `true` (from Step 2). This is
exactly how the sibling bools (`is_needs_attention_enabled`, etc.) get their defaults — match
that pattern; do **not** add a per-field `#[serde(default = …)]`.

- [ ] **Step 4: Add the context-flag constant**

In `app/src/settings_view/mod.rs`, in the `flags` module (after `AGENT_IN_APP_NOTIFICATIONS_FLAG`, line ~451):

```rust
    pub const AGENT_IN_APP_NOTIFICATIONS_FLAG: &str = "Agent_In_App_Notifications";
    pub const AGENT_STATUS_ON_TABS_FLAG: &str = "Agent_Status_On_Tabs";
```

- [ ] **Step 5: Sync the flag from the setting**

In `app/src/workspace/view.rs`, after the `play_notification_sound` block (~line 22404):

```rust
        if session_settings.notifications.play_notification_sound {
            context.set.insert(flags::NOTIFICATION_SOUND_FLAG);
        }
        if session_settings.notifications.show_agent_status_on_tabs {
            context.set.insert(flags::AGENT_STATUS_ON_TABS_FLAG);
        }
```

- [ ] **Step 6: Add the action variant**

In `app/src/settings_view/features_page.rs`, in `pub enum FeaturesPageAction` (after `ToggleNotificationSound`, ~line 780):

```rust
    ToggleNotificationSound,
    ToggleAgentStatusOnTabs,
```

- [ ] **Step 7: Add the toggle row**

In `features_page.rs`, after the "notification sounds" `toggle_binding_pairs.push(...)` block (~line 384, before the "in-app agent notifications" row):

```rust
    toggle_binding_pairs.push(
        ToggleSettingActionPair::new(
            "show agent status on tabs",
            builder(SettingsAction::FeaturesPageToggle(
                FeaturesPageAction::ToggleAgentStatusOnTabs,
            )),
            &(context.to_owned() & id!(flags::NOTIFICATIONS_CONTEXT_FLAG)),
            flags::AGENT_STATUS_ON_TABS_FLAG,
        )
        .is_supported_on_current_platform(
            SessionSettings::as_ref(app)
                .notifications
                .is_supported_on_current_platform(),
        ),
    );
```

- [ ] **Step 8: Add the handler**

In `features_page.rs`, after the `ToggleNotificationSound => { … }` handler arm (~line 1783), add:

```rust
            ToggleAgentStatusOnTabs => {
                let current_settings = SessionSettings::as_ref(ctx).notifications.value().clone();
                let show_agent_status_on_tabs = !current_settings.show_agent_status_on_tabs;

                SessionSettings::handle(ctx).update(ctx, |settings, ctx| {
                    let new_settings = NotificationsSettings {
                        show_agent_status_on_tabs,
                        ..current_settings
                    };
                    if let Err(e) = settings.notifications.set_value(new_settings, ctx) {
                        log::error!("Error persisting notifications setting: {e}");
                    }
                });
                ctx.notify();
            }
```

- [ ] **Step 9: Add the telemetry arm**

In `features_page.rs`, in the telemetry `match` (where `ToggleAgentTaskCompletedNotifications => TelemetryEvent::FeaturesPageAction { … }` lives, ~line 1113), add a matching arm:

```rust
            Self::ToggleAgentStatusOnTabs => TelemetryEvent::FeaturesPageAction {
                action: "ToggleAgentStatusOnTabs".to_string(),
            },
```

(If `rustc` reports other non-exhaustive matches on `FeaturesPageAction`, add the analogous arm there — let the compiler enumerate them.)

- [ ] **Step 10: Build**

Run: `cargo check -p warp`
Expected: compiles with no non-exhaustive-match errors.

- [ ] **Step 11: Commit**

```bash
git add app/src/terminal/session_settings.rs app/src/settings_view/mod.rs \
        app/src/workspace/view.rs app/src/settings_view/features_page.rs
git commit -m "settings: add 'show agent status on tabs' toggle"
```

---

### Task 4: Horizontal tab strip CLI-agent indicator

The only net-new render surface. Reuses `terminal_view_agent_icon_variant` (the same helper that drives the sidebar + pane header) and `render_icon_with_status` so it looks identical. Follows the existing `agent_indicator`/`render_indicator` pattern (focused pane, matching the rest of the tab's focused-pane indicator logic).

**Files:**
- Modify: `app/src/tab.rs` (`enum Indicator`, `TabComponent::new` precedence, new `cli_agent_indicator`, `render_indicator`, tooltip helpers)

**Interfaces:**
- Consumes: `terminal_view_agent_icon_variant(&TerminalView, &AppContext) -> Option<IconWithStatusVariant>` (`ui_components/agent_icon.rs:35`, `pub(crate)`); `render_icon_with_status(variant, total_size, overhang, theme, status_container_background) -> Box<dyn Element>` (`ui_components/icon_with_status.rs:163`); `PaneGroup::focused_session_view(&AppContext) -> Option<ViewHandle<TerminalView>>` (`pane_group/mod.rs:6784`); `SessionSettings::as_ref(app).notifications.show_agent_status_on_tabs: bool` (Task 3).
- Produces: a new `Indicator::CLIAgent(IconWithStatusVariant)` variant rendered on the horizontal tab.

- [ ] **Step 1: Add imports at the top of `tab.rs`**

Add (next to existing `use` lines):

```rust
use crate::ui_components::agent_icon::terminal_view_agent_icon_variant;
use crate::ui_components::icon_with_status::{render_icon_with_status, IconWithStatusVariant};
use crate::terminal::session_settings::SessionSettings;
```

(Some of these may already be imported — dedupe as the compiler directs.)

- [ ] **Step 2: Add the `Indicator` variant**

In `enum Indicator` (`tab.rs:749`), add:

```rust
    AmbientAgent,
    /// A Claude/Codex CLI agent is running in this tab's focused pane.
    CLIAgent(IconWithStatusVariant),
```

`Indicator` derives `Clone`; `IconWithStatusVariant` must also be `Clone`. Verify with `cargo check`; if it is not, add `#[derive(Clone)]` to `IconWithStatusVariant` in `icon_with_status.rs` (it is rendered by value and is a small data enum, so deriving `Clone` is safe).

- [ ] **Step 3: Add the `cli_agent_indicator` helper**

In `impl<'a> TabComponent<'a>`, next to `agent_indicator` (`tab.rs:1019`), add:

```rust
    /// CLI-agent (Claude/Codex) status indicator for the tab's focused pane, or `None` when
    /// the focused pane isn't a recognized CLI agent or the setting is off. Mirrors how the
    /// sidebar/pane-header derive their badge so all three surfaces stay in lockstep.
    fn cli_agent_indicator(tab: &TabData, app: &AppContext) -> Option<Indicator> {
        if !SessionSettings::as_ref(app)
            .notifications
            .show_agent_status_on_tabs
        {
            return None;
        }
        let view = tab.pane_group.as_ref(app).focused_session_view(app)?;
        let variant = terminal_view_agent_icon_variant(view.as_ref(app), app)?;
        matches!(variant, IconWithStatusVariant::CLIAgent { .. })
            .then_some(Indicator::CLIAgent(variant))
    }
```

- [ ] **Step 4: Wire it into the precedence chain**

In `TabComponent::new` (`tab.rs:939`), insert the CLI-agent check between the Oz `agent_indicator` branch and the `shell_indicator_type` branch:

```rust
        } else if let Some(agent) = Self::agent_indicator(tab, ctx) {
            agent
        } else if let Some(cli_agent) = Self::cli_agent_indicator(tab, ctx) {
            cli_agent
        } else if let Some(shell_indicator_type) = shell_indicator_type {
            Indicator::Shell(shell_indicator_type)
```

- [ ] **Step 5: Render the variant**

In `render_indicator` (`tab.rs:1349`), add an arm to the `match &self.indicator` (after the `Indicator::AmbientAgent` arm, before the closing `}` at ~line 1449):

```rust
            Indicator::CLIAgent(variant) => Some(render_icon_with_status(
                variant.clone(),
                TAB_INDICATOR_HEIGHT,
                0.0,
                self.appearance.theme(),
                self.appearance.theme().background().into(),
            )),
```

If `theme()` / `background().into()` types don't line up with `render_icon_with_status`'s `&WarpTheme` / `WarpThemeFill` parameters, copy the exact argument expressions from the existing sidebar call site (`workspace/view/vertical_tabs.rs`, `render_pane_icon_with_status` → `render_icon_with_status(...)`), which renders the identical variant.

- [ ] **Step 6: Satisfy the other exhaustive matches on `Indicator`**

Run: `cargo check -p warp` and add a `CLIAgent` arm wherever rustc reports a non-exhaustive match (the tooltip helpers `get_tooltip_message` / `get_tooltip_directory` / `get_tooltip_git_branch`, and `is_title_from_agent` if it matches). Concrete arms:

In `get_tooltip_message` — return the agent name + status:

```rust
            Indicator::CLIAgent(IconWithStatusVariant::CLIAgent { agent, status, .. }) => {
                let suffix = match status {
                    Some(ConversationStatus::Blocked { .. }) => " — needs your attention",
                    Some(ConversationStatus::Success) => " — done",
                    Some(ConversationStatus::InProgress) => " — working",
                    _ => "",
                };
                Some(format!("{}{}", agent.display_name(), suffix))
            }
            Indicator::CLIAgent(_) => None,
```

In `get_tooltip_directory`, `get_tooltip_git_branch`, and `is_title_from_agent` (and any other match rustc flags), add:

```rust
            Indicator::CLIAgent(_) => None,   // or `true` for is_title_from_agent (title comes from the agent)
```

(For `is_title_from_agent`, return `true` so the tab title is treated as agent-derived, consistent with `Indicator::Agent`.)

- [ ] **Step 7: Build**

Run: `cargo check -p warp`
Expected: compiles, all matches exhaustive.

- [ ] **Step 8: Commit**

```bash
git add app/src/tab.rs app/src/ui_components/icon_with_status.rs
git commit -m "tab: show Claude/Codex status badge on the horizontal tab strip"
```

---

### Task 5: Pane-level notification focus gate

Today the CLI-agent desktop notification only fires when the whole Clinch **window** is unfocused (`is_navigated_away_from_window`). Narrow it so an agent asking on a background **pane/tab** notifies even while Clinch is focused — but the pane you're actively viewing stays quiet. Combine the existing window check with a pane check so the "Clinch fully backgrounded" case is preserved.

**Files:**
- Modify: `app/src/ai/agent_management/agent_management_model.rs` (`active_focused_terminal_id` → `pub(crate)`)
- Modify: `app/src/terminal/view.rs` (new `is_pane_actively_focused` helper + swap the guard at ~13325)

**Interfaces:**
- Consumes: `active_focused_terminal_id(&AppContext) -> Option<EntityId>` (currently private at `agent_management_model.rs:590`); `TerminalView::is_navigated_away_from_window(&self, &AppContext) -> bool` (`view.rs:21055`); `self.view_id: EntityId`.
- Produces: `TerminalView::is_pane_actively_focused(&self, &AppContext) -> bool`.

- [ ] **Step 1: Make `active_focused_terminal_id` reachable**

In `app/src/ai/agent_management/agent_management_model.rs:590`, change the helper's visibility:

```rust
pub(crate) fn active_focused_terminal_id(app: &AppContext) -> Option<EntityId> {
```

(Body unchanged: active window → its `Workspace` → `Workspace::active_terminal_id(app)`.)

- [ ] **Step 2: Add the pane-focus helper on `TerminalView`**

In `app/src/terminal/view.rs`, near `is_navigated_away_from_window` (~line 21055), add:

```rust
    /// True only when THIS pane is the focused pane of the OS-active Clinch window — i.e. the
    /// user is actively looking at this exact agent. Used to suppress a notification for the
    /// pane in view while still notifying for agents on other tabs/panes (and when Clinch is
    /// backgrounded, where `is_navigated_away_from_window` is true so this returns false).
    fn is_pane_actively_focused(&self, app: &AppContext) -> bool {
        !self.is_navigated_away_from_window(app)
            && crate::ai::agent_management::agent_management_model::active_focused_terminal_id(app)
                == Some(self.view_id)
    }
```

- [ ] **Step 3: Swap the guard**

In `view.rs`, in `handle_cli_agent_sessions_event` (the desktop-notification guard at ~line 13325), replace:

```rust
        // Desktop notifications — only when navigated away and not in-progress.
        if !self.is_navigated_away_from_window(ctx)
            || matches!(status, CLIAgentSessionStatus::InProgress)
        {
            return;
        }
```

with:

```rust
        // Desktop notifications — fire unless the user is actively looking at this exact pane,
        // and never while the agent is mid-turn.
        if self.is_pane_actively_focused(ctx)
            || matches!(status, CLIAgentSessionStatus::InProgress)
        {
            return;
        }
```

This keeps `is_navigated_away_from_window` in use (no dead code) for the non-CLI paths (long-running command, Oz agent-mode, password) which still call it directly.

- [ ] **Step 4: Build**

Run: `cargo check -p warp`
Expected: compiles. (`EntityId` is already in scope in both files; `active_focused_terminal_id`'s return type matches `self.view_id`.)

- [ ] **Step 5: Commit**

```bash
git add app/src/ai/agent_management/agent_management_model.rs app/src/terminal/view.rs
git commit -m "notifications: notify for agents on background panes, not just background windows"
```

---

### Task 6: Build, install, and verify end-to-end

The pipeline is interaction-heavy and ctx-bound (no headless UI harness), so the integration check is a scripted manual walkthrough. This task gates the whole feature.

**Files:** none (verification only).

- [ ] **Step 1: Run all shell tests**

Run: `for t in tools/agent-resume/tests/*.sh; do echo "== $t"; bash "$t" || break; done`
Expected: every test prints `PASS` (or its success line). Fix any failure before continuing.

- [ ] **Step 2: Build + install the app**

Run: `./tools/agent-resume/build-app.sh`
Expected: builds the OSS-channel "Clinch" app and installs it to `/Applications`. (This is the only build that compiles the `codex_plugin` change in.)

- [ ] **Step 3: Install the emitter plugins + reload shell**

Run: `./tools/agent-resume/install.sh`
Expected: output includes "Installing Claude notification plugin..." and "Installing Codex notification plugin..." (or a clear skip/warn if a CLI is missing/offline). Then restart your shell.

- [ ] **Step 4: Verify Claude — done (✓)**

In Clinch, open a tab, run `claude`, restart it once (so the plugin loads), ask it a trivial question, and switch to a *different* tab while it answers.
Expected: the Claude tab shows the brand circle with a spinner while working, then a ✓ when done, on **all three** surfaces (horizontal tab strip, sidebar, pane header). Because you're on another tab, you also get a macOS "done" notification (if "agent task completion notifications" is on).

- [ ] **Step 5: Verify Claude — asking (❗ + push)**

Trigger a permission prompt (e.g. ask Claude to run a command that needs approval) while focused on another tab.
Expected: the tab shows the Blocked/❗ badge AND a macOS push fires (because "needs-attention notifications" is on and you're not looking at that pane). Switch back to the asking pane while it's still blocked and trigger another: with that pane focused, the badge shows but **no** push.

- [ ] **Step 6: Verify Codex — asking (❗ + push)**

Run `codex` in a tab, restart it once, and trigger an approval request from a different tab.
Expected: ❗ badge + macOS push. (This is the case that requires Task 2's `codex_plugin`; if it shows only a coarse "done" and never "asking," confirm `codex_plugin` is in `default` and the codex plugin is installed/restarted.)

- [ ] **Step 7: Verify the settings**

Open Settings → the notifications section. Toggle **"show agent status on tabs"** off → badges disappear from all surfaces; on → they return. Toggle **"needs-attention notifications"** off → asking no longer pushes (badge still shows). Toggle **"agent task completion notifications"** and **"notification sounds"** and confirm effect.

- [ ] **Step 8: Final commit (if any verification fixes were needed)**

```bash
git add -A
git commit -m "agent attention badges: verification fixes"
```

(Skip if no fixes were required.)

---

## Self-Review

**Spec coverage:**
- Goal: badge on horizontal tab + sidebar + pane header → Task 4 (horizontal, net-new) + Tasks 1/2 (light up the existing sidebar/pane-header path). ✓
- Goal: push for both Claude & Codex when asking → Task 1 (plugins) + Task 2 (Codex rich events) + Task 5 (pane-level gate); both agents flow through `handle_cli_agent_sessions_event`. ✓
- Goal: push only when not viewing that pane → Task 5. ✓
- Goal: granular settings (3 reused + 1 new) → Task 3. ✓
- Non-goal guards (no AI-gate change, no own emitter, keep OSC-9, no new deps) → encoded in Global Constraints; no task violates them. ✓
- Spec's "Component 2: enable codex_plugin" → Task 2. ✓

**Placeholder scan:** No "TBD/TODO/handle edge cases." The two compiler-guided steps (Task 3 Step 9, Task 4 Step 6) give concrete arms and use rustc exhaustiveness as the enumerator, which is a real mechanism, not a placeholder.

**Type consistency:** `show_agent_status_on_tabs` (bool) defined in Task 3, read in Task 4 Step 3 — names match. `Indicator::CLIAgent(IconWithStatusVariant)` defined Task 4 Step 2, rendered Step 5, matched Step 6 — consistent. `active_focused_terminal_id` made `pub(crate)` (Task 5 Step 1) and called with full path (Step 2) — consistent. `is_pane_actively_focused` defined Step 2, used Step 3 — consistent.

**Deviation from spec, noted:** the spec's Component 4 suggested `active_focused_terminal_id(ctx) != Some(self.view_id)` alone; Task 5 combines it with `is_navigated_away_from_window` to avoid regressing the "Clinch fully backgrounded" case (the active-window lookup returns the last-focused pane even when Clinch is in the background). This is strictly more correct and keeps the existing helper in use (no dead code). The spec's aggregation note (Blocked > InProgress > Success across split panes) is simplified to the **focused pane** in Task 4, matching the existing `agent_indicator` pattern; cross-split aggregation is a deferred refinement.
