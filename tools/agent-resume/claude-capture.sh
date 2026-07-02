#!/usr/bin/env bash
# Claude Code SessionStart hook: record the *actual* live session for this Warp pane so it
# can be resumed on restore. Fires on a fresh start, `claude --resume <id>`, the interactive
# session picker, and `claude --continue` -- in every case the stdin payload carries the
# real session_id. (The old claude() shell wrapper could only know an id it was given on the
# command line, so it silently missed the picker and --continue and left a stale entry.)
#
# Keyed by the pane UUID, so multiple agents in the same directory stay disambiguated.
# No removal on exit: the entry is overwritten by the next session in this pane, which keeps
# it present when Warp snapshots at quit (see README "Graceful-exit behavior").
#
# Beyond the session id we also carry forward *how* the session was launched -- the permission
# mode (--dangerously-skip-permissions / --permission-mode <mode>) and the --model -- so a
# restored session comes back the same way (e.g. the `CA` alias = claude
# --dangerously-skip-permissions). SessionStart's stdin payload does NOT include the permission
# mode, so we read it off the live `claude` process argv (the alias expands before exec).
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
  local payload sid cwd extra BIN
  payload="$(cat)"
  sid="$(printf '%s' "$payload" | jq -r '.session_id // empty')"
  cwd="$(printf '%s' "$payload" | jq -r '.cwd // empty')"
  [[ -n "$sid" ]] || return 0
  extra="$(_warp_agent_resume_extract_flags "$(_warp_agent_resume_claude_argv)")"
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
