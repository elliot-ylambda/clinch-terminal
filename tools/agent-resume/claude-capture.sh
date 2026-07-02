#!/usr/bin/env bash
# Claude Code capture hook: record the *actual* live session for this Warp pane so it
# can be resumed on restore. Wired to SessionStart (fresh start, `claude --resume <id>`,
# the interactive picker, and `claude --continue` -- in every case the stdin payload
# carries the real session_id) and to UserPromptSubmit + Stop for live-mode updates.
#
# Keyed by the pane UUID, so multiple agents in the same directory stay disambiguated.
# No removal on exit: the entry is overwritten by the next session in this pane, which keeps
# it present when Warp snapshots at quit (see README "Graceful-exit behavior").
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

# Carry forward the permission mode + model from a flattened `claude` argv string. Pure: takes
# the argv string, prints the extra flags to append to the resume command (leading space, or
# empty). Only mode + model are carried; everything else (incl. a stale --resume) is dropped.
_warp_agent_resume_extract_flags() {
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
      --permission-mode)   [[ -n "$next" ]] && { out+=" --permission-mode $next"; i=$((i + 1)); } ;;
      --permission-mode=*) out+=" --permission-mode ${tok#*=}" ;;
      --model)             [[ -n "$next" ]] && { out+=" --model $next"; i=$((i + 1)); } ;;
      --model=*)           out+=" --model ${tok#*=}" ;;
    esac
    i=$((i + 1))
  done
  printf '%s' "$out"
}

# Like _warp_agent_resume_extract_flags but carries only --model. Used when a hook
# payload already provides the authoritative permission mode.
_warp_agent_resume_extract_model() {
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
      --model)   [[ -n "$next" ]] && { out+=" --model $next"; i=$((i + 1)); } ;;
      --model=*) out+=" --model ${tok#*=}" ;;
    esac
    i=$((i + 1))
  done
  printf '%s' "$out"
}

# Maps a hook payload permission_mode to resume-command flags. Prints the flag tokens
# (leading space; empty for `default`, which must strip a previously-carried mode).
# Returns 1 for empty/unknown values so the caller can fall back to argv detection.
_warp_agent_resume_mode_flags_from_payload() {
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
_warp_agent_resume_claude_argv() {
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

_warp_agent_resume_capture_main() {
  set -uo pipefail
  [[ -n "${WARP_TERMINAL_SESSION_UUID:-}" ]] || return 0   # only act inside a Warp pane
  local payload sid cwd event pmode extra mode_part entry_file BIN
  payload="$(cat)"
  sid="$(printf '%s' "$payload" | jq -r '.session_id // empty')"
  cwd="$(printf '%s' "$payload" | jq -r '.cwd // empty')"
  event="$(printf '%s' "$payload" | jq -r '.hook_event_name // "SessionStart"')"
  [[ -n "$sid" ]] || return 0
  case "$event" in
    SessionStart)
      # Fresh capture: a new session in this pane takes over the entry unconditionally.
      extra="$(_warp_agent_resume_extract_flags "$(_warp_agent_resume_claude_argv)")"
      ;;
    UserPromptSubmit|Stop)
      # Live-mode update. Guard: only touch an entry this session owns -- a missing entry
      # is healed (pre-flag registries), but an entry recording a different session id
      # (e.g. a nested claude run from a tool in the same pane env) is left alone.
      entry_file="${WARP_AGENT_RESUME_DIR:-$HOME/.warp/agent-resume}/$WARP_TERMINAL_SESSION_UUID.json"
      if [[ -f "$entry_file" ]] && ! grep -qE "warp_agent_resume_launch claude $sid( |\")" "$entry_file"; then
        return 0
      fi
      pmode="$(printf '%s' "$payload" | jq -r '.permission_mode // empty')"
      if mode_part="$(_warp_agent_resume_mode_flags_from_payload "$pmode")"; then
        extra="${mode_part}$(_warp_agent_resume_extract_model "$(_warp_agent_resume_claude_argv)")"
      else
        extra="$(_warp_agent_resume_extract_flags "$(_warp_agent_resume_claude_argv)")"
      fi
      ;;
    *) return 0 ;;
  esac
  # Call the registry CLI by absolute path (sibling of this script) so the hook does not
  # depend on the agent inheriting the shell PATH.
  BIN="$(cd "$(dirname "$0")" && pwd)"
  "$BIN/warp-agent-resume" write "$WARP_TERMINAL_SESSION_UUID" \
    "warp_agent_resume_launch claude $sid$extra" "$cwd" >/dev/null 2>&1 || true
  return 0
}

# Run the capture only when executed directly; sourcing (tests) just loads the functions.
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  _warp_agent_resume_capture_main "$@"
fi
