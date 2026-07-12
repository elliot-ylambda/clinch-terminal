#!/usr/bin/env bash
# Claude Code capture hook: record the *actual* live session for this Clinch pane so it
# can be resumed on restore. Wired to SessionStart (fresh start, `claude --resume <id>`,
# the interactive picker, and `claude --continue` -- in every case the stdin payload
# carries the real session_id), UserPromptSubmit + Stop for live-mode updates, and
# SessionEnd so an agent the user exited is not resurrected on the next app launch.
#
# Keyed by the pane UUID, so multiple agents in the same directory stay disambiguated.
# Nested Claude/Codex processes inherit the same pane UUID. The registry CLI walks process
# ancestry and lets only the outermost CLI own the pane; nested prompts are still mirrored.
# During app shutdown a marker preserves the outer entry until the final snapshot is durable.
#
# Beyond the session id we also carry forward *how* the session is running -- the permission
# mode (--dangerously-skip-permissions / --permission-mode <mode>) and the --model -- so a
# restored session comes back the same way (e.g. the `CA` alias = claude
# --dangerously-skip-permissions). SessionStart's stdin payload does not include the
# permission mode, so launch capture reads it off the live `claude` process argv (the alias
# expands before exec). Per-turn payloads (UserPromptSubmit, Stop) DO include
# permission_mode, so those events keep the entry in sync with the session's live mode --
# including modes toggled mid-session (shift+tab) and entries that predate flag capture.
#
# Functions are defined unconditionally; the capture body only runs when this file is executed
# (not when sourced by the tests), so the parsing helpers can be unit-tested in isolation.

_clinch_agent_resume_json() {
  local bin
  bin="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  "$bin/agent-json" "$@"
}

_clinch_agent_resume_decode() {
  printf '%s' "$1" | /usr/bin/base64 -D 2>/dev/null
}

# Values below are persisted into a command Clinch later executes in a shell.
# Agent mode/model identifiers are token-like; rejecting shell syntax here keeps
# a malformed or hostile hook payload from turning into a restore-time command.
_clinch_agent_resume_safe_token() {
  [[ "${1:-}" =~ ^[A-Za-z0-9._:/-]+$ ]]
}

# Carry forward the permission mode + model from a flattened `claude` argv string. Pure: takes
# the argv string, prints the extra flags to append to the resume command (leading space, or
# empty). Only mode + model are carried; everything else (incl. a stale --resume) is dropped.
_clinch_agent_resume_extract_flags() {
  local argv="${1:-}"
  local -a toks=()
  read -ra toks <<<"$argv"
  local out="" tok next
  local i=0 n=${#toks[@]}
  while ((i < n)); do
    tok="${toks[i]}"
    next=""
    ((i + 1 < n)) && next="${toks[i + 1]}"
    case "$tok" in
      --dangerously-skip-permissions) out+=" --dangerously-skip-permissions" ;;
      --permission-mode)
        _clinch_agent_resume_safe_token "$next" && { out+=" --permission-mode $next"; i=$((i + 1)); }
        ;;
      --permission-mode=*)
        next="${tok#*=}"; _clinch_agent_resume_safe_token "$next" && out+=" --permission-mode $next"
        ;;
      --model)
        _clinch_agent_resume_safe_token "$next" && { out+=" --model $next"; i=$((i + 1)); }
        ;;
      --model=*)
        next="${tok#*=}"; _clinch_agent_resume_safe_token "$next" && out+=" --model $next"
        ;;
    esac
    i=$((i + 1))
  done
  printf '%s' "$out"
}

# Like _clinch_agent_resume_extract_flags but carries only --model. Used when a hook
# payload already provides the authoritative permission mode.
_clinch_agent_resume_extract_model() {
  local argv="${1:-}"
  local -a toks=()
  read -ra toks <<<"$argv"
  local out="" tok next
  local i=0 n=${#toks[@]}
  while ((i < n)); do
    tok="${toks[i]}"
    next=""
    ((i + 1 < n)) && next="${toks[i + 1]}"
    case "$tok" in
      --model)
        _clinch_agent_resume_safe_token "$next" && { out+=" --model $next"; i=$((i + 1)); }
        ;;
      --model=*)
        next="${tok#*=}"; _clinch_agent_resume_safe_token "$next" && out+=" --model $next"
        ;;
    esac
    i=$((i + 1))
  done
  printf '%s' "$out"
}

# Maps a hook payload permission_mode to resume-command flags. Prints the flag tokens
# (leading space; empty for `default`, which must strip a previously-carried mode).
# Returns 1 for empty/unknown values so the caller can fall back to argv detection.
_clinch_agent_resume_mode_flags_from_payload() {
  case "${1:-}" in
    bypassPermissions) printf ' --dangerously-skip-permissions' ;;
    plan|acceptEdits)  printf ' --permission-mode %s' "$1" ;;
    default)           ;;
    *) return 1 ;;
  esac
}

# Echo the flattened argv of the live `claude` process that owns this hook, by walking up the
# process ancestry from $1 (default $PPID) and returning the first ancestor that actually carries
# one of our carry-over launch flags. Matching on the flags -- not on the string "claude" -- is
# both precise (the only flag-bearing process in the hook's ancestry is the owning claude) and
# fail-safe (a plain `claude` launch matches nothing, so the resume command stays plain).
# Returns empty if none is found. `WARP_AGENT_RESUME_FAKE_ARGV` (set, even to empty) overrides
# the walk -- used by the tests for determinism.
_clinch_agent_resume_claude_argv() {
  if [[ -n "${WARP_AGENT_RESUME_FAKE_ARGV+x}" ]]; then
    printf '%s' "$WARP_AGENT_RESUME_FAKE_ARGV"
    return 0
  fi
  local pid="${1:-$PPID}" args hops=0
  while [[ -n "$pid" && "$pid" -gt 1 && "$hops" -lt 8 ]]; do
    args="$(ps -ww -o args= -p "$pid" 2>/dev/null)"   # -ww: don't truncate a long argv
    case " $args " in
      *" --dangerously-skip-permissions "*|*" --permission-mode "*|*" --permission-mode="*|*" --model "*|*" --model="*)
        printf '%s' "$args"; return 0 ;;
    esac
    pid="$(ps -o ppid= -p "$pid" 2>/dev/null | tr -d '[:space:]')"
    hops=$((hops + 1))
  done
  return 0
}

# True if claude session <id> has a real conversation on disk -- the same test as the
# replay side's clinch_agent_resume_resumable. WARP_AGENT_RESUME_CLAUDE_PROJECTS overrides
# the transcript root (used by the tests).
_clinch_agent_resume_has_conversation() {
  local id="$1" f
  [[ -n "$id" ]] || return 1
  f="$(find "${WARP_AGENT_RESUME_CLAUDE_PROJECTS:-$HOME/.claude/projects}" -name "$id.jsonl" -print -quit 2>/dev/null)"
  [[ -n "$f" ]] && grep -qE '"type":"(user|assistant)"' "$f"
}

# True if the pane's existing registry entry still points somewhere recoverable: a
# claude.ai bridge id (the cloud copy is authoritative for bridged sessions, with or
# without a local transcript) or a local session with a real conversation. Such an entry
# is the pane's only link to that conversation.
_clinch_agent_resume_entry_protected() {
  local entry_file="$1" old_sid
  [[ -f "$entry_file" ]] || return 1
  grep -q '"bridge": "session_' "$entry_file" && return 0
  old_sid="$(sed -nE 's/.*(clinch|warp)_agent_resume_launch claude ([A-Za-z0-9-]+).*/\2/p' "$entry_file")"
  _clinch_agent_resume_has_conversation "$old_sid"
}

# Mirror a user prompt to $DIR/prompts/<sid>.jsonl -- append-only, keyed by session id.
# A session launched with leaked Claude child identity can write NO local transcript at
# all, so without this mirror the prompt text of a lost conversation is unrecoverable from
# disk (the 2026-07-09 incident survived only as truncated OSC log lines). The launch paths
# now scrub that identity, but the mirror deliberately remains unconditional: local-jsonl
# sessions cost only a few KB of redundancy, and this stays cheap corruption insurance for
# nested sessions and any future transcript-persistence failure. See
# specs/claude-transcript-durability.
#
# These files hold the same class of sensitive content as ~/.claude/projects transcripts:
# 700 dir / 600 files, never shipped off-machine. Failure must not fail the hook.
_clinch_agent_resume_mirror_prompt() {
  local payload="$1" sid="$2" cwd="$3"
  local dir="${WARP_AGENT_RESUME_DIR:-$HOME/.warp/agent-resume}/prompts"
  local f="$dir/$sid.jsonl" ts size line
  # Only a non-empty .prompt is worth a line (Stop events never reach here; odd payloads may).
  # agent-json uses macOS's built-in JXA runtime, so capture has no jq/Homebrew dependency.
  ts="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  line="$(printf '%s' "$payload" | _clinch_agent_resume_json prompt-line "$ts" "$cwd" \
    "${CLAUDE_CODE_BRIDGE_SESSION_ID:-}" 2>/dev/null)" || return 0
  [[ -n "$line" ]] || return 0
  mkdir -p "$dir" 2>/dev/null || return 0
  chmod 700 "$dir" 2>/dev/null || true
  # Cap per session: a runaway agent loop must not fill the disk. Past ~5 MB the file gets
  # ONE final truncation marker and nothing further -- the marker itself must not become
  # the new runaway, hence the last-line check before appending it. That check is a [[ ]]
  # on a substitution, NOT `tail | grep -q`: under pipefail a huge last line makes tail
  # die of SIGPIPE when grep exits early, the check would "fail", and the marker would be
  # re-appended on every prompt -- the exact runaway this branch exists to prevent.
  size="$(stat -f %z "$f" 2>/dev/null || echo 0)"
  if (( size > 5242880 )); then
    [[ "$(tail -n 1 "$f" 2>/dev/null)" == *'"truncated":true'* ]] && return 0
    ( umask 077; printf '{"ts":"%s","truncated":true}\n' "$ts" >> "$f" ) 2>/dev/null || true
    return 0
  fi
  ( umask 077; printf '%s\n' "$line" >> "$f" ) 2>/dev/null || true
  return 0
}

_clinch_agent_resume_capture_main() {
  set -uo pipefail
  [[ -n "${WARP_TERMINAL_SESSION_UUID:-}" ]] || return 0   # only act inside a Clinch pane
  local payload fields sid64 cwd64 event64 pmode64 model64
  local sid cwd event pmode extra mode_part entry_file BIN nested=0
  local owner_fields owner_pid owner_tty64 owner_tty
  payload="$(cat)"
  fields="$(printf '%s' "$payload" | _clinch_agent_resume_json hook-fields 2>/dev/null)" || return 0
  IFS='|' read -r sid64 cwd64 event64 pmode64 model64 <<<"$fields"
  sid="$(_clinch_agent_resume_decode "$sid64")" || return 0
  cwd="$(_clinch_agent_resume_decode "$cwd64")" || return 0
  event="$(_clinch_agent_resume_decode "$event64")" || return 0
  pmode="$(_clinch_agent_resume_decode "$pmode64")" || return 0
  [[ "$sid" =~ ^[A-Za-z0-9-]+$ ]] || return 0
  BIN="$(cd "$(dirname "$0")" && pwd)"
  if owner_fields="$("$BIN/clinch-agent-resume" hook-owner-fields 2>/dev/null)"; then
    IFS='|' read -r owner_pid owner_tty64 <<<"$owner_fields"
    owner_tty="$(_clinch_agent_resume_decode "$owner_tty64")" || return 0
  else
    nested=1
  fi
  case "$event" in
    SessionStart)
      # The hook process has one Claude ancestor for a normal top-level session. Two agent
      # ancestors means a Claude/Codex tool launched this session inside the pane; it gets
      # prompt durability but must never replace the visible outer agent's restore target.
      (( nested )) && return 0
      # Fresh capture: a new session in this pane takes over the entry -- EXCEPT when the
      # restore machinery itself spawned it as a fresh fallback
      # (WARP_AGENT_RESUME_STARTED_FRESH, set by clinch_agent_resume_launch). Such a session
      # has no conversation yet, and in the 2026-07-08 incident these blanks overwrote
      # entries still pointing at recoverable conversations on every restart, cascading
      # into data loss. Until the user actually engages (first prompt, handled below), a
      # machinery-spawned blank must not clobber a protected entry.
      entry_file="${WARP_AGENT_RESUME_DIR:-$HOME/.warp/agent-resume}/$WARP_TERMINAL_SESSION_UUID.json"
      if [[ -n "${WARP_AGENT_RESUME_STARTED_FRESH:-}" ]] \
         && ! _clinch_agent_resume_has_conversation "$sid" \
         && _clinch_agent_resume_entry_protected "$entry_file"; then
        return 0
      fi
      extra="$(_clinch_agent_resume_extract_flags "$(_clinch_agent_resume_claude_argv)")"
      ;;
    UserPromptSubmit|Stop)
      # Mirror the prompt BEFORE the pane-ownership guard below: the mirror is keyed by
      # session id (no clobber risk), and a nested claude run's prompts -- exactly the
      # sessions the guard exists to keep out of the pane registry -- deserve durability
      # too. Only the registry write stays behind the guard.
      if [[ "$event" == UserPromptSubmit ]]; then
        _clinch_agent_resume_mirror_prompt "$payload" "$sid" "$cwd"
      fi
      (( nested )) && return 0
      # An outer session is authoritative even if a hook from an older build let a nested
      # child clobber the entry. Its next prompt/Stop event repairs that mapping in place.
      if mode_part="$(_clinch_agent_resume_mode_flags_from_payload "$pmode")"; then
        extra="${mode_part}$(_clinch_agent_resume_extract_model "$(_clinch_agent_resume_claude_argv)")"
      else
        extra="$(_clinch_agent_resume_extract_flags "$(_clinch_agent_resume_claude_argv)")"
      fi
      ;;
    SessionEnd)
      (( nested )) && return 0
      # Graceful app shutdown snapshots before tearing down PTYs. Preserve registry entries
      # across that teardown so restored sibling panes remain claimed during replay; normal
      # user exits have no marker and remove only the session that still owns this pane.
      "$BIN/clinch-agent-resume" app-terminating >/dev/null 2>&1 && return 0
      "$BIN/clinch-agent-resume" remove-if-matches \
        "$WARP_TERMINAL_SESSION_UUID" claude "$sid" >/dev/null 2>&1 || true
      return 0
      ;;
    *) return 0 ;;
  esac
  # Call the registry CLI by absolute path (sibling of this script) so the hook does not
  # depend on the agent inheriting the shell PATH.
  #
  # Also record the claude.ai cloud-copy id. Its cloud copy can include remotely continued
  # turns and is the only durable pane -> cloud-conversation link if the local jsonl is
  # missing or stale. The hook runs as a child of the
  # owning claude process, which exports CLAUDE_CODE_BRIDGE_SESSION_ID once bridged; the
  # per-turn events (UserPromptSubmit/Stop) keep the field fresh if the bridge attaches
  # after SessionStart.
  "$BIN/clinch-agent-resume" write "$WARP_TERMINAL_SESSION_UUID" \
    "clinch_agent_resume_launch claude $sid$extra" "$cwd" \
    "${CLAUDE_CODE_BRIDGE_SESSION_ID:-}" "$owner_pid" "$owner_tty" \
    >/dev/null 2>&1 || true
  return 0
}

# Run the capture only when executed directly; sourcing (tests) just loads the functions.
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  _clinch_agent_resume_capture_main "$@"
fi
