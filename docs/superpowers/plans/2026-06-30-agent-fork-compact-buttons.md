# Fork & Compact CLI-agent footer buttons — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add **Fork** and **Compact** buttons to the per-pane CLI-agent footer: Fork opens a new tab running a fork of this pane's Claude/Codex session (original untouched); Compact sends `/compact` to the live agent.

**Architecture:** Reuse machinery that already ships in Clinch. Fork derives a fork command from the agent-resume registry (`claude --resume <id>` → `… --fork-session`; `codex resume <id>` → `codex fork <id>`), creates a new tab pinned to the original pane's directory, and auto-runs the command via the existing restore-replay path (`Bootstrapped` → `write_command`). Compact reuses the footer's existing `WriteToPty` path to type `/compact\n` into the running agent. Buttons are new `AgentToolbarItemKind`s rendered in the CLI-agent footer; Fork's click bubbles footer → `TerminalView` → `PaneGroup` → `Workspace` exactly like the existing `ToggleCodeReviewPane` action.

**Tech Stack:** Rust, GPUI (Warp's UI framework), the `warp` app crate (`app/`), `serde`/`serde_json`.

## Global Constraints

- **Platform:** macOS, local-only Clinch build (logged-out / `skip_login`). The CLI-agent footer renders logged-out — confirmed: `should_render_use_agent_footer` (`app/src/terminal/view/use_agent_footer/mod.rs:290`) returns early for the CLI-agent branch *before* the `is_any_ai_enabled` gate. No AI/login gating to work around.
- **No inline test modules.** `script/check_no_inline_test_modules` forbids `mod tests { … }`. Tests go in a sibling `<file>_tests.rs`, included with `#[cfg(test)] #[path = "<file>_tests.rs"] mod tests;`.
- **Agents supported:** Claude Code + Codex only. Stay agent-agnostic where practical; the only agent-aware code is the two-rule fork-command derivation.
- **Fork command runs via `write_command`** (the restore-replay path), which appends the shell's own execute bytes — so the fork command string carries **no** trailing newline and **no** `cd` prefix (the new tab's directory is set by `initial_directory`).
- **Compact bytes:** `/compact\n` (LF) — the repo's convention for submitting a line to a CLI agent (`AIAgentPtyWriteMode::Line`, POSIX branch). `\r` is the documented manual-test fallback.
- **App crate name:** `warp`. Unit tests: `cargo test -p warp <filter>`. Compile check: `cargo check -p warp`. Full app build for manual testing: `./tools/agent-resume/build-app.sh` (or the project's normal run path).
- **`local_tty` feature** gates `set_on_restore_command` (the auto-run). It is on for the desktop build; the inner replay block is `#[cfg(feature = "local_tty")]`, mirroring the restore path at `app/src/pane_group/mod.rs:1672`.

---

## File Structure

**Create:**
- `app/src/agent_resume_tests.rs` — moved + new unit tests for `agent_resume` (satisfies the no-inline-tests rule).

**Modify:**
- `app/src/agent_resume.rs` — add `cwd` to `RegistryEntry`, `ForkLaunch` struct, `derive_fork_command`, `read_fork_launch`; convert inline tests to a sibling include.
- `app/src/pane_group/mod.rs` — add `PaneGroup::fork_launch_for_terminal_view` + `pane_group::Event::ForkCliAgentSession` variant.
- `app/src/workspace/view.rs` — add `Workspace::fork_cli_agent_session` + the `pane_group::Event::ForkCliAgentSession` handler arm.
- `app/src/ai/blocklist/agent_view/agent_input_footer/toolbar_item.rs` — add `Compact` and `ForkSession` to `AgentToolbarItemKind` (+ all match arms + CLI defaults).
- `app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs` — add `compact_button` + `fork_button` fields, their constructors, struct-literal entries, `render_cli_toolbar_item` arms, `AgentInputFooterAction::{Compact, ForkSession}` + `handle_action` arms, `AgentInputFooterEvent::ForkSession`.
- `app/src/terminal/view/use_agent_footer/mod.rs` — forward `AgentInputFooterEvent::ForkSession` → `UseAgentToolbarEvent::ForkSession`; handle it on `TerminalView`; add `UseAgentToolbarEvent::ForkSession`.
- `app/src/terminal/view.rs` — add `TerminalView::fork_cli_agent_session` + `Event::ForkCliAgentSession { terminal_view_id }` variant.
- `app/src/pane_group/pane/terminal_pane.rs` — re-emit `Event::ForkCliAgentSession` → `pane_group::Event::ForkCliAgentSession`.

---

## Task 1: Registry — read cwd + derive fork command

**Files:**
- Modify: `app/src/agent_resume.rs`
- Create: `app/src/agent_resume_tests.rs`

**Interfaces:**
- Produces:
  - `pub struct ForkLaunch { pub command: String, pub cwd: Option<String> }`
  - `pub fn read_fork_launch(uuid: &[u8]) -> Option<ForkLaunch>`
  - (private) `fn derive_fork_command(command: &str) -> Option<String>`

- [ ] **Step 1: Move existing tests to a sibling file and add the include.**

Create `app/src/agent_resume_tests.rs` with the current tests plus new ones:

```rust
use super::*;
use std::io::Write;

#[test]
fn reads_command_from_registry_file() {
    let dir = std::env::temp_dir().join(format!("agent_resume_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut f = std::fs::File::create(dir.join("deadbeef.json")).unwrap();
    write!(f, r#"{{ "command": "claude --resume abc-123", "cwd": "/tmp" }}"#).unwrap();

    assert_eq!(
        read_command_in(&dir, "deadbeef"),
        Some("claude --resume abc-123".to_string())
    );
    assert_eq!(read_command_in(&dir, "missing"), None);
}

#[test]
fn uuid_hex_is_lowercase() {
    // Must match $WARP_TERMINAL_SESSION_UUID casing.
    assert_eq!(hex::encode([0xAB, 0xCD]), "abcd");
}

#[test]
fn derives_claude_fork_command() {
    assert_eq!(
        derive_fork_command("claude --resume abc-123").as_deref(),
        Some("claude --resume abc-123 --fork-session")
    );
}

#[test]
fn derives_codex_fork_command() {
    assert_eq!(
        derive_fork_command("codex resume abc-123").as_deref(),
        Some("codex fork abc-123")
    );
}

#[test]
fn no_fork_command_for_unknown() {
    assert_eq!(derive_fork_command("vim"), None);
    assert_eq!(derive_fork_command(""), None);
    // A bare `claude` with no --resume is not resumable/forkable.
    assert_eq!(derive_fork_command("claude"), None);
}

#[test]
fn read_fork_launch_reads_derived_command_and_cwd() {
    let dir = std::env::temp_dir().join(format!("agent_resume_fork_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut f = std::fs::File::create(dir.join("feedface.json")).unwrap();
    write!(f, r#"{{ "command": "codex resume xyz-9", "cwd": "/work" }}"#).unwrap();

    let launch = read_fork_launch_in(&dir, "feedface").unwrap();
    assert_eq!(launch.command, "codex fork xyz-9");
    assert_eq!(launch.cwd.as_deref(), Some("/work"));

    // No cwd in the file → None cwd, still derives the command.
    let mut f2 = std::fs::File::create(dir.join("cafe.json")).unwrap();
    write!(f2, r#"{{ "command": "claude --resume id-1" }}"#).unwrap();
    let launch2 = read_fork_launch_in(&dir, "cafe").unwrap();
    assert_eq!(launch2.command, "claude --resume id-1 --fork-session");
    assert_eq!(launch2.cwd, None);

    assert!(read_fork_launch_in(&dir, "missing").is_none());
}
```

- [ ] **Step 2: Rewrite `app/src/agent_resume.rs`.** Replace the whole file with:

```rust
//! Reads the per-pane agent-resume registry written by the claude wrapper / codex hooks.
//! See docs/superpowers/specs/2026-06-20-warp-agent-session-resume-design.md.

use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Deserialize)]
struct RegistryEntry {
    command: String,
    #[serde(default)]
    cwd: Option<String>,
}

/// A ready-to-run "fork this session" command plus the directory it should run in.
/// Derived from the resume command the capture scripts already store.
pub struct ForkLaunch {
    pub command: String,
    pub cwd: Option<String>,
}

fn registry_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(Path::new(&home).join(".warp").join("agent-resume"))
}

fn read_entry_in(dir: &Path, uuid_hex: &str) -> Option<RegistryEntry> {
    let path = dir.join(format!("{uuid_hex}.json"));
    let contents = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&contents).ok()
}

fn read_command_in(dir: &Path, uuid_hex: &str) -> Option<String> {
    Some(read_entry_in(dir, uuid_hex)?.command)
}

/// Turns a stored resume command into a fork command. Returns `None` for commands
/// we don't know how to fork (the only forkable agents today are Claude and Codex).
fn derive_fork_command(command: &str) -> Option<String> {
    let command = command.trim();
    if command.starts_with("claude --resume ") {
        Some(format!("{command} --fork-session"))
    } else if command.starts_with("codex resume ") {
        Some(command.replacen("codex resume", "codex fork", 1))
    } else {
        None
    }
}

fn read_fork_launch_in(dir: &Path, uuid_hex: &str) -> Option<ForkLaunch> {
    let entry = read_entry_in(dir, uuid_hex)?;
    let command = derive_fork_command(&entry.command)?;
    Some(ForkLaunch {
        command,
        cwd: entry.cwd,
    })
}

/// Returns the resume command stored for `uuid`, if any. `uuid` is the raw pane UUID bytes;
/// it is hex-encoded (lowercase) to match `$WARP_TERMINAL_SESSION_UUID`.
pub fn read_on_restore_command(uuid: &[u8]) -> Option<String> {
    let dir = registry_dir()?;
    read_command_in(&dir, &hex::encode(uuid))
}

/// Returns the fork launch (command + cwd) for `uuid`, if the pane has a forkable
/// agent session in the registry.
pub fn read_fork_launch(uuid: &[u8]) -> Option<ForkLaunch> {
    let dir = registry_dir()?;
    read_fork_launch_in(&dir, &hex::encode(uuid))
}

#[cfg(test)]
#[path = "agent_resume_tests.rs"]
mod tests;
```

- [ ] **Step 3: Run the tests, expect PASS.**

Run: `cargo test -p warp agent_resume`
Expected: the 6 tests pass (`derives_claude_fork_command`, `derives_codex_fork_command`, `no_fork_command_for_unknown`, `read_fork_launch_reads_derived_command_and_cwd`, `reads_command_from_registry_file`, `uuid_hex_is_lowercase`).

- [ ] **Step 4: Verify the no-inline-tests lint passes.**

Run: `bash script/check_no_inline_test_modules`
Expected: exit 0 (the inline `mod tests` in `agent_resume.rs` is gone).

- [ ] **Step 5: Commit.**

```bash
git add app/src/agent_resume.rs app/src/agent_resume_tests.rs
git commit -m "feat(fork): derive fork command + cwd from agent-resume registry"
```

---

## Task 2: PaneGroup — resolve a fork launch from a terminal view id

**Files:**
- Modify: `app/src/pane_group/mod.rs`

**Interfaces:**
- Consumes: `crate::agent_resume::{ForkLaunch, read_fork_launch}` (Task 1); `find_pane_id_for_terminal_view` (mod.rs:2185), `downcast_pane_by_id::<TerminalPane>` (mod.rs:4369), `TerminalPane::session_uuid` (terminal_pane.rs:225).
- Produces: `pub fn fork_launch_for_terminal_view(&self, terminal_view_id: EntityId, ctx: &AppContext) -> Option<ForkLaunch>`

- [ ] **Step 1: Add the resolver method** in the main `impl PaneGroup` block in `app/src/pane_group/mod.rs` (place it near `find_terminal_pane_by_session_uuid`, ~mod.rs:2232):

```rust
/// Resolves the agent-resume "fork" launch (command + cwd) for the pane that owns
/// `terminal_view_id`, if that pane currently has a forkable Claude/Codex session.
///
/// Used by the Fork footer button: the pane UUID is the agent-resume registry key.
pub fn fork_launch_for_terminal_view(
    &self,
    terminal_view_id: EntityId,
    ctx: &AppContext,
) -> Option<crate::agent_resume::ForkLaunch> {
    let pane_id = self.find_pane_id_for_terminal_view(terminal_view_id, ctx)?;
    let pane = self.downcast_pane_by_id::<TerminalPane>(pane_id)?;
    crate::agent_resume::read_fork_launch(&pane.session_uuid())
}
```

> If `EntityId` / `AppContext` are not already imported in `mod.rs`, add `use warpui::{AppContext, EntityId};` (match the existing import style in the file — grep for `EntityId` usage already present at `find_pane_id_for_terminal_view`).

- [ ] **Step 2: Compile-check.**

Run: `cargo check -p warp`
Expected: compiles. (A dead-code warning for `fork_launch_for_terminal_view` is acceptable until Task 5 calls it.)

- [ ] **Step 3: Commit.**

```bash
git add app/src/pane_group/mod.rs
git commit -m "feat(fork): PaneGroup::fork_launch_for_terminal_view resolver"
```

---

## Task 3: Workspace — fork_cli_agent_session (create tab + auto-run)

**Files:**
- Modify: `app/src/workspace/view.rs`

**Interfaces:**
- Consumes: `PaneGroup::fork_launch_for_terminal_view` (Task 2); `NewTerminalOptions::with_initial_directory_opt`, `add_tab_with_pane_layout`, `PanesLayout::SingleTerminal`, `active_tab_pane_group`, `PaneGroup::terminal_manager` (mod.rs:7392), `crate::terminal::local_tty::TerminalManager::set_on_restore_command` (terminal_manager.rs:1052). Template: `Workspace::open_directory_in_new_tab` (view.rs:8337).
- Produces: `fn fork_cli_agent_session(&mut self, pane_group: &ViewHandle<PaneGroup>, terminal_view_id: EntityId, ctx: &mut ViewContext<Self>)`

- [ ] **Step 1: Add the method** in the main `impl Workspace` block in `app/src/workspace/view.rs` (place it next to `open_directory_in_new_tab`, ~view.rs:8337). The `#[allow(dead_code)]` is removed in Task 5 when the event arm calls it.

```rust
/// Forks the Claude/Codex session in the pane owning `terminal_view_id` into a NEW tab.
///
/// The original pane is untouched. The new tab opens in the session's original directory
/// and auto-runs the fork command (`claude --resume <id> --fork-session` / `codex fork <id>`)
/// after its shell bootstraps, reusing the agent-resume restore-replay path.
#[allow(dead_code)] // Wired to the Fork footer button in a later task.
fn fork_cli_agent_session(
    &mut self,
    pane_group: &ViewHandle<PaneGroup>,
    terminal_view_id: EntityId,
    ctx: &mut ViewContext<Self>,
) {
    let Some(fork) = pane_group.read(ctx, |pane_group, ctx| {
        pane_group.fork_launch_for_terminal_view(terminal_view_id, ctx)
    }) else {
        log::warn!("fork: no forkable CLI-agent session for the focused pane");
        return;
    };

    // Create a new tab pinned to the session's original directory (bypasses the
    // user's working-directory setting, so `claude --resume`'s cwd-scoping holds).
    let options = NewTerminalOptions::default()
        .with_initial_directory_opt(fork.cwd.as_deref().map(std::path::PathBuf::from));
    self.add_tab_with_pane_layout(
        PanesLayout::SingleTerminal(Box::new(options)),
        std::sync::Arc::new(std::collections::HashMap::new()),
        None,
        ctx,
    );

    // Attach the one-shot replay command to the new (now active) tab's single pane,
    // mirroring the snapshot-restore path (pane_group/mod.rs:1672).
    let command = fork.command;
    #[cfg(feature = "local_tty")]
    {
        let manager_handle = self
            .active_tab_pane_group()
            .read(ctx, |pane_group, ctx| pane_group.terminal_manager(0, ctx));
        if let Some(manager_handle) = manager_handle {
            manager_handle.update(ctx, |terminal_manager, ctx| {
                if let Some(manager) = terminal_manager
                    .as_any()
                    .downcast_ref::<crate::terminal::local_tty::TerminalManager>()
                {
                    manager.set_on_restore_command(command, ctx);
                }
            });
        }
    }
    #[cfg(not(feature = "local_tty"))]
    let _ = command;
}
```

> Imports: `NewTerminalOptions`, `PanesLayout`, `ViewHandle`, `EntityId`, `PaneGroup` are already used by `open_directory_in_new_tab` and neighbors in this file. If `with_initial_directory_opt` has a different exact name, check `NewTerminalOptions` builders at `app/src/pane_group/mod.rs:811-823` and use `with_initial_directory` with an explicit `if let Some(dir)` instead.

- [ ] **Step 2: Compile-check.**

Run: `cargo check -p warp`
Expected: compiles with a dead-code allowance (the `#[allow(dead_code)]` suppresses the warning).

- [ ] **Step 3: Commit.**

```bash
git add app/src/workspace/view.rs
git commit -m "feat(fork): Workspace::fork_cli_agent_session opens forked session in new tab"
```

---

## Task 4: Compact button (end-to-end)

Compact reuses the existing `WriteToPty` path — no new event/forward/handler plumbing. Just a new toolbar item + button + action.

**Files:**
- Modify: `app/src/ai/blocklist/agent_view/agent_input_footer/toolbar_item.rs`
- Modify: `app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs`

**Interfaces:**
- Consumes: existing `AgentInputFooterEvent::WriteToPty(String)` (mod.rs:2683), forwarded (use_agent_footer/mod.rs:1165) and handled (use_agent_footer/mod.rs:218).
- Produces: `AgentToolbarItemKind::Compact`, `AgentInputFooterAction::Compact`, `compact_button` field.

- [ ] **Step 1: Add `Compact` to `AgentToolbarItemKind`** in `toolbar_item.rs`. After `Settings,` (line 68) add:

```rust
    // CLI agent only – sends `/compact` to the running agent.
    Compact,
```

- [ ] **Step 2: Add the `Compact` match arms** in `toolbar_item.rs`:

In `available_in` (the `CLIAgentOnly` group, line 88):
```rust
            Self::FileExplorer | Self::RichInput | Self::Settings | Self::Compact => {
                ToolbarAvailability::CLIAgentOnly
            }
```
In `available_to_session_viewer` — a viewer must not drive the host's agent, so add to the host-only group (line 103):
```rust
            Self::Settings | Self::ShareSession | Self::FileExplorer | Self::Compact => {
                !status.is_viewer()
            }
```
In `display_label` (after line 128):
```rust
            Self::Compact => "Compact",
```
In `icon` (after line 145):
```rust
            Self::Compact => Some(Icon::Minimize),
```
In `is_available_during_handoff_compose` — not relevant to cloud-handoff compose, so `false` (add to the `false` group, line 169):
```rust
            | Self::Settings
            | Self::Compact => false,
```
(`is_available` has a `_ => true` catch-all — no edit needed.)

- [ ] **Step 3: Make Compact appear by default** in the CLI footer, on the **left** (matching the bottom-left mock). In `cli_default_left` (line 263), prepend `Self::Compact`:
```rust
    pub fn cli_default_left() -> Vec<Self> {
        let mut items = vec![
            Self::Compact,
            Self::FileAttach,
            Self::VoiceInput,
            Self::ContextChip(ContextChipKind::GitDiffStats),
        ];
        // …rest of the function unchanged (ShareSession, FileExplorer, RichInput)…
```
And register it in the configurator list `all_available_for_cli_input` (line 296 `items.extend`):
```rust
        items.extend([
            Self::FileExplorer,
            Self::RichInput,
            Self::FileAttach,
            Self::VoiceInput,
            Self::Compact,
            Self::Settings,
        ]);
```

- [ ] **Step 4: Add the `compact_button` field** to `struct AgentInputFooter` in `mod.rs` (next to `file_explorer_button`, ~mod.rs:221):
```rust
    compact_button: ViewHandle<ActionButton>,
```

- [ ] **Step 5: Construct the button** in the `AgentInputFooter` constructor (mirror `file_explorer_button` at mod.rs:387-401):
```rust
        let compact_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new("Compact", AgentInputButtonTheme)
                .with_icon(Icon::Minimize)
                .with_tooltip("Compact this agent's context (/compact)")
                .with_size(cli_button_size)
                .with_tooltip_alignment(TooltipAlignment::Left)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(AgentInputFooterAction::Compact);
                })
        });
```
Add `compact_button,` to the struct-literal returned by the constructor (next to `file_explorer_button,`, ~mod.rs:841).

- [ ] **Step 6: Render it** — add an arm in `render_cli_toolbar_item` (mod.rs:1422-1491), next to the `FileExplorer` arm (mod.rs:1449):
```rust
            AgentToolbarItemKind::Compact => {
                Some(ChildView::new(&self.compact_button).finish())
            }
```

- [ ] **Step 7: Add the action + handler.** In `enum AgentInputFooterAction` (mod.rs:2440) add:
```rust
    Compact,
```
In `handle_action` (mod.rs:2470), add an arm (next to `ToggleFileExplorer`, ~mod.rs:2516):
```rust
            AgentInputFooterAction::Compact => {
                // Type `/compact` + Enter straight into the live agent's PTY.
                // The footer WriteToPty path writes bytes verbatim, so include the newline.
                // Guard on a CLI agent being present (consistent with ForkSession) so a
                // stray keybinding can't type `/compact` into a plain shell.
                if self.cli_agent(ctx).is_some() {
                    ctx.emit(AgentInputFooterEvent::WriteToPty("/compact\n".to_string()));
                }
            }
```

- [ ] **Step 8: Compile-check.**

Run: `cargo check -p warp`
Expected: compiles. Fix any exhaustiveness errors the compiler flags by adding the missing `Self::Compact` arm it names.

- [ ] **Step 9: Commit.**

```bash
git add app/src/ai/blocklist/agent_view/agent_input_footer/toolbar_item.rs \
        app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs
git commit -m "feat(compact): add Compact button to CLI-agent footer"
```

---

## Task 5: Fork button + event chain (end-to-end)

Wires a Fork button up the 6-hop chain (mirroring `ToggleCodeReviewPane`) to `Workspace::fork_cli_agent_session` (Task 3). All variants/arms are added together to keep exhaustive matches compiling.

**Files:**
- Modify: `app/src/ai/blocklist/agent_view/agent_input_footer/toolbar_item.rs`
- Modify: `app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs`
- Modify: `app/src/terminal/view/use_agent_footer/mod.rs`
- Modify: `app/src/terminal/view.rs`
- Modify: `app/src/pane_group/pane/terminal_pane.rs`
- Modify: `app/src/pane_group/mod.rs`
- Modify: `app/src/workspace/view.rs`

**Interfaces:**
- Consumes: `Workspace::fork_cli_agent_session` (Task 3).
- Produces: `AgentToolbarItemKind::ForkSession`, `AgentInputFooterAction::ForkSession`, `AgentInputFooterEvent::ForkSession`, `UseAgentToolbarEvent::ForkSession`, `terminal::view::Event::ForkCliAgentSession { terminal_view_id }`, `pane_group::Event::ForkCliAgentSession { terminal_view_id }`, `fork_button` field, `TerminalView::fork_cli_agent_session`.

- [ ] **Step 1: Toolbar item.** In `toolbar_item.rs` add `ForkSession` to the enum (after `Compact`):
```rust
    // CLI agent only – forks this session into a new tab.
    ForkSession,
```
Add `ForkSession` to the same match arms as Compact in Task 4:
- `available_in` CLIAgentOnly group: `… | Self::Compact | Self::ForkSession =>`
- `available_to_session_viewer` host-only group: `… | Self::Compact | Self::ForkSession => !status.is_viewer()`
- `display_label`: `Self::ForkSession => "Fork",`
- `icon`: `Self::ForkSession => Some(Icon::GitBranch),`
- `is_available_during_handoff_compose` false group: `| Self::Compact | Self::ForkSession => false,`

Make it appear by default — in `cli_default_left` add `Self::ForkSession,` before `Self::Compact,` (so the order is Fork then Compact, matching the mock); in `all_available_for_cli_input` add `Self::ForkSession,` to the `items.extend([…])`.

- [ ] **Step 2: Button + action.** In `mod.rs`:
Field (next to `compact_button`):
```rust
    fork_button: ViewHandle<ActionButton>,
```
Constructor (mirror `compact_button`):
```rust
        let fork_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new("Fork", AgentInputButtonTheme)
                .with_icon(Icon::GitBranch)
                .with_tooltip("Fork this session into a new tab")
                .with_size(cli_button_size)
                .with_tooltip_alignment(TooltipAlignment::Left)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(AgentInputFooterAction::ForkSession);
                })
        });
```
Add `fork_button,` to the struct-literal. Render arm in `render_cli_toolbar_item`:
```rust
            AgentToolbarItemKind::ForkSession => {
                Some(ChildView::new(&self.fork_button).finish())
            }
```
Action enum `AgentInputFooterAction`:
```rust
    ForkSession,
```
`handle_action` arm (emits the footer event; only when a CLI agent is present, mirroring `ToggleFileExplorer` at mod.rs:2516):
```rust
            AgentInputFooterAction::ForkSession => {
                if self.cli_agent(ctx).is_some() {
                    ctx.emit(AgentInputFooterEvent::ForkSession);
                }
            }
```
Event enum `AgentInputFooterEvent` (mod.rs:2679):
```rust
    ForkSession,
```

- [ ] **Step 3: Forward through `UseAgentToolbar`.** In `app/src/terminal/view/use_agent_footer/mod.rs`:
`UseAgentToolbarEvent` (mod.rs:1270) add:
```rust
    /// Fork the CLI agent session in this pane into a new tab.
    ForkSession,
```
Forward arm in `handle_agent_input_footer_event` (next to `ToggleFileExplorer`, ~mod.rs:1174):
```rust
            AgentInputFooterEvent::ForkSession => {
                ctx.emit(UseAgentToolbarEvent::ForkSession);
            }
```

- [ ] **Step 4: Handle on `TerminalView`.** Still in `use_agent_footer/mod.rs`, in `handle_use_agent_footer_event` (the `impl TerminalView` match, next to the `ToggleFileExplorer` arm ~mod.rs:237):
```rust
            UseAgentToolbarEvent::ForkSession => {
                self.fork_cli_agent_session(ctx);
            }
```

- [ ] **Step 5: TerminalView method + view event.** In `app/src/terminal/view.rs`:
Add the `Event::ForkCliAgentSession` variant near `ToggleCodeReviewPane` (view.rs:1746):
```rust
    ForkCliAgentSession { terminal_view_id: EntityId },
```
Add the method in `impl TerminalView` (near `toggle_code_review_pane`, view.rs:7026):
```rust
/// Requests that the workspace fork this pane's CLI-agent session into a new tab.
pub fn fork_cli_agent_session(&mut self, ctx: &mut ViewContext<Self>) {
    ctx.emit(Event::ForkCliAgentSession {
        terminal_view_id: self.view_id,
    });
}
```

- [ ] **Step 6: Re-emit on the pane node.** In `app/src/pane_group/pane/terminal_pane.rs`, next to the `Event::ToggleCodeReviewPane` re-emit (terminal_pane.rs:1162):
```rust
                Event::ForkCliAgentSession { terminal_view_id } => {
                    ctx.emit(pane_group::Event::ForkCliAgentSession {
                        terminal_view_id: *terminal_view_id,
                    });
                }
```

- [ ] **Step 7: pane_group event variant.** In `app/src/pane_group/mod.rs`, next to `ToggleCodeReviewPane` (mod.rs:562):
```rust
    ForkCliAgentSession { terminal_view_id: EntityId },
```

- [ ] **Step 8: Workspace handler.** In `app/src/workspace/view.rs`, in the `pane_group::Event` match next to `ToggleCodeReviewPane` (view.rs:15843):
```rust
                pane_group::Event::ForkCliAgentSession { terminal_view_id } => {
                    self.fork_cli_agent_session(&pane_group, *terminal_view_id, ctx);
                }
```
Then remove the `#[allow(dead_code)]` from `fork_cli_agent_session` (Task 3 Step 1), since it's now called.

- [ ] **Step 9: Compile-check and fix exhaustiveness.**

Run: `cargo check -p warp`
Expected: compiles. Most consumers of `terminal::view::Event` / `pane_group::Event` have a `_ => {}` catch-all (as the existing `ToggleCodeReviewPane` variant relies on), so churn should be minimal. The compiler will name any *exhaustive* match that still needs a `ForkCliAgentSession` / `ForkSession` arm; add a no-op or forwarding arm there, matching how that match treats `ToggleCodeReviewPane`.

- [ ] **Step 10: Commit.**

```bash
git add app/src/ai/blocklist/agent_view/agent_input_footer/toolbar_item.rs \
        app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs \
        app/src/terminal/view/use_agent_footer/mod.rs \
        app/src/terminal/view.rs \
        app/src/pane_group/pane/terminal_pane.rs \
        app/src/pane_group/mod.rs \
        app/src/workspace/view.rs
git commit -m "feat(fork): wire Fork footer button to new-tab forked session"
```

---

## Task 6: Manual verification (build + click-through)

**Files:** none (verification only).

- [ ] **Step 1: Build the app.**

Run: `./tools/agent-resume/build-app.sh` (or the project's normal build/run path).
Expected: builds and launches Clinch.

- [ ] **Step 2: Compact — Claude.** In a tab, run `claude` and start a conversation. The CLI-agent footer shows **Compact** and **Fork**. Click **Compact**.
Expected: `/compact` runs in the live Claude session (context compaction begins). If nothing happens, re-test with `\r` instead of `\n` (Global Constraints) and update Task 4 Step 7.

- [ ] **Step 3: Compact — Codex.** Repeat in a `codex` tab. Expected: Codex compacts.

- [ ] **Step 4: Fork — Claude.** In the Claude tab, click **Fork**.
Expected: a NEW tab opens in the same directory and auto-runs `claude --resume <id> --fork-session` (forked conversation, full history). The ORIGINAL tab keeps its session, untouched.

- [ ] **Step 5: Fork — Codex.** Repeat in the `codex` tab. Expected: new tab runs `codex fork <id>`; original untouched.

- [ ] **Step 6: Negative case.** In a plain shell pane (no agent), confirm no Fork/Compact buttons appear (the CLI-agent footer isn't rendered).

- [ ] **Step 7: is-agent-in-control caveat.** Confirm Compact still delivers while the agent is mid-turn; if it's dropped, note that `write_user_bytes_to_pty` early-returns under `is_agent_in_control()` and decide whether to gate the button's enabled state on agent status (follow-up, not required for v1).

---

## Known limitations (documented, not blocking)

- **Restart-resume of a forked Claude tab points at the parent.** The `claude` capture wrapper records the *explicit* `--resume <id>` it sees, so a forked tab's registry entry stores the parent id (the `--fork-session` runtime id is not known at launch). If Clinch is quit and restored, that forked tab resumes the **parent** conversation, not the fork. The fork itself (at click time) is correct; only auto-resume-after-restart of that specific tab is affected. **Codex is unaffected** — its `SessionStart` hook reads the new session id from stdin and records `codex fork`'s real id. Future fix: pin the forked Claude id (e.g. `--session-id`) and teach the wrapper to record it.
- **Fork with no registry entry is a no-op.** If an agent was started outside the capture path (`claude --continue`, the interactive picker), no registry file exists, so Fork logs a warning and does nothing (Compact still works). Acceptable; avoids per-frame file IO to hide the button.
- **Default footer chips change.** Adding `Fork`/`Compact` to `cli_default_left` only affects users on the default selection; users with a saved custom toolbar layout add them via the configurator. (Left vs right is a one-line swap if the balance looks better on the right.)

---

## Self-Review

**1. Spec coverage:**
- Fork (both agents), new tab, original untouched → Tasks 1,2,3,5. ✓
- Compact (both agents), `/compact` to live agent → Task 4. ✓
- Host = existing CLI-agent footer → Tasks 4,5 (render in `render_cli_toolbar_item`). ✓
- Agent-agnostic-ish fork command, derived (capture-script change dropped per planning simplification — noted in plan header & limitations). ✓
- skip_login renders footer → confirmed in Global Constraints; no fallback bar needed. ✓
- `/split` retained → untouched (no task removes it). ✓
- No-dead-code → `#[allow(dead_code)]` added in Task 3 and removed in Task 5; no orphaned code. ✓

**2. Placeholder scan:** No TBD/TODO. Every code step shows real code. Compile-check steps name the compiler as the source of any remaining exhaustiveness arms (honest for a large GPUI codebase) rather than leaving logic undefined.

**3. Type consistency:** `ForkLaunch { command, cwd }`, `fork_launch_for_terminal_view`, `fork_cli_agent_session`, `Event::ForkCliAgentSession { terminal_view_id }`, `pane_group::Event::ForkCliAgentSession { terminal_view_id }`, `AgentInputFooterAction::{Compact, ForkSession}`, `AgentInputFooterEvent::{WriteToPty, ForkSession}`, `UseAgentToolbarEvent::ForkSession`, `AgentToolbarItemKind::{Compact, ForkSession}` — names are used identically across tasks. ✓
