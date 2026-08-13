//! Reads the per-pane agent-resume registry written by the claude wrapper / codex hooks,
//! plus the append-only journal and prompt mirror they maintain.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use unicode_segmentation::UnicodeSegmentation;
use walkdir::WalkDir;

use crate::channel::ChannelState;

/// A CLI-agent provider whose sessions Clinch can restore and inspect locally.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentResumeProvider {
    Claude,
    Codex,
}

impl AgentResumeProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }

    pub fn from_agent_name(agent: &str) -> Option<Self> {
        match agent {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            _ => None,
        }
    }
}

/// One exact user-authored prompt recovered for a provider session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentPrompt {
    pub timestamp: Option<String>,
    pub text: String,
}

/// Ordered prompt history for one provider session.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentPromptHistory {
    pub prompts: Vec<AgentPrompt>,
    /// True when the durable source reports that capture stopped at its safety cap.
    pub is_partial: bool,
}

/// Creates the stable, one-line title used for a CLI-agent session.
pub fn prompt_title(text: &str) -> Option<String> {
    const MAX_GRAPHEMES: usize = 80;

    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }

    let graphemes = UnicodeSegmentation::graphemes(collapsed.as_str(), true).collect::<Vec<_>>();
    let visible_prefix_len = graphemes.len().min(MAX_GRAPHEMES);
    if let Some(sentence_end) = graphemes[..visible_prefix_len]
        .iter()
        .position(|grapheme| matches!(*grapheme, "." | "!" | "?" | "。" | "！" | "？"))
    {
        return Some(graphemes[..=sentence_end].concat());
    }

    if graphemes.len() <= MAX_GRAPHEMES {
        Some(collapsed)
    } else {
        Some(format!(
            "{}…",
            graphemes[..MAX_GRAPHEMES].concat().trim_end()
        ))
    }
}

/// Formats a stored ISO-8601 UTC prompt timestamp as a short local clock time,
/// e.g. "2:32 PM". Returns None when the input is absent or unparseable.
pub fn format_prompt_time_short(timestamp: Option<&str>) -> Option<String> {
    let timestamp = DateTime::parse_from_rfc3339(timestamp?).ok()?;
    Some(
        timestamp
            .with_timezone(&Local)
            .format("%-I:%M %p")
            .to_string(),
    )
}

/// Formats a stored ISO-8601 UTC prompt timestamp as a short local date + time,
/// e.g. "Jul 14, 2:32 PM". Returns None when the input is absent or unparseable.
pub fn format_prompt_time_full(timestamp: Option<&str>) -> Option<String> {
    let timestamp = DateTime::parse_from_rfc3339(timestamp?).ok()?;
    Some(
        timestamp
            .with_timezone(&Local)
            .format("%b %-d, %-I:%M %p")
            .to_string(),
    )
}

/// Reads the best locally available prompt history for a provider session.
///
/// The implementation is intentionally synchronous so callers can place it on the repository's
/// blocking worker pool. Rendering paths must never call it directly.
pub fn read_prompt_history(
    provider: AgentResumeProvider,
    session_id: &str,
    transcript_path: Option<&Path>,
) -> AgentPromptHistory {
    let roots = agent_transcript_roots();
    read_prompt_history_in(
        provider,
        session_id,
        transcript_path,
        registry_dir().as_deref(),
        &roots,
    )
}

#[derive(Deserialize)]
struct RegistryEntry {
    command: String,
    #[serde(default)]
    cwd: Option<String>,
}

const CLINCH_RESUME_LAUNCHER: &str = "clinch_agent_resume_launch";
const LEGACY_WARP_RESUME_LAUNCHER: &str = "warp_agent_resume_launch";

/// Constructs the only restore command accepted from the Remote Control protocol.
///
/// Provider selection is typed and the opaque durable ID must satisfy the same strict identifier
/// grammar as persisted restore commands. This keeps the existing restore/replay machinery while
/// preventing phone-supplied text from being interpreted as shell syntax.
pub(crate) fn build_remote_resume_command(
    provider: AgentResumeProvider,
    durable_session_id: &str,
) -> Option<String> {
    let durable_session_id = durable_session_id.trim();
    if durable_session_id.len() > 256 || !is_safe_session_id(durable_session_id) {
        return None;
    }
    Some(format!(
        "{CLINCH_RESUME_LAUNCHER} {} {durable_session_id}",
        provider.as_str()
    ))
}

fn app_id_enables_runtime(app_id: &str) -> bool {
    matches!(app_id, "sh.clinch.Clinch" | "sh.clinch.ClinchDev")
}

fn runtime_enabled_for(app_id: &str, capture_enabled: bool, explicit_override: bool) -> bool {
    explicit_override || (app_id_enables_runtime(app_id) && capture_enabled)
}

fn runtime_enabled() -> bool {
    #[cfg(target_os = "macos")]
    let capture_enabled = capture_layer_enabled();
    #[cfg(not(target_os = "macos"))]
    let capture_enabled = false;

    runtime_enabled_for(
        &ChannelState::app_id().to_string(),
        capture_enabled,
        std::env::var_os("CLINCH_AGENT_RESUME_ENABLE").is_some(),
    )
}

#[cfg(target_os = "macos")]
fn bundled_capture_installer() -> Option<PathBuf> {
    warp_core::paths::bundled_resources_dir()
        .map(|resources| resources.join("agent-resume").join("install.sh"))
}

#[cfg(target_os = "macos")]
fn default_capture_state_dir(base_config_dir: &Path) -> PathBuf {
    // The provider hooks and helper runtime are shared by Clinch and Clinch Dev, so both apps
    // must also share one persisted setting. The shell installer uses this same production-owned
    // location by default.
    base_config_dir
        .join("sh.clinch.Clinch")
        .join("agent-integration")
}

#[cfg(target_os = "macos")]
fn capture_state_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("CLINCH_AGENT_STATE_DIR") {
        return PathBuf::from(path);
    }
    default_capture_state_dir(&warp_core::paths::base_config_dir())
}

#[cfg(target_os = "macos")]
fn capture_state_file(name: &str) -> PathBuf {
    capture_state_dir().join(name)
}

#[cfg(target_os = "macos")]
fn capture_state_file_is_regular(name: &str) -> bool {
    std::fs::symlink_metadata(capture_state_file(name))
        .is_ok_and(|metadata| metadata.file_type().is_file())
}

/// Returns the persisted Claude/Codex session-capture state.
#[cfg(target_os = "macos")]
pub fn capture_layer_enabled() -> bool {
    capture_state_file_is_regular("enabled")
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CaptureStartupAction {
    Enable,
    Repair,
    Skip,
}

#[cfg(target_os = "macos")]
fn capture_startup_action(
    enabled: bool,
    explicitly_disabled: bool,
    has_legacy_receipt: bool,
) -> CaptureStartupAction {
    if enabled {
        CaptureStartupAction::Repair
    } else if explicitly_disabled || has_legacy_receipt {
        // Older builds kept their receipt after `disable` but did not write a disabled marker.
        // Treat that state as an opt-out so an upgrade never reverses the user's choice.
        CaptureStartupAction::Skip
    } else {
        CaptureStartupAction::Enable
    }
}

#[cfg(target_os = "macos")]
fn capture_startup_action_from_disk() -> CaptureStartupAction {
    capture_startup_action(
        capture_layer_enabled(),
        capture_state_file_is_regular("disabled"),
        capture_state_file_is_regular("receipt"),
    )
}

#[cfg(target_os = "macos")]
fn capture_installer_failure(status: std::process::ExitStatus, stderr: &[u8]) -> String {
    const MAX_DETAIL_CHARS: usize = 400;

    let detail = String::from_utf8_lossy(stderr)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if detail.is_empty() {
        return format!("the session-capture installer exited with {status}");
    }

    let detail = if detail.chars().count() > MAX_DETAIL_CHARS {
        format!(
            "{}…",
            detail.chars().take(MAX_DETAIL_CHARS).collect::<String>()
        )
    } else {
        detail
    };
    format!("{detail} ({status})")
}

#[cfg(target_os = "macos")]
fn run_capture_installer(command: &str) -> Result<(), String> {
    use command::blocking::Command;

    let installer = bundled_capture_installer()
        .filter(|path| path.is_file())
        .ok_or_else(|| "the Clinch session-capture installer is missing".to_owned())?;
    let output = Command::new("/bin/bash")
        .arg(installer)
        .arg(command)
        .arg("--quiet")
        .output()
        .map_err(|error| format!("could not run the session-capture installer: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(capture_installer_failure(output.status, &output.stderr))
    }
}

/// Enables or disables the bundled capture integration after a direct settings action.
#[cfg(target_os = "macos")]
pub fn set_capture_layer_enabled(enabled: bool) -> Result<(), String> {
    run_capture_installer(if enabled { "enable" } else { "disable" })?;

    let actual = capture_layer_enabled();
    if actual != enabled {
        return Err(format!(
            "the session-capture installer did not persist the requested {} state",
            if enabled { "enabled" } else { "disabled" }
        ));
    }
    Ok(())
}

/// Enables the bundled capture hooks on first launch, refreshes them while enabled, and honors a
/// persisted opt-out.
///
/// This is intentionally fail-open: a hand-edited third-party config must never prevent the
/// terminal itself from launching.
#[cfg(target_os = "macos")]
pub fn install_bundled_capture_layer() {
    if !app_id_enables_runtime(&ChannelState::app_id().to_string()) {
        return;
    }

    let action = capture_startup_action_from_disk();
    let command = match action {
        CaptureStartupAction::Enable => "enable",
        CaptureStartupAction::Repair => "repair",
        CaptureStartupAction::Skip => return,
    };

    if !bundled_capture_installer().is_some_and(|path| path.is_file()) {
        // Expected for unbundled local/test binaries.
        return;
    }

    // A graceful previous shutdown intentionally left this marker while PTYs emitted
    // SessionEnd. It must be gone before the first restored/new agent can exit.
    clear_app_terminating_marker();

    if let Err(error) = run_capture_installer(command) {
        eprintln!("clinch: Claude/Codex session-capture {command} failed: {error}");
    } else if action == CaptureStartupAction::Enable && !capture_layer_enabled() {
        eprintln!("clinch: Claude/Codex session capture did not persist its enabled state");
    }
}

/// One line of the append-only registry journal (`journal.jsonl`), written by
/// `clinch-agent-resume` on every `write`/`remove`. `remove` lines omit command/cwd/bridge.
#[derive(Deserialize)]
struct JournalRecord {
    ts: String,
    op: String,
    #[serde(default)]
    pane: Option<String>,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    bridge: Option<String>,
}

/// One line of a provider prompt-mirror file (`prompts/<provider>/<sid>.jsonl`), written by
/// the opted-in prompt hooks. The final line of a capped file is a bare
/// `{"truncated":true}` marker, so every field must tolerate being absent.
#[derive(Clone, Deserialize)]
struct PromptMirrorRecord {
    #[serde(default)]
    ts: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    bridge: Option<String>,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    stop: bool,
    #[serde(default)]
    truncated: bool,
}

/// A ready-to-run "fork this session" command plus the directory it should run in.
/// Derived from the resume command the capture scripts already store.
pub struct ForkLaunch {
    pub command: String,
    pub cwd: Option<String>,
}

fn registry_dir() -> Option<PathBuf> {
    if let Some(override_dir) = std::env::var_os("WARP_AGENT_RESUME_DIR") {
        return Some(PathBuf::from(override_dir));
    }

    warp_core::paths::warp_home_config_dir().map(|dir| dir.join("agent-resume"))
}

const ACTIVE_PANES_FILE: &str = "active-panes";
const APP_TERMINATING_FILE: &str = ".app-terminating";
const CONVERSATION_BOOKMARKS_FILE: &str = "conversation-bookmarks.json";
const TOOLBELT_LEARNING_FILE: &str = "toolbelt-learning.json";
const TOOLBELT_RESOLUTIONS_FILE: &str = "toolbelt-learning-resolutions.json";
const MAX_TOOLBELT_LEARNING_BYTES: u64 = 4 * 1024 * 1024;
const TOMBSTONES_DIR: &str = "tombstones";

/// A safe, aggregate quick-insert candidate learned by the local prompt-capture hook.
/// Raw transcript/session contents are intentionally not part of this API.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LearnedToolbeltSuggestion {
    pub id: String,
    pub text: String,
    pub conversation_count: usize,
    pub providers: Vec<String>,
    pub last_seen: String,
}

#[derive(Default, Deserialize)]
struct ToolbeltLearningFile {
    #[serde(default)]
    schema: u32,
    #[serde(default)]
    patterns: Vec<ToolbeltLearningPattern>,
}

#[derive(Deserialize)]
struct ToolbeltLearningPattern {
    #[serde(default)]
    id: String,
    #[serde(default)]
    sessions: Vec<String>,
    #[serde(default)]
    conversation_count: usize,
    #[serde(default)]
    providers: Vec<String>,
    #[serde(default)]
    last_seen: String,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ToolbeltSuggestionResolutionOutcome {
    Accepted,
    Declined,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct ToolbeltSuggestionResolution {
    suggestion_id: String,
    outcome: ToolbeltSuggestionResolutionOutcome,
    resolved_at: String,
}

#[derive(Default, Serialize, Deserialize)]
struct ToolbeltSuggestionResolutionsFile {
    #[serde(default)]
    schema: u32,
    #[serde(default)]
    resolutions: Vec<ToolbeltSuggestionResolution>,
}

/// Whether cross-conversation quick-insert learning is active for this Clinch user.
pub fn toolbelt_learning_enabled() -> bool {
    #[cfg(target_os = "macos")]
    {
        capture_layer_enabled()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

fn read_toolbelt_resolutions_in(dir: &Path) -> ToolbeltSuggestionResolutionsFile {
    let Some(contents) = read_small_regular_file(
        &dir.join(TOOLBELT_RESOLUTIONS_FILE),
        MAX_TOOLBELT_LEARNING_BYTES,
    ) else {
        return ToolbeltSuggestionResolutionsFile::default();
    };
    serde_json::from_str(&contents).unwrap_or_default()
}

fn learned_toolbelt_suggestions_in(dir: &Path) -> Vec<LearnedToolbeltSuggestion> {
    let Some(contents) = read_small_regular_file(
        &dir.join(TOOLBELT_LEARNING_FILE),
        MAX_TOOLBELT_LEARNING_BYTES,
    ) else {
        return Vec::new();
    };
    let Ok(file) = serde_json::from_str::<ToolbeltLearningFile>(&contents) else {
        return Vec::new();
    };
    if file.schema != 1 {
        return Vec::new();
    }
    let resolved = read_toolbelt_resolutions_in(dir)
        .resolutions
        .into_iter()
        .map(|resolution| resolution.suggestion_id)
        .collect::<HashSet<_>>();
    let mut suggestions = file
        .patterns
        .into_iter()
        .filter_map(|pattern| {
            if resolved.contains(&pattern.id) || uuid::Uuid::parse_str(&pattern.id).is_err() {
                return None;
            }
            let text = pattern.text.filter(|text| !text.trim().is_empty())?;
            let observed_conversation_count = pattern
                .sessions
                .into_iter()
                .filter(|session| {
                    session.split_once(':').is_some_and(|(provider, id)| {
                        matches!(provider, "claude" | "codex") && is_safe_session_id(id)
                    })
                })
                .collect::<HashSet<_>>()
                .len();
            let conversation_count = pattern.conversation_count.max(observed_conversation_count);
            if conversation_count < 2 {
                return None;
            }
            let mut providers = pattern
                .providers
                .into_iter()
                .filter(|provider| matches!(provider.as_str(), "claude" | "codex"))
                .collect::<Vec<_>>();
            providers.sort_unstable();
            providers.dedup();
            Some(LearnedToolbeltSuggestion {
                id: pattern.id,
                text,
                conversation_count,
                providers,
                last_seen: pattern.last_seen,
            })
        })
        .collect::<Vec<_>>();
    suggestions.sort_by(|left, right| {
        right
            .conversation_count
            .cmp(&left.conversation_count)
            .then_with(|| right.last_seen.cmp(&left.last_seen))
    });
    suggestions.truncate(3);
    suggestions
}

fn read_small_regular_file(path: &Path, max_bytes: u64) -> Option<String> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.file_type().is_file() || metadata.len() > max_bytes {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

fn toolbelt_learning_pattern_ids_in(dir: &Path) -> HashSet<String> {
    let Some(contents) = read_small_regular_file(
        &dir.join(TOOLBELT_LEARNING_FILE),
        MAX_TOOLBELT_LEARNING_BYTES,
    ) else {
        return HashSet::new();
    };
    serde_json::from_str::<ToolbeltLearningFile>(&contents)
        .ok()
        .filter(|file| file.schema == 1)
        .map(|file| {
            file.patterns
                .into_iter()
                .filter_map(|pattern| {
                    uuid::Uuid::parse_str(&pattern.id)
                        .is_ok()
                        .then_some(pattern.id)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Returns at most three eligible unresolved candidates, ordered by recurrence and recency.
pub fn learned_toolbelt_suggestions() -> Vec<LearnedToolbeltSuggestion> {
    let dir = registry_dir();
    learned_toolbelt_suggestions_for_state(toolbelt_learning_enabled(), dir.as_deref())
}

fn learned_toolbelt_suggestions_for_state(
    enabled: bool,
    dir: Option<&Path>,
) -> Vec<LearnedToolbeltSuggestion> {
    if !enabled {
        return Vec::new();
    }
    dir.map(learned_toolbelt_suggestions_in).unwrap_or_default()
}

fn resolve_toolbelt_suggestion_in(
    dir: &Path,
    suggestion_id: &str,
    outcome: ToolbeltSuggestionResolutionOutcome,
) -> std::io::Result<()> {
    if uuid::Uuid::parse_str(suggestion_id).is_err() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "eligible toolbelt suggestion was not found",
        ));
    }
    let mut file = read_toolbelt_resolutions_in(dir);
    let current_ids = toolbelt_learning_pattern_ids_in(dir);
    file.resolutions
        .retain(|resolution| current_ids.contains(&resolution.suggestion_id));
    if let Some(existing) = file
        .resolutions
        .iter()
        .find(|resolution| resolution.suggestion_id == suggestion_id)
    {
        if existing.outcome == outcome {
            return Ok(());
        }
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "toolbelt suggestion is already resolved",
        ));
    }
    if !learned_toolbelt_suggestions_in(dir)
        .iter()
        .any(|suggestion| suggestion.id == suggestion_id)
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "eligible toolbelt suggestion was not found",
        ));
    }
    file.schema = 1;
    file.resolutions.push(ToolbeltSuggestionResolution {
        suggestion_id: suggestion_id.to_owned(),
        outcome,
        resolved_at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
    });
    let contents = serde_json::to_vec_pretty(&file).map_err(std::io::Error::other)?;
    write_private_atomic(dir, TOOLBELT_RESOLUTIONS_FILE, &contents)
}

/// Permanently accepts or declines an eligible learned suggestion for this user.
pub fn resolve_toolbelt_suggestion(suggestion_id: &str, accepted: bool) -> std::io::Result<()> {
    if !toolbelt_learning_enabled() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "Claude Code and Codex session capture is disabled",
        ));
    }
    let dir = registry_dir().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "agent conversation registry is unavailable",
        )
    })?;
    resolve_toolbelt_suggestion_in(
        &dir,
        suggestion_id,
        if accepted {
            ToolbeltSuggestionResolutionOutcome::Accepted
        } else {
            ToolbeltSuggestionResolutionOutcome::Declined
        },
    )
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct ConversationBookmark {
    provider: AgentResumeProvider,
    session_id: String,
}

#[derive(Default, Serialize, Deserialize)]
struct ConversationBookmarksFile {
    #[serde(default)]
    bookmarks: Vec<ConversationBookmark>,
}

fn read_conversation_bookmarks_in(dir: &Path) -> HashSet<ConversationBookmark> {
    let Ok(contents) = std::fs::read_to_string(dir.join(CONVERSATION_BOOKMARKS_FILE)) else {
        return HashSet::new();
    };
    let Ok(file) = serde_json::from_str::<ConversationBookmarksFile>(&contents) else {
        return HashSet::new();
    };
    file.bookmarks
        .into_iter()
        .filter(|bookmark| is_safe_session_id(&bookmark.session_id))
        .collect()
}

fn write_conversation_bookmarks_in(
    dir: &Path,
    bookmarks: &HashSet<ConversationBookmark>,
) -> std::io::Result<()> {
    let mut bookmarks = bookmarks.iter().cloned().collect::<Vec<_>>();
    bookmarks.sort_by(|left, right| {
        left.provider
            .as_str()
            .cmp(right.provider.as_str())
            .then_with(|| left.session_id.cmp(&right.session_id))
    });
    let contents = serde_json::to_vec_pretty(&ConversationBookmarksFile { bookmarks })
        .map_err(std::io::Error::other)?;
    write_private_atomic(dir, CONVERSATION_BOOKMARKS_FILE, &contents)
}

/// Returns whether a durable Claude Code or Codex conversation is bookmarked.
pub fn is_conversation_bookmarked(provider: AgentResumeProvider, session_id: &str) -> bool {
    let Some(dir) = registry_dir() else {
        return false;
    };
    read_conversation_bookmarks_in(&dir).contains(&ConversationBookmark {
        provider,
        session_id: session_id.to_owned(),
    })
}

/// Toggles a durable conversation bookmark and returns its new state.
pub fn toggle_conversation_bookmark(
    provider: AgentResumeProvider,
    session_id: &str,
) -> std::io::Result<bool> {
    if !is_safe_session_id(session_id) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid agent conversation session id",
        ));
    }
    let dir = registry_dir().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "agent conversation registry is unavailable",
        )
    })?;
    toggle_conversation_bookmark_in(&dir, provider, session_id)
}

/// Sets a durable conversation bookmark to an explicit state and returns that state.
///
/// This is intentionally separate from [`toggle_conversation_bookmark`]: drop targets and other
/// idempotent affordances must never remove an existing bookmark when they are activated twice.
pub fn set_conversation_bookmark(
    provider: AgentResumeProvider,
    session_id: &str,
    is_bookmarked: bool,
) -> std::io::Result<bool> {
    if !is_safe_session_id(session_id) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid agent conversation session id",
        ));
    }
    let dir = registry_dir().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "agent conversation registry is unavailable",
        )
    })?;
    set_conversation_bookmark_in(&dir, provider, session_id, is_bookmarked)
}

fn toggle_conversation_bookmark_in(
    dir: &Path,
    provider: AgentResumeProvider,
    session_id: &str,
) -> std::io::Result<bool> {
    if !is_safe_session_id(session_id) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid agent conversation session id",
        ));
    }
    let mut bookmarks = read_conversation_bookmarks_in(dir);
    let bookmark = ConversationBookmark {
        provider,
        session_id: session_id.to_owned(),
    };
    let is_bookmarked = if bookmarks.remove(&bookmark) {
        false
    } else {
        bookmarks.insert(bookmark);
        true
    };
    write_conversation_bookmarks_in(dir, &bookmarks)?;
    Ok(is_bookmarked)
}

fn set_conversation_bookmark_in(
    dir: &Path,
    provider: AgentResumeProvider,
    session_id: &str,
    is_bookmarked: bool,
) -> std::io::Result<bool> {
    if !is_safe_session_id(session_id) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid agent conversation session id",
        ));
    }
    let mut bookmarks = read_conversation_bookmarks_in(dir);
    let bookmark = ConversationBookmark {
        provider,
        session_id: session_id.to_owned(),
    };
    let changed = if is_bookmarked {
        bookmarks.insert(bookmark)
    } else {
        bookmarks.remove(&bookmark)
    };
    if changed {
        write_conversation_bookmarks_in(dir, &bookmarks)?;
    }
    Ok(is_bookmarked)
}

/// Atomically publish the pane UUIDs in the snapshot Clinch is about to persist. Replay
/// consults this manifest instead of treating every historical registry file as a live
/// ownership claim.
pub fn write_active_pane_manifest(app_state: &crate::app_state::AppState) {
    if !runtime_enabled() {
        return;
    }
    let Some(dir) = registry_dir() else { return };
    if let Err(err) = write_active_pane_manifest_in(&dir, &app_state.terminal_pane_uuids()) {
        log::warn!("could not update agent-resume active pane manifest: {err}");
    }
}

fn write_active_pane_manifest_in(dir: &Path, uuids: &[Vec<u8>]) -> std::io::Result<()> {
    let mut pane_ids = uuids.iter().map(hex::encode).collect::<Vec<_>>();
    pane_ids.sort_unstable();
    pane_ids.dedup();
    let contents = if pane_ids.is_empty() {
        String::new()
    } else {
        format!("{}\n", pane_ids.join("\n"))
    };
    write_private_atomic(dir, ACTIVE_PANES_FILE, contents.as_bytes())
}

fn write_private_atomic(dir: &Path, name: &str, contents: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let temp = dir.join(format!(".{name}.tmp.{}", std::process::id()));
    std::fs::write(&temp, contents)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(temp, dir.join(name))
}

/// Preserve live registry entries while graceful app shutdown sends SIGHUP to agent PTYs.
/// SessionEnd hooks remove entries for normal user exits, but skip removal while this marker
/// exists. The next Clinch launch clears it before any pane can start.
pub fn mark_app_terminating() {
    if !runtime_enabled() {
        return;
    }
    let Some(dir) = registry_dir() else { return };
    if let Err(err) = write_private_atomic(
        &dir,
        APP_TERMINATING_FILE,
        format!("{}\n", std::process::id()).as_bytes(),
    ) {
        log::warn!("could not mark agent-resume app shutdown: {err}");
    }
}

fn clear_app_terminating_marker() {
    let Some(dir) = registry_dir() else { return };
    match std::fs::remove_file(dir.join(APP_TERMINATING_FILE)) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => log::warn!("could not clear agent-resume app shutdown marker: {err}"),
    }
}

fn read_entry_in(dir: &Path, uuid_hex: &str) -> Option<RegistryEntry> {
    let path = dir.join(format!("{uuid_hex}.json"));
    let contents = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&contents).ok()
}

fn read_command_in(dir: &Path, uuid_hex: &str) -> Option<String> {
    Some(normalize_restore_command(
        read_entry_in(dir, uuid_hex)?.command,
    ))
}

/// Rebrand persisted commands written by older Clinch builds at the read boundary. This makes
/// existing panes display and execute the Clinch launcher immediately, while the shell keeps a
/// legacy alias as a fallback for commands restored by an older app build.
pub(crate) fn normalize_restore_command(command: String) -> String {
    let trimmed = command.trim();
    if trimmed == LEGACY_WARP_RESUME_LAUNCHER {
        return CLINCH_RESUME_LAUNCHER.to_string();
    }
    let Some(rest) = trimmed.strip_prefix("warp_agent_resume_launch ") else {
        return command;
    };
    format!("{CLINCH_RESUME_LAUNCHER} {rest}")
}

/// A stored resume command (`clinch_agent_resume_launch <agent> <id> [flags…]`) split into
/// its parts. The legacy Warp-prefixed form remains readable while saved registries migrate.
/// `flags` carries a leading space when non-empty so it can be appended verbatim.
struct LaunchCommand<'a> {
    agent: &'a str,
    id: &'a str,
    flags: String,
}

fn parse_launch_command(command: &str) -> Option<LaunchCommand<'_>> {
    let command = command.trim();
    let rest = command
        .strip_prefix("clinch_agent_resume_launch ")
        .or_else(|| command.strip_prefix("warp_agent_resume_launch "))?;
    let mut parts = rest.split_whitespace();
    let agent = parts.next()?;
    let id = parts.next()?;
    let flags = parts.collect::<Vec<_>>().join(" ");
    let flags = if flags.is_empty() {
        String::new()
    } else {
        format!(" {flags}")
    };
    Some(LaunchCommand { agent, id, flags })
}

/// Extracts the durable provider/session identity from a stored pane resume command.
///
/// Both the current Clinch launcher and the legacy Warp-prefixed launcher remain readable so
/// snapshots created by older Clinch builds can hydrate history before the resumed agent emits a
/// new event. Unknown providers and malformed session identifiers are rejected.
pub fn agent_session_seed_from_restore_command(
    command: &str,
) -> Option<(AgentResumeProvider, String)> {
    let launch = parse_launch_command(command)?;
    let provider = AgentResumeProvider::from_agent_name(launch.agent)?;
    is_safe_session_id(launch.id).then(|| (provider, launch.id.to_string()))
}

fn is_safe_session_id(session_id: &str) -> bool {
    !session_id.is_empty()
        && session_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

/// Turns a stored resume command (`clinch_agent_resume_launch <agent> <id> [flags…]`) into
/// a fork command, carrying the session's launch flags into the fork. Returns `None` for
/// commands we don't know how to fork (the only forkable agents today are Claude and
/// Codex). The legacy Warp-prefixed form remains supported.
fn derive_fork_command(command: &str) -> Option<String> {
    let LaunchCommand { agent, id, flags } = parse_launch_command(command)?;
    match agent {
        "claude" => Some(format!("claude --resume {id}{flags} --fork-session")),
        "codex" => Some(format!("codex fork {id}{flags}")),
        _ => None,
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

/// Reconcile a persisted command with the mutable registry at launch. The registry may be
/// newer than SQLite when an agent starts immediately before quit, while an explicit journaled
/// remove means an older SQLite command must not resurrect an agent the user already exited.
pub fn resolve_on_restore_command(
    uuid: &[u8],
    persisted_command: Option<String>,
) -> Option<String> {
    if !runtime_enabled() {
        return persisted_command.map(normalize_restore_command);
    }
    let Some(dir) = registry_dir() else {
        return persisted_command.map(normalize_restore_command);
    };
    let uuid_hex = hex::encode(uuid);
    resolve_on_restore_command_in(&dir, &uuid_hex, persisted_command)
}

fn resolve_on_restore_command_in(
    dir: &Path,
    uuid_hex: &str,
    persisted_command: Option<String>,
) -> Option<String> {
    if let Some(command) = read_command_in(dir, uuid_hex) {
        return Some(command);
    }
    if dir.join(TOMBSTONES_DIR).join(uuid_hex).is_file() {
        return None;
    }
    if latest_journal_op_for_pane(dir, uuid_hex).as_deref() == Some("remove") {
        return None;
    }
    persisted_command.map(normalize_restore_command)
}

fn latest_journal_op_for_pane(dir: &Path, uuid_hex: &str) -> Option<String> {
    let contents = std::fs::read_to_string(dir.join("journal.jsonl")).ok()?;
    contents.lines().rev().find_map(|line| {
        let record = serde_json::from_str::<JournalRecord>(line).ok()?;
        (record.pane.as_deref() == Some(uuid_hex)).then_some(record.op)
    })
}

/// Returns the fork launch (command + cwd) for `uuid`, if the pane has a forkable
/// agent session in the registry.
pub fn read_fork_launch(uuid: &[u8]) -> Option<ForkLaunch> {
    let dir = registry_dir()?;
    read_fork_launch_in(&dir, &hex::encode(uuid))
}

/// A past CLI-agent conversation registered to a Clinch pane. The append-only journal
/// provides session identity and provider, while prompt mirrors and native agent
/// transcripts enrich matching sessions with their first prompt.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentConversation {
    /// `claude` or `codex`, as recorded by the Clinch registry journal.
    pub agent: String,
    pub session_id: String,
    /// The conversation's recorded working directory, if any sighting carried one.
    pub cwd: Option<String>,
    /// The claude.ai cloud-copy id (`session_*`) once the session bridged, latest wins.
    pub bridge: Option<String>,
    /// ISO-8601 UTC timestamp of the conversation's first sighting (journal write or
    /// first mirrored prompt, whichever is earlier).
    pub start_ts: String,
    /// Single-line excerpt of the first user prompt, if it can be recovered from a prompt
    /// mirror or the agent's native transcript.
    pub first_prompt: Option<String>,
    /// Whether the provider's native transcript contains a locally resumable user turn.
    /// A bridged Claude conversation can have both this and `bridge`; local resume wins so the
    /// reopened terminal paints the original visible history.
    pub local_resumable: bool,
    /// Launch flags recorded with the latest journal write (leading space when
    /// non-empty), e.g. ` --dangerously-skip-permissions --model opus`.
    pub flags: String,
    /// Whether this durable provider/session identity is saved in the conversation finder.
    pub bookmarked: bool,
}

impl AgentConversation {
    /// The command that reopens this conversation in a fresh pane, mirroring the
    /// priority pane restore uses (`clinch_agent_resume_launch` in claude.zsh): a Claude
    /// session with a usable local transcript resumes locally so its visible history is
    /// repainted; a bridge is the fallback for cloud-only sessions. Launch flags are forwarded
    /// either way. Unlike pane restore there is
    /// deliberately no adopt/fresh fallback: the user picked *this* conversation, so a
    /// dead id should fail visibly in the pane instead of silently opening another
    /// session. Returns `None` for agents we don't know how to resume.
    pub fn reopen_command(&self) -> Option<String> {
        let AgentConversation {
            agent,
            session_id,
            bridge,
            local_resumable,
            flags,
            ..
        } = self;
        match agent.as_str() {
            // Only claude.ai-shaped bridge ids are teleported, matching the shell
            // replay's `[[ "$bridge" == session_* ]]` guard; anything else a corrupt or
            // hand-edited record might contain falls back to local resume.
            "claude" if *local_resumable => Some(format!("claude --resume {session_id}{flags}")),
            "claude" => Some(match bridge {
                Some(bridge) if bridge.starts_with("session_") => {
                    format!("claude --teleport {bridge}{flags}")
                }
                Some(_) | None => format!("claude --resume {session_id}{flags}"),
            }),
            "codex" => Some(format!("codex resume {session_id}{flags}")),
            _ => None,
        }
    }
}

/// One sighting of a session while scanning the journal / prompt mirror.
struct ConversationSighting {
    ts: String,
    session_id: String,
    agent: Option<String>,
    cwd: Option<String>,
    bridge: Option<String>,
    clear_bridge: Option<String>,
    flags: Option<String>,
}

/// Returns up to `limit` known conversations, newest first (by first sighting). Missing
/// or unreadable journal/mirror files simply contribute nothing, so an empty registry
/// yields an empty list rather than an error.
pub fn recent_conversations(limit: usize) -> Vec<AgentConversation> {
    match registry_dir() {
        Some(dir) => recent_conversations_in(&dir, limit),
        None => Vec::new(),
    }
}

fn recent_conversations_in(dir: &Path, limit: usize) -> Vec<AgentConversation> {
    recent_conversations_including_bookmarks(dir, limit, &HashSet::new())
}

/// Returns the normal recent pool plus every still-readable bookmarked conversation, even when a
/// bookmark is older than the pool limit. This is intentionally finder-specific so API callers
/// that request an exact recent limit keep their existing behavior.
pub fn conversations_for_finder(limit: usize) -> Vec<AgentConversation> {
    let Some(dir) = registry_dir() else {
        return Vec::new();
    };
    let bookmarks = read_conversation_bookmarks_in(&dir);
    recent_conversations_including_bookmarks(&dir, limit, &bookmarks)
}

/// Returns every still-readable bookmarked conversation, newest first.
///
/// This is the lightweight public entry point used by persistent bookmark surfaces such as the
/// vertical-tabs section. An empty bookmark file returns before scanning the journal or native
/// transcript trees, which keeps the default empty state effectively free for new users.
pub fn bookmarked_conversations() -> Vec<AgentConversation> {
    let Some(dir) = registry_dir() else {
        return Vec::new();
    };
    bookmarked_conversations_in(&dir)
}

fn bookmarked_conversations_in(dir: &Path) -> Vec<AgentConversation> {
    let bookmarks = read_conversation_bookmarks_in(dir);
    if bookmarks.is_empty() {
        return Vec::new();
    }
    recent_conversations_including_bookmarks(dir, 0, &bookmarks)
}

fn recent_conversations_including_bookmarks(
    dir: &Path,
    limit: usize,
    bookmarks: &HashSet<ConversationBookmark>,
) -> Vec<AgentConversation> {
    let mut sightings = Vec::new();
    let mut journal_session_providers = HashMap::new();

    // Journal writes: session id + flags come from the recorded launch command; lines
    // whose command is not a Clinch/legacy resume-launch form (or that are malformed)
    // are skipped rather than treated as errors, mirroring `clinch-agent-resume list`.
    if let Ok(contents) = std::fs::read_to_string(dir.join("journal.jsonl")) {
        for line in contents.lines() {
            let Ok(record) = serde_json::from_str::<JournalRecord>(line) else {
                continue;
            };
            if record.op != "write" && record.op != "scrub-bridge" {
                continue;
            }
            let Some(launch) = record.command.as_deref().and_then(parse_launch_command) else {
                continue;
            };
            if let Some(provider) = AgentResumeProvider::from_agent_name(launch.agent) {
                journal_session_providers.insert(launch.id.to_string(), provider);
            }
            let recorded_bridge = record.bridge.filter(|bridge| !bridge.is_empty());
            let (bridge, clear_bridge) = if record.op == "write" {
                (recorded_bridge, None)
            } else {
                (None, recorded_bridge)
            };
            sightings.push(ConversationSighting {
                ts: record.ts,
                session_id: launch.id.to_string(),
                agent: Some(launch.agent.to_string()),
                cwd: record.cwd.filter(|cwd| !cwd.is_empty()),
                bridge,
                clear_bridge,
                flags: Some(launch.flags),
            });
        }
    }

    // Prompt-mirror files enrich journal-backed sessions with their first prompt and earliest
    // timestamp. Mirror-only sessions are deliberately excluded: global hooks also see nested
    // helpers that were never owned by a Clinch pane, and those entries otherwise swamp the
    // in-app finder. Provider-scoped files are canonical; legacy flat files remain a Claude-only
    // fallback for sessions captured by older Clinch builds.
    let mut first_prompts = HashMap::new();
    collect_conversation_mirrors(
        dir,
        &journal_session_providers,
        &mut first_prompts,
        &mut sightings,
    );

    // Chronological fold, exactly like `clinch-agent-resume list`: the first sighting
    // fixes a conversation's start timestamp and ordering; the latest non-empty
    // cwd/bridge/flags win (the timestamps are uniform `%Y-%m-%dT%H:%M:%SZ` UTC, so
    // lexicographic order is chronological order).
    sightings.sort_by_key(|sighting| sighting.ts.clone());
    let mut order = Vec::new();
    let mut by_session = HashMap::new();
    for sighting in sightings {
        let conversation = by_session
            .entry(sighting.session_id.clone())
            .or_insert_with(|| {
                order.push(sighting.session_id.clone());
                AgentConversation {
                    agent: "claude".to_string(),
                    session_id: sighting.session_id.clone(),
                    cwd: None,
                    bridge: None,
                    start_ts: sighting.ts.clone(),
                    first_prompt: None,
                    local_resumable: false,
                    flags: String::new(),
                    bookmarked: false,
                }
            });
        if let Some(agent) = sighting.agent {
            conversation.agent = agent;
        }
        if let Some(cwd) = sighting.cwd {
            conversation.cwd = Some(cwd);
        }
        if let Some(bridge) = sighting.bridge {
            conversation.bridge = Some(bridge);
        }
        if let Some(clear_bridge) = sighting.clear_bridge {
            if conversation.bridge.as_deref() == Some(clear_bridge.as_str()) {
                conversation.bridge = None;
            }
        }
        if let Some(flags) = sighting.flags {
            conversation.flags = flags;
        }
    }

    let mut conversations: Vec<_> = order
        .into_iter()
        .rev()
        .filter_map(|session_id| {
            let mut conversation = by_session.remove(&session_id)?;
            conversation.first_prompt = first_prompts.remove(&session_id);
            let provider = AgentResumeProvider::from_agent_name(&conversation.agent);
            conversation.bookmarked = provider.is_some_and(|provider| {
                bookmarks.contains(&ConversationBookmark {
                    provider,
                    session_id: conversation.session_id.clone(),
                })
            });
            Some(conversation)
        })
        .enumerate()
        .filter_map(|(index, conversation)| {
            (index < limit || conversation.bookmarked).then_some(conversation)
        })
        .collect();
    enrich_conversations_from_transcripts(&mut conversations, &agent_transcript_roots());
    conversations
}

fn collect_conversation_mirrors(
    dir: &Path,
    journal_session_providers: &HashMap<String, AgentResumeProvider>,
    first_prompts: &mut HashMap<String, String>,
    sightings: &mut Vec<ConversationSighting>,
) {
    for (session_id, provider) in journal_session_providers {
        let mirror = read_scoped_prompt_mirror(dir, *provider, session_id).or_else(|| {
            (*provider == AgentResumeProvider::Claude)
                .then(|| read_legacy_claude_prompt_mirror(dir, session_id))
                .flatten()
        });
        let Some(mirror) = mirror else { continue };

        if let Some(prompt) = mirror.history.prompts.first() {
            first_prompts.insert(session_id.clone(), single_line_excerpt(&prompt.text, 160));
        }
        let Some(record) = mirror.first_record else {
            continue;
        };
        let Some(ts) = record.ts.filter(|ts| !ts.is_empty()) else {
            continue;
        };
        sightings.push(ConversationSighting {
            ts,
            session_id: session_id.clone(),
            agent: None,
            cwd: record.cwd.filter(|cwd| !cwd.is_empty()),
            bridge: record.bridge.filter(|bridge| !bridge.is_empty()),
            clear_bridge: None,
            flags: None,
        });
    }
}

#[derive(Default)]
struct AgentTranscriptRoots {
    claude_projects: Option<PathBuf>,
    codex_sessions: Option<PathBuf>,
}

fn agent_transcript_roots() -> AgentTranscriptRoots {
    let home = dirs::home_dir();
    let claude_config = std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|home| home.join(".claude")));
    let codex_home = std::env::var_os("CODEX_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.map(|home| home.join(".codex")));
    AgentTranscriptRoots {
        claude_projects: claude_config.map(|root| root.join("projects")),
        codex_sessions: codex_home.map(|root| root.join("sessions")),
    }
}

fn read_prompt_history_in(
    provider: AgentResumeProvider,
    session_id: &str,
    transcript_path: Option<&Path>,
    registry: Option<&Path>,
    roots: &AgentTranscriptRoots,
) -> AgentPromptHistory {
    if !is_safe_session_id(session_id) {
        return AgentPromptHistory::default();
    }

    if let Some(registry) = registry {
        if let Some(mirror) = read_scoped_prompt_mirror(registry, provider, session_id) {
            return mirror.history;
        }
        if provider == AgentResumeProvider::Claude {
            if let Some(mirror) = read_legacy_claude_prompt_mirror(registry, session_id) {
                return mirror.history;
            }
        }
    }

    if let Some(path) =
        transcript_path.and_then(|path| safe_provider_transcript(path, provider, session_id, roots))
    {
        return prompt_history_from_transcript(provider, &path);
    }

    find_provider_transcript(provider, session_id, roots)
        .map(|path| prompt_history_from_transcript(provider, &path))
        .unwrap_or_default()
}

fn provider_transcript_root(
    provider: AgentResumeProvider,
    roots: &AgentTranscriptRoots,
) -> Option<&Path> {
    match provider {
        AgentResumeProvider::Claude => roots.claude_projects.as_deref(),
        AgentResumeProvider::Codex => roots.codex_sessions.as_deref(),
    }
}

fn transcript_path_matches_provider(
    path: &Path,
    provider: AgentResumeProvider,
    session_id: &str,
) -> bool {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    match provider {
        AgentResumeProvider::Claude => {
            file_name == format!("{session_id}.jsonl")
                && !path
                    .components()
                    .any(|component| component.as_os_str() == "subagents")
        }
        AgentResumeProvider::Codex => {
            file_name.starts_with("rollout-")
                && file_name
                    .strip_suffix(".jsonl")
                    .is_some_and(|stem| stem.ends_with(&format!("-{session_id}")))
        }
    }
}

/// Accept an event-provided transcript only when its canonical location remains inside the
/// configured provider root and its filename identifies the requested provider session.
fn safe_provider_transcript(
    path: &Path,
    provider: AgentResumeProvider,
    session_id: &str,
    roots: &AgentTranscriptRoots,
) -> Option<PathBuf> {
    let root = std::fs::canonicalize(provider_transcript_root(provider, roots)?).ok()?;
    let path = std::fs::canonicalize(path).ok()?;
    (path.starts_with(&root) && transcript_path_matches_provider(&path, provider, session_id))
        .then_some(path)
}

fn find_provider_transcript(
    provider: AgentResumeProvider,
    session_id: &str,
    roots: &AgentTranscriptRoots,
) -> Option<PathBuf> {
    let root = provider_transcript_root(provider, roots)?;
    if !root.is_dir() {
        return None;
    }
    WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .find(|path| transcript_path_matches_provider(path, provider, session_id))
}

fn prompt_history_from_transcript(
    provider: AgentResumeProvider,
    path: &Path,
) -> AgentPromptHistory {
    match provider {
        AgentResumeProvider::Claude => prompt_history_from_claude_transcript(path),
        AgentResumeProvider::Codex => prompt_history_from_codex_transcript(path),
    }
}

/// Prompt mirrors are the cheapest source for titles, but reopen policy also needs to know whether
/// the provider has a usable native transcript. Enrich title gaps and local-resume availability in
/// one pass per transcript tree when the picker opens.
fn enrich_conversations_from_transcripts(
    conversations: &mut [AgentConversation],
    roots: &AgentTranscriptRoots,
) {
    for (agent, root) in [
        ("claude", roots.claude_projects.as_deref()),
        ("codex", roots.codex_sessions.as_deref()),
    ] {
        let session_ids: HashSet<_> = conversations
            .iter()
            .filter(|conversation| conversation.agent == agent)
            .map(|conversation| conversation.session_id.clone())
            .collect();
        let Some(root) = root.filter(|root| !session_ids.is_empty() && root.is_dir()) else {
            continue;
        };
        let first_prompts = discover_first_prompts(root, agent, session_ids);
        for conversation in conversations
            .iter_mut()
            .filter(|conversation| conversation.agent == agent)
        {
            if let Some(prompt) = first_prompts.get(&conversation.session_id) {
                conversation.local_resumable = true;
                if conversation.first_prompt.is_none() {
                    conversation.first_prompt = Some(prompt.clone());
                }
            }
        }
    }
}

fn discover_first_prompts(
    root: &Path,
    agent: &str,
    mut remaining_session_ids: HashSet<String>,
) -> HashMap<String, String> {
    let mut first_prompts = HashMap::new();
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
    {
        if remaining_session_ids.is_empty() {
            break;
        }
        let path = entry.path();
        let Some(session_id) = transcript_session_id(path, agent, &remaining_session_ids) else {
            continue;
        };
        let prompt = match agent {
            "claude" => first_prompt_from_claude_transcript(path),
            "codex" => first_prompt_from_codex_transcript(path),
            _ => None,
        };
        let Some(prompt) = prompt else { continue };
        remaining_session_ids.remove(&session_id);
        first_prompts.insert(session_id, prompt);
    }
    first_prompts
}

fn transcript_session_id(
    path: &Path,
    agent: &str,
    remaining_session_ids: &HashSet<String>,
) -> Option<String> {
    let file_name = path.file_name()?.to_str()?;
    let stem = file_name.strip_suffix(".jsonl")?;
    match agent {
        "claude" => {
            if path
                .components()
                .any(|component| component.as_os_str() == "subagents")
            {
                return None;
            }
            remaining_session_ids
                .contains(stem)
                .then(|| stem.to_string())
        }
        "codex" if file_name.starts_with("rollout-") => remaining_session_ids
            .iter()
            .find(|session_id| stem.ends_with(&format!("-{session_id}")))
            .cloned(),
        _ => None,
    }
}

const MAX_PROMPT_HISTORY_SOURCE_BYTES: u64 = 5 * 1024 * 1024;

struct BoundedJsonl {
    values: Vec<Value>,
    source_non_empty: bool,
    is_partial: bool,
}

/// Reads at most the same five MiB budget used by the prompt mirrors. Invalid UTF-8 or JSON on
/// one line does not hide later records, but it does mark the result partial so the UI never
/// presents a corrupted source as complete.
fn read_bounded_jsonl(path: &Path) -> Option<BoundedJsonl> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.file_type().is_file() {
        return None;
    }
    let source_non_empty = metadata.len() > 0;
    let source_exceeds_limit = metadata.len() > MAX_PROMPT_HISTORY_SOURCE_BYTES;
    let file = std::fs::File::open(path).ok()?;
    let mut reader = std::io::BufReader::new(file).take(MAX_PROMPT_HISTORY_SOURCE_BYTES);
    let mut values = Vec::new();
    let mut is_partial = source_exceeds_limit;
    let mut line = Vec::new();

    loop {
        line.clear();
        let bytes_read = reader.read_until(b'\n', &mut line).ok()?;
        if bytes_read == 0 {
            break;
        }
        let is_complete_line = line.last() == Some(&b'\n');
        if source_exceeds_limit && reader.limit() == 0 && !is_complete_line {
            // The cap landed in the middle of a record; never parse a truncated prompt body.
            break;
        }
        while matches!(line.last(), Some(b'\n' | b'\r')) {
            line.pop();
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let Ok(line) = std::str::from_utf8(&line) else {
            is_partial = true;
            continue;
        };
        match serde_json::from_str::<Value>(line) {
            Ok(value) => values.push(value),
            Err(_) => is_partial = true,
        }
    }

    Some(BoundedJsonl {
        values,
        source_non_empty,
        is_partial,
    })
}

struct PromptMirrorRead {
    history: AgentPromptHistory,
    first_record: Option<PromptMirrorRecord>,
}

fn prompt_mirror_path(dir: &Path, provider: AgentResumeProvider, session_id: &str) -> PathBuf {
    dir.join("prompts")
        .join(provider.as_str())
        .join(format!("{session_id}.jsonl"))
}

fn legacy_claude_prompt_mirror_path(dir: &Path, session_id: &str) -> PathBuf {
    dir.join("prompts").join(format!("{session_id}.jsonl"))
}

fn path_is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink())
}

fn read_scoped_prompt_mirror(
    dir: &Path,
    provider: AgentResumeProvider,
    session_id: &str,
) -> Option<PromptMirrorRead> {
    let prompts = dir.join("prompts");
    let provider_dir = prompts.join(provider.as_str());
    if path_is_symlink(&prompts) || path_is_symlink(&provider_dir) {
        return None;
    }
    read_prompt_mirror(&prompt_mirror_path(dir, provider, session_id))
}

fn read_legacy_claude_prompt_mirror(dir: &Path, session_id: &str) -> Option<PromptMirrorRead> {
    let prompts = dir.join("prompts");
    if path_is_symlink(&prompts) {
        return None;
    }
    read_prompt_mirror(&legacy_claude_prompt_mirror_path(dir, session_id))
}

/// `Some` means a physically non-empty mirror exists and is therefore canonical, even if all of
/// its records are malformed. An empty file returns `None` so a native transcript can recover it.
fn read_prompt_mirror(path: &Path) -> Option<PromptMirrorRead> {
    let jsonl = read_bounded_jsonl(path)?;
    if !jsonl.source_non_empty {
        return None;
    }

    let mut history = AgentPromptHistory {
        prompts: Vec::new(),
        is_partial: jsonl.is_partial,
    };
    let mut first_record = None;
    let mut latest_prompt_in_open_turn: Option<String> = None;
    for value in jsonl.values {
        let Ok(record) = serde_json::from_value::<PromptMirrorRecord>(value) else {
            history.is_partial = true;
            continue;
        };
        if record.truncated {
            history.is_partial = true;
            continue;
        }
        if record.stop {
            latest_prompt_in_open_turn = None;
            continue;
        }
        let Some(prompt) = record.prompt.as_deref() else {
            history.is_partial = true;
            continue;
        };
        if prompt.trim().is_empty() {
            continue;
        }
        // Task notifications flow through the same prompt hook as typed messages, but they
        // are harness-injected turns, not user history. They also don't close the open turn,
        // so the retry-coalescing state below stays untouched.
        if is_injected_task_notification(prompt) {
            continue;
        }
        // Claude can re-emit an unchanged retained input while the same turn is still in
        // progress (for example after a control sequence or history navigation). Treat those as
        // retries of one semantic submission. A Stop record clears this guard, so deliberately
        // asking the same thing again after an answer remains a distinct turn. Legacy mirrors
        // have no Stop records; coalescing only adjacent identical prompts is the least lossy
        // recovery rule for them.
        if latest_prompt_in_open_turn.as_deref() == Some(prompt) {
            continue;
        }
        if first_record.is_none() {
            first_record = Some(record.clone());
        }
        history.prompts.push(AgentPrompt {
            timestamp: record.ts.clone().filter(|timestamp| !timestamp.is_empty()),
            text: prompt.to_string(),
        });
        latest_prompt_in_open_turn = Some(prompt.to_string());
    }
    Some(PromptMirrorRead {
        history,
        first_record,
    })
}

fn prompt_history_from_claude_transcript(path: &Path) -> AgentPromptHistory {
    let Some(jsonl) = read_bounded_jsonl(path) else {
        return AgentPromptHistory::default();
    };
    let mut prompts = Vec::new();
    let mut latest_prompt_in_open_turn: Option<String> = None;
    for record in jsonl.values {
        if record.get("type").and_then(Value::as_str) == Some("assistant") {
            let has_visible_response = record
                .pointer("/message/content")
                .and_then(Value::as_array)
                .is_some_and(|blocks| {
                    blocks.iter().any(|block| {
                        block.get("type").and_then(Value::as_str) == Some("text")
                            && block
                                .get("text")
                                .and_then(Value::as_str)
                                .is_some_and(|text| !text.trim().is_empty())
                    })
                });
            if has_visible_response {
                latest_prompt_in_open_turn = None;
            }
            continue;
        }
        if record.get("type").and_then(Value::as_str) != Some("user")
            || record.get("isMeta").and_then(Value::as_bool) == Some(true)
        {
            continue;
        }
        let Some(text) = user_content_text(record.pointer("/message/content"), "text") else {
            continue;
        };
        if text.trim().is_empty() {
            continue;
        }
        if is_generated_claude_user_message(&record, &text) {
            continue;
        }
        if latest_prompt_in_open_turn.as_deref() == Some(text.as_str()) {
            continue;
        }
        prompts.push(AgentPrompt {
            timestamp: record
                .get("timestamp")
                .and_then(Value::as_str)
                .filter(|timestamp| !timestamp.is_empty())
                .map(str::to_string),
            text,
        });
        latest_prompt_in_open_turn = prompts.last().map(|prompt| prompt.text.clone());
    }
    AgentPromptHistory {
        prompts,
        is_partial: jsonl.is_partial,
    }
}

fn is_generated_claude_user_message(record: &Value, text: &str) -> bool {
    if record.get("interruptedMessageId").is_some() {
        return true;
    }
    let text = text.trim_start();
    text == "[Request interrupted by user]"
        || text.starts_with("This session is being continued from another machine.")
        || text.starts_with("<local-command-")
        || text.starts_with("<command-name>")
        || is_injected_task_notification(text)
}

/// True when a user-role turn is a harness-injected task notification (background command or
/// subagent completion) rather than text the user typed. These reach every capture path —
/// prompt-submit hook events, the on-disk prompt mirror, and native transcripts — and must
/// never surface as message history.
pub(crate) fn is_injected_task_notification(text: &str) -> bool {
    text.trim_start().starts_with("<task-notification>")
}

fn prompt_history_from_codex_transcript(path: &Path) -> AgentPromptHistory {
    let Some(jsonl) = read_bounded_jsonl(path) else {
        return AgentPromptHistory::default();
    };
    let mut event_prompts = Vec::new();
    let mut response_item_prompts = Vec::new();
    for record in jsonl.values {
        let timestamp = record
            .get("timestamp")
            .and_then(Value::as_str)
            .filter(|timestamp| !timestamp.is_empty())
            .map(str::to_string);
        if record.get("type").and_then(Value::as_str) == Some("event_msg")
            && record.pointer("/payload/type").and_then(Value::as_str) == Some("user_message")
        {
            if let Some(text) = record
                .pointer("/payload/message")
                .and_then(Value::as_str)
                .filter(|text| !text.trim().is_empty())
                .filter(|text| !is_generated_codex_context(text))
            {
                event_prompts.push(AgentPrompt {
                    timestamp,
                    text: text.to_string(),
                });
            }
            continue;
        }

        if record.get("type").and_then(Value::as_str) == Some("response_item")
            && record.pointer("/payload/type").and_then(Value::as_str) == Some("message")
            && record.pointer("/payload/role").and_then(Value::as_str) == Some("user")
        {
            let Some(text) = user_content_text(record.pointer("/payload/content"), "input_text")
            else {
                continue;
            };
            if !text.trim().is_empty() && !is_generated_codex_context(&text) {
                response_item_prompts.push(AgentPrompt { timestamp, text });
            }
        }
    }

    // Current rollouts record the same submission as both a response_item and an event_msg. The
    // event stream is canonical when present; response_item is an all-history fallback for older
    // rollouts. This avoids synthetic deduplication that would erase intentional repeated turns.
    AgentPromptHistory {
        prompts: if event_prompts.is_empty() {
            response_item_prompts
        } else {
            event_prompts
        },
        is_partial: jsonl.is_partial,
    }
}

fn first_prompt_from_claude_transcript(path: &Path) -> Option<String> {
    prompt_history_from_claude_transcript(path)
        .prompts
        .first()
        .and_then(|prompt| non_empty_prompt_excerpt(&prompt.text))
}

fn first_prompt_from_codex_transcript(path: &Path) -> Option<String> {
    prompt_history_from_codex_transcript(path)
        .prompts
        .first()
        .and_then(|prompt| non_empty_prompt_excerpt(&prompt.text))
}

fn user_content_text(content: Option<&Value>, text_block_type: &str) -> Option<String> {
    match content? {
        Value::String(text) => Some(text.clone()),
        Value::Array(blocks) => {
            let text = blocks
                .iter()
                .filter(|block| block.get("type").and_then(Value::as_str) == Some(text_block_type))
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n");
            (!text.is_empty()).then_some(text)
        }
        _ => None,
    }
}

fn is_generated_codex_context(text: &str) -> bool {
    let text = text.trim_start();
    [
        "# AGENTS.md instructions",
        "<environment_context>",
        "<permissions instructions>",
        "<collaboration_mode>",
        "<skills_instructions>",
        "<apps_instructions>",
        "<plugins_instructions>",
    ]
    .iter()
    .any(|prefix| text.starts_with(prefix))
}

fn non_empty_prompt_excerpt(text: &str) -> Option<String> {
    let excerpt = single_line_excerpt(text, 160);
    (!excerpt.is_empty()).then_some(excerpt)
}

/// Collapses a mirrored prompt onto one line and caps its length for display and fuzzy
/// matching (prompts are arbitrary user text — possibly multi-line and huge).
fn single_line_excerpt(text: &str, max_chars: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max_chars {
        return collapsed;
    }
    let truncated = collapsed.chars().take(max_chars).collect::<String>();
    format!("{}…", truncated.trim_end())
}

#[cfg(test)]
#[path = "agent_resume_tests.rs"]
mod tests;
