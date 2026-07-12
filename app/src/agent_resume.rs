//! Reads the per-pane agent-resume registry written by the claude wrapper / codex hooks,
//! plus the append-only journal and prompt mirror they maintain (see
//! specs/claude-transcript-durability/ and
//! docs/superpowers/specs/2026-06-20-warp-agent-session-resume-design.md).

use std::collections::HashMap;
use std::io::BufRead;
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Deserialize)]
struct RegistryEntry {
    command: String,
    #[serde(default)]
    cwd: Option<String>,
}

/// One line of the append-only registry journal (`journal.jsonl`), written by
/// `warp-agent-resume` on every `write`/`remove`. `remove` lines omit command/cwd/bridge.
#[derive(Deserialize)]
struct JournalRecord {
    ts: String,
    op: String,
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
    let home = std::env::var_os("HOME")?;
    Some(Path::new(&home).join(".warp").join("agent-resume"))
}

fn read_entry_in(dir: &Path, uuid_hex: &str) -> Option<RegistryEntry> {
    let path = dir.join(format!("{uuid_hex}.json"));
    let contents = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&contents).ok()
}

fn read_command_in(dir: &Path, uuid_hex: &str) -> Option<String> {
    Some(read_entry_in(dir, uuid_hex)?.command)
}

/// A stored resume command (`warp_agent_resume_launch <agent> <id> [flags…]`) split into
/// its parts. `flags` carries a leading space when non-empty so it can be appended to a
/// derived command verbatim.
struct LaunchCommand<'a> {
    agent: &'a str,
    id: &'a str,
    flags: String,
}

fn parse_launch_command(command: &str) -> Option<LaunchCommand<'_>> {
    let rest = command.trim().strip_prefix("warp_agent_resume_launch ")?;
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

/// Turns a stored resume command (`warp_agent_resume_launch <agent> <id> [flags…]`) into
/// a fork command, carrying the session's launch flags into the fork. Returns `None` for
/// commands we don't know how to fork (the only forkable agents today are Claude and
/// Codex).
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

/// Returns the fork launch (command + cwd) for `uuid`, if the pane has a forkable
/// agent session in the registry.
pub fn read_fork_launch(uuid: &[u8]) -> Option<ForkLaunch> {
    let dir = registry_dir()?;
    read_fork_launch_in(&dir, &hex::encode(uuid))
}

/// A past CLI-agent conversation, aggregated from the append-only registry journal and
/// the prompt mirror. This is the Rust equivalent of `warp-agent-resume list`: the
/// journal keeps every (pane, session, bridge, cwd) tuple ever recorded, and the mirror
/// covers sessions the registry never saw (e.g. nested claude runs).
#[derive(Clone, Debug, PartialEq)]
pub struct AgentConversation {
    /// `claude` or `codex` (mirror-only sessions are always claude — codex has no
    /// prompt mirror).
    pub agent: String,
    pub session_id: String,
    /// The conversation's recorded working directory, if any sighting carried one.
    pub cwd: Option<String>,
    /// The claude.ai cloud-copy id (`session_*`) once the session bridged, latest wins.
    pub bridge: Option<String>,
    /// ISO-8601 UTC timestamp of the conversation's first sighting (journal write or
    /// first mirrored prompt, whichever is earlier).
    pub start_ts: String,
    /// Single-line excerpt of the first mirrored prompt, if the session has a mirror.
    pub first_prompt: Option<String>,
    /// Launch flags recorded with the latest journal write (leading space when
    /// non-empty), e.g. ` --dangerously-skip-permissions --model opus`.
    pub flags: String,
}

impl AgentConversation {
    /// The command that reopens this conversation in a fresh pane, mirroring the
    /// priority pane restore uses (`warp_agent_resume_launch` in claude.zsh): a bridged
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

    // Journal writes: session id + flags come from the recorded launch command; lines
    // whose command is not a `warp_agent_resume_launch` form (or that are malformed)
    // are skipped rather than treated as errors, mirroring `warp-agent-resume list`.
    if let Ok(contents) = std::fs::read_to_string(dir.join("journal.jsonl")) {
        for line in contents.lines() {
            let Ok(record) = serde_json::from_str::<JournalRecord>(line) else {
                continue;
            };
            if record.op != "write" {
                continue;
            }
            let Some(launch) = record.command.as_deref().and_then(parse_launch_command) else {
                continue;
            };
            sightings.push(ConversationSighting {
                ts: record.ts,
                session_id: launch.id.to_string(),
                agent: Some(launch.agent.to_string()),
                cwd: record.cwd.filter(|cwd| !cwd.is_empty()),
                bridge: record.bridge.filter(|bridge| !bridge.is_empty()),
                flags: Some(launch.flags),
            });
        }
    }

    // Prompt-mirror files: cover sessions the registry never recorded, and provide the
    // first-prompt excerpt. Only the first line is read — it is the session's earliest
    // prompt (files are append-only), and mirror files can grow to ~5 MB.
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
                flags: None,
            });
        }
    }

    // Chronological fold, exactly like `warp-agent-resume list`: the first sighting
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
        if let Some(flags) = sighting.flags {
            conversation.flags = flags;
        }
    }

    order
        .into_iter()
        .rev()
        .take(limit)
        .filter_map(|session_id| {
            let mut conversation = by_session.remove(&session_id)?;
            conversation.first_prompt = first_prompts.remove(&session_id);
            Some(conversation)
        })
        .collect()
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
