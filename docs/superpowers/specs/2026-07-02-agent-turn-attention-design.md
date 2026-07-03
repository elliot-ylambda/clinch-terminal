# Agent turn-end "awaiting you" attention — design

- **Date:** 2026-07-02
- **Status:** Draft (pending user review)
- **Scope:** Clinch (local-only Warp fork), macOS. Claude Code + Codex CLI agents.
- **Related:** PR #7 (agent attention badges), `docs/superpowers/specs/2026-06-30-agent-attention-badges-design.md`, memory `agent-attention-badge-gap`.

## Problem

When Claude/Codex finishes a turn and hands control back — including when it ends by asking the
user a question — the user gets **no attention signal on the tab**. Verified root cause:

1. The OSC-777 → app → tab-badge pipeline is fully live (proven end-to-end on 2026-07-02 by
   emitting a real `permission_request` event to the pane pty and observing the ❗ badge render).
2. The user runs `claude --dangerously-skip-permissions`. In bypass mode Claude **never fires a
   `PermissionRequest` hook**, and `permission_request` is the only event wired to the ❗ `Blocked`
   ("needs you") state. So that trigger never fires.
3. Turn-end fires `Stop` → the app maps it to `CLIAgentSessionStatus::Success` → the tab shows a
   quiet ✓ "done" badge. A ✓ reads as *"nothing needed"* — the opposite of *"it's your turn."*
4. Claude's idle `Notification` (`idle_prompt`) maps to a no-op; `question_asked` is never emitted
   by the plugin.

Net: in normal bypass-mode use the user sees a spinner while working and a ✓ when done, but never a
signal that Claude is **waiting on them**.

## Goal

Turn "an agent turn ended on a surface the user is not currently viewing" into a visible
**"awaiting you"** attention treatment on all three existing surfaces (horizontal tab strip,
sidebar/vertical tabs, pane header), cleared automatically when the user views that pane/tab.

Chosen semantics (Option A): **every turn-end on a not-currently-viewed surface = "awaiting you."**
Rationale: `Stop` is the only signal that reliably fires in bypass mode, and from the user's point
of view any ended turn means the ball is in their court. Distinguishing "done" vs "asked a
question" is out of scope because Claude Code does not expose that distinction in bypass mode.

## Non-goals (scope guard)

- **No plugin changes.** We do not fork/patch the upstream `claude-code-warp` / `codex-warp`
  marketplace plugins. This is purely app-side and consumes events that already arrive.
- **No idle-`Notification` / `question_asked` wiring.** `Stop` already covers turn-end; the idle
  path would be redundant and slow (~60s), so it stays a no-op (documented, not changed).
- **No new `CLIAgentSessionStatus` variant.** The status model stays honest (`Success` = turn
  ended). "Awaiting you" is a **render-layer derivation**, not a new status — this avoids
  corrupting the in-app mailbox / tooltip semantics and prevents dead branches.
- **No new feature flag.** The feature reuses the already-shipped, default-on
  `show_agent_status_on_tabs` setting as its gate.

## Environment note

The user runs with `[appearance.vertical_tabs] enabled = true` (left sidebar tabs). The sidebar
already renders a generic per-terminal-view unread dot (`has_unread_activity_for_terminal_view`,
vertical_tabs.rs:3180), and `AgentNotificationsModel::handle_cli_agent_session_event`
(agent_management_model.rs:159-182) already adds a `NotificationCategory::Complete` item on
turn-end (`Success`) — which fires in bypass mode. So the user is not getting *no* signal; the
problem is that the signals they get are (a) the status badge reads ✓ "done", not "your turn", and
(b) the generic unread dot doesn't convey "the agent is waiting on you." The horizontal tab strip
renders neither the unread dot (confirmed absent in tab.rs) — relevant only if the user switches
layouts.

## Design

### Component 1 — "awaiting you" render treatment (primary)

Introduce a single render-time predicate at the one place all three surfaces derive from —
`terminal_view_agent_icon_variant` (ui_components/agent_icon.rs:36):

```
awaiting_user = (resolved CLI status == Success) && active_focused_terminal_id(app) != Some(terminal_view.id())
```

Focus is read with `active_focused_terminal_id(app)` (agent_management_model.rs:590), which takes
only `&AppContext` — this is the key enabler. (Note: `TerminalView::is_pane_actively_focused`
needs a `&mut ViewContext` and is therefore NOT usable here; do not attempt it.) No persistent
"seen/unread" state is needed: when the user focuses the terminal, `active_focused_terminal_id`
matches its id and the treatment clears automatically on the next render.

Because all three surfaces (horizontal tab strip via `Tab::cli_agent_indicator` tab.rs:1070,
sidebar + pane header via `terminal_view_agent_icon_variant_respecting_tab_setting`
agent_icon.rs:96) funnel through `terminal_view_agent_icon_variant`, computing `awaiting_user`
there covers all of them with one change.

Rendering: carry `awaiting_user` on the render variant and, when true, draw the agent glyph with
the existing **yellow attention overlay** (`yellow_stop_icon`, the same glyph `Blocked` uses) in
place of the ✓ `succeeded_icon`. When the user is viewing the terminal, `Success` renders the ✓ as
it does today. The tab tooltip in the awaiting state reads "— awaiting your input".

Implementation shape (minimal blast radius — one function, render-only):
- `IconWithStatusVariant::CLIAgent` is constructed at 10+ sites; adding a field would ripple to
  all of them and to the mirrored `Indicator::CLIAgent` fields. Avoid that.
- Instead, transform the *render status* at the single live-CLI derivation point. In
  `terminal_view_agent_icon_variant`, wrap the result:
  ```rust
  let variant = agent_icon_variant_from_terminal_inputs(&inputs)?;
  Some(apply_awaiting_user_treatment(variant, terminal_view.id(), app))
  ```
  where the pure helper substitutes the render status when awaiting:
  ```rust
  fn apply_awaiting_user_treatment(
      variant: IconWithStatusVariant,
      terminal_view_id: EntityId,
      focused_terminal_id: Option<EntityId>,
  ) -> IconWithStatusVariant { /* if CLIAgent{status: Some(Success)} && Some(id)!=focused
       => set status to ConversationStatus::Blocked{ blocked_action: String::new() } */ }
  ```
- This reuses the existing `Blocked → yellow_stop_icon` rendering and the existing
  `Blocked → "needs your attention"` tab tooltip. No new field, no new status variant, no changes
  to the session model, the desktop-notification path, or the mailbox — all of which read the
  session's `CLIAgentSessionStatus` directly, not this presentational variant.
- **Scope guard:** apply only on the live-CLI-session return, not the task-backed/ambient early
  return (cloud/orchestration runs are out of scope).
- **Tradeoff (documented):** this overloads `ConversationStatus::Blocked` for presentation of
  "awaiting you." It is contained to one function and commented. If a future reader needs a
  semantically distinct state, promote it to a dedicated render field then.

### Component 2 — real desktop notification on turn-end (optional, user-gated)

Today `handle_cli_agent_sessions_event` (view.rs:13229) already fires an `AgentTaskCompleted`
notification on turn-end when the pane isn't actively focused — but
`send_agent_desktop_notification_or_show_banner` only emits a real OS notification when
`NotificationsMode == Enabled`. The default is `Unset`, which shows a one-time in-app discovery
banner instead. The per-trigger toggles (`is_agent_task_completed_enabled`,
`is_needs_attention_enabled`) are already default-on.

So a real macOS notification already works **once the user enables notifications** (mode →
`Enabled`), via the existing discovery banner or Settings. Decision for this spec:

- **Default:** leave the global `NotificationsMode` default (`Unset`) untouched — do not silently
  turn on all notification classes (password-prompt, long-running, etc.).
- **Provide the on-ramp:** ensure the existing discovery banner surfaces on the first agent
  turn-end so the user can enable with one click; document that enabling Notifications makes
  turn-end fire a real OS alert (works even when Clinch is fully backgrounded).

(If the user prefers, we can instead default `NotificationsMode` to `Enabled` for Clinch — flagged
as a sub-decision in the review, not baked in.)

## Testing

- Unit: table test for the pure `apply_awaiting_user_treatment(variant, terminal_view_id,
  focused_terminal_id)` in `agent_icon_tests.rs`:
  - `CLIAgent{status: Some(Success)}` + focused_id ≠ id → status becomes `Blocked`.
  - `CLIAgent{status: Some(Success)}` + focused_id == id → unchanged (`Success`).
  - `CLIAgent{status: Some(InProgress)}` → unchanged; `OzAgent{..}` → unchanged.
- Manual: reuse the proven pty-injection harness (emit `stop` to `/dev/ttys00N`) to drive a
  turn-end on a background vertical tab and confirm the yellow attention badge appears in place of
  ✓ and reverts to ✓ when that tab is focused.

## Code cleanup / dead-code check

- No code is removed. The ✓-on-focused path stays valid (you're looking, no attention needed).
- If review decides "awaiting you" should fully replace the completion badge even when focused,
  the `Success` checkmark branch would become redundant and should be deleted rather than left
  dark — noted so it isn't orphaned.

## Open sub-decisions for review

1. Real macOS notification: keep default `Unset` + discovery-banner on-ramp (recommended), or
   default `NotificationsMode` to `Enabled` for Clinch?
2. Awaiting-state glyph: reuse the existing yellow attention overlay (recommended), or a subtler
   accent dot distinct from the `Blocked` error look?
