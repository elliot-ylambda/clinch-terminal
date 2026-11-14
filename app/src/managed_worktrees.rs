//! The Git worktrees Clinch makes for new project tabs, under `~/.clinch/worktrees/<repo>/`.
//!
//! Checking out a whole repository takes a while, and an agent in a fresh worktree installs its
//! dependencies all over again. So a new tab takes an unused worktree of the same repository when
//! there is one, switching it to a new branch from the base and keeping its dependencies and build
//! caches. Worktrees left unused for a week are removed, apart from a few kept for new tabs.
//!
//! A worktree is unused when:
//! - no Clinch pane, saved agent session or running process is in it, in any Clinch app, and no
//!   new tab has taken it in the last few minutes
//! - nothing in it exists only there: no changes or untracked files, in it or its submodules, and
//!   no commits missing from the base
//!
//! Removing one also needs each of its ignored files to be one that can be made again: something
//! in a dependency or build folder, or a copy of the same file in the main checkout.
use std::cmp::Reverse;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{anyhow, Result};
use warp_util::git::run_git_command;
use warp_util::worktree_names::{generate_worktree_branch_name, is_generated_worktree_name};
use warpui::r#async::Timer;
use warpui::{AppContext, Entity, EntityId, ModelContext, SingletonEntity};

use crate::settings::ClinchSettings;
use crate::tab_configs::tab_config::generated_worktree_repo_dir;

#[cfg(test)]
#[path = "managed_worktrees_tests.rs"]
mod tests;

/// How long after launch the first cleanup runs, so it stays out of the way of restoring tabs.
const FIRST_CLEANUP_AFTER: Duration = Duration::from_secs(2 * 60);
const CLEANUP_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// Unused worktrees of a repository that cleanup keeps for new tabs, the most recently used ones.
pub(crate) const KEPT_FOR_REUSE: usize = 3;
/// How long a worktree beyond those kept must have gone unused before cleanup removes it.
pub(crate) const REMOVE_AFTER: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// Taking a worktree writes this file into its Git directory, so another Clinch app doesn't take
/// it too before the new tab's shell is running in it. The new tab removes it once its shell is,
/// and it lapses after a while should the tab never get that far.
const TAKEN_MARKER: &str = "clinch-taken";
const TAKEN_FOR: Duration = Duration::from_secs(10 * 60);
/// The most worktrees a new tab checks before making a new one instead.
const MAX_REUSE_CANDIDATES: usize = 6;
/// Ignored folders that installs and builds make again.
const REBUILDABLE_DIRS: &[&str] = &[
    "node_modules",
    ".next",
    ".turbo",
    ".cache",
    ".parcel-cache",
    ".svelte-kit",
    ".nuxt",
    ".vercel",
    ".expo",
    "target",
    "dist",
    "build",
    "out",
    "coverage",
    ".venv",
    "venv",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
];
/// Ignored files the system and build tools make.
const REBUILDABLE_FILES: &[&str] = &[".DS_Store", ".eslintcache", "next-env.d.ts"];
const REBUILDABLE_FILE_SUFFIXES: &[&str] = &[".tsbuildinfo", ".pyc"];
/// The most commits of a HEAD's history checked against the branches.
const MAX_HISTORY_COMMITS_CHECKED: usize = 4000;
/// How long listing what runs where may take before it counts as unknown.
const LSOF_TIMEOUT: Duration = Duration::from_secs(20);
/// An ignored file bigger than this isn't compared with the main checkout's copy, and keeps its
/// worktree.
const MAX_COMPARED_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// A repository and the folder Clinch keeps its worktrees in.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Repository {
    /// The main checkout.
    pub(crate) primary: PathBuf,
    worktrees_dir: PathBuf,
}

impl Repository {
    pub(crate) fn new(primary: PathBuf) -> Self {
        let worktrees_dir = generated_worktree_repo_dir(&primary);
        Self::with_worktrees_dir(primary, worktrees_dir)
    }

    fn with_worktrees_dir(primary: PathBuf, worktrees_dir: PathBuf) -> Self {
        Self {
            primary: canonical(primary),
            worktrees_dir,
        }
    }

    fn worktree_path(&self, branch: &str) -> PathBuf {
        self.worktrees_dir.join(branch)
    }
}

/// One of Clinch's worktrees, as `git worktree list` reports it.
#[derive(Clone, Debug)]
pub(crate) struct ManagedWorktree {
    pub(crate) path: PathBuf,
    pub(crate) branch: Option<String>,
    /// Git's own files for the worktree: `<repo>/.git/worktrees/<name>`.
    git_dir: PathBuf,
    /// When Git last changed the worktree's HEAD or index.
    last_used: SystemTime,
}

/// A worktree ready for a new tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PreparedWorktree {
    pub(crate) path: PathBuf,
    pub(crate) branch: String,
    /// The branch it starts from, such as `origin/main`.
    pub(crate) base: String,
    /// An unused worktree was switched to the new branch, rather than a new one checked out.
    pub(crate) reused: bool,
}

/// A worktree cleanup removed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RemovedWorktree {
    pub(crate) path: PathBuf,
    pub(crate) branch: Option<String>,
}

/// Folders something works in. A worktree containing one of them is in use.
#[derive(Clone, Debug, Default)]
pub(crate) struct FoldersInUse {
    /// This app's panes, as of when the folders were gathered.
    panes: Vec<PathBuf>,
    /// Running processes and every Clinch app's saved agent sessions.
    others: Vec<PathBuf>,
}

impl FoldersInUse {
    /// Starts from the folders of this app's panes.
    pub(crate) fn new(folders: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            panes: folders.into_iter().map(canonical).collect(),
            others: Vec::new(),
        }
    }

    /// Adds, freshly read, where running processes work and where every Clinch app's saved agent
    /// sessions reopen, so another Clinch app's tabs count too. `None` when the processes can't be
    /// listed: then nothing can be known to be unused.
    async fn with_other_apps(&self) -> Option<Self> {
        let mut folders = self.clone();
        // Unit tests use temporary repositories nothing else runs in.
        if cfg!(test) {
            return Some(folders);
        }
        folders.others = process_working_folders().await?;
        folders
            .others
            .extend(saved_agent_session_folders().into_iter().map(canonical));
        Some(folders)
    }

    fn covers(&self, worktree: &Path) -> bool {
        self.panes
            .iter()
            .chain(&self.others)
            .any(|folder| folder.starts_with(worktree))
    }
}

/// `path` with symbolic links resolved. A folder that's gone, or not made yet, is resolved
/// through the part of it that exists.
fn canonical(path: PathBuf) -> PathBuf {
    if let Ok(resolved) = dunce::canonicalize(&path) {
        return resolved;
    }
    let mut missing = Vec::new();
    let mut existing = path.as_path();
    while let (Some(parent), Some(name)) = (existing.parent(), existing.file_name()) {
        missing.push(name);
        existing = parent;
        if let Ok(resolved) = dunce::canonicalize(existing) {
            return missing
                .iter()
                .rev()
                .fold(resolved, |resolved, name| resolved.join(name));
        }
    }
    path
}

/// The working folders of this user's running processes.
#[cfg(not(unix))]
async fn process_working_folders() -> Option<Vec<PathBuf>> {
    None
}

/// The working folders of this user's running processes, or `None` when they can't be listed.
#[cfg(unix)]
async fn process_working_folders() -> Option<Vec<PathBuf>> {
    use std::os::unix::fs::MetadataExt as _;

    let uid = std::fs::metadata(dirs::home_dir()?).ok()?.uid().to_string();
    let lsof = async {
        command::r#async::Command::new("/usr/sbin/lsof")
            .args(["-a", "-d", "cwd", "-u", &uid, "-Fn", "-w"])
            .stdout(command::Stdio::piped())
            .stderr(command::Stdio::null())
            .kill_on_drop(true)
            .output()
            .await
            .ok()
    };
    let timeout = async {
        async_io::Timer::after(LSOF_TIMEOUT).await;
        None
    };
    let Some(output) = futures_lite::future::or(lsof, timeout).await else {
        log::warn!("could not list the folders processes run in");
        return None;
    };
    // This process always has a working folder, so empty output is a failure too.
    let folders = lsof_folders(&String::from_utf8_lossy(&output.stdout));
    (output.status.success() && !folders.is_empty()).then_some(folders)
}

/// The `n` (name) fields of `lsof -F n` output.
fn lsof_folders(output: &str) -> Vec<PathBuf> {
    output
        .lines()
        .filter_map(|line| line.strip_prefix('n'))
        .map(PathBuf::from)
        .collect()
}

/// Where the agent sessions every Clinch app will reopen run: each app's agent-resume registry
/// lists its open panes in `active-panes`, with a `<pane>.json` entry holding the folder.
fn saved_agent_session_folders() -> Vec<PathBuf> {
    let mut registries: Vec<PathBuf> = std::env::var_os("WARP_AGENT_RESUME_DIR")
        .map(PathBuf::from)
        .into_iter()
        .collect();
    if let Some(entries) = dirs::home_dir().and_then(|home| std::fs::read_dir(home).ok()) {
        registries.extend(
            entries
                .flatten()
                .filter(|entry| entry.file_name().to_string_lossy().starts_with('.'))
                .map(|entry| entry.path().join("agent-resume"))
                .filter(|registry| registry.join("active-panes").is_file()),
        );
    }
    registries
        .iter()
        .flat_map(|registry| registry_folders(registry))
        .collect()
}

fn registry_folders(registry: &Path) -> Vec<PathBuf> {
    #[derive(serde::Deserialize)]
    struct Entry {
        cwd: Option<String>,
    }
    let Ok(panes) = std::fs::read_to_string(registry.join("active-panes")) else {
        return Vec::new();
    };
    panes
        .lines()
        .map(str::trim)
        .filter(|pane| !pane.is_empty() && pane.bytes().all(|b| b.is_ascii_hexdigit()))
        .filter_map(|pane| std::fs::read_to_string(registry.join(format!("{pane}.json"))).ok())
        .filter_map(|entry| serde_json::from_str::<Entry>(&entry).ok()?.cwd)
        .map(PathBuf::from)
        .collect()
}

/// The branch new worktrees start from: `origin/main` when the local `main` is behind it or
/// missing, otherwise `main`. Nothing is fetched.
pub(crate) async fn base_branch(primary: &Path) -> Option<String> {
    let exists = |reference: &'static str| async move {
        run_git_command(primary, &["rev-parse", "--verify", "--quiet", reference])
            .await
            .is_ok()
    };
    let local = exists("refs/heads/main").await;
    let remote = exists("refs/remotes/origin/main").await;
    let base = match (local, remote) {
        (true, true) => {
            let behind = run_git_command(
                primary,
                &["merge-base", "--is-ancestor", "main", "origin/main"],
            )
            .await
            .is_ok();
            if behind {
                "origin/main"
            } else {
                "main"
            }
        }
        (true, false) => "main",
        (false, true) => "origin/main",
        (false, false) => return None,
    };
    Some(base.to_owned())
}

/// Clinch's worktrees of `repository`.
async fn managed_worktrees(repository: &Repository) -> Result<Vec<ManagedWorktree>> {
    let managed_dir = canonical(repository.worktrees_dir.clone());
    let listed = run_git_command(&repository.primary, &["worktree", "list", "--porcelain"]).await?;
    Ok(listed_worktrees(&listed, &managed_dir)
        .into_iter()
        .filter_map(|(path, branch)| {
            let git_dir = worktree_git_dir(&path)?;
            Some(ManagedWorktree {
                last_used: last_used(&git_dir),
                path,
                branch,
                git_dir,
            })
        })
        .collect())
}

/// The worktrees in `git worktree list --porcelain` output that live in `managed_dir`, with their
/// branches. Locked worktrees are left out, since someone asked Git to keep them, and so are ones
/// Git already knows are gone.
fn listed_worktrees(porcelain: &str, managed_dir: &Path) -> Vec<(PathBuf, Option<String>)> {
    porcelain
        .split("\n\n")
        .filter_map(|record| {
            let mut path = None;
            let mut branch = None;
            for line in record.lines() {
                if let Some(listed) = line.strip_prefix("worktree ") {
                    path = Some(canonical(PathBuf::from(listed)));
                } else if let Some(reference) = line.strip_prefix("branch ") {
                    branch = Some(
                        reference
                            .strip_prefix("refs/heads/")
                            .unwrap_or(reference)
                            .to_owned(),
                    );
                } else if line == "locked"
                    || line.starts_with("locked ")
                    || line == "prunable"
                    || line.starts_with("prunable ")
                {
                    return None;
                }
            }
            let path = path?;
            (path.parent() == Some(managed_dir)).then_some((path, branch))
        })
        .collect()
}

/// A linked worktree's Git directory, from the `gitdir:` line in its `.git` file.
fn worktree_git_dir(worktree: &Path) -> Option<PathBuf> {
    let link = std::fs::read_to_string(worktree.join(".git")).ok()?;
    let git_dir = Path::new(link.strip_prefix("gitdir:")?.trim());
    Some(worktree.join(git_dir))
}

fn last_used(git_dir: &Path) -> SystemTime {
    ["HEAD", "index", "logs/HEAD"]
        .iter()
        .filter_map(|file| std::fs::metadata(git_dir.join(file)).ok()?.modified().ok())
        .max()
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

/// Whether the worktree at `path` still has `branch` checked out, read from its Git directory's
/// `HEAD` file without running Git.
pub(crate) fn worktree_is_on_branch(path: &Path, branch: &str) -> bool {
    worktree_git_dir(path)
        .and_then(|git_dir| std::fs::read_to_string(git_dir.join("HEAD")).ok())
        .is_some_and(|head| head.trim() == format!("ref: refs/heads/{branch}"))
}

/// Whether the taken marker at `marker` was written in the last [`TAKEN_FOR`]. A marker dated
/// in the future, after a clock change, counts as fresh unless it is days ahead.
fn marker_is_fresh(marker: &Path) -> bool {
    let Ok(taken_at) = std::fs::metadata(marker).and_then(|metadata| metadata.modified()) else {
        return false;
    };
    match SystemTime::now().duration_since(taken_at) {
        Ok(age) => age < TAKEN_FOR,
        Err(ahead) => ahead.duration() < Duration::from_secs(24 * 60 * 60),
    }
}

fn is_taken(git_dir: &Path) -> bool {
    marker_is_fresh(&git_dir.join(TAKEN_MARKER))
}

/// Takes the worktree with this Git directory for a new tab or for removal. Fails when someone
/// else took it first, in this app or another.
fn take(git_dir: &Path) -> bool {
    let marker = git_dir.join(TAKEN_MARKER);
    for _ in 0..2 {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&marker)
        {
            Ok(_) => return true,
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                if marker_is_fresh(&marker) {
                    return false;
                }
                // Moving a stale marker aside succeeds for only one of several apps doing it at
                // once. One that moved a marker someone has just made puts it back.
                let nanos = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos();
                let aside = git_dir.join(format!("{TAKEN_MARKER}.{}.{nanos}", std::process::id()));
                if std::fs::rename(&marker, &aside).is_err() {
                    return false;
                }
                if marker_is_fresh(&aside) {
                    let _ = std::fs::rename(&aside, &marker);
                    return false;
                }
                let _ = std::fs::remove_file(aside);
            }
            Err(_) => return false,
        }
    }
    false
}

fn release(git_dir: &Path) {
    let _ = std::fs::remove_file(git_dir.join(TAKEN_MARKER));
}

/// A new tab's shell now works in the worktree at `path`, which marks it as in use from here on.
pub(crate) fn release_taken_worktree(path: &Path) {
    if let Some(git_dir) = worktree_git_dir(path) {
        release(&git_dir);
    }
}

/// Why the worktree must be kept, when something in it exists only there: changes or untracked
/// files in it or its submodules, commits missing from `base`, or commits only its HEAD's history
/// reaches, such as one made on a detached HEAD and left.
async fn unique_work(worktree: &Path, base: &str) -> Option<&'static str> {
    match run_git_command(
        worktree,
        &[
            "status",
            "--porcelain",
            "--untracked-files=normal",
            "--ignore-submodules=none",
        ],
    )
    .await
    {
        Ok(status) if status.trim().is_empty() => {}
        Ok(_) => return Some("it has changes or untracked files"),
        Err(_) => return Some("Git could not read its status"),
    }
    match run_git_command(worktree, &["rev-list", "--count", &format!("{base}..HEAD")]).await {
        Ok(count) if count.trim() == "0" => {}
        Ok(_) => return Some("it has commits that are not on the base branch"),
        Err(_) => return Some("Git could not compare it with the base branch"),
    }
    if has_commits_only_its_history_reaches(worktree).await {
        return Some("its HEAD's history reaches commits no branch has");
    }
    None
}

/// Whether the repository at `root` has commits only its HEAD's history (reflog) reaches. Such
/// commits are lost when a linked worktree, and its HEAD's history with it, is removed.
async fn has_commits_only_its_history_reaches(root: &Path) -> bool {
    let Ok(history) = run_git_command(root, &["reflog", "--format=%H", "HEAD"]).await else {
        // No history at all is nothing to lose; an unreadable one might be.
        return run_git_command(root, &["rev-parse", "--verify", "--quiet", "HEAD"])
            .await
            .is_ok();
    };
    let mut commits: Vec<&str> = history.lines().map(str::trim).collect();
    commits.sort_unstable();
    commits.dedup();
    commits.retain(|commit| !commit.is_empty());
    if commits.is_empty() {
        return false;
    }
    // Too long a history to pass on the command line counts as holding work.
    if commits.len() > MAX_HISTORY_COMMITS_CHECKED {
        return true;
    }
    let mut args = vec!["rev-list", "--max-count=1"];
    args.extend(commits);
    args.extend(["--not", "--all"]);
    !matches!(run_git_command(root, &args).await, Ok(unreachable) if unreachable.trim().is_empty())
}

/// What in an initialized submodule exists only there: a stash, commits on its branches that none
/// of its remotes has, or commits only its HEAD's history reaches. Removing a linked worktree
/// deletes its submodules' repositories, which live in its Git directory.
async fn submodule_unique_work(submodule: &Path) -> Option<&'static str> {
    match run_git_command(submodule, &["stash", "list"]).await {
        Ok(stashes) if stashes.trim().is_empty() => {}
        _ => return Some("a submodule has stashed changes"),
    }
    match run_git_command(
        submodule,
        &[
            "rev-list",
            "--max-count=1",
            "--branches",
            "--not",
            "--remotes",
        ],
    )
    .await
    {
        Ok(unpushed) if unpushed.trim().is_empty() => {}
        _ => return Some("a submodule has commits its remotes don't"),
    }
    if has_commits_only_its_history_reaches(submodule).await {
        return Some("a submodule's HEAD's history reaches commits no branch has");
    }
    None
}

/// The initialized submodules of the worktree, or `None` when they can't be listed.
async fn worktree_submodules(worktree: &Path) -> Option<Vec<PathBuf>> {
    let status = run_git_command(worktree, &["submodule", "status", "--recursive"])
        .await
        .ok()?;
    Some(initialized_submodules(&status))
}

/// Why the worktree can't be removed, when something about it would be lost: an ignored file that
/// can't be made again, in it or a submodule, or work only a submodule holds.
async fn removal_loses(worktree: &Path, primary: &Path) -> Option<String> {
    let Some(submodules) = worktree_submodules(worktree).await else {
        return Some("its submodules can't be listed".to_owned());
    };
    for submodule in &submodules {
        if let Some(reason) = submodule_unique_work(&worktree.join(submodule)).await {
            return Some(format!("{reason} ({})", submodule.display()));
        }
    }
    for root in std::iter::once(PathBuf::new()).chain(submodules) {
        let Ok(status) = run_git_command(
            &worktree.join(&root),
            &[
                "status",
                "--porcelain",
                "--ignored=traditional",
                "--untracked-files=normal",
            ],
        )
        .await
        else {
            return Some(format!("the status of {} can't be read", root.display()));
        };
        for ignored in status.lines().filter_map(|line| line.strip_prefix("!! ")) {
            let relative = root.join(ignored.trim_end_matches('/'));
            let is_copy = !ignored.ends_with('/')
                && is_copy_of(&worktree.join(&relative), &primary.join(&relative));
            if !is_rebuildable(ignored) && !is_copy {
                return Some(format!(
                    "its ignored file {} can't be made again",
                    relative.display()
                ));
            }
        }
    }
    None
}

/// The paths of the initialized submodules in `git submodule status --recursive` output.
fn initialized_submodules(status: &str) -> Vec<PathBuf> {
    status
        .lines()
        // `-` marks a submodule that isn't initialized, so it has no files.
        .filter(|line| !line.starts_with('-'))
        .filter_map(|line| line.get(1..)?.split_whitespace().nth(1))
        .map(PathBuf::from)
        .collect()
}

/// Whether an ignored path, as `git status --ignored` prints it, is one installs, builds or the
/// system make again: a wholly ignored folder named like a dependency or build folder, or a file
/// named like a generated one. A file inside a tracked folder of such a name, like
/// `build/signing.p12`, is not. Quoted paths have unusual characters and are not either.
fn is_rebuildable(ignored: &str) -> bool {
    if ignored.starts_with('"') {
        return false;
    }
    match ignored.strip_suffix('/') {
        Some(folder) => folder
            .rsplit('/')
            .next()
            .is_some_and(|name| REBUILDABLE_DIRS.contains(&name)),
        None => ignored.rsplit('/').next().is_some_and(|name| {
            REBUILDABLE_FILES.contains(&name)
                || REBUILDABLE_FILE_SUFFIXES
                    .iter()
                    .any(|suffix| name.ends_with(suffix))
        }),
    }
}

/// Whether `file` has the same contents as `original`, such as a `.env` copied from the main
/// checkout.
fn is_copy_of(file: &Path, original: &Path) -> bool {
    let (Ok(file_info), Ok(original_info)) = (std::fs::metadata(file), std::fs::metadata(original))
    else {
        return false;
    };
    file_info.is_file()
        && original_info.is_file()
        && file_info.len() == original_info.len()
        && file_info.len() <= MAX_COMPARED_FILE_BYTES
        && std::fs::read(file).ok() == std::fs::read(original).ok()
}

/// The branch the worktree has checked out right now, if any.
async fn current_branch(worktree: &Path) -> Option<String> {
    run_git_command(worktree, &["symbolic-ref", "--short", "-q", "HEAD"])
        .await
        .ok()
        .map(|branch| branch.trim().to_owned())
        .filter(|branch| !branch.is_empty())
}

/// Deletes `branch` only if it still points at `commit`, so a branch someone has just moved stays.
async fn delete_branch_at(repo: &Path, branch: &str, commit: &str) {
    let reference = format!("refs/heads/{branch}");
    match run_git_command(repo, &["update-ref", "-d", &reference, commit]).await {
        Ok(_) => log::info!("deleted the merged branch {branch}"),
        Err(err) => log::warn!("did not delete the branch {branch}: {err}"),
    }
}

/// Gets a worktree of `repository` ready for a new tab: an unused one switched to a new branch
/// from the base, or else a new one. `reserved` holds branch names other new tabs are about to
/// use.
pub(crate) async fn prepare_worktree(
    repository: Repository,
    in_use: FoldersInUse,
    reserved: HashSet<String>,
) -> Result<PreparedWorktree> {
    let primary = &repository.primary;
    let base = base_branch(primary)
        .await
        .ok_or_else(|| anyhow!("{} has no main branch", primary.display()))?;
    let worktrees = managed_worktrees(&repository).await.unwrap_or_default();
    let branch = new_branch_name(&repository, &worktrees, &reserved).await?;

    // Without knowing what runs where, a new worktree is the only safe choice.
    if let Some(in_use) = in_use.with_other_apps().await {
        let mut candidates: Vec<&ManagedWorktree> = worktrees
            .iter()
            .filter(|worktree| !in_use.covers(&worktree.path))
            .collect();
        // The most recently used have the freshest dependencies.
        candidates.sort_by_key(|worktree| Reverse(worktree.last_used));
        for worktree in candidates.into_iter().take(MAX_REUSE_CANDIDATES) {
            if let Some(prepared) = reuse(worktree, &branch, &base).await {
                return Ok(prepared);
            }
        }
    }

    let path = repository.worktree_path(&branch);
    let path_text = path.to_string_lossy();
    // Locked until taken, so no other app takes it in between.
    run_git_command(
        primary,
        &[
            "worktree",
            "add",
            "--quiet",
            "--lock",
            "--reason",
            TAKEN_MARKER,
            "--no-track",
            "-b",
            &branch,
            &path_text,
            &base,
        ],
    )
    .await?;
    if let Some(git_dir) = worktree_git_dir(&path) {
        take(&git_dir);
    }
    if let Err(err) = run_git_command(primary, &["worktree", "unlock", &path_text]).await {
        log::warn!(
            "could not unlock the new worktree {}: {err}",
            path.display()
        );
    }
    Ok(PreparedWorktree {
        path,
        branch,
        base,
        reused: false,
    })
}

/// Takes the unused `worktree` and switches it to a new `branch` from `base`, if it still holds
/// nothing of its own.
async fn reuse(worktree: &ManagedWorktree, branch: &str, base: &str) -> Option<PreparedWorktree> {
    if !take(&worktree.git_dir) {
        return None;
    }
    // Read once taken: the list it came from may be out of date.
    let old_branch = current_branch(&worktree.path).await;
    let old_commit = run_git_command(&worktree.path, &["rev-parse", "--verify", "HEAD"])
        .await
        .ok()
        .map(|commit| commit.trim().to_owned());
    let switched = unique_work(&worktree.path, base).await.is_none()
        && old_commit.is_some()
        // An ignored file the base now tracks makes the switch fail rather than replace it.
        && run_git_command(
            &worktree.path,
            &[
                "switch",
                "--quiet",
                "--no-overwrite-ignore",
                "--no-track",
                "-c",
                branch,
                base,
            ],
        )
        .await
        .is_ok();
    if !switched {
        release(&worktree.git_dir);
        return None;
    }
    // Everything on the old branch is on the base, as checked above.
    if let (Some(old_branch), Some(old_commit)) = (old_branch, old_commit) {
        if is_generated_worktree_name(&old_branch) {
            delete_branch_at(&worktree.path, &old_branch, &old_commit).await;
        }
    }
    Some(PreparedWorktree {
        path: worktree.path.clone(),
        branch: branch.to_owned(),
        base: base.to_owned(),
        reused: true,
    })
}

/// A branch name no branch, worktree folder or other new tab uses.
async fn new_branch_name(
    repository: &Repository,
    worktrees: &[ManagedWorktree],
    reserved: &HashSet<String>,
) -> Result<String> {
    let branches = run_git_command(
        &repository.primary,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
    )
    .await?;
    let mut taken: HashSet<String> = branches
        .lines()
        .map(|line| line.trim().to_owned())
        .collect();
    taken.extend(reserved.iter().cloned());
    taken.extend(
        worktrees
            .iter()
            .filter_map(|worktree| worktree.path.file_name())
            .map(|name| name.to_string_lossy().into_owned()),
    );
    for _ in 0..16 {
        let branch = generate_worktree_branch_name(&taken.iter().map(String::as_str).collect());
        // A folder can be left behind without its worktree.
        if !repository.worktree_path(&branch).exists() {
            return Ok(branch);
        }
        taken.insert(branch);
    }
    Err(anyhow!("could not find an unused worktree name"))
}

/// Removes the unused worktrees of `repository` that haven't been used for `remove_after`, apart
/// from the `keep` most recently used, and then the branches Clinch named that are merged into the
/// base and checked out nowhere.
pub(crate) async fn clean_up(
    repository: Repository,
    in_use: FoldersInUse,
    keep: usize,
    remove_after: Duration,
) -> Vec<RemovedWorktree> {
    let primary = &repository.primary;
    let Some(base) = base_branch(primary).await else {
        return Vec::new();
    };
    let mut worktrees = match managed_worktrees(&repository).await {
        Ok(worktrees) => worktrees,
        Err(err) => {
            log::warn!(
                "could not list the worktrees of {}: {err}",
                primary.display()
            );
            return Vec::new();
        }
    };
    let Some(in_use_at_start) = in_use.with_other_apps().await else {
        return Vec::new();
    };
    worktrees.sort_by_key(|worktree| Reverse(worktree.last_used));
    let now = SystemTime::now();
    let mut kept = 0;
    let mut removed = Vec::new();
    for worktree in worktrees {
        if in_use_at_start.covers(&worktree.path)
            || is_taken(&worktree.git_dir)
            || unique_work(&worktree.path, &base).await.is_some()
        {
            continue;
        }
        if kept < keep {
            kept += 1;
            continue;
        }
        let unused_for = now.duration_since(worktree.last_used).unwrap_or_default();
        if unused_for < remove_after {
            continue;
        }
        if let Some(reason) = removal_loses(&worktree.path, primary).await {
            log::info!(
                "keeping the unused worktree {}: {reason}",
                worktree.path.display()
            );
            continue;
        }
        if let Some(gone) = remove(&worktree, primary, &base, &in_use).await {
            log::info!(
                "removed the worktree {}, unused for {} days",
                worktree.path.display(),
                unused_for.as_secs() / 86_400
            );
            removed.push(gone);
        }
    }
    remove_merged_generated_branches(primary, &base).await;
    removed
}

/// Removes `worktree` after taking it and checking again, just before, that nothing uses it and
/// nothing in it would be lost.
async fn remove(
    worktree: &ManagedWorktree,
    primary: &Path,
    base: &str,
    in_use: &FoldersInUse,
) -> Option<RemovedWorktree> {
    if !take(&worktree.git_dir) {
        return None;
    }
    let still_unused = match in_use.with_other_apps().await {
        Some(in_use_now) => !in_use_now.covers(&worktree.path),
        None => false,
    } && unique_work(&worktree.path, base).await.is_none();
    let submodules = worktree_submodules(&worktree.path).await;
    let Some(submodules) = submodules.filter(|_| still_unused) else {
        release(&worktree.git_dir);
        return None;
    };
    let path_text = worktree.path.to_string_lossy();
    // Git won't remove a worktree with submodules without --force; what they hold was checked.
    // Without submodules, Git's own check stays on.
    let mut args = vec!["worktree", "remove"];
    if !submodules.is_empty() {
        args.push("--force");
    }
    args.push(&path_text);
    if let Err(err) = run_git_command(primary, &args).await {
        log::warn!(
            "could not remove the unused worktree {}: {err}",
            worktree.path.display()
        );
        release(&worktree.git_dir);
        return None;
    }
    Some(RemovedWorktree {
        path: worktree.path.clone(),
        branch: worktree.branch.clone(),
    })
}

/// Deletes the branches Clinch named that no worktree has checked out and whose commits are all on
/// `base`, so nothing is lost.
async fn remove_merged_generated_branches(primary: &Path, base: &str) {
    let Ok(branches) = run_git_command(
        primary,
        &[
            "for-each-ref",
            "--format=%(refname:short) %(objectname)",
            "refs/heads",
        ],
    )
    .await
    else {
        return;
    };
    let Ok(listed) = run_git_command(primary, &["worktree", "list", "--porcelain"]).await else {
        return;
    };
    let checked_out: HashSet<&str> = listed
        .lines()
        .filter_map(|line| line.strip_prefix("branch refs/heads/"))
        .collect();
    for (branch, commit) in branches
        .lines()
        .filter_map(|line| line.trim().split_once(' '))
    {
        if !is_generated_worktree_name(branch) || checked_out.contains(branch) {
            continue;
        }
        let merged = run_git_command(primary, &["merge-base", "--is-ancestor", commit, base])
            .await
            .is_ok();
        if merged {
            delete_branch_at(primary, branch, commit).await;
        }
    }
}

/// The repositories with worktrees in Clinch's worktree folder.
pub(crate) fn repositories_with_worktrees() -> Vec<Repository> {
    repositories_in(&warp_core::paths::data_dir().join("worktrees"))
}

/// The repositories with worktrees in `root`, which holds a folder of worktrees per repository.
fn repositories_in(root: &Path) -> Vec<Repository> {
    let Ok(repo_dirs) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut repositories = HashSet::new();
    for repo_dir in repo_dirs.flatten() {
        let Ok(worktrees) = std::fs::read_dir(repo_dir.path()) else {
            continue;
        };
        // Any one worktree names the repository it belongs to.
        if let Some(primary) = worktrees
            .flatten()
            .find_map(|worktree| primary_checkout_of(&worktree.path()))
        {
            repositories.insert(Repository::with_worktrees_dir(primary, repo_dir.path()));
        }
    }
    repositories.into_iter().collect()
}

/// The main checkout of the repository a linked worktree belongs to: the folder holding the
/// repository's `.git` directory, which the worktree's Git directory sits in.
fn primary_checkout_of(worktree: &Path) -> Option<PathBuf> {
    let git_dir = worktree_git_dir(worktree)?;
    // `<primary>/.git/worktrees/<name>`
    let dot_git = git_dir.parent()?.parent()?;
    if dot_git.file_name()? != ".git" || !dot_git.is_dir() {
        return None;
    }
    Some(canonical(dot_git.parent()?.to_path_buf()))
}

/// Gets worktrees ready for new tabs and removes unused ones on a schedule.
pub(crate) struct ManagedWorktreesModel {
    /// Branch names handed to new tabs, which may not exist in Git yet.
    reserved: HashSet<String>,
    /// Never touches Git: unit tests run with the user's real home folder.
    inert: bool,
}

pub(crate) enum ManagedWorktreesEvent {
    /// The worktree asked for by the new tab whose terminal is `terminal_view_id`.
    Prepared {
        terminal_view_id: EntityId,
        result: Result<PreparedWorktree, String>,
    },
}

impl Entity for ManagedWorktreesModel {
    type Event = ManagedWorktreesEvent;
}

impl SingletonEntity for ManagedWorktreesModel {}

impl ManagedWorktreesModel {
    pub(crate) fn new(ctx: &mut ModelContext<Self>) -> Self {
        Self::clean_up_after(FIRST_CLEANUP_AFTER, ctx);
        Self {
            reserved: HashSet::new(),
            inert: false,
        }
    }

    /// A model that prepares and removes nothing.
    #[cfg(test)]
    pub(crate) fn new_for_test() -> Self {
        Self {
            reserved: HashSet::new(),
            inert: true,
        }
    }

    /// Gets a worktree of `repository` ready for the new tab whose terminal is
    /// `terminal_view_id`, and reports it with [`ManagedWorktreesEvent::Prepared`].
    pub(crate) fn prepare_for(
        &mut self,
        terminal_view_id: EntityId,
        repository: Repository,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.inert {
            return;
        }
        // The panes are read once this update is over, since the terminal asking is part of it.
        ctx.spawn(async {}, move |me, (), ctx| {
            let in_use = folders_in_use(ctx);
            let reserved = me.reserved.clone();
            ctx.spawn(
                prepare_worktree(repository, in_use, reserved),
                move |me, result, ctx| {
                    if let Ok(prepared) = &result {
                        me.reserved.insert(prepared.branch.clone());
                    }
                    ctx.emit(ManagedWorktreesEvent::Prepared {
                        terminal_view_id,
                        result: result.map_err(|err| format!("{err:#}")),
                    });
                },
            );
        });
    }

    fn clean_up_after(delay: Duration, ctx: &mut ModelContext<Self>) {
        ctx.spawn(Timer::after(delay), |me, _, ctx| {
            me.clean_up_now(ctx);
            Self::clean_up_after(CLEANUP_EVERY, ctx);
        });
    }

    fn clean_up_now(&mut self, ctx: &mut ModelContext<Self>) {
        if !*ClinchSettings::as_ref(ctx).remove_unused_worktrees {
            return;
        }
        let in_use = folders_in_use(ctx);
        let repositories = repositories_with_worktrees();
        ctx.spawn(
            async move {
                let mut removed = 0;
                for repository in repositories {
                    removed += clean_up(repository, in_use.clone(), KEPT_FOR_REUSE, REMOVE_AFTER)
                        .await
                        .len();
                }
                removed
            },
            |_, removed, _| {
                if removed > 0 {
                    log::info!("removed {removed} unused worktrees");
                }
            },
        );
    }
}

/// The folders of this app's panes, in every window.
fn folders_in_use(app: &AppContext) -> FoldersInUse {
    FoldersInUse::new(crate::app_state::get_app_state(app).terminal_pane_folders())
}
