use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use warp::features::FeatureFlag;
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::terminal::wait_until_bootstrapped_single_pane_for_tab;
use warp::integration_testing::view_getters::workspace_view;
use warp::terminal::cli_agent::CLIAgent;
use warp::workspace::WorkspaceAction;
use warp::{SkillDescriptor, SkillManager};
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui_core::{async_assert, App, SingletonEntity};

use super::{new_builder, Builder};
use crate::util::write_all_rc_files_for_test;

fn open_skills_panel(app: &mut App) {
    let window_id = app.read(|ctx| {
        ctx.windows()
            .active_window()
            .expect("should have active window")
    });
    let workspace = workspace_view(app, window_id);
    app.update(|ctx| {
        ctx.dispatch_typed_action_for_view(
            window_id,
            workspace.id(),
            &WorkspaceAction::OpenSkillsPanel,
        );
    });
}

fn write_skill(dir: &std::path::Path, relative_path: &str, name: &str) {
    let path = dir.join(relative_path);
    std::fs::create_dir_all(
        path.parent()
            .expect("SKILL.md should have a parent directory"),
    )
    .expect("Failed to create skill directory");
    std::fs::write(
        &path,
        format!("# {name}\nA {name} skill used by an integration test."),
    )
    .expect("Failed to write SKILL.md");
}

/// Verifies the Skills panel (Phase 1) end-to-end: enabling the feature flag and dispatching
/// `WorkspaceAction::OpenSkillsPanel` opens the left panel, and the data layer the panel renders
/// (`SkillManager`) lists a project skill regardless of which supported provider directory it's
/// under (`.agents/skills` vs `.claude/skills`), while a per-agent filter still reaches a skill
/// that lives only under a provider Claude supports (the reachability property).
pub fn test_skills_panel_lists_and_filters() -> Builder {
    FeatureFlag::SkillsPanel.set_enabled(true);

    // The setup callback is the only place `TestSetupUtils::test_dir()` is available; stash the
    // canonicalized path here so later assertion steps can query `SkillManager` with the exact
    // same working directory the app itself tracks (which canonicalizes paths, see
    // `dunce::canonicalize` use in `pane_group::working_directories` / the skill file watcher).
    let test_dir_cell: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(None));
    let test_dir_for_setup = test_dir_cell.clone();
    let test_dir_for_all_assertion = test_dir_cell.clone();
    let test_dir_for_claude_assertion = test_dir_cell.clone();

    new_builder()
        .with_setup(move |utils| {
            let test_dir = utils.test_dir();
            let dir_string = test_dir
                .to_str()
                .expect("Should be able to convert test dir to str");
            write_all_rc_files_for_test(&test_dir, format!("cd {dir_string}"));

            // A project skill under both `.agents/skills` and `.claude/skills` with identical
            // content (a repo supporting multiple CLI agents), plus a second, Agents-only skill.
            write_skill(&test_dir, ".agents/skills/foo/SKILL.md", "foo");
            write_skill(&test_dir, ".claude/skills/foo/SKILL.md", "foo");
            write_skill(&test_dir, ".agents/skills/bar/SKILL.md", "bar");

            let canonical_dir = std::fs::canonicalize(&test_dir).unwrap_or(test_dir);
            *test_dir_for_setup.borrow_mut() = Some(canonical_dir);
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Open Skills panel")
                .with_action(|app, _, _| open_skills_panel(app)),
        )
        .with_step(
            new_step_with_default_assertions(
                "Skills panel is open and SkillManager surfaces both skills",
            )
            .add_named_assertion("left panel is open after OpenSkillsPanel", |app, window_id| {
                let workspace = workspace_view(app, window_id);
                let is_open =
                    workspace.read(app, |workspace, ctx| workspace.is_left_panel_open(ctx));
                async_assert!(is_open, "expected left panel to be open after OpenSkillsPanel")
            })
            .add_named_assertion(
                "SkillManager lists foo and bar for the working directory",
                move |app, _window_id| {
                    let test_dir = test_dir_for_all_assertion
                        .borrow()
                        .clone()
                        .expect("setup should run before any test step");
                    app.read(|ctx| {
                        let manager = SkillManager::as_ref(ctx);
                        let cwd = LocalOrRemotePath::Local(test_dir);
                        let all: Vec<SkillDescriptor> =
                            manager.get_skills_for_working_directory(Some(&cwd), ctx);
                        let names: Vec<&str> = all.iter().map(|s| s.name.as_str()).collect();
                        let has_foo = names.contains(&"foo");
                        let has_bar = names.contains(&"bar");
                        async_assert!(has_foo && has_bar, "expected foo and bar in {names:?}")
                    })
                },
            )
            .add_named_assertion(
                "Claude's supported providers still reach foo (reachability property)",
                move |app, _window_id| {
                    let test_dir = test_dir_for_claude_assertion
                        .borrow()
                        .clone()
                        .expect("setup should run before any test step");
                    app.read(|ctx| {
                        let manager = SkillManager::as_ref(ctx);
                        let cwd = LocalOrRemotePath::Local(test_dir);
                        let providers = CLIAgent::Claude.supported_skill_providers();
                        let claude_visible = manager.skills_for_providers(Some(&cwd), providers, ctx);
                        let has_foo = claude_visible.iter().any(|s| s.name == "foo");
                        async_assert!(
                            has_foo,
                            "expected foo to be reachable via Claude's supported skill providers, got {:?}",
                            claude_visible.iter().map(|s| s.name.as_str()).collect::<Vec<_>>()
                        )
                    })
                },
            ),
        )
}
