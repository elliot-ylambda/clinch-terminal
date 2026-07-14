//! Reads the per-pane agent-resume registry written by the claude wrapper / codex hooks,
//! plus the append-only journal and prompt mirror they maintain.

use std::collections::{HashMap, HashSet};
use std::io::BufRead;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;
use walkdir::WalkDir;

use crate::channel::ChannelState;

#[derive(Deserialize)]
struct RegistryEntry {
    command: String,
    #[serde(default)]
    cwd: Option<String>,
}

const CLINCH_RESUME_LAUNCHER: &str = "clinch_agent_resume_launch";
const LEGACY_WARP_RESUME_LAUNCHER: &str = "warp_agent_resume_launch";

fn app_id_enables_runtime(app_id: &str) -> bool {
    matches!(app_id, "sh.clinch.Clinch" | "sh.clinch.ClinchDev")
}

fn runtime_enabled_for(app_id: &str, has_consent: bool, explicit_override: bool) -> bool {
    explicit_override || (app_id_enables_runtime(app_id) && has_consent)
}

fn runtime_enabled() -> bool {
    #[cfg(target_os = "macos")]
    let has_consent = capture_layer_enabled();
    #[cfg(not(target_os = "macos"))]
    let has_consent = false;

    runtime_enabled_for(
        &ChannelState::app_id().to_string(),
        has_consent,
        std::env::var_os("CLINCH_AGENT_RESUME_ENABLE").is_some(),
    )
}

#[cfg(target_os = "macos")]
fn bundled_capture_installer() -> Option<PathBuf> {
    warp_core::paths::bundled_resources_dir()
        .map(|resources| resources.join("agent-resume").join("install.sh"))
}

#[cfg(target_os = "macos")]
fn capture_consent_marker() -> PathBuf {
    if let Some(path) = std::env::var_os("CLINCH_AGENT_STATE_DIR") {
        return PathBuf::from(path).join("enabled");
    }
    warp_core::paths::state_dir()
        .join("agent-integration")
        .join("enabled")
}

/// Returns whether the user has explicitly enabled Claude/Codex session capture.
#[cfg(target_os = "macos")]
pub fn capture_layer_enabled() -> bool {
    std::fs::symlink_metadata(capture_consent_marker())
        .is_ok_and(|metadata| metadata.file_type().is_file())
}

/// Enables or disables the bundled capture integration after a direct settings action.
#[cfg(target_os = "macos")]
pub fn set_capture_layer_enabled(enabled: bool) -> Result<(), String> {
    use std::process::Stdio;

    use command::blocking::Command;

    let installer = bundled_capture_installer()
        .filter(|path| path.is_file())
        .ok_or_else(|| "the Clinch session-capture installer is missing".to_owned())?;
    let status = Command::new("/bin/bash")
        .arg(installer)
        .arg(if enabled { "enable" } else { "disable" })
        .arg("--quiet")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .status()
        .map_err(|error| format!("could not run the session-capture installer: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "the session-capture installer exited with status {status}"
        ))
    }
}

/// Refreshes the capture hooks shipped inside the Clinch app bundle, but only after consent.
///
/// This is intentionally fail-open: a hand-edited third-party config must never prevent the
/// terminal itself from launching. `repair` is a no-op unless the durable consent marker exists.
#[cfg(target_os = "macos")]
pub fn install_bundled_capture_layer() {
    use std::process::Stdio;

    use command::blocking::Command;

    // With no consent, startup must not create, rewrite, or clean up capture state.
    if !capture_layer_enabled() {
        return;
    }

    // A graceful previous shutdown intentionally left this marker while PTYs emitted
    // SessionEnd. It must be gone before the first restored/new agent can exit.
    clear_app_terminating_marker();

    let Some(installer) = bundled_capture_installer().filter(|path| path.is_file()) else {
        // Expected for unbundled local/test binaries.
        return;
    };

    let status = Command::new("/bin/bash")
        .arg(installer)
        .arg("repair")
        .arg("--quiet")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if !status.is_ok_and(|status| status.success()) {
        eprintln!("clinch: enabled Claude/Codex session capture could not be refreshed");
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

/// One line of a prompt-mirror file (`prompts/<sid>.jsonl`), written by
/// `claude-capture.sh` on every user prompt. The final line of a capped file is a bare
/// `{"truncated":true}` marker, so every field must tolerate being absent.
#[derive(Deserialize)]
struct PromptMirrorRecord {
    #[serde(default)]
    ts: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    bridge: Option<String>,
    #[serde(default)]
    prompt: Option<String>,
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
const TOMBSTONES_DIR: &str = "tombstones";

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
    /// Launch flags recorded with the latest journal write (leading space when
    /// non-empty), e.g. ` --dangerously-skip-permissions --model opus`.
    pub flags: String,
}

impl AgentConversation {
    /// The command that reopens this conversation in a fresh pane, mirroring the
    /// priority pane restore uses (`clinch_agent_resume_launch` in claude.zsh): a bridged
    /// claude session teleports its authoritative cloud copy, everything else resumes
    /// locally, and launch flags are forwarded either way. Unlike pane restore there is
    /// deliberately no adopt/fresh fallback: the user picked *this* conversation, so a
    /// dead id should fail visibly in the pane instead of silently opening another
    /// session. Returns `None` for agents we don't know how to resume.
    pub fn reopen_command(&self) -> Option<String> {
        let AgentConversation {
            agent,
            session_id,
            bridge,
            flags,
            ..
        } = self;
        match agent.as_str() {
            // Only claude.ai-shaped bridge ids are teleported, matching the shell
            // replay's `[[ "$bridge" == session_* ]]` guard; anything else a corrupt or
            // hand-edited record might contain falls back to local resume.
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
    let mut sightings = Vec::new();
    let mut journal_claude_session_ids = HashSet::new();

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
            if launch.agent == "claude" {
                journal_claude_session_ids.insert(launch.id.to_string());
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

    // Prompt-mirror files enrich journal-backed Claude sessions with their first-prompt
    // excerpt and earliest timestamp. Mirror-only sessions are deliberately excluded:
    // the global Claude hook also sees nested/background helpers that were never owned
    // by a Clinch pane, and those entries otherwise swamp the in-app finder. Only the
    // first line is read because mirror files are append-only and can grow to ~5 MB.
    let mut first_prompts = HashMap::new();
    if let Ok(entries) = std::fs::read_dir(dir.join("prompts")) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(session_id) = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_suffix(".jsonl"))
                .map(str::to_string)
            else {
                continue;
            };
            if !journal_claude_session_ids.contains(&session_id) {
                continue;
            }
            let Some(line) = read_first_line(&path) else {
                continue;
            };
            let Ok(record) = serde_json::from_str::<PromptMirrorRecord>(&line) else {
                continue;
            };
            let Some(ts) = record.ts else { continue };
            if let Some(prompt) = record.prompt.as_deref() {
                first_prompts.insert(session_id.clone(), single_line_excerpt(prompt, 160));
            }
            sightings.push(ConversationSighting {
                ts,
                session_id,
                agent: None,
                cwd: record.cwd.filter(|cwd| !cwd.is_empty()),
                bridge: record.bridge.filter(|bridge| !bridge.is_empty()),
                clear_bridge: None,
                flags: None,
            });
        }
    }

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
                    flags: String::new(),
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
        .take(limit)
        .filter_map(|session_id| {
            let mut conversation = by_session.remove(&session_id)?;
            conversation.first_prompt = first_prompts.remove(&session_id);
            Some(conversation)
        })
        .collect();
    enrich_missing_first_prompts_from_transcripts(&mut conversations, &agent_transcript_roots());
    conversations
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

/// Prompt mirrors are the cheapest and most precise source, but older Claude sessions and
/// Codex sessions may not have one. Fill only those gaps from the agents' native transcripts.
/// This runs once when the picker opens and scans each transcript tree at most once.
fn enrich_missing_first_prompts_from_transcripts(
    conversations: &mut [AgentConversation],
    roots: &AgentTranscriptRoots,
) {
    for (agent, root) in [
        ("claude", roots.claude_projects.as_deref()),
        ("codex", roots.codex_sessions.as_deref()),
    ] {
        let session_ids: HashSet<_> = conversations
            .iter()
            .filter(|conversation| {
                conversation.agent == agent && conversation.first_prompt.is_none()
            })
            .map(|conversation| conversation.session_id.clone())
            .collect();
        let Some(root) = root.filter(|root| !session_ids.is_empty() && root.is_dir()) else {
            continue;
        };
        let first_prompts = discover_first_prompts(root, agent, session_ids);
        for conversation in conversations.iter_mut().filter(|conversation| {
            conversation.agent == agent && conversation.first_prompt.is_none()
        }) {
            conversation.first_prompt = first_prompts.get(&conversation.session_id).cloned();
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

fn first_prompt_from_claude_transcript(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    for line in std::io::BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(record) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if record.get("type").and_then(Value::as_str) != Some("user")
            || record.get("isMeta").and_then(Value::as_bool) == Some(true)
        {
            continue;
        }
        let Some(text) = user_content_text(record.pointer("/message/content"), "text") else {
            continue;
        };
        if let Some(preview) = non_empty_prompt_excerpt(&text) {
            return Some(preview);
        }
    }
    None
}

fn first_prompt_from_codex_transcript(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut response_item_fallback = None;
    for line in std::io::BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(record) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if record.get("type").and_then(Value::as_str) == Some("event_msg")
            && record.pointer("/payload/type").and_then(Value::as_str) == Some("user_message")
        {
            if let Some(preview) = record
                .pointer("/payload/message")
                .and_then(Value::as_str)
                .and_then(non_empty_prompt_excerpt)
            {
                return Some(preview);
            }
        }

        if response_item_fallback.is_none()
            && record.get("type").and_then(Value::as_str) == Some("response_item")
            && record.pointer("/payload/type").and_then(Value::as_str) == Some("message")
            && record.pointer("/payload/role").and_then(Value::as_str) == Some("user")
        {
            let Some(text) = user_content_text(record.pointer("/payload/content"), "input_text")
            else {
                continue;
            };
            if !is_generated_codex_context(&text) {
                response_item_fallback = non_empty_prompt_excerpt(&text);
            }
        }
    }
    response_item_fallback
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

fn read_first_line(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut line = String::new();
    std::io::BufReader::new(file).read_line(&mut line).ok()?;
    let line = line.trim();
    (!line.is_empty()).then(|| line.to_string())
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
