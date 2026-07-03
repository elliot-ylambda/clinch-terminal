# Agent turn-end "awaiting you" attention — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When a Claude/Codex turn ends on a tab the user is not currently viewing, show a
"awaiting your input" attention badge instead of the quiet ✓ "done" badge, so the user sees that
the agent is waiting on them — including in `--dangerously-skip-permissions` mode.

**Architecture:** Render-only transform at the single point all three agent-icon surfaces derive
from (`terminal_view_agent_icon_variant`). When a live CLI session's resolved status is `Success`
and the terminal is not the focused one (`active_focused_terminal_id`), present the render status
as `Blocked`, which already renders as the yellow attention glyph. No plugin, event-pipeline,
session-model, notification, or feature-flag changes.

**Tech Stack:** Rust, WarpUI (Entity/Handle), existing `IconWithStatusVariant` /
`ConversationStatus` rendering.

## Global Constraints

- Platform: macOS; Clinch fork (local-only). Agents in scope: Claude + Codex (via existing
  `IconWithStatusVariant::CLIAgent`).
- No changes to the marketplace plugins, the OSC-777 event pipeline, `CLIAgentSessionStatus`, the
  desktop-notification path, or the in-app mailbox — all read session status directly, not this
  presentational variant.
- No new feature flag. Feature stays gated by the already-default-on `show_agent_status_on_tabs`
  setting (via `terminal_view_agent_icon_variant_respecting_tab_setting`) and the already-on
  `FeatureFlag::HOANotifications`.
- Follow repo style (WARP.md): context param named `ctx` and last; no unnecessary type
  annotations; inline format args; exhaustive matching (no `_` wildcard where variants can be
  enumerated); unit tests in the existing `agent_icon_tests.rs`.
- Scope guard: apply the treatment only to the live-CLI-session return path, NOT the
  task-backed/ambient (cloud/orchestration) early return in `terminal_view_agent_icon_variant`.

---

### Task 1: "Awaiting you" render treatment for unfocused completed CLI-agent terminals

**Files:**
- Modify: `app/src/ui_components/agent_icon.rs` (imports; add `apply_awaiting_user_treatment`; wire it into `terminal_view_agent_icon_variant` at the final return, currently line 90)
- Test: `app/src/ui_components/agent_icon_tests.rs`

**Interfaces:**
- Consumes: `active_focused_terminal_id(app: &AppContext) -> Option<EntityId>` (from
  `crate::ai::agent_management::agent_management_model`); `IconWithStatusVariant::CLIAgent { agent,
  status: Option<ConversationStatus>, is_ambient }`; `ConversationStatus::{Success, Blocked{ blocked_action: String }}`;
  `terminal_view.id() -> EntityId`.
- Produces: `fn apply_awaiting_user_treatment(variant: IconWithStatusVariant, terminal_view_id:
  EntityId, focused_terminal_id: Option<EntityId>) -> IconWithStatusVariant` (module-private, pure,
  unit-tested).

- [ ] **Step 1: Write the failing test**

Add to `app/src/ui_components/agent_icon_tests.rs`:

```rust
#[test]
fn awaiting_user_treatment_marks_unfocused_completed_cli_agent() {
    use warpui::EntityId;

    use crate::ai::agent::conversation::ConversationStatus;
    use crate::terminal::CLIAgent;
    use crate::ui_components::icon_with_status::IconWithStatusVariant;

    let id = EntityId::new();
    let focused_elsewhere = EntityId::new();

    let completed = IconWithStatusVariant::CLIAgent {
        agent: CLIAgent::Claude,
        status: Some(ConversationStatus::Success),
        is_ambient: false,
    };

    // Not the focused terminal -> Success is presented as Blocked (yellow attention glyph).
    let out = super::apply_awaiting_user_treatment(completed.clone(), id, Some(focused_elsewhere));
    assert!(matches!(
        out,
        IconWithStatusVariant::CLIAgent {
            status: Some(ConversationStatus::Blocked { .. }),
            ..
        }
    ));

    // The focused terminal -> unchanged (stays Success / ✓).
    let out = super::apply_awaiting_user_treatment(completed, id, Some(id));
    assert!(matches!(
        out,
        IconWithStatusVariant::CLIAgent {
            status: Some(ConversationStatus::Success),
            ..
        }
    ));

    // In-progress is never overridden.
    let running = IconWithStatusVariant::CLIAgent {
        agent: CLIAgent::Codex,
        status: Some(ConversationStatus::InProgress),
        is_ambient: false,
    };
    let out = super::apply_awaiting_user_treatment(running, id, Some(focused_elsewhere));
    assert!(matches!(
        out,
        IconWithStatusVariant::CLIAgent {
            status: Some(ConversationStatus::InProgress),
            ..
        }
    ));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p warp awaiting_user_treatment_marks_unfocused_completed_cli_agent`
Expected: FAIL to compile — `apply_awaiting_user_treatment` not found in module `super`.

- [ ] **Step 3: Add imports to `agent_icon.rs`**

Change the existing import line:

```rust
use warpui::{AppContext, SingletonEntity};
```

to:

```rust
use warpui::{AppContext, EntityId, SingletonEntity};
```

and add, alongside the other `use crate::...` lines near the top:

```rust
use crate::ai::agent_management::agent_management_model::active_focused_terminal_id;
```

- [ ] **Step 4: Add the pure helper**

Add this function to `app/src/ui_components/agent_icon.rs` (place it directly above
`agent_icon_variant_from_terminal_inputs`):

```rust
/// Presentational treatment for "the agent's turn ended and it's now waiting on you."
///
/// When a live CLI-agent session has finished a turn (`Success`) on a terminal the user is not
/// currently viewing, we render the yellow attention glyph (via `ConversationStatus::Blocked`,
/// which already maps to `yellow_stop_icon`) instead of the ✓ "done" glyph, so a background
/// tab visibly signals "your turn". This is presentation only — the session's real
/// `CLIAgentSessionStatus`, the desktop-notification path, and the mailbox are untouched.
///
/// Pure and focus-parameterized so it is unit-testable without an `AppContext`.
fn apply_awaiting_user_treatment(
    variant: IconWithStatusVariant,
    terminal_view_id: EntityId,
    focused_terminal_id: Option<EntityId>,
) -> IconWithStatusVariant {
    match variant {
        IconWithStatusVariant::CLIAgent {
            agent,
            status: Some(ConversationStatus::Success),
            is_ambient,
        } if focused_terminal_id != Some(terminal_view_id) => IconWithStatusVariant::CLIAgent {
            agent,
            status: Some(ConversationStatus::Blocked {
                blocked_action: String::new(),
            }),
            is_ambient,
        },
        other => other,
    }
}
```

- [ ] **Step 5: Wire the helper into the live-CLI return path**

In `terminal_view_agent_icon_variant`, replace the final expression (currently
`agent_icon_variant_from_terminal_inputs(&inputs)` at the end of the function, ~line 90):

```rust
    agent_icon_variant_from_terminal_inputs(&inputs)
```

with:

```rust
    let variant = agent_icon_variant_from_terminal_inputs(&inputs)?;
    Some(apply_awaiting_user_treatment(
        variant,
        terminal_view.id(),
        active_focused_terminal_id(app),
    ))
```

Leave the task-backed early return (the `return Some(agent_icon_variant_for_run(...))` around
line 68) untouched — cloud/ambient runs are out of scope.

- [ ] **Step 6: Run the test to verify it passes**

Run: `cargo nextest run -p warp awaiting_user_treatment_marks_unfocused_completed_cli_agent`
Expected: PASS.

- [ ] **Step 7: Verify no regression in the icon-variant suite**

Run: `cargo nextest run -p warp agent_icon`
Expected: PASS (existing cross-surface consistency tests unaffected — they exercise the pure
`agent_icon_variant_from_terminal_inputs`, which is unchanged).

- [ ] **Step 8: Format + clippy**

Run: `./script/format` then
`cargo clippy -p warp --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 9: Manual end-to-end check (pty injection)**

With Clinch running and a Claude/Codex tab NOT focused, emit a turn-end to that pane's pty
(resolve the pane tty as in the diagnostic, then):

```bash
BODY=$(jq -nc --argjson v 1 --arg agent claude --arg event stop \
  --arg session_id manual --arg cwd "$PWD" --arg project clinch-terminal \
  '{v:$v,agent:$agent,event:$event,session_id:$session_id,cwd:$cwd,project:$project}')
printf '\033]777;notify;warp://cli-agent;%s\007' "$BODY" > /dev/ttysNNN
```

Expected: the unfocused tab shows the yellow attention badge (not ✓); focusing that tab reverts it
to ✓.

- [ ] **Step 10: Commit**

```bash
git add app/src/ui_components/agent_icon.rs app/src/ui_components/agent_icon_tests.rs
git commit -m "feat(agent): show 'awaiting you' badge when an agent turn ends on an unfocused tab"
```

---

## Component 2 (out of code scope): real macOS notification

No code task. The turn-end desktop notification already exists
(`handle_cli_agent_sessions_event` → `AgentTaskCompleted`) but is gated by
`NotificationsMode`; the default is `Unset`, which shows a one-time in-app "enable notifications"
banner. Enabling Notifications (Settings, or the banner's action) makes turn-end fire a real OS
alert. Documented here so it isn't mistaken for a missing feature. If the user later wants it on by
default for Clinch, that is a separate one-line default change in
`app/src/terminal/session_settings.rs` and should be its own plan.

---

## Self-Review

**Spec coverage:**
- Component 1 ("awaiting you" render treatment) → Task 1. ✓
- Component 2 (OS notification) → documented as config-only, no code, per spec. ✓
- Non-goals (no plugin/event/status/flag changes) → honored by the render-only transform. ✓

**Placeholder scan:** No TBD/TODO; every code step shows complete code. ✓

**Type consistency:** `apply_awaiting_user_treatment(IconWithStatusVariant, EntityId,
Option<EntityId>) -> IconWithStatusVariant` is used identically in the test (Step 1) and the wiring
(Step 5). `active_focused_terminal_id(&AppContext) -> Option<EntityId>` matches source
(agent_management_model.rs:590). `ConversationStatus::Blocked { blocked_action: String }` and
`CLIAgent::{Claude,Codex}` match source. ✓

**Principal-engineer critique (applied):**
- Rejected the `awaiting_user: bool` field approach — it would ripple to 10+
  `IconWithStatusVariant::CLIAgent` construction sites and the mirrored `Indicator::CLIAgent`
  fields for no functional gain over the one-function status transform.
- Rejected mutating `CLIAgentSessionStatus` / `apply_event` — would corrupt the honest session
  model and leak "awaiting" semantics into the notification/mailbox paths.
- Dead code: none introduced; the ✓ `Success` render path stays live for the focused terminal.
  Documented that if a future reader needs a semantically distinct "awaiting" state, promote it to
  a dedicated render field rather than overloading `Blocked`.
