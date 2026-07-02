# "Continue" & "LGTM" quick-reply buttons for CLI agents — design

- **Date:** 2026-07-02
- **Status:** Design — approved, implementing
- **Scope:** Clinch (the local-only, skip_login OSS build of this Warp fork), macOS. Agents: Claude Code + Codex.
- **Related:**
  - `docs/superpowers/specs/2026-06-30-agent-fork-compact-buttons-design.md` — the Fork/Compact footer buttons this sits next to and mirrors.
  - `app/src/terminal/view/use_agent_footer/mod.rs` — hosts `submit_text_to_cli_agent_pty`, the per-agent submit primitive this feature reuses.

## Summary

Add two one-click "quick reply" buttons to the per-pane CLI-agent footer, immediately right of the
existing **Fork** and **Compact** buttons:

- **Continue** — types `Continue` into the live agent in this pane and submits it (presses Enter).
- **LGTM** — types `Looks good to me, continue` into the live agent and submits it.

Both are pure convenience shortcuts for the two most common "keep going" replies a user types to a
coding agent. They target the agent (Claude Code or Codex) running in the footer's own pane.

The decisive finding: **the whole mechanism already exists.** The footer already renders configurable
buttons via the `AgentToolbarItemKind` enum, and `UseAgentToolbar::submit_text_to_cli_agent_pty` already
"given a String, resolve the pane's agent and submit it with the correct per-agent Enter strategy." This
feature is **two enum variants + two buttons + one event routed to an existing function** — not
build-from-scratch.

## Goals

- A pane running Claude or Codex shows **Continue** and **LGTM** buttons in its CLI-agent footer,
  immediately after Fork/Compact in the default left group.
- **Continue** submits the literal text `Continue` + Enter to the live agent in **this** pane.
- **LGTM** submits the literal text `Looks good to me, continue` + Enter to the live agent in **this** pane.
- Submission works reliably on **both** Claude and Codex (correct per-agent Enter handling).
- Buttons appear **only** when the pane has a detected CLI agent (same visibility rule as Fork/Compact);
  hidden from shared-session viewers.
- Buttons are user-configurable in the footer editor (can be reordered or removed), like Fork/Compact.

## Non-goals (scope guard)

- **Not** a general canned-prompt manager or user-editable snippet list. Two fixed phrases only. A third
  phrase later is a small additive change (one enum variant + one button), not a new subsystem.
- **Not** routing into the rich-input composer. The buttons always submit **directly** to the agent PTY
  so one click == send. (See Edge cases for the composer-open case.)
- **Not** a confirmation dialog. Clicking sends immediately, like Compact.
- **Not** behind a feature flag. Matches the adjacent unflagged Fork/Compact buttons; the toolbar editor
  already lets users remove them, so a flag would add cleanup debt for no rollout benefit. (Revisit only
  if these need staged rollout.)
- **Not** supporting agents beyond Claude + Codex specially — the submit primitive already picks a
  strategy for every `CLIAgent` variant, so this works for any detected CLI agent with no extra code.

## Background: the live machinery (with anchors)

```
detect agent ──────────────► CLIAgentSessionsModel::session(view_id) -> Option<CLIAgentSession>
                              (.agent : CLIAgent = Claude | Codex | …)

configurable footer button ─► AgentToolbarItemKind enum
                              (app/src/ai/blocklist/agent_view/agent_input_footer/toolbar_item.rs)
                              each variant declares label / icon / availability / default rows

button -> action -> event ──► ActionButton.on_click -> dispatch AgentInputFooterAction::X
                              handler emits AgentInputFooterEvent::Y
                              (app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs)

footer event -> toolbar ────► UseAgentToolbar::handle_agent_input_footer_event
                              re-emits as UseAgentToolbarEvent (use_agent_footer/mod.rs:1161)

submit text to agent ──────► UseAgentToolbar::submit_text_to_cli_agent_pty(text, ctx)
                              (use_agent_footer/mod.rs:715) — resolves the pane's CLIAgent and calls
                              write_cli_agent_text_then_submit(bytes, strategy) with the per-agent
                              Enter strategy (Codex: bracketed-paste + separate \r; Claude:
                              text then \r after a 50 ms delay).
```

### The key correctness decision: reuse the per-agent submit, not `WriteToPty`

The Compact button emits `AgentInputFooterEvent::WriteToPty("/compact\n")`, which writes bytes verbatim
to the PTY (no per-agent handling). That is adequate for a short slash-command, but a plain `"text\r"`
blob is **not** reliable for arbitrary prompt text:

- **Claude Code** needs the text to land in its TUI input box *before* the Enter — hence the existing
  `DelayedEnter` strategy (write text, then `\r` after 50 ms). A contiguous `"Continue\r"` can submit
  before the word registers.
- **Codex** expects pasted input wrapped in **bracketed-paste** markers with the `\r` as a separate
  write (`BracketedPaste` strategy).

`submit_text_to_cli_agent_pty` encapsulates exactly this per-agent logic and is documented as the entry
point "for callers that produce prompts outside the rich input editor." Routing the buttons through it —
rather than copying Compact's naive path — is the deliberate design choice that makes the feature
reliable on both agents. The submit primitive appends the Enter itself, so the buttons pass the phrase
**without** a trailing newline.

## Architecture

Three small, additive changes across the two footer files:

```
 toolbar_item.rs            agent_input_footer/mod.rs           use_agent_footer/mod.rs
 ┌──────────────────┐   ┌──────────────────────────────┐   ┌──────────────────────────────┐
 2 new item kinds    ─► 2 ActionButton fields + build   ─► handle new footer event by
 ContinuePrompt        + 2 AgentInputFooterAction         calling submit_text_to_cli_
 LooksGoodPrompt       + handlers emit new footer event   agent_pty(phrase, ctx)
 (label/icon/avail/    + 2 render arms (item -> button)   (1 new UseAgentToolbarEvent
  default rows)        + 1 new AgentInputFooterEvent       variant to forward it)
```

### Component 1 — Toolbar item kinds (`toolbar_item.rs`)

Add two unit variants to `AgentToolbarItemKind`: `ContinuePrompt` and `LooksGoodPrompt`. (Unit variants,
mirroring `Compact`/`ForkSession`; `#[serde(rename_all = "snake_case")]` already applies so they persist
in user toolbar configs as `continue_prompt` / `looks_good_prompt`.) Wire the methods that match over the
enum — each treated exactly like `Compact`:

- `available_in` → `ToolbarAvailability::CLIAgentOnly`.
- `available_to_session_viewer` → `!status.is_viewer()` (host-initiated actions, like Compact/Fork).
- `display_label` → `"Continue"` / `"LGTM"`.
- `icon` → `Icon::Play` (Continue) / `Icon::ThumbsUp` (LGTM). Both exist in `crates/warp_core/src/ui/icons.rs`.
- `is_available_during_handoff_compose` → `false`.
- Add both to `cli_default_left()` right after `ForkSession, Compact` so they render next to Fork by default.
- Add both to `all_available_for_cli_input()` so they appear in the CLI footer configurator.
- No change needed to `is_available`, `is_context_chip`, `context_chip_kind` (their existing arms/`_`
  already cover unit variants), nor to any agent-view-only list.

### Component 2 — Buttons, actions, and footer event (`agent_input_footer/mod.rs`)

- Two `ViewHandle<ActionButton>` fields: `continue_button`, `looks_good_button` (next to `compact_button`,
  `fork_button`).
- Build each in `AgentInputFooter::new` mirroring `compact_button` (lines ~415–424): `ActionButton::new`
  with the label + `AgentInputButtonTheme`, `.with_icon(..)`, `.with_tooltip(..)`, `.with_size(cli_button_size)`,
  and `.on_click(|ctx| ctx.dispatch_typed_action(AgentInputFooterAction::X))`.
  - Continue: label `"Continue"`, icon `Icon::Play`, tooltip `Send "Continue" to the agent`.
  - LGTM: label `"LGTM"`, icon `Icon::ThumbsUp`, tooltip `Looks good to me, continue`.
- Store both into the struct in the constructor's return (next to `compact_button, fork_button,`).
- Two new `AgentInputFooterAction` variants (`SendContinue`, `SendLooksGood`), whose handlers — guarded by
  `self.cli_agent(ctx).is_some()`, exactly like the `Compact` handler — emit the new footer event with the
  literal phrase:
  - `SendContinue` → `AgentInputFooterEvent::SubmitTextToCliAgent("Continue".to_string())`
  - `SendLooksGood` → `AgentInputFooterEvent::SubmitTextToCliAgent("Looks good to me, continue".to_string())`
- One new `AgentInputFooterEvent` variant: `SubmitTextToCliAgent(String)`.
- Render mapping: in **both** match sites over `AgentToolbarItemKind` (the render arm ~1492 and the
  no-op/`None` arm ~2274), add `ContinuePrompt => Some(ChildView::new(&self.continue_button).finish())` and
  `LooksGoodPrompt => Some(ChildView::new(&self.looks_good_button).finish())`, and include them in whichever
  arm currently lists `Compact | ForkSession` as producing no element in the *other* renderer, so both
  match statements stay exhaustive (no `_` wildcard, per repo convention).

### Component 3 — Route the event to the submit primitive (`use_agent_footer/mod.rs`)

- Add one `UseAgentToolbarEvent` variant: `SubmitTextToCliAgent(String)`.
- In `handle_agent_input_footer_event` (line 1161), add an arm:
  `AgentInputFooterEvent::SubmitTextToCliAgent(text) => ctx.emit(UseAgentToolbarEvent::SubmitTextToCliAgent(text.clone()))`.
- In the `UseAgentToolbarEvent` handler `handle_use_agent_footer_event` (the block at lines 207–285 that
  handles `WriteToPty`), add an arm for `SubmitTextToCliAgent(text)` that calls
  `self.submit_text_to_cli_agent_pty(text.clone(), ctx)`.
  **cfg note (resolved in review):** that handler match is compiled *unconditionally* (the `WriteToPty`
  arm is not gated), but `submit_text_to_cli_agent_pty` is `#[cfg(feature = "local_tty")]`. So the new
  arm must gate the call inline — `#[cfg(feature = "local_tty")]` on the submit call, with a
  `#[cfg(not(feature = "local_tty"))] { let _ = text; }` no-op — so non-`local_tty` builds (e.g. wasm)
  still compile. A build without a local PTY has nothing to submit to, so the no-op is correct.

## Data flow

**Continue:** click → `AgentInputFooterAction::SendContinue` → (guard: CLI agent present) →
`AgentInputFooterEvent::SubmitTextToCliAgent("Continue")` → `UseAgentToolbarEvent::SubmitTextToCliAgent("Continue")`
→ `submit_text_to_cli_agent_pty("Continue")` → resolves Claude/Codex → writes `Continue` + per-agent Enter.

**LGTM:** identical, with `Looks good to me, continue`.

## Edge cases & failure handling

- **No CLI agent in pane:** buttons not rendered (CLI-only availability); and the action handler's
  `cli_agent(ctx).is_some()` guard plus `submit_text_to_cli_agent_pty`'s own "no session → return" make it a
  double-safe no-op.
- **Rich-input composer open:** the buttons still submit **directly** to the PTY (they do not touch the
  composer). This is the intended "quick reply" semantics; predictable and matches how the phrase would be
  entered if the composer were closed. Accepted trade-off; a composer-aware variant is out of scope.
- **Agent mid-turn / not at a prompt:** the text is delivered as typed input; the agent handles it exactly
  as if the user typed it while busy. No Clinch-side special-casing (same posture as Compact).
- **Shared-session viewer:** `available_to_session_viewer` returns `false`, so viewers never see the buttons
  (they must not drive another user's agent).
- **Empty/whitespace:** N/A — the phrases are fixed non-empty literals; `submit_text_to_cli_agent_pty` also
  guards empty input.

## Security / privacy

- No new data leaves the machine. The buttons write two fixed local strings to the local PTY. No network,
  no files, no new persisted state beyond the existing user toolbar-config serialization (which now may
  contain the two new item-kind tags).

## Testing

- **Rust unit (`toolbar_item.rs` / footer tests):**
  - `ContinuePrompt` and `LooksGoodPrompt` report `available_in() == CLIAgentOnly`,
    `available_to_session_viewer(viewer) == false`, correct `display_label()` / `icon()`.
  - Both appear in `cli_default_left()` (immediately after `ForkSession`/`Compact`) and in
    `all_available_for_cli_input()`; absent from the agent-view lists.
- **Rust unit (`agent_input_footer` action handlers):** dispatching `SendContinue` / `SendLooksGood` with a
  CLI agent present emits `AgentInputFooterEvent::SubmitTextToCliAgent` carrying exactly `"Continue"` /
  `"Looks good to me, continue"`; with no CLI agent present, emits nothing (guard holds). Mirror the existing
  Compact handler test if present.
- **Manual (`build-app.sh` / `cargo run`):** run `claude` and `codex` in panes; click **Continue** → the word
  `Continue` is entered and submitted (a new turn starts); click **LGTM** → `Looks good to me, continue` is
  entered and submitted. Verify on **both** agents (this is the per-agent-Enter check). Confirm both buttons
  are absent in a plain shell pane and reorderable/removable in the footer editor.

## No-dead-code audit

- Purely additive: two enum variants, two buttons + fields, two actions + handlers, one new
  `AgentInputFooterEvent` variant, one new `UseAgentToolbarEvent` variant, one routing arm.
- Nothing is removed or orphaned. `submit_text_to_cli_agent_pty` gains a second caller (previously used by
  shared-session follow-up prompts), so it is not dead and not duplicated.
- No `_` wildcards introduced in the `AgentToolbarItemKind` matches (repo convention: exhaustive matching).
- No feature flag added, so no future flag-cleanup debt.

## Decision log

- Host = the existing CLI-agent footer, per-pane, next to Fork/Compact. (User.)
- Second button label = **"LGTM"** with full `Looks good to me, continue` as tooltip; sends the full phrase. (User.)
- Both buttons use icon + label (`Icon::Play`, `Icon::ThumbsUp`) to match Fork/Compact styling. (User.)
- Exact text sent: `Continue` and `Looks good to me, continue`, no trailing punctuation. (User.)
- Submit via `submit_text_to_cli_agent_pty` (per-agent Enter), **not** `WriteToPty` — reliability on both
  Claude and Codex. (Design — the one substantive correctness choice.)
- No feature flag; consistent with the unflagged Fork/Compact siblings and the removable-via-editor model. (Design.)

## Resolved during self-review (verified against code)

1. **`local_tty` cfg gating** — `handle_use_agent_footer_event`'s match is compiled unconditionally, but
   `submit_text_to_cli_agent_pty` is `#[cfg(feature = "local_tty")]`. The new `SubmitTextToCliAgent` arm
   therefore gates the call inline (`#[cfg(feature = "local_tty")]` + a `#[cfg(not(...))] { let _ = text; }`
   no-op), so all build configs compile.
2. **Button size / theme** — Compact uses `cli_button_size = ButtonSize::AgentInputButton` and
   `AgentInputButtonTheme` (`agent_input_footer/mod.rs:399, 415–424`). The two new buttons use the same, so
   styling matches exactly.
3. **Render match sites** — there are two exhaustive matches over `AgentToolbarItemKind`:
   `render_cli_toolbar_item` (`mod.rs:1485`, returns `Some(ChildView…)` for CLI items — the two new buttons
   go here) and `render_toolbar_item` (`mod.rs:2203`, the agent-view renderer that returns `None` for CLI-only
   items — the two new variants join the `Compact | ForkSession => None` arm at line 2271). Both stay
   exhaustive with no `_` wildcard.
4. **Guard pattern** — the action handlers mirror `Compact`/`ForkSession` exactly:
   `if self.cli_agent(ctx).is_some() { ctx.emit(…) }` (`mod.rs:2624–2636`).
