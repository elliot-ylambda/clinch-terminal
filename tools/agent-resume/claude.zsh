# Clinch agent-resume shell integration (loaded by the bundled standalone launcher; older
# installs may also source it from ~/.zshrc).
#
# Capture is done by Claude's hooks (claude-capture.sh) and Codex's
# SessionStart hook -- they record the live session per pane. This file only provides the
# *replay* side, the functions Clinch invokes on restore:
#
#   clinch_agent_resume_resumable()   true if an agent session id has a resumable conversation
#   clinch_agent_resume_fallback_id() newest unclaimed resumable session for a directory
#   clinch_agent_resume_launch()      resume if possible, else adopt, else start fresh
#
# On restore Clinch replays the recorded command `clinch_agent_resume_launch <agent> <id>` in
# this (interactive) shell, so these functions are in scope. A fresh fallback calls the
# agent normally, so its SessionStart hook re-captures it for next time.

# A pane shell must never launch Claude with another session's identity in its environment.
# A `make update` launched from inside Claude once leaked stale child/bridge ids through the
# app relaunch; every later session then behaved as a child and skipped its local transcript
# entirely (2026-07-09, see specs/claude-transcript-durability). `env` resolves the real
# executable through PATH, bypassing this function, and "$@" preserves every launch arg.
#
# Scrub identity/implementation markers only. User-selected behavior such as
# CLAUDE_EFFORT and CLAUDE_CODE_DISABLE_ADAPTIVE_THINKING must continue to pass through.
claude() {
  env \
    -u CLAUDE_CODE_SESSION_ID \
    -u CLAUDE_CODE_BRIDGE_SESSION_ID \
    -u CLAUDE_CODE_REMOTE_SESSION_ID \
    -u CLAUDE_CODE_CHILD_SESSION \
    -u CLAUDECODE \
    -u CLAUDE_CODE_ENTRYPOINT \
    -u CLAUDE_CODE_EXECPATH \
    -u AI_AGENT \
    claude "$@"
}

# Returns 0 if <agent>'s session <id> has a *resumable* conversation on disk.
#
# Resumable means a session file exists AND contains at least one real turn. We locate the
# file by its globally-unique session id (so we never replicate each agent's brittle
# cwd->directory hashing). A session that was opened but never used has only a stub/metadata
# line and no real turn -- that is exactly the case `<agent> resume <id>` rejects with
# "No conversation found", so we must treat it as not-resumable and start fresh instead.
clinch_agent_resume_resumable() {
  local agent="$1" id="$2" f
  [[ -n "$id" ]] || return 1
  case "$agent" in
    claude)
      f="$(find "$HOME/.claude/projects" -name "$id.jsonl" -print -quit 2>/dev/null)"
      [[ -n "$f" ]] && grep -Eq '"type":"(user|assistant)"' "$f"
      ;;
    codex)
      f="$(find "$HOME/.codex/sessions" -name "*-$id.jsonl" -print -quit 2>/dev/null)"
      [[ -n "$f" ]] && grep -Eq '"role":"(user|assistant)"' "$f"
      ;;
    *) return 1 ;;
  esac
}

# True when one of the *currently restored/live* panes owns a Claude session. Older builds
# left hundreds of zombie `<pane>.json` files behind, and the append-only journal is history,
# not ownership; neither may permanently block recovery. Clinch atomically maintains the
# active-pane manifest whenever it snapshots app state. Legacy installs without a manifest
# fall back to scanning current entry files only (never journal.jsonl or prompt mirrors).
clinch_agent_resume_session_claimed() {
  setopt localoptions nullglob
  local id="$1" reg="${WARP_AGENT_RESUME_DIR:-$HOME/.warp/agent-resume}"
  local active="${WARP_AGENT_RESUME_ACTIVE_PANES_FILE:-$reg/active-panes}" pane entry
  if [[ -r "$active" ]]; then
    while IFS= read -r pane; do
      case "$pane" in ""|*[!A-Za-z0-9-]*) continue ;; esac
      entry="$reg/$pane.json"
      [[ -f "$entry" ]] || continue
      grep -Eq "(clinch|warp)_agent_resume_launch claude $id( |\")" "$entry" && return 0
    done < "$active"
    return 1
  fi
  for entry in "$reg"/*.json(N); do
    grep -Eq "(clinch|warp)_agent_resume_launch claude $id( |\")" "$entry" && return 0
  done
  return 1
}

# Print the newest *unclaimed* resumable claude session whose transcript records <cwd>
# (default $PWD) as its working directory. This is the safety net for registry rot: a
# recorded id can go stale (its entry overwritten by a session that was never used, its
# transcript rolled away by retention), and starting fresh in that case silently orphans
# the pane's real conversation -- the 2026-07-08 blank-session incident. Matching by the
# transcripts' own cwd field keeps us out of claude's brittle cwd->directory encoding;
# only the head of each file is read, newest file first, so the common case touches a
# handful of files.
#
# "Unclaimed" = not recorded in any pane's registry entry, so a pane whose id died can
# never steal a sibling pane's live session (several panes often share one project
# directory; at restore they all replay at once). The cost: a session claimed by a
# zombie entry (a pane that no longer exists) is not adopted -- still strictly better
# than before, when no dead id was ever recovered at all.
#
# Claude only: codex session files are named rollout-<timestamp>-<id>.jsonl, so a bare
# filename does not yield the session id; codex panes keep the resume-or-fresh behavior.
clinch_agent_resume_fallback_id() {
  setopt localoptions extendedglob nullglob noclobber
  local agent="$1" cwd="${2:-$PWD}"
  [[ "$agent" == claude ]] || return 1
  local reg="${WARP_AGENT_RESUME_DIR:-$HOME/.warp/agent-resume}"
  local match="\"cwd\":\"$cwd\"" f id claim mtime now ttl
  for f in "$HOME"/.claude/projects/**/*.jsonl(N.om); do
    [[ "$f" == */subagents/* ]] && continue   # sidechain transcripts are not resumable sessions
    head -c 131072 "$f" 2>/dev/null | grep -qF -- "$match" || continue
    id="${${f:t}%.jsonl}"
    clinch_agent_resume_resumable "$agent" "$id" || continue
    clinch_agent_resume_session_claimed "$id" && continue
    # At restore, panes race this scan before an adopted session's SessionStart hook can
    # record the new owner. Claim the fallback atomically so two panes cannot adopt the
    # same conversation. A claim older than the configurable TTL is treated as abandoned.
    mkdir -p "$reg" 2>/dev/null
    claim="$reg/.adopt-claim-$id"
    if ! ( : > "$claim" ) 2>/dev/null; then
      ttl="${WARP_AGENT_RESUME_CLAIM_TTL:-120}"
      mtime="$(stat -f '%m' "$claim" 2>/dev/null || printf 0)"
      now="$(date +%s)"
      (( now - mtime < ttl )) && continue
      rm -f "$claim"
      ( : > "$claim" ) 2>/dev/null || continue
    fi
    printf '%s' "$id"
    return 0
  done
  return 1
}

# Print this pane's recorded claude.ai bridge id (session_<...>), if any.
#
# Bridged Claude sessions also keep a cloud recovery copy at
# https://claude.ai/code/<bridge>. Local resume is preferred when its transcript has a real turn
# because it repaints the original terminal history; this id is the fallback when the local copy
# is absent or unusable. The id is read back from the pane's registry entry (written by
# claude-capture.sh). Only claude.ai-shaped ids (session_*) are emitted; anything else a
# hand-edited entry might contain is ignored. The value is only echoed, never evaluated.
clinch_agent_resume_bridge_id() {
  [[ -n "${WARP_TERMINAL_SESSION_UUID:-}" ]] || return 0
  local entry="${WARP_AGENT_RESUME_DIR:-$HOME/.warp/agent-resume}/$WARP_TERMINAL_SESSION_UUID.json"
  [[ -f "$entry" ]] || return 0
  local bridge
  bridge="$(sed -nE 's/.*"bridge": "([^"]+)".*/\1/p' "$entry" 2>/dev/null)"
  [[ "$bridge" == session_* ]] && printf '%s' "$bridge"
  return 0
}

# Relaunch <agent>'s session <id> in this pane. Called by Clinch on restore. Any trailing args
# are launch flags carried over from how the session was originally started (e.g.
# --dangerously-skip-permissions, --model <m>); they are forwarded on every path so the
# session reopens the same way.
#
# Path order:
# 1. Local transcript with a real turn -> `claude --resume <id>` / `codex resume <id>`.
#    Native resume keeps the provider's original session id and repaints the complete visible
#    terminal history. Claude's `--teleport` creates a new local id and can restore model context
#    without repainting the prior exchange, leaving an apparently blank restored pane.
# 2. Bridged claude session without a usable local transcript -> `claude --teleport <bridge>`.
#    This preserves cloud-only recovery (including turns continued from another device) without
#    replacing a complete local transcript. A teleport that fails *fast* (dirty tree, git lock
#    race, API error) falls through to fresh recovery; a non-zero exit after a real run is the
#    user quitting the session, so it must NOT relaunch on top
#    (WARP_AGENT_RESUME_TELEPORT_GRACE seconds distinguishes the two, default 15).
# 3. Dead id in an unbridged claude pane -> adopt the newest unclaimed session recorded
#    for this directory (clinch_agent_resume_fallback_id): a stale registry entry must
#    degrade into a near-miss, not silently orphan the pane's real conversation. Bridged
#    panes skip this -- their conversation lives at claude.ai, and adopting some other
#    local session would silently swap conversations.
# 4. Otherwise start fresh (the SessionStart hook re-captures the new session for next
#    time). WARP_AGENT_RESUME_STARTED_FRESH tells the capture hook the fresh session was
#    machinery-spawned, so until its first real prompt it must not overwrite a pane entry
#    that still points somewhere recoverable (see claude-capture.sh).
clinch_agent_resume_launch() {
  local agent="$1" id="$2"
  shift 2
  local bridge=""
  [[ "$agent" == claude ]] && bridge="$(clinch_agent_resume_bridge_id)"
  if clinch_agent_resume_resumable "$agent" "$id"; then
    case "$agent" in
      claude) claude --resume "$id" "$@" ;;
      codex)  codex resume "$id" "$@" ;;
    esac
    return $?
  fi
  if [[ -n "$bridge" ]]; then
    echo "clinch: local claude transcript is unavailable -- teleporting cloud session ($bridge)." >&2
    local _clinch_start=$SECONDS _clinch_rc
    claude --teleport "$bridge" "$@"
    _clinch_rc=$?
    if (( _clinch_rc == 0 || SECONDS - _clinch_start > ${WARP_AGENT_RESUME_TELEPORT_GRACE:-15} )); then
      return $_clinch_rc
    fi
    echo "clinch: teleport failed -- falling back (cloud copy: https://claude.ai/code/$bridge)." >&2
  fi
  local adopt=""
  if [[ -z "$bridge" ]]; then
    adopt="$(clinch_agent_resume_fallback_id "$agent" "$PWD")" || adopt=""
  fi
  if [[ -n "$adopt" ]]; then
    echo "clinch: recorded $agent session ($id) has no conversation -- resuming newest session in this directory ($adopt) instead." >&2
    case "$agent" in
      claude) claude --resume "$adopt" "$@" ;;
      codex)  codex resume "$adopt" "$@" ;;
    esac
    return $?
  fi
  echo "clinch: no resumable $agent session ($id) -- starting fresh." >&2
  case "$agent" in
    claude) WARP_AGENT_RESUME_STARTED_FRESH=1 claude "$@" ;;
    codex)  codex "$@" ;;
  esac
}

# Compatibility for registry entries captured by older Clinch builds. New captures and
# restore reads use the Clinch-branded command, so this alias should disappear naturally
# once every persisted pane entry has been refreshed.
warp_agent_resume_launch() {
  clinch_agent_resume_launch "$@"
}
