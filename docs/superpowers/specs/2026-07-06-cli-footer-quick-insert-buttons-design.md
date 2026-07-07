# CLI-agent footer: custom quick-insert buttons

**Date:** 2026-07-06
**Status:** Design — approved, awaiting implementation plan
**Follows:** `2026-07-06-header-file-explorer-and-footer-cleanup-design.md`
(builds on the same `CLIAgentToolbarChipSelection` footer system)

## Goal

Let the user create their own footer buttons that insert a saved piece of text
into the active CLI-agent input. Clicking a button **inserts and sends** the
text (like the existing Continue/LGTM buttons). Buttons are created through a
popup that has a free-text field **and** a pick-list of the user's existing
**slash commands and skills** (user + project scope); picking a list item
pre-fills the editable text field.

### Confirmed product decisions

1. **Click behavior:** insert **and** send (auto-submit), reusing the per-agent
   Enter-handling used by Continue/LGTM.
2. **Popup scope (v1):** everything — free text, discovered slash commands, and
   discovered skills. (No phasing; slash-command discovery is built now.)
3. **Pick = pre-fill editable text:** selecting a command/skill row drops a
   sensible default into the text field (`/review` for a command; an editable
   reference for a skill). The user can edit before saving. This sidesteps the
   ambiguity of "how do you invoke a skill as literal text."
4. **Entry points:** a small always-present **"＋ Add" button** at the end of the
   footer that opens the popup, **and** an "Add quick-insert button…" entry in
   the footer's right-click context menu (next to "Edit CLI agent toolbelt").

### Feature flag

Gate the whole feature behind a new `FeatureFlag::CliAgentQuickInsertButtons`
(`crates/warp_features/src/lib.rs` + `app/src/features.rs`), default-on for
dogfood (`DOGFOOD_FLAGS`). It gates: the Add button, the modal, `CustomInsert`
rendering, command discovery, and the context-menu entry. This follows the
project's flag guidance for new UI and keeps the feature toggleable/cleanable.

## Architecture

Five units, four reusing existing machinery and one net-new (command discovery).

### 1. Slash-command discovery (NET-NEW)

The app has **no** knowledge of CLI-agent slash commands today (verified: no code
reads `.claude/commands`). Add a lightweight, on-demand scanner — no file
watchers or caching (the popup opens on demand, so a synchronous scan at
open-time is sufficient; this is deliberately simpler than the skills
subsystem's live watchers).

**Location:** new module `app/src/ai/cli_commands/`, app-level (parallel to
`app/src/ai/skills/`).

**Data model:**
```rust
pub struct DiscoveredCommand {
    pub name: String,          // display, e.g. "/review" or "/frontend:component"
    pub invocation: String,    // text inserted when picked, e.g. "/review"
    pub description: Option<String>,
    pub scope: CommandScope,   // Home | Project
    pub provider: CommandProvider, // Claude | Codex
    pub path: PathBuf,
}
```

**Provider table** (mirrors `SKILL_PROVIDER_DEFINITIONS`), extensible:
- **Claude Code** — user `~/.claude/commands`, project `<repo-root>/.claude/commands`.
  Recursively scan `*.md`; command name = path relative to the commands dir, minus
  `.md`, with directory separators joined by `:` and a leading `/`
  (`.../frontend/component.md` → `/frontend:component`), matching Claude Code's
  namespacing. `description` from optional YAML front-matter, else `None`.
- **Codex** — user `~/.codex/prompts/*.md`, name `/<filename>`. Project scope for
  Codex is left out of v1 (convention not established); the table makes it a
  one-line addition later.

**Reuse:** the existing YAML front-matter parser in
`crates/ai/src/skills/parser.rs` for the `description` field; `dirs::home_dir()`
for home; `repo_metadata::repositories::DetectedRepositories::as_ref(ctx)
.get_root_for_path(cwd)` for the project root (same helper the skills subsystem
uses).

**Entry point:** `fn discover_commands(cwd: &Path, ctx: &AppContext) -> Vec<DiscoveredCommand>`.

### 2. Skills (REUSE — no new code)

Call `SkillManager::as_ref(app).get_skills_for_working_directory(cwd, app)` →
`Vec<SkillDescriptor { name, description, scope, provider, reference, .. }>`
(`app/src/ai/skills/skill_manager.rs:98`, `listed_skill.rs:5`). Already
enumerates home + project scope, live and deduped. When a skill row is picked,
pre-fill the text field with an editable default (proposed: `/<skill-name>`,
which the user edits to whatever their agent expects).

### 3. Data model + persistence (REUSE the footer setting)

Add a data-carrying variant to `AgentToolbarItemKind`
(`app/src/ai/blocklist/agent_view/agent_input_footer/toolbar_item.rs`):
```rust
CustomInsert { label: String, text: String },
```
The enum already derives `Serialize, Deserialize, JsonSchema, SettingsValue`
and has a struct-variant precedent (`ContextChipKind::Custom { title }`), so this
persists cleanly. Stored in `CLIAgentToolbarChipSelection::Custom { left, right }`
(`app/src/terminal/session_settings.rs:246`) via the existing
`save_toolbar_selection(...).set_value(selection, ctx)` path
(`.../agent_input_footer/editor.rs:261-294`).

**Creation flow:** appending a button reads the current selection; if it is
`Default`, it converts to `Custom` snapshotting the current (already-trimmed)
default left + right, then appends the new `CustomInsert` to the left list — the
same `Default → Custom` conversion the toolbar editor already performs.

**Exhaustive matches to update** for the new variant (`toolbar_item.rs`):
`available_in()` → `CLIAgentOnly`; `available_to_session_viewer()` → host-only
(a shared-session viewer must not drive the host, mirror the quick-reply
buttons); `display_label()` → the button's `label`; `icon()` → a fixed glyph
(e.g. `Icon::Play` or a snippet/bolt icon) or `None`; `is_available_during_handoff_compose()`;
`is_available()`; and include it in `all_available_for_cli_input()` only if it
should appear in the editor's generic "Available" bank (it should NOT — custom
buttons are user-created, not template items — so leave it out of that list, and
confirm the editor tolerates rendering persisted custom items it doesn't offer).

### 4. Button render + insert-and-send action

`AgentToolbarItemKind::CustomInsert` cannot be a pre-built singleton (label/text
vary per item), so render it dynamically in the match arm of
`render_cli_toolbar_item` (`.../agent_input_footer/mod.rs:1470`), building an
`ActionButton` from the item's `label`, click →
`AgentInputFooterAction::InsertCustomText(text.clone())`. The agent-view render
twin (`mod.rs:2160`) gets a `CustomInsert { .. } => None` arm (CLI-only).

New action `AgentInputFooterAction::InsertCustomText(String)` (mirrors
`InsertFilePath(String)`), handled to emit
`AgentInputFooterEvent::SubmitTextToCliAgent(text)` (`mod.rs:2790`) — the
insert-and-send path that flows to `TerminalView::submit_text_to_cli_agent_pty`
(`app/src/terminal/view/use_agent_footer/mod.rs:727`) and applies the per-agent
`RichInputSubmitStrategy` (correct Enter handling per Claude/Codex/etc.).

### 5. The "Create button" modal

Reuse the `Modal<T>` shell (`app/src/modal.rs:27,117`) + `EditorView::single_line`
(`crate::editor`), following the `ProviderKeysModalView` /
`PasteAuthTokenModalView` template (`app/src/auth/`). Contents:
- **Text-to-insert** field (`EditorView::single_line`), read via `.buffer_text(ctx)`.
- **Label** field, auto-derived from the text (e.g. the command name, or the
  text truncated) and editable.
- **Pick list**: discovered commands + skills, grouped by scope (Home / Project),
  reusing the Skills panel's `group_skills_by_scope` ordering and
  `render_skill_button` (`app/src/ai/skills/skill_utils.rs:155`) for skill rows,
  with analogous rows for commands. Clicking a row pre-fills the text (and the
  derived label). Include a small search/filter if the list is long (optional).
- **Save**: append the `CustomInsert` via the persistence path (unit 3) and close.

**Workspace wiring** (mirror an existing modal): add a
`ViewHandle<QuickInsertModal>` field on `WorkspaceView`, build it in the
constructor, add it to the render stack, and an `open_quick_insert_modal(cwd,
ctx)` method that seeds the modal with the current working directory (for scope
resolution) and focuses it. Escape-to-cancel via a `FixedBinding`.

### Entry points

- **Footer "＋ Add" button:** an always-rendered control (not a configurable
  chip), added alongside the always-on CLI brand icon in `render_cli_mode_footer`
  (`mod.rs:1592` region), shown only when a CLI-agent session is active and the
  feature flag is on. Distinct icon from the removed attach `+` (proposed:
  `Icon::Plus` is acceptable here since the attach `+` is gone from the default
  layout, or a snippet/command glyph to differentiate). Click → dispatches an
  action that opens the modal (routes through the footer → `UseAgentToolbar` →
  `TerminalView` → `WorkspaceAction::OpenQuickInsertModal`, mirroring how
  `OpenAgentToolbarEditor` is routed from the footer).
- **Context-menu entry:** add "Add quick-insert button…" to the footer's
  right-click menu next to "Edit CLI agent toolbelt"
  (`app/src/terminal/view.rs:17311`, `ContextMenuAction::EditCLIAgentToolbar` →
  add a sibling `AddQuickInsertButton`).

## Scope resolution

Working directory from the active pane group's most-recent directory (as the
Skills panel does, `app/src/workspace/view/left_panel.rs:343`); home via
`dirs::home_dir()`; project root via `DetectedRepositories::get_root_for_path`.
Both command discovery and `SkillManager` take this cwd.

## Testing

- **Command discovery:** unit tests over a temp dir tree — Claude namespacing
  (`frontend/component.md` → `/frontend:component`), front-matter `description`
  parsing, user vs project scope classification, empty/missing dirs, non-`.md`
  files ignored.
- **Persistence / model:** `CustomInsert { label, text }` round-trips through
  serde/`SettingsValue`; appending one converts `Default → Custom` and preserves
  existing items; `available_in()` is `CLIAgentOnly` and hidden from session
  viewers.
- **Render:** a `CustomInsert` item renders a button whose click produces
  `SubmitTextToCliAgent(text)` (assert on the emitted event/action, following
  the existing quick-reply button tests in `toolbar_item_tests.rs`).
- **Manual:** create a button via the popup (typed text, a picked command, a
  picked skill), confirm it appears, click sends to the agent, survives restart,
  and is removable/reorderable via the toolbar editor.

## Exhaustive-match callouts (per CLAUDE.md)

Adding `AgentToolbarItemKind::CustomInsert` will break every non-wildcard match
on the enum: `available_in`, `available_to_session_viewer`, `display_label`,
`icon`, `is_available_during_handoff_compose`, `is_available` (all in
`toolbar_item.rs`), and the two render dispatches (`render_cli_toolbar_item` and
its agent-view twin in `mod.rs`). The compiler enumerates any others.

## No dead code

Purely additive. The only removal-adjacent note: `CustomInsert` is intentionally
kept OUT of `all_available_for_cli_input()` (it is user-generated, not a template
item), so confirm the editor's render path tolerates a persisted item that is not
in its "Available" bank; if it does not, add a minimal handling branch rather
than forcing it into the bank.

## Out of scope (possible follow-ups)

- Codex project-scope commands and other providers (table makes these trivial adds).
- Editing an existing custom button (v1: remove + re-add). A later "edit" action
  can reopen the modal pre-seeded from the `CustomInsert`.
- Live re-scan while the popup is open (v1 scans once on open).
- Arguments/placeholders in inserted text (e.g. `$ARGUMENTS`) — v1 inserts the
  literal saved text.
