//! Preflight check for the Codex `codex-code-mode-host` sidecar binary.
//!
//! The Codex CLI spawns a sibling `codex-code-mode-host` binary for every
//! tool call. Homebrew's `codex` cask ships only the `codex` binary, so
//! `brew upgrade --cask codex` silently removes the sidecar and every
//! subsequent tool call fails with "failed to spawn code-mode host". When a
//! Codex session starts we verify the sidecar is reachable and warn once per
//! app run if it is not, turning the silent breakage into an actionable hint.
//!
//! This module holds the pure, testable pieces (PATH construction, binary
//! resolution, and the once-per-run guard); the async orchestration and the
//! toast live in `TerminalView::preflight_codex_code_mode_host`.

use std::env;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::util::path::{file_exists_and_is_executable, resolve_executable_in_path};

/// Name of the sidecar binary the Codex CLI spawns for tool calls.
pub const CODE_MODE_HOST_BINARY: &str = "codex-code-mode-host";

/// Well-known install directories searched in addition to the PATH. The GUI
/// app's process PATH can be minimal and the interactive-shell PATH capture
/// can fail, so these keep the check useful in degraded environments.
const FALLBACK_BIN_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin"];

/// Result of the Codex code-mode-host preflight check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexHostHealth {
    /// `codex` itself was not found, so there is nothing to warn about:
    /// either our PATH view is incomplete or the session will surface its
    /// own failure. Never warn in this state.
    CodexNotFound,
    /// Both `codex` and `codex-code-mode-host` are reachable.
    Healthy,
    /// `codex` exists but `codex-code-mode-host` is missing: every Codex
    /// tool call will fail.
    CodeModeHostMissing,
}

/// Builds the PATH to search: the interactive login-shell PATH when capture
/// succeeded, otherwise the process's own PATH, extended with the well-known
/// install directories from [`FALLBACK_BIN_DIRS`].
pub fn search_path(interactive_path: Option<String>) -> OsString {
    let base = interactive_path
        .map(OsString::from)
        .or_else(|| env::var_os("PATH"))
        .unwrap_or_default();
    let mut dirs: Vec<PathBuf> = env::split_paths(&base).collect();
    for fallback in FALLBACK_BIN_DIRS {
        let fallback = PathBuf::from(fallback);
        if !dirs.contains(&fallback) {
            dirs.push(fallback);
        }
    }
    env::join_paths(dirs).unwrap_or(base)
}

/// Checks that the `codex-code-mode-host` sidecar is reachable for the
/// `codex` binary found on `path_env`.
///
/// The sidecar counts as reachable when it exists and is executable either
/// next to the resolved `codex` binary — after resolving symlinks, since
/// Homebrew installs `codex` as a symlink into its bin directory while the
/// sidecar lives next to the symlink target — or anywhere on `path_env`.
/// Only performs filesystem stats; no processes are spawned.
pub fn check_codex_host(path_env: &OsStr) -> CodexHostHealth {
    let Some(codex) = resolve_executable_in_path("codex", path_env) else {
        return CodexHostHealth::CodexNotFound;
    };

    let canonical = codex.canonicalize().unwrap_or_else(|_| codex.into_owned());
    if let Some(dir) = canonical.parent() {
        if file_exists_and_is_executable(&dir.join(CODE_MODE_HOST_BINARY)) {
            return CodexHostHealth::Healthy;
        }
    }

    // The sidecar may also live in another PATH directory (which includes
    // the directory holding the `codex` symlink itself).
    if resolve_executable_in_path(CODE_MODE_HOST_BINARY, path_env).is_some() {
        return CodexHostHealth::Healthy;
    }

    CodexHostHealth::CodeModeHostMissing
}

/// User-facing warning shown when the sidecar is missing.
pub fn missing_host_message() -> String {
    format!(
        "Codex tool calls will fail: `{CODE_MODE_HOST_BINARY}` was not found next to `codex` or \
         on your PATH. `brew upgrade --cask codex` is known to remove it — reinstall Codex or \
         restore the binary."
    )
}

/// Once-per-app-run guard for the missing-sidecar warning.
#[derive(Debug)]
pub struct WarnOnce(AtomicBool);

impl Default for WarnOnce {
    fn default() -> Self {
        Self::new()
    }
}

impl WarnOnce {
    pub const fn new() -> Self {
        Self(AtomicBool::new(false))
    }

    /// Returns `true` if [`Self::claim`] has already succeeded.
    pub fn claimed(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    /// Claims the warning slot. Returns `true` exactly once; every later
    /// call returns `false`.
    pub fn claim(&self) -> bool {
        !self.0.swap(true, Ordering::Relaxed)
    }
}

/// Global guard ensuring the missing-sidecar warning shows at most once per
/// app run.
pub static CODEX_HOST_WARN_ONCE: WarnOnce = WarnOnce::new();

#[cfg(test)]
#[path = "codex_host_check_tests.rs"]
mod tests;
