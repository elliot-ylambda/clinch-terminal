# CLI-Agent Footer Quick-Insert Buttons — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the user create footer buttons that insert-and-send a saved piece of text (free text, a slash command, or a skill reference) to the active CLI agent, created via a popup that also lists their existing slash commands and skills (user + project scope).

**Architecture:** Reuse the existing `CLIAgentToolbarChipSelection` footer-item system by adding a data-carrying `AgentToolbarItemKind::CustomInsert { label, text }` variant; clicking dispatches a new `InsertCustomText(String)` action that emits the existing `SubmitTextToCliAgent` (per-agent Enter handling). A net-new on-demand scanner discovers slash commands from `~/.claude/commands` / `<repo>/.claude/commands` / `~/.codex/prompts`; skills come from the existing `SkillManager`. A reused `Modal<T>` + `EditorView::single_line` popup creates buttons. Whole feature gated behind `FeatureFlag::CliAgentQuickInsertButtons`.

**Tech Stack:** Rust, WarpUI (Entity-Component-Handle), Diesel-free settings (`settings_value::SettingsValue`), serde/serde_yaml, `dirs`, `repo_metadata`.

**Spec:** `docs/superpowers/specs/2026-07-06-cli-footer-quick-insert-buttons-design.md`

## Global Constraints

- Exhaustive matching only — no `_` wildcards in any `match` you add or edit (project CLAUDE.md).
- Context params named `ctx`, placed last (except when a closure is last).
- Inline format args (`format!("{x}")`, not `format!("{}", x)`).
- Unit tests in a sibling `${file}_tests.rs`, wired via `#[cfg(test)] #[path=...] mod tests;`.
- Run `./script/format` and `cargo clippy --package warp --tests -- -D warnings` before every commit; both must pass.
- Feature flag name: `FeatureFlag::CliAgentQuickInsertButtons`; default-on for dogfood only.
- All new UI/behavior gated behind that flag.
- Build/test package is `warp`; toolchain at `~/.cargo/bin`.

---

### Task 1: Add the feature flag

**Files:**
- Modify: `crates/warp_features/src/lib.rs` (the `FeatureFlag` enum + `DOGFOOD_FLAGS`)
- Modify: `app/src/features.rs` (re-export / mirror, matching how `CLIAgentRichInput` is referenced)

**Interfaces:**
- Produces: `FeatureFlag::CliAgentQuickInsertButtons` (a `#[derive(Sequence)]` enum variant), `FeatureFlag::CliAgentQuickInsertButtons.is_enabled() -> bool`.

- [ ] **Step 1: Add the enum variant.** In `crates/warp_features/src/lib.rs`, find the `FeatureFlag` enum (search `CLIAgentRichInput`) and add a variant next to it:

```rust
    CliAgentQuickInsertButtons,
```

- [ ] **Step 2: Default-on for dogfood.** In the same file, find `DOGFOOD_FLAGS` and add:

```rust
    FeatureFlag::CliAgentQuickInsertButtons,
```

- [ ] **Step 3: Mirror in app if required.** Grep `rg "CLIAgentRichInput" app/src/features.rs`. If `CLIAgentRichInput` is listed/re-exported there, add `CliAgentQuickInsertButtons` the same way; if the app uses `crate::features::FeatureFlag` directly with no per-flag listing, no change is needed.

- [ ] **Step 4: Verify it compiles.**

Run: `~/.cargo/bin/cargo check --package warp_features`
Expected: PASS.

- [ ] **Step 5: Commit.**

```bash
git add crates/warp_features/src/lib.rs app/src/features.rs
git commit -m "feat(cli-agent): add CliAgentQuickInsertButtons feature flag"
```

---

### Task 2: Slash-command discovery module

**Files:**
- Create: `app/src/ai/cli_commands/mod.rs`
- Create: `app/src/ai/cli_commands/mod_test.rs`
- Modify: `app/src/ai/mod.rs` (add `pub mod cli_commands;`)

**Interfaces:**
- Produces:
  - `pub struct DiscoveredCommand { pub name: String, pub invocation: String, pub description: Option<String>, pub scope: CommandScope, pub provider: CommandProvider, pub path: std::path::PathBuf }`
  - `pub enum CommandScope { Home, Project }` (derive `Clone, Copy, Debug, PartialEq, Eq`)
  - `pub enum CommandProvider { Claude, Codex }` (derive `Clone, Copy, Debug, PartialEq, Eq`)
  - `pub fn discover_commands(working_directory: &std::path::Path, ctx: &warpui::AppContext) -> Vec<DiscoveredCommand>`
- Consumes: `dirs::home_dir()`, `repo_metadata::repositories::DetectedRepositories::as_ref(ctx).get_root_for_path(path)`.

- [ ] **Step 1: Write the failing test.** Create `app/src/ai/cli_commands/mod_test.rs`:

```rust
use super::*;
use std::fs;

// Builds a temp dir with a fake `.claude/commands` tree and returns its path.
fn write(root: &std::path::Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

#[test]
fn claude_commands_are_named_and_namespaced() {
    let tmp = tempfile::tempdir().unwrap();
    let cmd_dir = tmp.path().join(".claude/commands");
    write(&cmd_dir, "review.md", "---\ndescription: Review the diff\n---\nReview it.");
    write(&cmd_dir, "frontend/component.md", "Make a component.");
    write(&cmd_dir, "notes.txt", "ignored, not markdown");

    let mut got = scan_claude_commands_dir(&cmd_dir, CommandScope::Project);
    got.sort_by(|a, b| a.name.cmp(&b.name));

    assert_eq!(got.len(), 2);
    assert_eq!(got[0].name, "/frontend:component");
    assert_eq!(got[0].invocation, "/frontend:component");
    assert_eq!(got[0].description, None);
    assert_eq!(got[1].name, "/review");
    assert_eq!(got[1].description.as_deref(), Some("Review the diff"));
    assert_eq!(got[1].scope, CommandScope::Project);
    assert_eq!(got[1].provider, CommandProvider::Claude);
}

#[test]
fn missing_dir_yields_empty() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(scan_claude_commands_dir(&tmp.path().join("nope"), CommandScope::Home).is_empty());
}

#[test]
fn description_absent_without_front_matter() {
    let desc = front_matter_description("Just a body, no front matter.");
    assert_eq!(desc, None);
    let desc = front_matter_description("---\ndescription: Hi\nmodel: opus\n---\nbody");
    assert_eq!(desc.as_deref(), Some("Hi"));
}
```

- [ ] **Step 2: Run it to verify it fails.**

Run: `~/.cargo/bin/cargo test --package warp cli_commands:: --no-run`
Expected: FAIL to compile (`scan_claude_commands_dir` not found).

- [ ] **Step 3: Implement the module.** Create `app/src/ai/cli_commands/mod.rs`:

```rust
//! On-demand discovery of CLI-agent slash commands (Claude Code / Codex) for the
//! quick-insert-button picker. Unlike skills, there is no live watcher — the
//! popup scans when it opens.

use std::path::{Path, PathBuf};

use warpui::AppContext;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandScope {
    Home,
    Project,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandProvider {
    Claude,
    Codex,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredCommand {
    /// Display + insert text, e.g. "/review" or "/frontend:component".
    pub name: String,
    pub invocation: String,
    pub description: Option<String>,
    pub scope: CommandScope,
    pub provider: CommandProvider,
    pub path: PathBuf,
}

/// Discovers slash commands across user (home) and project scope.
pub fn discover_commands(working_directory: &Path, ctx: &AppContext) -> Vec<DiscoveredCommand> {
    let mut out = Vec::new();

    if let Some(home) = dirs::home_dir() {
        out.extend(scan_claude_commands_dir(
            &home.join(".claude/commands"),
            CommandScope::Home,
        ));
        out.extend(scan_codex_prompts_dir(
            &home.join(".codex/prompts"),
            CommandScope::Home,
        ));
    }

    if let Some(root) = repo_metadata::repositories::DetectedRepositories::as_ref(ctx)
        .get_root_for_path(working_directory)
    {
        out.extend(scan_claude_commands_dir(
            &root.join(".claude/commands"),
            CommandScope::Project,
        ));
    }

    out
}

/// Recursively scans a Claude `commands` dir; names are namespaced by subdir
/// (`frontend/component.md` -> `/frontend:component`).
fn scan_claude_commands_dir(dir: &Path, scope: CommandScope) -> Vec<DiscoveredCommand> {
    let mut out = Vec::new();
    collect_markdown(dir, dir, &mut out, scope, CommandProvider::Claude);
    out
}

/// Scans a Codex `prompts` dir (flat; name = `/<file-stem>`).
fn scan_codex_prompts_dir(dir: &Path, scope: CommandScope) -> Vec<DiscoveredCommand> {
    let mut out = Vec::new();
    collect_markdown(dir, dir, &mut out, scope, CommandProvider::Codex);
    // Codex prompts are not namespaced by subdir; the ":" join is harmless for a
    // flat dir but keep provider distinct for future divergence.
    out
}

fn collect_markdown(
    base: &Path,
    dir: &Path,
    out: &mut Vec<DiscoveredCommand>,
    scope: CommandScope,
    provider: CommandProvider,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_markdown(base, &path, out, scope, provider);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Ok(rel) = path.strip_prefix(base) else {
            continue;
        };
        let stem = rel.with_extension("");
        let name = format!(
            "/{}",
            stem.components()
                .filter_map(|c| c.as_os_str().to_str())
                .collect::<Vec<_>>()
                .join(":")
        );
        let description = std::fs::read_to_string(&path)
            .ok()
            .and_then(|c| front_matter_description(&c));
        out.push(DiscoveredCommand {
            invocation: name.clone(),
            name,
            description,
            scope,
            provider,
            path,
        });
    }
}

/// Extracts a `description:` value from optional YAML front matter.
fn front_matter_description(content: &str) -> Option<String> {
    let rest = content.strip_prefix("---")?;
    let end = rest.find("\n---")?;
    let yaml = &rest[..end];
    let value: serde_yaml::Value = serde_yaml::from_str(yaml.trim()).ok()?;
    value
        .get("description")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod tests;
```

- [ ] **Step 4: Register the module.** In `app/src/ai/mod.rs`, add `pub mod cli_commands;` (alphabetically near `pub mod skills;`).

- [ ] **Step 5: Ensure `tempfile` is a dev-dependency.** Run `rg "^tempfile" app/Cargo.toml`. If absent, add under `[dev-dependencies]`: `tempfile = "3"` (match the workspace version used elsewhere — `rg "tempfile" crates/*/Cargo.toml | head -1`).

- [ ] **Step 6: Run tests.**

Run: `~/.cargo/bin/cargo nextest run --package warp cli_commands::`
Expected: 3 tests PASS.

- [ ] **Step 7: format + clippy + commit.**

```bash
./script/format
~/.cargo/bin/cargo clippy --package warp --tests -- -D warnings
git add app/src/ai/cli_commands/ app/src/ai/mod.rs app/Cargo.toml
git commit -m "feat(cli-agent): discover slash commands for quick-insert picker"
```

---

### Task 3: `CustomInsert` toolbar item + insert-and-send action

**Files:**
- Modify: `app/src/ai/blocklist/agent_view/agent_input_footer/toolbar_item.rs` (enum + 5 exhaustive matches; `display_label` → `Cow`)
- Modify: `app/src/ai/blocklist/agent_view/agent_input_footer/toolbar_item_tests.rs`
- Modify: `app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs` (`InsertCustomText` action + handler + `render_cli_toolbar_item` arm + agent-view twin arm)
- Modify call sites of `display_label`: `app/src/chip_configurator/mod.rs`, `app/src/ai/blocklist/agent_view/agent_input_footer/toolbar_item_tests.rs`

**Interfaces:**
- Consumes: `AgentInputFooterEvent::SubmitTextToCliAgent(String)` (`mod.rs:2790`).
- Produces:
  - `AgentToolbarItemKind::CustomInsert { label: String, text: String }`
  - `AgentToolbarItemKind::display_label(&self) -> std::borrow::Cow<'static, str>`
  - `AgentInputFooterAction::InsertCustomText(String)`

- [ ] **Step 1: Write the failing test.** Append to `toolbar_item_tests.rs`:

```rust
#[test]
fn custom_insert_is_cli_only_and_host_only() {
    let item = AgentToolbarItemKind::CustomInsert {
        label: "Ship it".to_string(),
        text: "/deploy".to_string(),
    };
    assert_eq!(item.available_in(), ToolbarAvailability::CLIAgentOnly);
    assert!(!item.available_to_session_viewer(&SharedSessionStatus::reader(), false));
    assert_eq!(item.display_label(), "Ship it");
    assert_eq!(item.icon(), Some(Icon::Play));
}

#[test]
fn custom_insert_round_trips_through_serde() {
    let item = AgentToolbarItemKind::CustomInsert {
        label: "Review".to_string(),
        text: "/review".to_string(),
    };
    let json = serde_json::to_string(&item).unwrap();
    let back: AgentToolbarItemKind = serde_json::from_str(&json).unwrap();
    assert_eq!(item, back);
}
```

- [ ] **Step 2: Run it to verify it fails.**

Run: `~/.cargo/bin/cargo test --package warp toolbar_item:: --no-run`
Expected: FAIL (variant/`Cow` not present).

- [ ] **Step 3: Add the enum variant.** In `toolbar_item.rs`, inside `enum AgentToolbarItemKind`, add after `LooksGoodPrompt`:

```rust
    // CLI agent only – user-defined button that inserts-and-sends saved text.
    CustomInsert { label: String, text: String },
```

- [ ] **Step 4: Wire the exhaustive matches.** Edit each match to add a `CustomInsert` arm:

`available_in` — add `Self::CustomInsert { .. }` to the `CLIAgentOnly` group:
```rust
            Self::FileExplorer
            | Self::RichInput
            | Self::Settings
            | Self::Compact
            | Self::ForkSession
            | Self::ContinuePrompt
            | Self::CustomInsert { .. }
            | Self::LooksGoodPrompt => ToolbarAvailability::CLIAgentOnly,
```

`available_to_session_viewer` — add to the host-only (`!status.is_viewer()`) group:
```rust
            Self::Settings
            | Self::ShareSession
            | Self::FileExplorer
            | Self::Compact
            | Self::ForkSession
            | Self::ContinuePrompt
            | Self::CustomInsert { .. }
            | Self::LooksGoodPrompt => !status.is_viewer(),
```

`display_label` — change signature to `Cow` and wrap every existing arm; add the CustomInsert arm:
```rust
    pub fn display_label(&self) -> std::borrow::Cow<'static, str> {
        use std::borrow::Cow;
        match self {
            Self::ContextChip(_) => Cow::Borrowed("Context Chip"),
            Self::ModelSelector => Cow::Borrowed("Model Selector"),
            Self::NLDToggle => Cow::Borrowed("Autodetection"),
            Self::VoiceInput => Cow::Borrowed("Voice Input"),
            Self::FileAttach => Cow::Borrowed("Attach File"),
            Self::ContextWindowUsage => Cow::Borrowed("Context Usage"),
            Self::FileExplorer => Cow::Borrowed("File Explorer"),
            Self::RichInput => Cow::Borrowed("Rich Input"),
            Self::ShareSession => Cow::Borrowed("/remote-control"),
            Self::Settings => Cow::Borrowed("Settings"),
            Self::Compact => Cow::Borrowed("Compact"),
            Self::ForkSession => Cow::Borrowed("Fork"),
            Self::ContinuePrompt => Cow::Borrowed("Continue"),
            Self::LooksGoodPrompt => Cow::Borrowed("LGTM"),
            Self::FastForwardToggle => Cow::Borrowed("Fast Forward"),
            Self::HandoffToCloud => Cow::Borrowed("Hand off to cloud"),
            Self::CustomInsert { label, .. } => Cow::Owned(label.clone()),
        }
    }
```

`icon` — add before the closing brace:
```rust
            Self::CustomInsert { .. } => Some(Icon::Play),
```

`is_available_during_handoff_compose` — add `Self::CustomInsert { .. }` to the `=> false` group (read the tail of that match at `toolbar_item.rs:185-205` and include it there).

- [ ] **Step 5: Fix `display_label` call sites for `Cow`.**
  - In `toolbar_item_tests.rs` the two existing `display_label()` assertions compare to `&str`; `Cow` derefs, so change them to `item.display_label().as_ref()` or compare with `assert_eq!(x.display_label(), "Continue")` (works via `PartialEq<&str> for Cow` — verify; if it does not compile, use `.as_ref()`).
  - In `app/src/chip_configurator/mod.rs`, find the `AgentToolbarItemKind::display_label()` use (grep in that file). It likely does `.display_label().to_string()` or passes `&str`; `Cow` supports `.to_string()` and `.as_ref()`. Update to `.as_ref()` / `.into_owned()` as the local type requires.

- [ ] **Step 6: Add the action + handler.** In `mod.rs`, add to `enum AgentInputFooterAction` (near `InsertFilePath(String)` at `:2504`):
```rust
    InsertCustomText(String),
```
Add the handler next to `InsertFilePath` (`:2560`) / `SendContinue` (`:2602`):
```rust
            AgentInputFooterAction::InsertCustomText(text) => {
                // Insert-and-send the user's saved text, using the per-agent
                // submit strategy. Guard on a live CLI agent (like Continue).
                if self.cli_agent(ctx).is_some() {
                    ctx.emit(AgentInputFooterEvent::SubmitTextToCliAgent(text.clone()));
                }
            }
```

- [ ] **Step 7: Render the button.** In `render_cli_toolbar_item` (`mod.rs:1492`), add an arm before the `ContextChip` arm or alongside the control arms. Build the button dynamically from the item's fields:
```rust
            AgentToolbarItemKind::CustomInsert { label, text } => {
                if !FeatureFlag::CliAgentQuickInsertButtons.is_enabled() {
                    return None;
                }
                let text = text.clone();
                Some(
                    ActionButton::new(label.clone(), AgentInputButtonTheme)
                        .with_size(cli_button_size(appearance, app))
                        .with_tooltip(format!("Insert: {text}"))
                        .with_tooltip_alignment(TooltipAlignment::Left)
                        .on_click(move |ctx| {
                            ctx.dispatch_typed_action(AgentInputFooterAction::InsertCustomText(
                                text.clone(),
                            ));
                        })
                        .into_element(ctx), // match how other inline ActionButtons are finalized here
                )
            }
```
Note: match the exact `ActionButton` construction + finalization used by the `continue_button` (`mod.rs:426-435`) — copy its builder calls (`with_size`, theme, `into`/`finish`) so styling is identical. The render fn's signature exposes `appearance`/`app`; confirm names at `mod.rs:1470`.

- [ ] **Step 8: Agent-view twin arm.** In the agent-view render match (`mod.rs:2160-2238`) add `AgentToolbarItemKind::CustomInsert { .. } => None,` (CLI-only).

- [ ] **Step 9: Run tests.**

Run: `~/.cargo/bin/cargo nextest run --package warp toolbar_item::`
Expected: PASS (new + existing).

- [ ] **Step 10: format + clippy + commit.**

```bash
./script/format
~/.cargo/bin/cargo clippy --package warp --tests -- -D warnings
git add app/src/ai/blocklist/agent_view/agent_input_footer/ app/src/chip_configurator/mod.rs
git commit -m "feat(cli-agent): add CustomInsert toolbar item + insert-and-send action"
```

---

### Task 4: Persist a new custom button (Default → Custom)

**Files:**
- Modify: `app/src/ai/blocklist/agent_view/agent_input_footer/editor.rs` (or a small helper near `save_toolbar_selection`)
- Test: same file's tests, or `toolbar_item_tests.rs`

**Interfaces:**
- Consumes: `CLIAgentToolbarChipSelection` (`session_settings.rs:246`), `SessionSettings`, `save_toolbar_selection`/`set_value` (`editor.rs:261-294`), `AgentToolbarItemKind::cli_default_left/right`.
- Produces: `pub fn append_cli_custom_button(label: String, text: String, ctx: &mut impl ...) ` — a function that materializes `Default → Custom` and appends `CustomInsert { label, text }` to the CLI footer's left list, then persists.

- [ ] **Step 1: Write the failing test** (pure logic — build the next selection without touching global settings). Add a pure helper `next_selection_with_custom_button(current: CLIAgentToolbarChipSelection, label, text) -> CLIAgentToolbarChipSelection` and test it:

```rust
#[test]
fn appends_custom_button_and_materializes_default() {
    let next = next_selection_with_custom_button(
        CLIAgentToolbarChipSelection::Default,
        "Ship".into(),
        "/deploy".into(),
    );
    let CLIAgentToolbarChipSelection::Custom { left, .. } = next else {
        panic!("expected Custom");
    };
    // Default left items are materialized, then the new button is appended last.
    assert_eq!(
        left.last(),
        Some(&AgentToolbarItemKind::CustomInsert { label: "Ship".into(), text: "/deploy".into() })
    );
    assert!(left.contains(&AgentToolbarItemKind::ForkSession)); // materialized default
}
```

- [ ] **Step 2: Run it to verify it fails.**

Run: `~/.cargo/bin/cargo test --package warp next_selection_with_custom_button --no-run`
Expected: FAIL (fn not found).

- [ ] **Step 3: Implement the pure helper + the persisting wrapper:**

```rust
pub fn next_selection_with_custom_button(
    current: CLIAgentToolbarChipSelection,
    label: String,
    text: String,
) -> CLIAgentToolbarChipSelection {
    let (mut left, right) = match current {
        CLIAgentToolbarChipSelection::Default => (
            AgentToolbarItemKind::cli_default_left(),
            AgentToolbarItemKind::cli_default_right(),
        ),
        CLIAgentToolbarChipSelection::Custom { left, right } => (left, right),
    };
    left.push(AgentToolbarItemKind::CustomInsert { label, text });
    CLIAgentToolbarChipSelection::Custom { left, right }
}
```
Then the persisting wrapper reads the current setting, computes `next_selection_with_custom_button`, and writes via the same `set_value` path `save_toolbar_selection` uses (`editor.rs:261-294`).

- [ ] **Step 4: Run tests.**

Run: `~/.cargo/bin/cargo nextest run --package warp next_selection_with_custom_button`
Expected: PASS.

- [ ] **Step 5: format + clippy + commit.**

```bash
./script/format
~/.cargo/bin/cargo clippy --package warp --tests -- -D warnings
git add app/src/ai/blocklist/agent_view/agent_input_footer/editor.rs
git commit -m "feat(cli-agent): persist quick-insert buttons into footer selection"
```

---

### Task 5: The "Create quick-insert button" modal

**Files:**
- Create: `app/src/ai/blocklist/agent_view/agent_input_footer/quick_insert_modal.rs`
- Modify: `app/src/workspace/view.rs` (build/store/open/render-stack wiring)
- Modify: `app/src/workspace/action.rs` (`OpenQuickInsertModal` workspace action)

**Interfaces:**
- Consumes: `Modal<T>` (`app/src/modal.rs:27,117`), `EditorView::single_line` (`crate::editor`), `SkillManager::as_ref(app).get_skills_for_working_directory(cwd, app)` (`app/src/ai/skills/skill_manager.rs:98`), `crate::ai::cli_commands::discover_commands` (Task 2), `next_selection_with_custom_button` + persist wrapper (Task 4), `render_skill_button` (`app/src/ai/skills/skill_utils.rs:155`), `group_skills_by_scope` (`app/src/workspace/view/skills_panel/grouping.rs:37`).
- Produces: `pub struct QuickInsertModal` (a WarpUI View), `QuickInsertModal::new(ctx)`, `QuickInsertModal::open(cwd: PathBuf, ctx)`, `Entity::Event = QuickInsertModalEvent { Save { label, text }, Cancel }`.

**Implementation guidance:** copy the structure of `app/src/auth/provider_keys_modal.rs` (`ProviderKeysModalView`) verbatim as the skeleton — it already demonstrates: a modal `View`, `Entity::Event`, `TypedActionView`, a `single_line` `EditorView` field read via `.buffer_text(ctx)`, an escape-to-cancel `FixedBinding`, and the workspace build/subscribe/focus/render-stack wiring (`view.rs:2277` / field `view.rs:1628`). Adapt it to:

- [ ] **Step 1:** Two `EditorView::single_line` fields: `text_input` (the insert text) and `label_input` (auto-derived, editable). When `text_input` changes and the label was not manually edited, set `label_input` to a derived label (first line of text, trimmed to ~24 chars).
- [ ] **Step 2:** A scrollable pick list built each render from `discover_commands(cwd, ctx)` + `get_skills_for_working_directory(cwd, ctx)`, grouped Home/Project. Render command rows as simple labeled buttons (`ActionButton` with the command `name` + optional `description` sublabel) and skill rows via `render_skill_button`. Clicking a command row sets `text_input` to `command.invocation`; clicking a skill row sets `text_input` to `/<skill.name>` (editable default).
- [ ] **Step 3:** A Save button: emits `QuickInsertModalEvent::Save { label, text }` using `label_input.buffer_text(ctx)` / `text_input.buffer_text(ctx)`; ignore when `text` is empty. Escape / dismiss emits `Cancel`.
- [ ] **Step 4:** Workspace wiring in `view.rs`: add field `quick_insert_modal: ViewHandle<QuickInsertModal>`, construct in the ctor, add `ChildView::new(&self.quick_insert_modal)` to the render stack next to the other modals (grep `agent_toolbar_editor_modal` for the exact stack site `view.rs:26545`), subscribe to its events: on `Save`, call the Task 4 persist wrapper with the active pane group cwd; on `Cancel`/`Save`, close + refocus. Add `open_quick_insert_modal(ctx)` that resolves the active pane group cwd (grep `set_working_directory` usage at `left_panel.rs:343`) and opens the modal.
- [ ] **Step 5:** Add `WorkspaceAction::OpenQuickInsertModal` (`action.rs`) dispatched to `open_quick_insert_modal`.

- [ ] **Step 6: Test (headless where possible).** A full modal View is hard to unit-test headlessly; cover the derivable logic instead: a pure `fn derive_label(text: &str) -> String` (first non-empty line, trimmed to 24 chars, fallback "Custom") in the modal module with a `mod_test.rs`:

```rust
#[test]
fn derive_label_trims_and_falls_back() {
    assert_eq!(derive_label("/review the code"), "/review the code");
    assert_eq!(derive_label(""), "Custom");
    assert_eq!(derive_label(&"x".repeat(40)), "x".repeat(24));
}
```
Run: `~/.cargo/bin/cargo nextest run --package warp derive_label`
Expected: PASS.

- [ ] **Step 7: format + clippy + commit.**

```bash
./script/format
~/.cargo/bin/cargo clippy --package warp --tests -- -D warnings
git add app/src/ai/blocklist/agent_view/agent_input_footer/quick_insert_modal.rs app/src/workspace/view.rs app/src/workspace/action.rs
git commit -m "feat(cli-agent): add create-quick-insert-button modal"
```

---

### Task 6: Entry points — footer "＋ Add" button + context-menu item

**Files:**
- Modify: `app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs` (always-render an Add button in CLI mode; new event to open the modal)
- Modify: `app/src/terminal/view/use_agent_footer/mod.rs` (re-emit the open event)
- Modify: `app/src/terminal/view.rs` (context-menu item + route open to `WorkspaceAction::OpenQuickInsertModal`)

**Interfaces:**
- Consumes: `WorkspaceAction::OpenQuickInsertModal` (Task 5), the footer→toolbar→terminal event chain used by `OpenAgentToolbarEditor` (routing precedent: `terminal/view.rs:24557`, `use_agent_footer/mod.rs`, `pane_group/mod.rs:537`).
- Produces: `AgentInputFooterEvent::OpenQuickInsertModal`, `UseAgentToolbarEvent::OpenQuickInsertModal`.

- [ ] **Step 1:** In `render_cli_mode_footer` (`mod.rs:1592` region, where the CLI brand icon is always rendered), when `FeatureFlag::CliAgentQuickInsertButtons.is_enabled()` and a CLI agent is active, append a fixed trailing `ActionButton` (icon `Icon::Plus`, tooltip "Add quick-insert button") whose click dispatches `AgentInputFooterAction::OpenQuickInsertModal`.
- [ ] **Step 2:** Add `AgentInputFooterAction::OpenQuickInsertModal` + handler emitting `AgentInputFooterEvent::OpenQuickInsertModal`; add that event variant (near `SubmitTextToCliAgent` `:2790`).
- [ ] **Step 3:** In `use_agent_footer/mod.rs`, re-emit → `UseAgentToolbarEvent::OpenQuickInsertModal`, and in `TerminalView`'s handler dispatch `WorkspaceAction::OpenQuickInsertModal` (mirror the `OpenAgentToolbarEditor` chain exactly).
- [ ] **Step 4:** In `terminal/view.rs` context menu (`:17311`, next to `EditCLIAgentToolbar`, gated by the flag), add "Add quick-insert button…" → same `OpenQuickInsertModal` route.
- [ ] **Step 5: Manual verification** (no headless test for the render stack): `cargo run` a debug build, start a CLI agent, click the ＋, create a button by (a) typing text, (b) picking a command, (c) picking a skill; confirm each appears and, on click, is sent to the agent; confirm the context-menu entry opens the same modal.
- [ ] **Step 6: format + clippy + commit.**

```bash
./script/format
~/.cargo/bin/cargo clippy --package warp --tests -- -D warnings
git add app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs app/src/terminal/view/use_agent_footer/mod.rs app/src/terminal/view.rs
git commit -m "feat(cli-agent): add entry points to create quick-insert buttons"
```

---

### Task 7: Final verification

- [ ] **Step 1:** `./script/format` (clean).
- [ ] **Step 2:** `~/.cargo/bin/cargo clippy --package warp --tests -- -D warnings` (clean).
- [ ] **Step 3:** `~/.cargo/bin/cargo nextest run --package warp -E 'test(cli_commands) or test(toolbar_item) or test(next_selection) or test(derive_label)'` (all PASS).
- [ ] **Step 4:** Manual QA pass per Task 6 Step 5, plus: created buttons survive an app restart (persisted), and are removable/reorderable in the toolbar editor. Confirm the editor renders custom buttons with their labels (this exercises the `display_label` `Cow` change / Task 3 Step 5). If the editor shows them poorly, file the `all_available_for_cli_input` / editor-render caveat from the spec.
- [ ] **Step 5:** Update `MEMORY.md` with a one-line pointer to this feature per the project memory convention.

---

## Self-Review

**Spec coverage:** command discovery (Task 2) ✓; skills reuse (Task 5) ✓; `CustomInsert` data model + persistence (Tasks 3–4) ✓; insert-and-send action (Task 3) ✓; modal with text + label + pick list (Task 5) ✓; footer Add button + context-menu entry (Task 6) ✓; feature flag (Task 1) ✓; scope resolution (Tasks 2 & 5) ✓; exhaustive-match callouts (Task 3) ✓.

**Known open risk (from spec):** whether the drag-drop editor cleanly renders a persisted `CustomInsert` that is deliberately absent from `all_available_for_cli_input()` — verified in Task 7 Step 4; fallback is a minimal editor-render branch.

**Type consistency:** `CustomInsert { label, text }` field names are identical across Tasks 3–5; `display_label -> Cow<'static, str>` is defined in Task 3 and its call-site fixes are in the same task; `discover_commands`/`DiscoveredCommand` names match between Tasks 2 and 5; `next_selection_with_custom_button` is defined in Task 4 and consumed in Task 5; `OpenQuickInsertModal` action/event names match across Tasks 5–6.

## Post-launch cleanup (prevents permanent dead code)

Per CLAUDE.md flag guidance, once the feature has stabilized in release:
- Remove `FeatureFlag::CliAgentQuickInsertButtons` from the enum + `DOGFOOD_FLAGS`
  (Task 1) and delete every `CliAgentQuickInsertButtons.is_enabled()` guard
  (Tasks 3 Step 7, 6 Steps 1 & 4), keeping the now-unconditional branches.
- This is the only code the plan introduces that is designed to be removed
  later; nothing in the existing codebase becomes dead as a result of this plan.
