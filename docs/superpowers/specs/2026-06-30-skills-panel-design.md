# Skills Panel + Live Skill-Read Signal — Design

- **Date:** 2026-06-30
- **Branch (work):** TBD (isolate from `image-preview-pane`; see Build Notes)
- **Status:** Draft for review

## Summary

Add a first-party **Skills inspector** to Clinch that makes the otherwise-invisible
skill layer visible, in two complementary halves:

1. **Skills panel** — a toggleable entry in the existing left side panel that shows
   every skill on disk, grouped by **scope** (Home → Project → Bundled) and tagged by
   **provider** (Claude, Codex, Agents, Warp, …), with **All / Claude / Codex** subtabs
   that filter to *what that agent can actually read in the current directory*
   (access-based, not folder-literal).
2. **Live skill-read signal** — when an agent reads a skill mid-session, Clinch shows a
   brief **inline banner** in the terminal (`[Claude] read skill → brainstorming`) **and**
   elevates that skill into a **"Used this session"** group at the top of the panel.

The defining property: both halves read from the **same `SkillManager`** that already
indexes and live-watches skills, so the panel is always fresh and the live event merely
*annotates* entries the panel already knows about. This is overwhelmingly a new **view**
over existing, already-maintained data — not a new subsystem.

## Goals

- Give users transparency into which skills exist, where they come from (scope +
  provider), and **which agent can actually use each one here**.
- Surface, in the moment, when Claude/Codex pulls a skill into the active session.
- Reuse existing infrastructure (`SkillManager`, `SkillDescriptor`, the left-panel
  `ToolPanelView` host, the `cli_agent_sessions` OSC-777 event channel) rather than build
  parallel machinery.

## Non-goals

- Editing, creating, installing, or deleting skills from the panel (read-only inspector).
- Replacing the existing slash-command skill menu or AI context-menu skill list.
- Guaranteeing live-signal coverage for every CLI agent in v1 (Claude is the priority;
  Codex is best-effort — see Phasing/Risks).
- Any change to how skills are sent to or executed by an agent.

## Background — what already exists (and what we build on)

Investigation (2026-06-30) found the skill layer is already first-class; almost nothing
about the *data* is net-new.

| Concern | Already in the codebase |
|---|---|
| Skill discovery + live filesystem watching | `app/src/ai/skills/skill_manager.rs` (`SkillManager`) + `app/src/ai/skills/file_watchers/skill_watcher.rs` |
| Scope model (Home/Project/Bundled) | `crates/ai/src/skills/skill_provider.rs` (`SkillScope`) |
| Provider model + precedence + branded icons | `skill_provider.rs` (`SkillProvider`, `SKILL_PROVIDER_DEFINITIONS`, `icon()`, `icon_fill()` → Claude salmon, Codex OpenAI logo) |
| **Per-directory skill list (the panel's data source)** | `SkillManager::get_skills_for_working_directory(cwd, ctx) -> Vec<SkillDescriptor>` (`skill_manager.rs:98`) — already powers the slash menu (`terminal/input/skills/data_source.rs`), AI context menu (`search/ai_context_menu/skills/data_source.rs`), and zero-state |
| UI descriptor type | `app/src/ai/skills/listed_skill.rs` (`SkillDescriptor`: reference, name, description, scope, provider, icon_override) — **live, widely used; reuse, do not duplicate** |
| Per-agent access map | `app/src/terminal/cli_agent.rs:244` (`CLIAgent::supported_skill_providers()`) |
| Left-panel host with activity-bar icon strip | `app/src/workspace/view/left_panel.rs` (`LeftPanelView`, `ToolPanelView` enum) |
| Live CLI-agent event channel (OSC 777) | `app/src/terminal/cli_agent_sessions/` — `CLIAgentEventType { SessionStart, PromptSubmit, ToolComplete, Stop, PermissionRequest, … }` with payload `tool_name` + `tool_input_preview` |
| Inline banner render surface | block-level `InlineBanner` / `RichContent` blocks (`app/src/terminal/block_list_viewport.rs`) |

**Net-new is therefore:** one `ToolPanelView` variant (the panel view), the grouping/subtab
presentation, the click-to-detail view, and the handler that turns a `tool_name == "Skill"`
event into a banner + a session-scoped "used" set.

## Design

### 1. Surface — a new left-panel tool view

Add `ToolPanelView::Skills` to the existing `LeftPanelView`
(`app/src/workspace/view/left_panel.rs`), mirroring `ConversationListView` (the simplest
self-contained, directly-owned `ViewHandle` template). This inherits — for free — the
activity-bar icon strip, open/close state, drag-to-resize, and SQLite persistence.

- **Icon:** `Icon::Stars` (preferred) or `Icon::Lightning` from `crates/warp_core/src/ui/icons.rs`.
- **Toggle:** new `WorkspaceAction::ToggleSkillsPanel` + a binding-name constant +
  `EditableBinding`, gated by a settings/flag context predicate so the icon only appears
  when enabled.
- The activity-bar strip only renders when ≥2 tool views are active, which is already the
  case in normal configs; the Skills icon joins the existing row.

### 2. Data source — reuse `get_skills_for_working_directory`

The panel owns **no new model**. On render (and on watcher events) it calls
`SkillManager::get_skills_for_working_directory(active_cwd, ctx) -> Vec<SkillDescriptor>`
for the active session's working directory, then groups/filters for display. It subscribes
to `SkillManagerEvent` (watcher-driven) to refresh automatically when skills change on disk.

**Active working directory tracking (must handle):** the left panel is workspace-level,
but `get_skills_for_working_directory` is cwd-relative (project + ancestor skills depend on
which directory the focused session is in). The panel must therefore resolve the *active
tab's active pane cwd* and refresh when the active tab/pane or its cwd changes. The file
tree already solves exactly this via `working_directories_model`
(`left_panel.rs` `get_or_create_file_tree_view_for_pane_group`); the Skills panel reads the
same source for the active session's cwd and re-queries on change. Home and Bundled skills
are cwd-independent and always present; only Project-scope groups vary by cwd.

`SkillDescriptor` already carries everything a row needs: `name`, `description`, `scope`,
`provider`, `icon_override`, and a `reference` (`SkillReference::Path | BundledSkillId`).
For the detail view's full body/path, resolve `reference → ParsedSkill` via
`SkillManager::skill_by_reference` / `active_skill_by_reference` (gives `content`, `path`,
`line_range`).

### 3. Grouping and the All / Claude / Codex subtabs

- **All** subtab: show the deduplicated descriptor list grouped by `SkillScope`
  (Home → Project → Bundled). Dedup-by-precedence (the function's default) is correct here —
  we don't want to show the same skill twice.
- **Claude / Codex** subtabs: **access-based** — show what that agent can actually read,
  computed from `CLIAgent::<agent>.supported_skill_providers()`. Each inherited skill is
  tagged with its source provider (e.g. under the Codex tab, an `.agents` skill is shown
  tagged `agents (inherited)`).

  **Correctness note (must handle):** `get_skills_for_working_directory` deduplicates
  identical skills across providers in the same directory, keeping the *highest-precedence*
  provider's copy. So a skill present in both `.agents/skills` and `.claude/skills` survives
  tagged `agents`. A naive "filter the All list where `provider == Claude`" would then
  *under-count* Claude's access (it can read the `.claude` copy). The per-agent filter must
  therefore test **reachability** — include a skill if *any* on-disk copy lives under a
  provider in the agent's supported set — not just the surviving descriptor's tag.
  Implementation options to resolve during planning:
  (a) compute per-agent lists from the pre-dedup provider set, or
  (b) extend `SkillManager` with a helper that answers "is this skill reachable by provider
  set P in dir D" (the building blocks `skill_exists_for_any_provider` /
  `best_supported_provider` already exist at `skill_manager.rs:272,297`).
  Preference: (b) — keep the reachability logic in `SkillManager`, next to the existing
  helpers, so the panel stays a thin view.

- Subtab set is derived from agents Clinch knows about; for v1 we surface **All, Claude,
  Codex** explicitly (the two the user named). The design leaves room to add more
  (`Gemini`, `Agents`, …) later without structural change, since they're all `CLIAgent`
  variants with `supported_skill_providers()`.

### 4. Skill row + detail view

- **Row:** provider icon (via `SkillProvider::icon()`/`icon_fill()` or `icon_override`) +
  skill name + a small provider/scope tag. Hover + click states mirror the file tree's
  `render_item_with_hover` (`app/src/code/file_tree/view.rs`). Scope groups are
  collapsible (chevron), expansion state held in the panel view.
- **Click → detail:** a lightweight in-panel detail (or a slide-over within the panel)
  showing: description, full path, scope, provider, **"Available to: Claude, Codex"**
  (computed by iterating `CLIAgent` variants and keeping those whose
  `supported_skill_providers()` contains the skill's provider — same reachability rule as
  §3, kept consistent with the subtab filter), and an **Open SKILL.md** affordance that
  opens the file via the existing file-open path.
  *Open question for review:* in-panel detail vs. open-raw-file-directly (see Open
  Questions).

### 5. Live skill-read signal

Reuse the existing `cli_agent_sessions` OSC-777 pipeline; do **not** add a new transport.

- A skill read in Claude Code is a `Skill` **tool call**, so it arrives today as a
  `ToolComplete` (and/or `PermissionRequest`) event whose payload carries
  `tool_name == "Skill"` and `tool_input_preview` = the skill name/argument.
- Add handling in `app/src/terminal/cli_agent_sessions/` for `tool_name == "Skill"`:
  1. **Terminal banner** — emit an inline banner block: `[<Agent>] read skill → <name>`,
     using the agent's branded icon/color (`CLIAgent::icon()` / Claude salmon).
  2. **Panel tie-in** — record `(agent, skill identity, timestamp)` in a **session-scoped
     "used skills" set** (keyed to the CLI-agent session / pane). The panel renders a
     **"Used this session"** group at the top, resolving each entry back to the
     `SkillDescriptor` it already shows (match by name + provider/path).
- The "used" set lives with the session model (it is session state, not persisted across
  restarts in v1).

### 6. Empty / degraded states

- No skills for a provider in this dir → `"No skills <Agent> can read in this directory."`
- Live channel silent (e.g. the Codex plugin doesn't emit skill tool events, or the
  marketplace plugin isn't installed) → the panel still works as a **static browser**; the
  "Used this session" group simply stays empty and no banner appears. The feature degrades
  to "still useful" with zero errors. Note `CLIAgentEventSource::CodexOsc9Fallback` exists —
  Codex has a weaker native OSC-9 path that may not include tool detail; this is why Codex
  live coverage is best-effort.

## Architecture — concrete touch points

Net-new files:
- `app/src/workspace/view/skills_panel/` (or `app/src/skills_panel.rs`) — the `SkillsPanel`
  view (state: active subtab, expansion state, cached descriptor list, "used this session"
  set handle), mirroring `ConversationListView`.

Edits to wire the panel into the left-panel host (`left_panel.rs`, all compile-forced):
- `ToolPanelView::Skills` enum variant; `LeftPanelAction::Skills`.
- `create_toolbelt_button_config` (icon/tooltip/action/keybinding arm).
- `render` content-area match; `on_focus` match; `update_button_active_states`;
  `handle_action_with_force_open`; mouse-state handle vec.
- Own a `ViewHandle<SkillsPanel>` on `LeftPanelView`; build it in `LeftPanelView::new`.

Workspace + actions:
- `WorkspaceAction::ToggleSkillsPanel` (`app/src/workspace/action.rs`) + handler near the
  other `Toggle*Panel` handlers in `app/src/workspace/view.rs` calling
  `toggle_left_panel_view(&LeftPanelAction::Skills, …)`.
- Binding-name constant + `EditableBinding` registration (`app/src/workspace/mod.rs`),
  context-predicate gated.
- `compute_left_panel_views` gate (`app/src/workspace/view.rs`) so the icon appears only
  when enabled.

Persistence (so the chosen sub-view restores):
- `LeftPanelDisplayedTab::Skills` (`app/src/app_state.rs`) + `From<ToolPanelView>` mapping +
  the restore match in `restore_left_panel_for_tab` (`app/src/workspace/view.rs`).
- SQLite layer slots in automatically (variant is `Serialize/Deserialize`; older snapshots
  still deserialize).

Live signal:
- `app/src/terminal/cli_agent_sessions/` — handle `tool_name == "Skill"` in the event
  apply path; expose a session-scoped "used skills" set + an event the panel subscribes to.
- Banner emission via the existing inline-banner/block surface.

Data helper (correctness note in §3):
- Add a reachability helper to `SkillManager` for per-agent (provider-set) filtering.

## Feature flag

Gate everything behind a new flag, following the repo's two-layer pattern (the
`add-feature-flag` skill automates this):
- Runtime: `FeatureFlag::SkillsPanel` (`crates/warp_features/src/lib.rs`).
- Compile bridge: `#[cfg(feature = "skills_panel")]` registration in
  `app/src/features.rs`.
- Cargo: `skills_panel = []` in `app/Cargo.toml` (and integration crate's default set so
  tests compile the path; runtime flag still defaults off).

## Phasing

- **Phase 1 — Skills panel (static browser).** Fully self-contained in this repo. The
  panel, subtabs (incl. the reachability-correct per-agent filter), grouping, row +
  detail, feature flag, persistence, tests. Ships value alone.
- **Phase 2 — Live signal off existing events.** Handle `tool_name == "Skill"` →
  banner + "Used this session". Degrades gracefully if no events arrive. Still 100% in
  this repo (assuming the events already flow, which investigation indicates they do).
- **Phase 3 — Fidelity (DEFERRED, external).** *Only if* Phase 2's data proves lossy:
  add a dedicated `SkillRead` event (new versioned payload in
  `cli_agent_sessions/event/`) emitted by the external `warpdotdev/claude-code-warp`
  plugin (separate repo) via a Claude Code `PreToolUse`/`PostToolUse` hook on the `Skill`
  tool, and firm up Codex coverage. Documented now, built only on evidence. Risk is
  isolated and does not block Phases 1–2.

## Testing

- **Unit (Rust):** the per-agent reachability filter (the dedup/inheritance edge case
  from §3 — a skill in both `.agents` and `.claude` must appear under both All and the
  Claude subtab); scope grouping; "Available to" inversion. Mirror
  `app/src/ai/skills/skill_manager_tests.rs`.
- **Integration:** mirror `crates/integration/src/test/file_tree.rs` — enable
  `FeatureFlag::SkillsPanel`, write skills into `.agents/skills` and `.claude/skills`,
  toggle the panel, assert the tree groups/tags, switch subtabs, assert access-based
  filtering. For Phase 2, feed a synthetic `tool_name == "Skill"` event and assert the
  banner + "Used this session" entry appear.
- **Degraded path:** assert the panel renders with zero live events and with no
  marketplace plugin installed.

## Cleanup / dead-code review

- `SkillDescriptor` (`listed_skill.rs`) is **live and widely used** (slash commands,
  agent conversation, API conversion) — **reuse it; introduce no parallel row type.**
- Skill telemetry types (`SkillOpenOrigin`, `SkillTelemetryEvent`) exist; the panel should
  emit through them (add a `SkillOpenOrigin::SkillsPanel`-style origin) rather than add new
  telemetry plumbing.
- No vestigial skill types were found that this work makes dead. If, during Phase 2, the
  existing `ToolComplete` path fully covers skill reads, ensure Phase 3's `SkillRead` event
  is **not** added speculatively (avoid an unused event variant).

## Open questions (for review)

1. **Click behavior:** in-panel detail pane vs. directly opening `SKILL.md` in an editor.
   (Spec currently proposes in-panel detail + an explicit "Open SKILL.md".)
2. **Subtab set in v1:** just All/Claude/Codex, or also surface Agents/Gemini now? (Spec:
   All/Claude/Codex; structure allows more.)
3. **"Used this session" lifetime:** session-only (current proposal) vs. persisted with the
   left-panel snapshot.
4. **Banner placement:** a true block in the scrollback vs. a transient toast/footer line.
   (Spec proposes an inline banner block; confirm against existing CLI-agent footer UX so
   we don't double-signal with the existing agent-session status badges.)

## Build notes / risks

- **Working-tree isolation:** multiple Claude sessions share this checkout; do Phase work
  in a dedicated branch/worktree before building or branch ops.
- **Largest risk** is Phase 2's dependency on the marketplace plugin actually emitting a
  `Skill` tool event with a usable `tool_input_preview`. Mitigation: Phase 1 delivers value
  independently; Phase 2 degrades gracefully; Phase 3 is the explicit fallback.
- **Codex live coverage** is best-effort (OSC-9 fallback may lack tool detail); the static
  browser covers Codex fully regardless.
