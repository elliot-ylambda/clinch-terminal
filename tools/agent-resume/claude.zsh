# Warp agent-resume shell integration (sourced from ~/.zshrc).
#
# Capture is done by Claude's SessionStart hook (claude-session-start.sh) and Codex's
# SessionStart hook -- they record the live session per pane. This file only provides the
# *replay* side, the functions Warp invokes on restore:
#
#   warp_agent_resume_resumable() true if an agent session id has a resumable conversation
#   warp_agent_resume_launch()    resume if possible, else start fresh
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

# Resume <agent>'s session <id> if it is resumable, otherwise start a fresh session in this
# pane. Called by Warp on restore. Any trailing args are launch flags carried over from how the
# session was originally started (e.g. --dangerously-skip-permissions, --model <m>); they are
# forwarded to both the resume and the fresh-fallback path so the session reopens the same way.
# The fresh fallback runs the agent normally (so its SessionStart hook re-captures it under this
# pane for the next restore).
warp_agent_resume_launch() {
  local agent="$1" id="$2"
  shift 2
  if warp_agent_resume_resumable "$agent" "$id"; then
    case "$agent" in
      claude) claude --resume "$id" "$@" ;;
      codex)  codex resume "$id" "$@" ;;
    esac
  else
    echo "warp: no resumable $agent session ($id) -- starting fresh." >&2
    case "$agent" in
      claude) claude "$@" ;;
      codex)  codex "$@" ;;
    esac
  fi
}
