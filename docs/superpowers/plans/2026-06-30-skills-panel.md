# Skills Panel (Phase 1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a read-only "Skills" tool view to Clinch's left panel that lists every on-disk skill grouped by scope (Home/Project/Bundled) and tagged by provider, with access-based All/Claude/Codex subtabs.

**Architecture:** A new `ToolPanelView::Skills` variant inside the existing `LeftPanelView` host (mirroring `ConversationListView`/`WarpDrive`), backed entirely by the existing `SkillManager` (which already discovers, indexes, and live-watches skills). The panel adds no new data model — it calls `SkillManager` for the active working directory, groups the returned `SkillDescriptor`s, and renders rows with the existing `render_skill_button` helper. Per-agent subtabs use a new reachability-correct `SkillManager` method so a skill installed under multiple providers isn't under-counted.

**Tech Stack:** Rust; Warp's custom `warpui`/`warpui_core` immediate-mode UI framework; existing crates `ai::skills` (`SkillManager`, `SkillDescriptor`, `SkillProvider`, `SkillScope`), `warp_features` (`FeatureFlag`), `app/src/terminal/cli_agent.rs` (`CLIAgent::supported_skill_providers`).

## Global Constraints

- **Feature-flag gated.** Everything ships behind `FeatureFlag::SkillsPanel` (runtime) + cargo feature `skills_panel` (compile). Runtime flag defaults **off**.
- **Reuse `SkillDescriptor`** (`app/src/ai/skills/listed_skill.rs`) — it is live, widely-used code. Do **not** introduce a parallel skill/row type.
- **Reuse `render_skill_button`** (`app/src/ai/skills/skill_utils.rs`) for skill rows — keep visual parity with the slash menu.
- **Read-only.** No create/edit/delete of skills from the panel.
- **Per-agent subtab correctness:** "Claude"/"Codex" show what the agent can actually *read* (reachability across the agent's `supported_skill_providers()`), not a literal `provider ==` filter on the deduplicated list.
- **Work in the isolated worktree** `.claude/worktrees/skills-panel` (branch `skills-panel`, off `master`). Do not run builds/branch ops in the shared main checkout.
- **Provider branding** comes from existing `SkillProvider::icon()`/`icon_fill()` and `SkillDescriptor::icon_override`; do not hardcode colors.
- **Build/test command** for this crate: `cargo build -p warp --features skills_panel` and `cargo test -p warp --features skills_panel`. Run from the worktree root.
- **Every commit** in this plan must include the repo's standard trailers:
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` and the
  `Claude-Session:` line. The `-m` messages shown in each task are the subject line only.
- **No speculative dead code:** the `collect_in_scope_skill_paths` refactor (Task 2) must be
  called by *both* `get_skills_for_working_directory_with_origin` and `skills_for_providers`
  (no orphaned helper). Do **not** add the Phase 3 `SkillRead` event or a `SkillOpenOrigin`
  variant until Phase 2 needs them. `SkillsPanelAction` is intentionally an empty enum (the
  `TypedActionView` trait requires an `Action` type; the panel has no keyboard actions in
  Phase 1).

---

## File map

**Create:**
- `app/src/workspace/view/skills_panel/mod.rs` — the `SkillsPanel` view (subtab state, grouping, render). One responsibility: present skills for the active working directory.
- `app/src/workspace/view/skills_panel/grouping.rs` — pure, unit-tested helpers: `SkillsSubtab`, `group_skills_by_scope`, `providers_for_subtab`.

**Modify (wiring — the compiler's exhaustive matches force each site):**
- `crates/warp_features/src/lib.rs` — add `FeatureFlag::SkillsPanel`.
- `app/src/features.rs` — register the flag under `#[cfg(feature = "skills_panel")]`.
- `app/Cargo.toml` + `crates/integration/Cargo.toml` — declare the cargo feature.
- `app/src/ai/skills/skill_manager.rs` — add `skills_for_providers(...)` reachability method + a small `collect_in_scope_skill_paths` refactor.
- `app/src/workspace/view/left_panel.rs` — `ToolPanelView::Skills`, `LeftPanelAction::Skills`, toolbelt config, render arm, `on_focus` arm, `update_button_active_states` arm, `handle_action_with_force_open` arm, `MouseStateHandles.skills_button`, `mouse_state_handles` vec, struct field `skills_panel_view` + construction in `new`, `focus_active_view_on_entry` arm, imports.
- `app/src/workspace/view.rs` — `compute_left_panel_views` gate, `ToggleSkillsPanel`/`OpenSkillsPanel` handler arms, `restore_left_panel_for_tab` arm, binding-name constants.
- `app/src/workspace/action.rs` — `ToggleSkillsPanel`/`OpenSkillsPanel` variants + the dispatch-classification list.
- `app/src/workspace/mod.rs` — `EditableBinding` registration(s) + binding-name import.
- `app/src/settings_view/mod.rs` — `flags::SHOW_SKILLS_PANEL` context flag (mirror `SHOW_CONVERSATION_HISTORY`).
- `app/src/app_state.rs` — `LeftPanelDisplayedTab::Skills` + `From<ToolPanelView>` arm.
- `crates/integration/src/test/skills_panel.rs` (create) + registration — integration test.

---

## Task 1: Feature flag scaffolding

**Files:**
- Modify: `crates/warp_features/src/lib.rs` (near `ImagePreviewPane`, ~line 215)
- Modify: `app/src/features.rs` (near the `#[cfg(feature = "image_preview_pane")]` registration, ~line 123)
- Modify: `app/Cargo.toml` (features section, near `image_preview_pane = []`, ~line 760)
- Modify: `crates/integration/Cargo.toml` (near `image_preview_pane`, ~line 76)

**Interfaces:**
- Produces: `FeatureFlag::SkillsPanel` (runtime flag, checked via `.is_enabled()` / `.override_enabled(bool)` / `.set_enabled(bool)`).

- [ ] **Step 1: Add the runtime flag variant**

In `crates/warp_features/src/lib.rs`, add next to `ImagePreviewPane`:

```rust
    /// Enables the read-only Skills inspector in the left panel.
    SkillsPanel,
```

- [ ] **Step 2: Register the flag behind the cargo feature**

In `app/src/features.rs`, in `enabled_features()`, add next to the image-preview registration:

```rust
        #[cfg(feature = "skills_panel")]
        FeatureFlag::SkillsPanel,
```

- [ ] **Step 3: Declare the cargo features**

In `app/Cargo.toml` features section:

```toml
skills_panel = []
```

In `crates/integration/Cargo.toml` features section (and add to the integration `default` set, mirroring `image_preview_pane`):

```toml
skills_panel = ["warp/skills_panel"]
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo build -p warp --features skills_panel`
Expected: builds (flag unused for now → allow with `#[allow(dead_code)]` is unnecessary since enum variants don't warn).

- [ ] **Step 5: Commit**

```bash
git add crates/warp_features/src/lib.rs app/src/features.rs app/Cargo.toml crates/integration/Cargo.toml
git commit -m "feat(skills-panel): add SkillsPanel feature flag"
```

---

## Task 2: Reachability-correct per-agent skill lookup

The spec's core correctness requirement. `get_skills_for_working_directory` deduplicates identical skills across providers keeping the highest-precedence one, so filtering that result by `provider == Claude` under-counts skills Claude can actually read (e.g. a skill present in both `.agents/skills` and `.claude/skills`). This task adds a provider-set-aware lookup.

**Files:**
- Modify: `app/src/ai/skills/skill_manager.rs` (refactor `get_skills_for_working_directory_with_origin` ~lines 113-203; add new method after it)
- Test: `app/src/ai/skills/skill_manager_tests.rs` (mirror existing tests ~line 23+)

**Interfaces:**
- Consumes: existing private state `directory_skills`, `skills_by_path`; existing `SkillDeduplicator` (`skill_utils.rs`), `SKILL_PROVIDER_DEFINITIONS`/`provider_rank`.
- Produces:
  ```rust
  // On SkillManager:
  pub fn skills_for_providers(
      &self,
      working_directory: Option<&LocalOrRemotePath>,
      providers: &[SkillProvider],
      ctx: &AppContext,
  ) -> Vec<SkillDescriptor>
  ```
  Returns file-backed skills in scope for `working_directory` whose `provider` is in `providers`, deduplicated by the same rule as the All list but only among the allowed providers, with icon overrides applied. Bundled skills are included only when `providers` contains `SkillProvider::Warp`.

- [ ] **Step 1: Write the failing test**

In `app/src/ai/skills/skill_manager_tests.rs` (follow the setup pattern already used by `get_skills_for_working_directory_scopes_subdirectory_skills` — it constructs a `SkillManager`, writes `SKILL.md` files under provider dirs, and queries). Add:

```rust
#[test]
fn skills_for_providers_includes_reachable_cross_provider_skill() {
    // A skill named "foo" exists (identical content) under BOTH .agents/skills and
    // .claude/skills in the same project dir. The deduplicated All list keeps the
    // .agents copy (higher precedence). But Claude CAN read .claude/skills/foo, so the
    // Claude provider-set query must still surface "foo".
    let (manager, cwd, _tmp) = manager_with_project_skills(&[
        (".agents/skills/foo", "# foo\nshared body"),
        (".claude/skills/foo", "# foo\nshared body"),
    ]);

    with_app(|app| {
        // All: deduped -> single "foo", tagged agents (higher precedence).
        let all = manager.get_skills_for_working_directory(Some(&cwd), app);
        assert_eq!(all.iter().filter(|s| s.name == "foo").count(), 1);

        // Claude (supported providers = [Claude]) must still include "foo".
        let claude = manager.skills_for_providers(Some(&cwd), &[SkillProvider::Claude], app);
        assert!(
            claude.iter().any(|s| s.name == "foo"),
            "Claude should reach .claude/skills/foo even though All dedupes to the agents copy"
        );
    });
}
```

> Note: `manager_with_project_skills` / `with_app` are the harness helpers used by the existing tests in this file. If they have different names, reuse whatever the neighboring tests use — do not invent a new harness.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p warp --features skills_panel skills_for_providers_includes_reachable_cross_provider_skill`
Expected: FAIL — `no method named skills_for_providers`.

- [ ] **Step 3: Extract the scope-collection helper**

In `app/src/ai/skills/skill_manager.rs`, extract the in-scope path collection (currently inline in `get_skills_for_working_directory_with_origin`, the block that fills `skill_paths` from `directory_skills`, lines ~119-176) into a private method, and call it from the existing method so behavior is unchanged:

```rust
    /// Collects `(owning_dir, skill_path)` pairs for every file-backed skill in scope for
    /// `working_directory` (home skills + ancestor project skills, honoring cloud mode).
    fn collect_in_scope_skill_paths(
        &self,
        working_directory: Option<&LocalOrRemotePath>,
        path_origin: &SkillPathOrigin,
        ctx: &AppContext,
    ) -> Vec<(LocalOrRemotePath, LocalOrRemotePath)> {
        let mut skill_paths = Vec::new();
        let path_matches_location = |path: &LocalOrRemotePath| match (working_directory, path) {
            (Some(LocalOrRemotePath::Local(_)), LocalOrRemotePath::Local(_)) => true,
            (
                Some(LocalOrRemotePath::Remote(working_directory)),
                LocalOrRemotePath::Remote(path),
            ) => working_directory.host_id == path.host_id,
            (None, LocalOrRemotePath::Local(_)) => self.is_cloud_environment,
            (Some(LocalOrRemotePath::Local(_)), LocalOrRemotePath::Remote(_))
            | (Some(LocalOrRemotePath::Remote(_)), LocalOrRemotePath::Local(_))
            | (None, LocalOrRemotePath::Remote(_)) => false,
        };

        if let Some(home_dir) = self.home_directory_for_origin(path_origin) {
            if let Some(home_skill_paths) = self.directory_skills.get(&home_dir) {
                skill_paths.extend(
                    home_skill_paths.iter().cloned().map(|path| (home_dir.clone(), path)),
                );
            }
        }

        if self.is_cloud_environment {
            for (dir, dir_skill_paths) in &self.directory_skills {
                if self.is_home_directory(dir) || !path_matches_location(dir) {
                    continue;
                }
                for path in dir_skill_paths {
                    skill_paths.push((dir.clone(), path.clone()));
                }
            }
        } else if let Some(working_directory) = working_directory {
            let repo_root = repo_metadata::repositories::DetectedRepositories::as_ref(ctx)
                .get_root_for_path(working_directory);
            for (dir, dir_skill_paths) in &self.directory_skills {
                if self.is_home_directory(dir) {
                    continue;
                }
                if working_directory.starts_with(dir) {
                    if repo_root.as_ref().is_none_or(|root| dir.starts_with(root)) {
                        for path in dir_skill_paths {
                            skill_paths.push((dir.clone(), path.clone()));
                        }
                    }
                }
            }
        }
        skill_paths
    }
```

Then replace the collection block inside `get_skills_for_working_directory_with_origin` with:

```rust
        let mut deduplicator = SkillDeduplicator::default();
        let skill_paths = self.collect_in_scope_skill_paths(working_directory, path_origin, ctx);
```

(Leave the rest of that method — `deduplicator.extend_paths`, icon overrides, bundled append — unchanged.)

- [ ] **Step 4: Add the provider-set method**

Add after `get_skills_for_working_directory_with_origin`:

```rust
    /// Returns file-backed skills in scope for `working_directory` that belong to one of
    /// `providers`, deduplicated among those providers. Used by the Skills panel's per-agent
    /// subtabs so a skill reachable under any of an agent's supported providers is not
    /// under-counted by the cross-provider dedup used for the "All" list. Bundled skills are
    /// included only when `providers` contains [`SkillProvider::Warp`].
    pub fn skills_for_providers(
        &self,
        working_directory: Option<&LocalOrRemotePath>,
        providers: &[SkillProvider],
        ctx: &AppContext,
    ) -> Vec<SkillDescriptor> {
        let path_origin = match working_directory {
            Some(LocalOrRemotePath::Remote(path)) => SkillPathOrigin::Remote {
                host_id: path.host_id.clone(),
            },
            Some(LocalOrRemotePath::Local(_)) | None => SkillPathOrigin::Local,
        };

        let skill_paths: Vec<_> = self
            .collect_in_scope_skill_paths(working_directory, &path_origin, ctx)
            .into_iter()
            .filter(|(_dir, path)| {
                self.skills_by_path
                    .get(path)
                    .is_some_and(|skill| providers.contains(&skill.provider))
            })
            .collect();

        let mut deduplicator = SkillDeduplicator::default();
        deduplicator.extend_paths(&skill_paths, &self.skills_by_path);
        let mut skills = deduplicator.into_descriptors();

        for skill in &mut skills {
            if skill.icon_override.is_none() {
                skill.icon_override =
                    crate::ai::skills::skill_utils::icon_override_for_skill_name(&skill.name);
            }
        }

        if providers.contains(&SkillProvider::Warp) && FeatureFlag::BundledSkills.is_enabled() {
            skills.extend(self.bundled_skills.active_descriptors(&path_origin, ctx));
        }

        skills
    }
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test -p warp --features skills_panel skills_for_providers_includes_reachable_cross_provider_skill`
Expected: PASS. Also run the existing `skill_manager_tests` to confirm the refactor didn't change behavior: `cargo test -p warp --features skills_panel get_skills_for_working_directory`.

- [ ] **Step 6: Commit**

```bash
git add app/src/ai/skills/skill_manager.rs app/src/ai/skills/skill_manager_tests.rs
git commit -m "feat(skills-panel): add reachability-correct skills_for_providers lookup"
```

---

## Task 3: Pure grouping + subtab helpers

**Files:**
- Create: `app/src/workspace/view/skills_panel/grouping.rs`
- Test: same file (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `ai::skills::{SkillDescriptor, SkillScope, SkillProvider}`, `crate::terminal::CLIAgent`.
- Produces:
  ```rust
  pub enum SkillsSubtab { All, Claude, Codex }
  pub fn providers_for_subtab(subtab: SkillsSubtab) -> Option<&'static [SkillProvider]>; // None = All
  pub fn group_skills_by_scope(skills: Vec<SkillDescriptor>) -> Vec<(SkillScope, Vec<SkillDescriptor>)>;
  pub fn agents_that_can_read(provider: SkillProvider) -> Vec<CLIAgent>;
  ```

- [ ] **Step 1: Write the failing tests**

Create `app/src/workspace/view/skills_panel/grouping.rs`:

```rust
use ai::skills::{SkillDescriptor, SkillProvider, SkillScope};

use crate::terminal::CLIAgent;

/// Which agent's accessible skill set the panel is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkillsSubtab {
    All,
    Claude,
    Codex,
}

impl SkillsSubtab {
    pub fn label(self) -> &'static str {
        match self {
            SkillsSubtab::All => "All",
            SkillsSubtab::Claude => "Claude",
            SkillsSubtab::Codex => "Codex",
        }
    }
}

/// The provider set to query for a subtab. `None` means "All" (no provider filter — use
/// `get_skills_for_working_directory`). `Some(providers)` means query
/// `skills_for_providers(providers)`.
pub fn providers_for_subtab(subtab: SkillsSubtab) -> Option<&'static [SkillProvider]> {
    match subtab {
        SkillsSubtab::All => None,
        SkillsSubtab::Claude => Some(CLIAgent::Claude.supported_skill_providers()),
        SkillsSubtab::Codex => Some(CLIAgent::Codex.supported_skill_providers()),
    }
}

/// Ordered display grouping: Home, then Project, then Bundled. Empty groups are omitted.
/// Within a group, skills are sorted case-insensitively by name.
pub fn group_skills_by_scope(
    skills: Vec<SkillDescriptor>,
) -> Vec<(SkillScope, Vec<SkillDescriptor>)> {
    const ORDER: [SkillScope; 3] = [SkillScope::Home, SkillScope::Project, SkillScope::Bundled];
    let mut out: Vec<(SkillScope, Vec<SkillDescriptor>)> = Vec::new();
    for scope in ORDER {
        let mut group: Vec<SkillDescriptor> =
            skills.iter().filter(|s| s.scope == scope).cloned().collect();
        if group.is_empty() {
            continue;
        }
        group.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        out.push((scope, group));
    }
    out
}

/// Which known CLI agents can read a skill from `provider` (for the detail "Available to" line).
pub fn agents_that_can_read(provider: SkillProvider) -> Vec<CLIAgent> {
    [CLIAgent::Claude, CLIAgent::Codex]
        .into_iter()
        .filter(|agent| agent.supported_skill_providers().contains(&provider))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc(name: &str, scope: SkillScope, provider: SkillProvider) -> SkillDescriptor {
        SkillDescriptor {
            reference: ai::skills::SkillReference::BundledSkillId(name.to_string()),
            name: name.to_string(),
            description: String::new(),
            scope,
            provider,
            icon_override: None,
        }
    }

    #[test]
    fn groups_are_ordered_home_project_bundled_and_sorted() {
        let skills = vec![
            desc("zebra", SkillScope::Project, SkillProvider::Agents),
            desc("alpha", SkillScope::Project, SkillProvider::Agents),
            desc("home-one", SkillScope::Home, SkillProvider::Claude),
        ];
        let grouped = group_skills_by_scope(skills);
        assert_eq!(grouped[0].0, SkillScope::Home);
        assert_eq!(grouped[1].0, SkillScope::Project);
        assert_eq!(
            grouped[1].1.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            vec!["alpha", "zebra"]
        );
    }

    #[test]
    fn all_subtab_has_no_provider_filter() {
        assert!(providers_for_subtab(SkillsSubtab::All).is_none());
    }

    #[test]
    fn codex_reads_more_providers_than_claude() {
        let claude = providers_for_subtab(SkillsSubtab::Claude).unwrap();
        let codex = providers_for_subtab(SkillsSubtab::Codex).unwrap();
        assert!(claude.contains(&SkillProvider::Claude));
        assert!(codex.len() > claude.len());
    }

    #[test]
    fn agents_that_can_read_claude_provider_includes_both() {
        // Both Claude and Codex support the Claude provider.
        let agents = agents_that_can_read(SkillProvider::Claude);
        assert!(agents.contains(&CLIAgent::Claude));
        assert!(agents.contains(&CLIAgent::Codex));
    }
}
```

- [ ] **Step 2: Register the module and run tests**

Add to `app/src/workspace/view/skills_panel/mod.rs` (created in Task 4, but declare early so tests run): create a minimal `mod.rs` now containing just `mod grouping;` — or, if executing strictly in order, add `pub(crate) mod grouping;` to `app/src/workspace/view/mod.rs` temporarily. Simplest: create `app/src/workspace/view/skills_panel/mod.rs` with:

```rust
mod grouping;
pub(crate) use grouping::{
    agents_that_can_read, group_skills_by_scope, providers_for_subtab, SkillsSubtab,
};
```

and declare `pub(crate) mod skills_panel;` in `app/src/workspace/view/mod.rs` (near the other `mod` lines).

Run: `cargo test -p warp --features skills_panel skills_panel::grouping`
Expected: 4 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add app/src/workspace/view/skills_panel/ app/src/workspace/view/mod.rs
git commit -m "feat(skills-panel): pure scope-grouping and subtab helpers"
```

---

## Task 4: The `SkillsPanel` view

**Files:**
- Modify/Create: `app/src/workspace/view/skills_panel/mod.rs`

**Interfaces:**
- Consumes: `grouping::*` (Task 3); `SkillManager::get_skills_for_working_directory` / `skills_for_providers` (Task 2); `ai::skills::{render_skill_button, SkillDescriptor, SkillScope}`; `warpui` elements.
- Produces:
  ```rust
  pub struct SkillsPanel { /* see below */ }
  impl SkillsPanel { pub fn new(ctx: &mut ViewContext<Self>) -> Self }
  // Called by LeftPanelView (Task 5):
  impl SkillsPanel {
      pub fn set_working_directory(&mut self, cwd: Option<LocalOrRemotePath>, ctx: &mut ViewContext<Self>);
      pub fn on_left_panel_focused(&mut self, ctx: &mut ViewContext<Self>); // may be a no-op
  }
  pub enum SkillsPanelEvent { OpenSkillFile(LocalOrRemotePath) }
  ```

> **Data-refresh note:** `SkillManagerEvent` has only one variant, `HomeSkillsChanged`. Subscribe to it for home-skill edits; project skills are re-read on `set_working_directory` and on each render (cheap — `get_skills_for_working_directory` is a HashMap walk). Do **not** claim a single unified change event.

- [ ] **Step 1: Write the view struct and constructor**

Replace `app/src/workspace/view/skills_panel/mod.rs` contents (keeping the `mod grouping;` line) with the module below. This mirrors the minimal shape of `ConversationListView` but has no inner editors.

```rust
mod grouping;
pub(crate) use grouping::{
    agents_that_can_read, group_skills_by_scope, providers_for_subtab, SkillsSubtab,
};

use std::collections::{HashMap, HashSet};

use ai::skills::{render_skill_button, SkillDescriptor, SkillManager, SkillScope};
use warp_core::ui::Appearance;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::elements::{
    ChildAnchor, Container, CrossAxisAlignment, Element, Flex, MainAxisSize, MouseStateHandle,
    ParentElement, Scrollable, ScrollStateHandle, Shrinkable, Text,
};
use warpui::{AppContext, Entity, EntityId, TypedActionView, View, ViewContext, WindowId};

/// Read-only skills inspector shown as a left-panel tool view.
pub struct SkillsPanel {
    window_id: WindowId,
    view_id: EntityId,
    active_subtab: SkillsSubtab,
    /// Working directory of the active session; drives project-scope skills.
    working_directory: Option<LocalOrRemotePath>,
    collapsed_scopes: HashSet<SkillScope>,
    scroll_state: ScrollStateHandle,
    subtab_button_states: [MouseStateHandle; 3],
    /// Per-skill row mouse states, keyed by skill name (stable within a render pass).
    row_states: HashMap<String, MouseStateHandle>,
}

#[derive(Clone, Debug)]
pub enum SkillsPanelEvent {
    /// User clicked a skill row's "open" affordance; workspace opens the SKILL.md file.
    OpenSkillFile(LocalOrRemotePath),
}

/// Actions for keyboard handling. No keyboard nav in Phase 1 → empty.
#[derive(Clone, Debug)]
pub enum SkillsPanelAction {}

impl SkillsPanel {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        // Refresh when home skills change on disk.
        ctx.subscribe_to_model(&SkillManager::model_handle(ctx), |_me, _model, event, ctx| {
            match event {
                ai::skills::SkillManagerEvent::HomeSkillsChanged => ctx.notify(),
            }
        });
        Self {
            window_id: ctx.window_id(),
            view_id: ctx.view_id(),
            active_subtab: SkillsSubtab::All,
            working_directory: None,
            collapsed_scopes: HashSet::new(),
            scroll_state: ScrollStateHandle::default(),
            subtab_button_states: Default::default(),
            row_states: HashMap::new(),
        }
    }

    pub fn set_working_directory(
        &mut self,
        cwd: Option<LocalOrRemotePath>,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.working_directory != cwd {
            self.working_directory = cwd;
            ctx.notify();
        }
    }

    /// The SkillsPanel has no focusable inner widget in Phase 1.
    pub fn on_left_panel_focused(&mut self, _ctx: &mut ViewContext<Self>) {}

    fn set_subtab(&mut self, subtab: SkillsSubtab, ctx: &mut ViewContext<Self>) {
        if self.active_subtab != subtab {
            self.active_subtab = subtab;
            ctx.notify();
        }
    }

    /// Fetch + filter skills for the active subtab and working directory.
    fn current_skills(&self, app: &AppContext) -> Vec<SkillDescriptor> {
        let manager = SkillManager::as_ref(app);
        let cwd = self.working_directory.as_ref();
        match providers_for_subtab(self.active_subtab) {
            None => manager.get_skills_for_working_directory(cwd, app),
            Some(providers) => manager.skills_for_providers(cwd, providers, app),
        }
    }
}
```

> **`SkillManager::model_handle(ctx)`**: if the manager isn't reachable via such a helper, use the same access the slash-command data source uses — grep `SkillManager::as_ref` and how a view subscribes to it (e.g. `ctx.global_model::<SkillManager>()` or an app-global). Wire the subscription to match the codebase; if no view currently subscribes, it is acceptable in Phase 1 to skip the subscription and rely on re-render + `set_working_directory` (remove the `subscribe_to_model` block). Do not invent a handle API.

- [ ] **Step 2: Implement `render`, `Entity`, `TypedActionView`, `View`**

Append to the same file:

```rust
impl SkillsPanel {
    fn render_subtab_bar(&self, appearance: &Appearance) -> Box<dyn Element> {
        let mut row = Flex::row().with_spacing(4.0);
        for (i, subtab) in [SkillsSubtab::All, SkillsSubtab::Claude, SkillsSubtab::Codex]
            .into_iter()
            .enumerate()
        {
            let is_active = self.active_subtab == subtab;
            let state = self.subtab_button_states[i].clone();
            // A simple text pill; reuse an existing pill/toggle component if one exists
            // (grep ui_components for a segmented/tab control before hand-rolling).
            let label = Text::new_inline(
                subtab.label(),
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .finish();
            let pill = Container::new(label)
                .with_horizontal_padding(8.)
                .with_vertical_padding(4.)
                .with_mouse_state(state, move |ctx: &mut ViewContext<Self>| {
                    ctx.view().update(ctx, |me, ctx| me.set_subtab(subtab, ctx));
                })
                .finish();
            let _ = is_active; // active styling applied via appearance; see styling note
            row = row.with_child(pill);
        }
        Container::new(row.finish())
            .with_horizontal_padding(8.)
            .with_vertical_padding(6.)
            .finish()
    }

    fn render_group(
        &self,
        scope: SkillScope,
        skills: &[SkillDescriptor],
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let header = Text::new_inline(
            scope_header_label(scope),
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .finish();
        let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Start);
        column = column.with_child(
            Container::new(header)
                .with_horizontal_padding(8.)
                .with_vertical_padding(4.)
                .finish(),
        );
        if !self.collapsed_scopes.contains(&scope) {
            for skill in skills {
                let state = self
                    .row_states
                    .get(&skill.name)
                    .cloned()
                    .unwrap_or_default();
                let path = match &skill.reference {
                    ai::skills::SkillReference::Path(p) => Some(p.clone()),
                    ai::skills::SkillReference::BundledSkillId(_) => None,
                };
                let row = render_skill_button(
                    &skill.name,
                    state,
                    appearance,
                    skill.provider,
                    skill.icon_override,
                    move |ctx: &mut ViewContext<Self>| {
                        if let Some(path) = path.clone() {
                            ctx.emit(SkillsPanelEvent::OpenSkillFile(path));
                        }
                    },
                );
                column = column.with_child(row);
            }
        }
        column.finish()
    }
}

fn scope_header_label(scope: SkillScope) -> &'static str {
    match scope {
        SkillScope::Home => "Home",
        SkillScope::Project => "Project",
        SkillScope::Bundled => "Bundled",
    }
}

impl Entity for SkillsPanel {
    type Event = SkillsPanelEvent;
}

impl TypedActionView for SkillsPanel {
    type Action = SkillsPanelAction;
    fn handle_action(&mut self, _action: &Self::Action, _ctx: &mut ViewContext<Self>) {}
}

impl View for SkillsPanel {
    fn ui_name() -> &'static str {
        "SkillsPanel"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let skills = self.current_skills(app);
        let grouped = group_skills_by_scope(skills);

        let mut column = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Start);
        column = column.with_child(self.render_subtab_bar(appearance));

        if grouped.is_empty() {
            let empty_msg = match self.active_subtab {
                SkillsSubtab::All => "No skills found for this directory.".to_string(),
                other => format!("No skills {} can read in this directory.", other.label()),
            };
            column = column.with_child(
                Container::new(
                    Text::new_inline(&empty_msg, appearance.ui_font_family(), appearance.ui_font_size())
                        .with_color(theme.sub_text_color(theme.background()).into())
                        .finish(),
                )
                .with_horizontal_padding(12.)
                .with_vertical_padding(8.)
                .finish(),
            );
        } else {
            for (scope, group) in &grouped {
                column = column.with_child(self.render_group(*scope, group, appearance));
            }
        }

        let scrollable = Scrollable::vertical(self.scroll_state.clone(), column.finish()).finish();
        Shrinkable::new(1.0, scrollable).finish()
    }
}
```

> **Styling / element-API note:** `warpui` element method names (`with_mouse_state`, `Scrollable::vertical`, `with_child`, `Text::new_inline`) must match the exact framework API. Before implementing, open `app/src/workspace/view/conversation_list/view.rs` and copy the *exact* idioms it uses for: (a) a clickable container with a `MouseStateHandle`, (b) a vertical scroll region, (c) active/inactive pill styling. This task's code shows structure and intent; align the method calls with that reference file. Active-subtab styling: mirror how the toolbelt buttons show `render_with_active_state`.
>
> **`render_skill_button` closure bound (verify first):** its signature is
> `render_skill_button<F>(label, MouseStateHandle, &Appearance, SkillProvider, Option<Icon>, on_click: F) -> Box<dyn Element> where F: <bound>`. Before writing Task 4 Step 2, read `app/src/ai/skills/skill_utils.rs:155+` for the exact `where` bound on `F` (it may be `Fn(&mut ViewContext<V>)`, `FnMut`, or a boxed action callback) and match the `move |ctx| …` closure to it. If `render_skill_button` is generic over the view type `V`, confirm it accepts `SkillsPanel`; if it's hard-bound to another view, fall back to rendering rows with the same primitives it uses internally (copy its body) rather than forcing the call.

- [ ] **Step 3: Verify it compiles**

Run: `cargo build -p warp --features skills_panel`
Expected: compiles. Fix any `warpui` method-name mismatches against `conversation_list/view.rs` (see note).

- [ ] **Step 4: Commit**

```bash
git add app/src/workspace/view/skills_panel/mod.rs
git commit -m "feat(skills-panel): SkillsPanel view (subtabs, grouping, rows)"
```

---

## Task 5: Wire `SkillsPanel` into `LeftPanelView`

**Files:**
- Modify: `app/src/workspace/view/left_panel.rs`

**Interfaces:**
- Consumes: `SkillsPanel`, `SkillsPanelEvent` (Task 4).
- Produces: `ToolPanelView::Skills`, `LeftPanelAction::Skills`, and a `skills_panel_view: ViewHandle<SkillsPanel>` field on `LeftPanelView`.

> All arms below sit next to the existing `ConversationListView` arms shown in the extraction. The compiler's exhaustive matches will flag any site missed — add a `ToolPanelView::Skills`/`LeftPanelAction::Skills` arm at each.

- [ ] **Step 1: Import the view + event**

Near the existing `ConversationListView, Event as ConversationListViewEvent` import (~lines 52-53):

```rust
use super::skills_panel::{SkillsPanel, SkillsPanelEvent};
```

- [ ] **Step 2: Add enum variants**

`ToolPanelView` (lines 102-108) — add:

```rust
    Skills,
```

`LeftPanelAction` (lines 75-81) — add:

```rust
    Skills,
```

- [ ] **Step 3: MouseStateHandles field + toolbelt config**

In `MouseStateHandles` (lines 67-73) add `skills_button: MouseStateHandle,`.

In `create_toolbelt_button_config` add an arm (mirror the `ConversationListView` arm):

```rust
            ToolPanelView::Skills => {
                let tooltip_keybinding_names = vec![LEFT_PANEL_SKILLS_BINDING_NAME];
                ToolbeltButtonConfig {
                    icon: Icon::Stars,
                    active_icon: Some(Icon::Stars),
                    tooltip_text: "Skills".to_string(),
                    action: LeftPanelAction::Skills,
                    render_with_active_state: false,
                    tooltip_keybinding: toolbelt_tooltip_keybinding(&tooltip_keybinding_names, ctx),
                    tooltip_keybinding_names,
                }
            }
```

- [ ] **Step 4: Struct field + construction in `new`**

Add field to `LeftPanelView` struct (lines 169-182):

```rust
    skills_panel_view: ViewHandle<SkillsPanel>,
```

In `new` (after `conversation_list_view` is built, ~line 219):

```rust
        let skills_panel_view = ctx.add_typed_action_view(SkillsPanel::new);
        ctx.subscribe_to_view(&skills_panel_view, |_me, _, event, ctx| match event {
            SkillsPanelEvent::OpenSkillFile(path) => {
                ctx.emit(LeftPanelEvent::OpenSkillFile(path.clone()));
            }
        });
```

Add `skills_panel_view,` to the struct literal (lines 328-343).

> Add a `LeftPanelEvent::OpenSkillFile(LocalOrRemotePath)` variant to the `LeftPanelEvent` enum (grep `enum LeftPanelEvent` in this file). The workspace will handle it in Task 6.

- [ ] **Step 5: render arm, on_focus arm, update_button_active_states arm, handle_action arm, mouse_state vec, focus_active_view_on_entry arm**

`render` content match (add near ConversationListView arm ~line 1201):

```rust
            ToolPanelView::Skills => {
                Shrinkable::new(1.0, ChildView::new(&self.skills_panel_view).finish()).finish()
            }
```

`on_focus` match (~line 1130):

```rust
                ToolPanelView::Skills => ctx.focus(&self.skills_panel_view),
```

`update_button_active_states` (~line 890):

```rust
                LeftPanelAction::Skills => self.active_view.get() == ToolPanelView::Skills,
```

`handle_action_with_force_open` (~line 1030, mirror ConversationListView, no telemetry for Phase 1 or add a `TelemetryEvent` if one exists):

```rust
            LeftPanelAction::Skills => {
                active_view_state::set(self, ToolPanelView::Skills, ctx);
            }
```

`mouse_state_handles` vec in `render` (~line 1139) — append in the **same relative order** used by `compute_left_panel_views` (Task 6 pushes Skills last):

```rust
            self.mouse_state_handles.skills_button.clone(),
```

`focus_active_view_on_entry` (~line 721) — add, mirroring the WarpDrive arm:

```rust
            ToolPanelView::Skills => self.skills_panel_view.update(ctx, |v, ctx| v.on_left_panel_focused(ctx)),
```

- [ ] **Step 6: Propagate the active working directory**

Find `set_active_pane_group` in `left_panel.rs` (called from `restore_left_panel_for_tab` and on tab switch). After it updates the file-tree/conversation views, push the cwd into the skills panel. Resolve the cwd the same way the file tree does — grep how `working_directories_model` yields a directory for a pane group (e.g. `WorkingDirectoriesModel::…` accessor). Then:

```rust
        let cwd = /* active working directory for `pane_group` via working_directories_model */;
        self.skills_panel_view.update(ctx, |v, ctx| v.set_working_directory(cwd, ctx));
```

> If resolving the cwd here is non-trivial, the acceptable Phase-1 fallback is to resolve it lazily inside `SkillsPanel::render` from an app-global "foreground session working directory" if one exists (grep for how the slash-command data source obtains `cwd` — `slash_command_model.rs:255` passes a `cwd_path`; reuse that source). Pick whichever the codebase already exposes; do not invent an accessor.

- [ ] **Step 7: Verify it compiles**

Run: `cargo build -p warp --features skills_panel`
Expected: compiles once every exhaustive match has a `Skills` arm. The compiler error list is your checklist.

- [ ] **Step 8: Commit**

```bash
git add app/src/workspace/view/left_panel.rs
git commit -m "feat(skills-panel): wire SkillsPanel into LeftPanelView"
```

---

## Task 6: Workspace actions, bindings, gate, and file-open handling

**Files:**
- Modify: `app/src/workspace/action.rs`, `app/src/workspace/view.rs`, `app/src/workspace/mod.rs`, `app/src/settings_view/mod.rs`

**Interfaces:**
- Consumes: `LeftPanelAction::Skills`, `FeatureFlag::SkillsPanel`, `LeftPanelEvent::OpenSkillFile`.
- Produces: `WorkspaceAction::{ToggleSkillsPanel, OpenSkillsPanel}`, `flags::SHOW_SKILLS_PANEL`, `LEFT_PANEL_SKILLS_BINDING_NAME`, and a `ToolPanelView::Skills` entry in `compute_left_panel_views`.

- [ ] **Step 1: Add the actions**

`app/src/workspace/action.rs` — near `ToggleConversationListView`/`OpenConversationListView` (~line 677):

```rust
    ToggleSkillsPanel,
    OpenSkillsPanel,
```

Add them to the dispatch-classification `matches!` list (the one containing `| ToggleConversationListView | OpenConversationListView` ~line 1107) so they're classified the same way:

```rust
            | ToggleSkillsPanel | OpenSkillsPanel
```

> Verify what that match arm returns (it groups actions by a property, e.g. "affects left panel"). Place the new variants in the arm that matches the conversation-list actions.

- [ ] **Step 2: Add the context flag**

`app/src/settings_view/mod.rs` (~line 599, near `SHOW_CONVERSATION_HISTORY`) — declare `SHOW_SKILLS_PANEL` mirroring the existing flags, and set it enabled when `FeatureFlag::SkillsPanel.is_enabled()` (follow exactly how `SHOW_CONVERSATION_HISTORY` is toggled — grep `SHOW_CONVERSATION_HISTORY` to find both its declaration and where it's set).

- [ ] **Step 3: Binding-name constant + registration**

`app/src/workspace/view.rs` (~line 638, in the left-panel binding constants block):

```rust
pub(crate) const LEFT_PANEL_SKILLS_BINDING_NAME: &str = "workspace:left_panel_skills";
```

`app/src/workspace/mod.rs` — import the constant (line ~68-75 block) and register an `EditableBinding` (mirror the Warp Drive binding — no custom action needed):

```rust
        EditableBinding::new(
            LEFT_PANEL_SKILLS_BINDING_NAME,
            BindingDescription::new("Left Panel: Skills"),
            WorkspaceAction::ToggleSkillsPanel,
        )
        .with_group(bindings::BindingGroup::Navigation.as_str())
        .with_context_predicate(id!("Workspace") & id!(flags::SHOW_SKILLS_PANEL))
        .with_enabled(|| FeatureFlag::SkillsPanel.is_enabled()),
```

- [ ] **Step 4: Handler arms**

`app/src/workspace/view.rs` — near the `ToggleConversationListView` handler (~line 25229):

```rust
            ToggleSkillsPanel => {
                if FeatureFlag::SkillsPanel.is_enabled() {
                    let is_showing =
                        self.left_panel_view.as_ref(ctx).active_view() == ToolPanelView::Skills;
                    self.toggle_left_panel_view(&LeftPanelAction::Skills, is_showing, ctx);
                }
            }
            OpenSkillsPanel => {
                if FeatureFlag::SkillsPanel.is_enabled() {
                    self.open_left_panel_view(&LeftPanelAction::Skills, ctx);
                }
            }
```

- [ ] **Step 5: Gate in `compute_left_panel_views`**

`app/src/workspace/view.rs` `compute_left_panel_views` (~line 23037, before `views` is returned) — push Skills **last** (matches the mouse-state vec order in Task 5):

```rust
        if FeatureFlag::SkillsPanel.is_enabled() {
            views.push(ToolPanelView::Skills);
        }
```

- [ ] **Step 6: Handle the open-skill-file event**

Where the workspace subscribes to `LeftPanelEvent` (grep `handle_left_panel_event`), add an arm for `LeftPanelEvent::OpenSkillFile(path)` that opens the file using the existing file-open path (the same one the file tree uses — grep `open_file`/`FileTarget`). Opening a `SKILL.md` is a normal editor open:

```rust
            LeftPanelEvent::OpenSkillFile(path) => {
                if let LocalOrRemotePath::Local(local) = path {
                    let session = self.get_active_session(ctx);
                    self.open_file(local.clone(), session, EditorLayout::NewTab, ctx);
                }
            }
```

> Match the exact `open_file` signature the workspace already exposes (grep `fn open_file` in `view.rs`); adjust arguments accordingly. This is not net-new plumbing.

- [ ] **Step 7: Verify it compiles + manual smoke**

Run: `cargo build -p warp --features skills_panel`
Expected: compiles.

- [ ] **Step 8: Commit**

```bash
git add app/src/workspace/action.rs app/src/workspace/view.rs app/src/workspace/mod.rs app/src/settings_view/mod.rs
git commit -m "feat(skills-panel): workspace actions, binding, gate, file-open"
```

---

## Task 7: Persistence

**Files:**
- Modify: `app/src/app_state.rs`, `app/src/workspace/view.rs`

**Interfaces:**
- Consumes: `ToolPanelView::Skills`.
- Produces: `LeftPanelDisplayedTab::Skills` (persisted).

- [ ] **Step 1: Add the persisted variant + mapping**

`app/src/app_state.rs` (~line 311) — add to `LeftPanelDisplayedTab`:

```rust
    Skills,
```

And to `From<ToolPanelView>` (~line 319):

```rust
            ToolPanelView::Skills => LeftPanelDisplayedTab::Skills,
```

- [ ] **Step 2: Restore arm**

`app/src/workspace/view.rs` `restore_left_panel_for_tab` (~line 4093), add to the `match left_panel_snapshot.left_panel_displayed_tab`:

```rust
                LeftPanelDisplayedTab::Skills => ToolPanelView::Skills,
```

- [ ] **Step 3: Verify + confirm back-compat**

Run: `cargo build -p warp --features skills_panel`
Expected: compiles. `LeftPanelDisplayedTab` is `Serialize/Deserialize`; older snapshots without `Skills` still deserialize (additive enum variant). No SQLite migration needed.

- [ ] **Step 4: Commit**

```bash
git add app/src/app_state.rs app/src/workspace/view.rs
git commit -m "feat(skills-panel): persist Skills as the active left-panel tab"
```

---

## Task 8: Integration test

**Files:**
- Create: `crates/integration/src/test/skills_panel.rs`
- Modify: `crates/integration/src/test/mod.rs` (or wherever tests are listed), `crates/integration/src/bin/integration.rs`, `crates/integration/tests/integration/ui_tests.rs` (mirror how `test_file_tree_opens_image_in_image_pane` is registered)

**Interfaces:**
- Consumes: the whole feature end-to-end.

- [ ] **Step 1: Write the integration test**

Mirror `crates/integration/src/test/file_tree.rs::test_file_tree_opens_image_in_image_pane` for setup/registration. Create `crates/integration/src/test/skills_panel.rs`:

```rust
// Verifies the Skills panel lists skills grouped by scope and that the per-agent subtab
// surfaces a skill reachable via provider inheritance.
pub async fn test_skills_panel_lists_and_filters(cx: &mut TestAppContext) {
    FeatureFlag::SkillsPanel.set_enabled(true);

    // Arrange: a project skill under .agents/skills and an identical one under .claude/skills.
    let dir = tempdir();
    write_skill(&dir, ".agents/skills/foo/SKILL.md", "# foo\nbody");
    write_skill(&dir, ".claude/skills/foo/SKILL.md", "# foo\nbody");
    write_skill(&dir, ".agents/skills/bar/SKILL.md", "# bar\nbody");

    let (workspace, _) = open_workspace_in(cx, &dir).await;

    // Act: toggle the Skills panel open.
    workspace.update(cx, |w, ctx| {
        w.handle_action(&WorkspaceAction::OpenSkillsPanel, ctx);
    });

    // Assert: left panel is open, active tool view is Skills, and both skills show under All.
    workspace.read_with(cx, |w, ctx| {
        assert!(w.active_tab_pane_group().as_ref(ctx).left_panel_open);
        assert_eq!(
            w.left_panel_view().as_ref(ctx).active_view(),
            ToolPanelView::Skills
        );
    });

    // Assert data layer (the panel renders exactly this): All has foo+bar; Claude reaches foo.
    workspace.read_with(cx, |_w, ctx| {
        let mgr = SkillManager::as_ref(ctx);
        let cwd = LocalOrRemotePath::Local(dir.path().to_path_buf());
        let all = mgr.get_skills_for_working_directory(Some(&cwd), ctx);
        assert!(all.iter().any(|s| s.name == "foo"));
        assert!(all.iter().any(|s| s.name == "bar"));
        let claude = mgr.skills_for_providers(Some(&cwd), CLIAgent::Claude.supported_skill_providers(), ctx);
        assert!(claude.iter().any(|s| s.name == "foo"));
    });
}
```

> Use the exact harness names the neighboring integration tests use (`TestAppContext`, `tempdir`, `open_workspace_in`, `write_skill`/inline file writes). If a helper doesn't exist, write the file inline as the file-tree test does. `w.left_panel_view()` / `active_view()` accessors: confirm the public accessor names (grep in `view.rs`).

- [ ] **Step 2: Register and run**

Register the test in the integration runner (mirror the image-pane test registration), then:

Run: `cargo test -p integration --features skills_panel test_skills_panel_lists_and_filters`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/integration/
git commit -m "test(skills-panel): integration test for listing + per-agent filter"
```

---

## Task 9: Manual verification + flag-off safety

- [ ] **Step 1: Build with the feature and run**

Run: `cargo build -p warp --features skills_panel`. Launch the app, enable `FeatureFlag::SkillsPanel` (dev override), confirm: the Skills icon appears in the left-panel toolbelt; clicking it opens the panel; All/Claude/Codex subtabs switch; skills appear grouped Home/Project/Bundled; clicking a non-bundled skill opens its `SKILL.md`; the chosen tab persists across restart.

- [ ] **Step 2: Confirm flag-off is a no-op**

Run: `cargo build -p warp` (no `skills_panel` feature). Confirm the app builds and the Skills icon does not appear (the flag isn't registered without the cargo feature, and `compute_left_panel_views` skips it).

- [ ] **Step 3: Final commit (if any cleanup)**

```bash
git add -A
git commit -m "chore(skills-panel): phase 1 cleanup"
```

---

## Deferred to Phase 2 (separate plan)

- Live "skill read" signal: handle `tool_name == "Skill"` events in `app/src/terminal/cli_agent_sessions/`, render an inline banner, and add a session-scoped "Used this session" group to this panel.
- Click → in-panel detail view (description, path, "Available to", provider/scope) as an alternative/addition to opening `SKILL.md`. `agents_that_can_read` (Task 3) already computes the "Available to" set.
- Telemetry: add a `SkillOpenOrigin::SkillsPanel` origin and emit through the existing `SkillTelemetryEvent`.

## Self-review notes (addressed)

- **Spec coverage:** panel surface (Tasks 4–7), scope grouping (Task 3), access-based subtabs + reachability correctness (Tasks 2–3), reuse of `SkillDescriptor`/`render_skill_button` (Tasks 2–4), feature flag (Task 1), persistence (Task 7), empty/degraded states (Task 4 render), tests (Tasks 2, 3, 8). Live signal + detail view are explicitly deferred to Phase 2 per the spec's phasing.
- **Known implementation unknowns** are flagged inline with exact grep targets rather than invented APIs: `SkillManager` view-subscription handle (Task 4 Step 1), active-cwd resolution (Task 5 Step 6), `open_file` signature (Task 6 Step 6), and exact `warpui` element idioms (Task 4 Step 2). Each has a concrete, existing reference to copy from.
- **Type consistency:** `SkillsSubtab`, `providers_for_subtab`, `group_skills_by_scope`, `skills_for_providers`, `LeftPanelEvent::OpenSkillFile`, `SkillsPanelEvent::OpenSkillFile`, `ToolPanelView::Skills`, `LeftPanelAction::Skills`, `LeftPanelDisplayedTab::Skills`, `WorkspaceAction::{ToggleSkillsPanel,OpenSkillsPanel}`, `LEFT_PANEL_SKILLS_BINDING_NAME`, `flags::SHOW_SKILLS_PANEL` are used consistently across tasks.
