use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use futures::executor::block_on;
use tempfile::TempDir;

use super::*;

fn git(dir: &Path, args: &[&str]) -> String {
    let output = command::blocking::Command::new("git")
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
            "-c",
            "protocol.file.allow=always",
        ])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// A repository with one commit on `main`, whose worktrees go in the same temporary folder.
struct Fixture {
    _dir: TempDir,
    repository: Repository,
}

impl Fixture {
    fn new() -> Self {
        let dir = TempDir::new().expect("a temporary folder");
        let primary = dir.path().join("project");
        std::fs::create_dir(&primary).unwrap();
        git(&primary, &["init", "--quiet"]);
        std::fs::write(
            primary.join(".gitignore"),
            "node_modules/\n.env\nnotes.txt\n",
        )
        .unwrap();
        std::fs::write(primary.join("README.md"), "hello\n").unwrap();
        std::fs::write(primary.join(".env"), "KEY=1\n").unwrap();
        git(&primary, &["add", "."]);
        git(&primary, &["commit", "--quiet", "-m", "first"]);
        let worktrees_dir = dir.path().join("worktrees").join("project");
        Self {
            repository: Repository::with_worktrees_dir(primary, worktrees_dir),
            _dir: dir,
        }
    }

    fn primary(&self) -> &Path {
        &self.repository.primary
    }

    fn prepare(&self, in_use: &[&Path]) -> PreparedWorktree {
        let in_use = FoldersInUse::new(in_use.iter().map(|path| path.to_path_buf()));
        let prepared = block_on(prepare_worktree(
            self.repository.clone(),
            in_use,
            HashSet::new(),
        ))
        .expect("a worktree is ready");
        // The new tab's shell would be running in it by now; these tests close that tab at once.
        release(&worktree_git_dir(&prepared.path).unwrap());
        prepared
    }

    fn clean_up(&self, keep: usize) -> Vec<RemovedWorktree> {
        block_on(clean_up(
            self.repository.clone(),
            FoldersInUse::default(),
            keep,
            REMOVE_AFTER,
        ))
    }

    fn branches(&self) -> Vec<String> {
        git(
            self.primary(),
            &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
        )
        .lines()
        .map(str::to_owned)
        .collect()
    }
}

/// Makes Git's files for the worktree look untouched for `age`.
fn age_worktree(worktree: &Path, age: Duration) {
    let when = SystemTime::now() - age;
    let git_dir = worktree_git_dir(worktree).unwrap();
    for file in ["HEAD", "index", "logs/HEAD"] {
        if let Ok(file) = std::fs::File::options()
            .write(true)
            .open(git_dir.join(file))
        {
            file.set_modified(when).unwrap();
        }
    }
}

fn current_branch(worktree: &Path) -> String {
    git(worktree, &["rev-parse", "--abbrev-ref", "HEAD"])
}

#[test]
fn a_new_tab_gets_a_new_worktree_on_a_new_branch_from_main() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare(&[]);
    assert!(!prepared.reused);
    assert_eq!(prepared.base, "main");
    assert!(is_generated_worktree_name(&prepared.branch));
    assert_eq!(prepared.path.file_name().unwrap(), prepared.branch.as_str());
    assert_eq!(current_branch(&prepared.path), prepared.branch);
}

#[test]
fn an_unused_worktree_is_reused_with_its_dependencies_on_a_new_branch() {
    let fixture = Fixture::new();
    let first = fixture.prepare(&[]);
    std::fs::create_dir_all(first.path.join("node_modules/left-pad")).unwrap();

    let second = fixture.prepare(&[]);
    assert!(second.reused);
    assert_eq!(
        canonical(second.path.clone()),
        canonical(first.path.clone())
    );
    assert_ne!(second.branch, first.branch);
    assert_eq!(current_branch(&second.path), second.branch);
    assert!(second.path.join("node_modules/left-pad").is_dir());
    // The old branch held nothing the base lacks, so it is gone.
    assert!(!fixture.branches().contains(&first.branch));
}

#[test]
fn worktrees_holding_work_or_in_use_are_not_reused() {
    let fixture = Fixture::new();
    let untracked = fixture.prepare(&[]);
    std::fs::write(untracked.path.join("draft.md"), "unsaved thoughts\n").unwrap();

    let committed = fixture.prepare(&[]);
    assert!(!committed.reused);
    std::fs::write(committed.path.join("README.md"), "changed\n").unwrap();
    git(&committed.path, &["commit", "--quiet", "-am", "work"]);

    let open = fixture.prepare(&[]);
    assert!(!open.reused);

    let fourth = fixture.prepare(&[&open.path.join("src")]);
    assert!(!fourth.reused, "every worktree holds work or is in use");
    let paths: HashSet<PathBuf> = [&untracked, &committed, &open, &fourth]
        .iter()
        .map(|prepared| canonical(prepared.path.clone()))
        .collect();
    assert_eq!(paths.len(), 4);
    assert!(untracked.path.join("draft.md").is_file());
    assert_eq!(current_branch(&committed.path), committed.branch);
}

#[test]
fn a_worktree_another_new_tab_just_took_is_not_reused() {
    let fixture = Fixture::new();
    let first = block_on(prepare_worktree(
        fixture.repository.clone(),
        FoldersInUse::default(),
        HashSet::new(),
    ))
    .unwrap();
    let second = fixture.prepare(&[]);
    assert!(!second.reused);
    assert_ne!(canonical(second.path), canonical(first.path));
}

#[test]
fn the_base_is_origin_main_when_the_local_main_is_behind_it() {
    let fixture = Fixture::new();
    let primary = fixture.primary();
    assert_eq!(block_on(base_branch(primary)).as_deref(), Some("main"));

    let first = git(primary, &["rev-parse", "HEAD"]);
    std::fs::write(primary.join("README.md"), "newer\n").unwrap();
    git(primary, &["commit", "--quiet", "-am", "second"]);
    git(primary, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    git(primary, &["reset", "--quiet", "--hard", &first]);
    assert_eq!(
        block_on(base_branch(primary)).as_deref(),
        Some("origin/main")
    );

    // Local commits the remote lacks keep the local branch.
    std::fs::write(primary.join("README.md"), "local\n").unwrap();
    git(primary, &["commit", "--quiet", "-am", "local"]);
    assert_eq!(block_on(base_branch(primary)).as_deref(), Some("main"));
}

#[test]
fn cleanup_removes_old_unused_worktrees_beyond_those_kept() {
    let fixture = Fixture::new();
    let in_use_worktree = |fixture: &Fixture, in_use: &[&Path]| fixture.prepare(in_use);
    // Each one is open in a tab while the next is made, so none is reused.
    let recent = in_use_worktree(&fixture, &[]);
    let with_dependencies = in_use_worktree(&fixture, &[&recent.path]);
    let with_env_copy = in_use_worktree(&fixture, &[&recent.path, &with_dependencies.path]);
    let with_notes = in_use_worktree(
        &fixture,
        &[&recent.path, &with_dependencies.path, &with_env_copy.path],
    );
    let with_changes = in_use_worktree(
        &fixture,
        &[
            &recent.path,
            &with_dependencies.path,
            &with_env_copy.path,
            &with_notes.path,
        ],
    );
    std::fs::create_dir_all(with_dependencies.path.join("node_modules/left-pad")).unwrap();
    std::fs::copy(
        fixture.primary().join(".env"),
        with_env_copy.path.join(".env"),
    )
    .unwrap();
    std::fs::write(with_notes.path.join("notes.txt"), "only here\n").unwrap();
    std::fs::write(with_changes.path.join("README.md"), "edited\n").unwrap();
    let week = Duration::from_secs(8 * 24 * 60 * 60);
    for old in [
        &with_dependencies,
        &with_env_copy,
        &with_notes,
        &with_changes,
    ] {
        age_worktree(&old.path, week);
    }
    // A merged branch Clinch didn't name stays.
    git(fixture.primary(), &["branch", "feature-x"]);

    let removed = fixture.clean_up(1);
    let removed_paths: HashSet<PathBuf> =
        removed.iter().map(|removed| removed.path.clone()).collect();
    assert_eq!(
        removed_paths,
        HashSet::from([
            canonical(with_dependencies.path.clone()),
            canonical(with_env_copy.path.clone())
        ])
    );
    assert!(recent.path.is_dir(), "the most recent unused one is kept");
    assert!(with_notes.path.join("notes.txt").is_file());
    assert!(with_changes.path.is_dir());
    assert!(!with_dependencies.path.exists());

    let branches = fixture.branches();
    assert!(!branches.contains(&with_dependencies.branch));
    assert!(!branches.contains(&with_env_copy.branch));
    assert!(branches.contains(&recent.branch));
    assert!(branches.contains(&"feature-x".to_owned()));
}

#[test]
fn cleanup_keeps_worktrees_used_this_week() {
    let fixture = Fixture::new();
    let first = fixture.prepare(&[]);
    let second = fixture.prepare(&[&first.path]);
    assert!(fixture.clean_up(0).is_empty());
    assert!(first.path.is_dir() && second.path.is_dir());
}

#[test]
fn repositories_are_found_from_their_worktrees() {
    let fixture = Fixture::new();
    fixture.prepare(&[]);
    let root = fixture.repository.worktrees_dir.parent().unwrap();
    let repositories = repositories_in(root);
    assert_eq!(repositories.len(), 1);
    assert_eq!(repositories[0].primary, fixture.repository.primary);
}

#[test]
fn listed_worktrees_are_clinch_ones_that_are_not_locked() {
    let managed = Path::new("/nonexistent/worktrees/project");
    let porcelain = "worktree /nonexistent/project\nHEAD 1\nbranch refs/heads/main\n\n\
                     worktree /nonexistent/worktrees/project/mesa-quail\nHEAD 2\nbranch refs/heads/mesa-quail\n\n\
                     worktree /nonexistent/worktrees/project/kept\nHEAD 3\nbranch refs/heads/kept\nlocked\n\n\
                     worktree /nonexistent/worktrees/project/gone\nHEAD 4\ndetached\nprunable gitdir file points to non-existent location\n\n\
                     worktree /nonexistent/worktrees/project/detached\nHEAD 5\ndetached\n";
    assert_eq!(
        listed_worktrees(porcelain, managed),
        vec![
            (managed.join("mesa-quail"), Some("mesa-quail".to_owned())),
            (managed.join("detached"), None),
        ]
    );
}

#[test]
fn only_dependency_and_build_folders_are_rebuildable() {
    for rebuildable in [
        "node_modules/",
        "packages/web/node_modules/",
        "apps/site/.next/",
        "target/",
        ".DS_Store",
        "src/.DS_Store",
        "tsconfig.tsbuildinfo",
    ] {
        assert!(is_rebuildable(rebuildable), "{rebuildable}");
    }
    for kept in [
        ".env",
        "notes.txt",
        "local-data/",
        "\"odd name\"",
        "nodemodules/",
        // A file in a tracked folder with a build folder's name.
        "build/signing.p12",
        "node_modules/kept.txt",
    ] {
        assert!(!is_rebuildable(kept), "{kept}");
    }
}

#[test]
fn a_commit_only_head_s_history_reaches_keeps_the_worktree() {
    let fixture = Fixture::new();
    let first = fixture.prepare(&[]);
    git(&first.path, &["switch", "--quiet", "--detach"]);
    git(
        &first.path,
        &["commit", "--quiet", "--allow-empty", "-m", "made and left"],
    );
    git(&first.path, &["switch", "--quiet", "--detach", "main"]);
    assert_eq!(git(&first.path, &["status", "--porcelain"]), "");

    let second = fixture.prepare(&[]);
    assert!(!second.reused);
    age_worktree(&first.path, Duration::from_secs(8 * 24 * 60 * 60));
    assert!(fixture.clean_up(0).is_empty());
    assert!(first.path.is_dir());
}

#[test]
fn reuse_never_replaces_an_ignored_file_the_base_now_tracks() {
    let fixture = Fixture::new();
    let first = fixture.prepare(&[]);
    std::fs::write(first.path.join("notes.txt"), "my notes\n").unwrap();
    // The main branch starts tracking a file of that name.
    let primary = fixture.primary();
    std::fs::write(primary.join(".gitignore"), "node_modules/\n.env\n").unwrap();
    std::fs::write(primary.join("notes.txt"), "tracked\n").unwrap();
    git(primary, &["add", "."]);
    git(primary, &["commit", "--quiet", "-m", "track notes"]);

    let second = fixture.prepare(&[]);
    assert!(!second.reused);
    assert_eq!(
        std::fs::read_to_string(first.path.join("notes.txt")).unwrap(),
        "my notes\n"
    );
}

#[test]
fn work_only_a_submodule_holds_keeps_the_worktree() {
    let fixture = Fixture::new();
    let library = fixture._dir.path().join("library");
    std::fs::create_dir(&library).unwrap();
    git(&library, &["init", "--quiet"]);
    git(
        &library,
        &["commit", "--quiet", "--allow-empty", "-m", "library"],
    );
    let primary = fixture.primary();
    git(
        primary,
        &[
            "submodule",
            "add",
            "--quiet",
            &library.to_string_lossy(),
            "library",
        ],
    );
    git(primary, &["commit", "--quiet", "-m", "add library"]);

    let first = fixture.prepare(&[]);
    git(&first.path, &["submodule", "update", "--quiet", "--init"]);
    let submodule = first.path.join("library");
    let recorded = git(&submodule, &["rev-parse", "HEAD"]);
    // A commit on a branch no remote has, then back to the recorded commit: nothing shows.
    git(&submodule, &["switch", "--quiet", "-c", "fix"]);
    git(
        &submodule,
        &["commit", "--quiet", "--allow-empty", "-m", "unpushed"],
    );
    git(&submodule, &["switch", "--quiet", "--detach", &recorded]);
    assert_eq!(git(&first.path, &["status", "--porcelain"]), "");

    let second = fixture.prepare(&[&first.path]);
    age_worktree(&first.path, Duration::from_secs(8 * 24 * 60 * 60));
    age_worktree(&second.path, Duration::from_secs(8 * 24 * 60 * 60));
    let removed = fixture.clean_up(0);
    assert!(first.path.is_dir(), "the submodule's branch would be lost");
    assert!(git(&submodule, &["branch", "--list", "fix"]).contains("fix"));
    // The second worktree never initialized the submodule, so it goes.
    assert_eq!(removed.len(), 1);
    assert!(!second.path.exists());
}

#[test]
fn a_worktree_reports_whether_it_is_still_on_its_branch() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare(&[]);
    assert!(worktree_is_on_branch(&prepared.path, &prepared.branch));
    git(&prepared.path, &["switch", "--quiet", "-c", "someone-else"]);
    assert!(!worktree_is_on_branch(&prepared.path, &prepared.branch));
}

#[test]
fn initialized_submodules_and_lsof_folders_are_parsed() {
    let status =
        " 1234567 vendor/lib (v1.0)\n-89abcde docs/site\n+fedcba9 tools/cli (heads/main)\n";
    assert_eq!(
        initialized_submodules(status),
        vec![PathBuf::from("vendor/lib"), PathBuf::from("tools/cli")]
    );
    assert_eq!(
        lsof_folders("p123\nfcwd\nn/Users/me/project\np456\nfcwd\nn/tmp\n"),
        vec![PathBuf::from("/Users/me/project"), PathBuf::from("/tmp")]
    );
}
