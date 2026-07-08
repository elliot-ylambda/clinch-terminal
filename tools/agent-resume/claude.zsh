# Warp agent-resume shell integration (sourced from ~/.zshrc).
#
# Capture is done by Claude's hooks (claude-capture.sh) and Codex's
# SessionStart hook -- they record the live session per pane. This file only provides the
# *replay* side, the functions Warp invokes on restore:
#
#   warp_agent_resume_resumable()   true if an agent session id has a resumable conversation
#   warp_agent_resume_fallback_id() newest unclaimed resumable session for a directory
#   warp_agent_resume_launch()      resume if possible, else adopt, else start fresh
#
# On restore Warp replays the recorded command `warp_agent_resume_launch <agent> <id>` in
# this (interactive) shell, so these functions are in scope. A fresh fallback calls the
# agent normally, so its SessionStart hook re-captures it for next time.

# Returns 0 if <agent>'s session <id> has a *resumable* conversation on disk.
#
# Resumable means a session file exists AND contains at least one real turn. We locate the
# file by its globally-unique session id (so we never replicate each agent's brittle
# cwd->directory hashing). A session that was opened but never used has only a stub/metadata
# line and no real turn -- that is exactly the case `<agent> resume <id>` rejects with
# "No conversation found", so we must treat it as not-resumable and start fresh instead.
warp_agent_resume_resumable() {
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
warp_agent_resume_fallback_id() {
  setopt localoptions extendedglob nullglob
  local agent="$1" cwd="${2:-$PWD}"
  [[ "$agent" == claude ]] || return 1
  local reg="${WARP_AGENT_RESUME_DIR:-$HOME/.warp/agent-resume}"
  local match="\"cwd\":\"$cwd\"" f id
  for f in "$HOME"/.claude/projects/**/*.jsonl(N.om); do
    [[ "$f" == */subagents/* ]] && continue   # sidechain transcripts are not resumable sessions
    head -c 131072 "$f" 2>/dev/null | grep -qF -- "$match" || continue
    id="${${f:t}%.jsonl}"
    warp_agent_resume_resumable "$agent" "$id" || continue
    grep -Eqsr "warp_agent_resume_launch claude $id( |\")" "$reg" && continue
    printf '%s' "$id"
    return 0
  done
  return 1
}

# Print this pane's recorded claude.ai bridge id (session_<...>), if any.
#
# Bridged Claude sessions ("repl bridge") keep the full conversation at
# https://claude.ai/code/<bridge> and stop updating their local jsonl the moment they bridge,
# so the cloud copy -- not the local file -- is authoritative for them. The id is read back
# from the pane's registry entry (written by claude-capture.sh). Only claude.ai-shaped ids
# (session_*) are emitted; anything else a hand-edited entry might contain is ignored. The
# value is only echoed, never evaluated.
warp_agent_resume_bridge_id() {
  [[ -n "${WARP_TERMINAL_SESSION_UUID:-}" ]] || return 0
  local entry="${WARP_AGENT_RESUME_DIR:-$HOME/.warp/agent-resume}/$WARP_TERMINAL_SESSION_UUID.json"
  [[ -f "$entry" ]] || return 0
  local bridge
  bridge="$(sed -nE 's/.*"bridge": "([^"]+)".*/\1/p' "$entry" 2>/dev/null)"
  [[ "$bridge" == session_* ]] && printf '%s' "$bridge"
  return 0
}

# Relaunch <agent>'s session <id> in this pane. Called by Warp on restore. Any trailing args
# are launch flags carried over from how the session was originally started (e.g.
# --dangerously-skip-permissions, --model <m>); they are forwarded on every path so the
# session reopens the same way.
#
# Path order:
# 1. Bridged claude session (bridge id recorded) -> `claude --teleport <bridge>`: the local
#    jsonl of a bridged session is a stale husk at best (it stops updating at bridge time)
#    and `claude --resume` on it either fails with "No conversation found" or silently
#    resumes the husk, so the cloud copy is fetched instead. A teleport that fails *fast*
#    (dirty tree, git lock race, API error) falls through to the local paths; a non-zero
#    exit after a real run is the user quitting the session, so it must NOT relaunch on top
#    (WARP_AGENT_RESUME_TELEPORT_GRACE seconds distinguishes the two, default 15).
# 2. Local transcript with a real turn -> `claude --resume <id>` / `codex resume <id>`.
# 3. Dead id in an unbridged claude pane -> adopt the newest unclaimed session recorded
#    for this directory (warp_agent_resume_fallback_id): a stale registry entry must
#    degrade into a near-miss, not silently orphan the pane's real conversation. Bridged
#    panes skip this -- their conversation lives at claude.ai, and adopting some other
#    local session would silently swap conversations.
# 4. Otherwise start fresh (the SessionStart hook re-captures the new session for next
#    time). WARP_AGENT_RESUME_STARTED_FRESH tells the capture hook the fresh session was
#    machinery-spawned, so until its first real prompt it must not overwrite a pane entry
#    that still points somewhere recoverable (see claude-capture.sh).
warp_agent_resume_launch() {
  local agent="$1" id="$2"
  shift 2
  local bridge=""
  [[ "$agent" == claude ]] && bridge="$(warp_agent_resume_bridge_id)"
  if [[ -n "$bridge" ]]; then
    echo "warp: teleporting bridged claude session ($bridge)." >&2
    local _war_start=$SECONDS _war_rc
    claude --teleport "$bridge" "$@"
    _war_rc=$?
    if (( _war_rc == 0 || SECONDS - _war_start > ${WARP_AGENT_RESUME_TELEPORT_GRACE:-15} )); then
      return $_war_rc
    fi
    echo "warp: teleport failed -- falling back (cloud copy: https://claude.ai/code/$bridge)." >&2
  fi
  if warp_agent_resume_resumable "$agent" "$id"; then
    case "$agent" in
      claude) claude --resume "$id" "$@" ;;
      codex)  codex resume "$id" "$@" ;;
    esac
    return $?
  fi
  local adopt=""
  if [[ -z "$bridge" ]]; then
    adopt="$(warp_agent_resume_fallback_id "$agent" "$PWD")" || adopt=""
  fi
  if [[ -n "$adopt" ]]; then
    echo "warp: recorded $agent session ($id) has no conversation -- resuming newest session in this directory ($adopt) instead." >&2
    case "$agent" in
      claude) claude --resume "$adopt" "$@" ;;
      codex)  codex resume "$adopt" "$@" ;;
    esac
    return $?
  fi
  echo "warp: no resumable $agent session ($id) -- starting fresh." >&2
  case "$agent" in
    claude) WARP_AGENT_RESUME_STARTED_FRESH=1 claude "$@" ;;
    codex)  codex "$@" ;;
  esac
}
