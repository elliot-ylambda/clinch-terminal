#!/usr/bin/env bash
# Devin CLI lifecycle hook, wired once per event into ~/.config/devin/config.json:
#
#   devin-hook.sh <SessionStart|UserPromptSubmit|PermissionRequest|PostToolUse|PostCompaction|Stop|SessionEnd>
#
# It does for Devin what claude-capture.sh and the bundled notification plugins do for Claude:
# records this pane's live session so a restart can reopen it, keeps the private prompt log, and
# reports status to Clinch. The event name comes from the command line because a payload without
# one must never read as a new session start.
#
# Every failure is ignored. A hook must never reject a prompt or stop Devin from working, and Devin
# treats printed output as a decision, so only the SessionStart role context below is printed.
#
# Functions are defined unconditionally; the body only runs when this file is executed (not when
# sourced by the tests), so the parsing helpers can be unit-tested in isolation.

_clinch_devin_decode() {
  printf '%s' "$1" | /usr/bin/base64 -D 2>/dev/null
}

# Values below are persisted into a command Clinch later executes in a shell. Mode and model names
# are token-like, so anything else is dropped rather than quoted.
_clinch_devin_safe_token() {
  [[ "${1:-}" =~ ^[A-Za-z0-9._:/-]+$ ]]
}

# Carries the permission mode, model and sandbox from a flattened `devin` argv so a restored session
# comes back the same way. Everything else, including a stale --resume, is dropped. The environment
# fallbacks are the variables the Devin CLI documents for the same settings.
_clinch_devin_extract_flags() {
  local argv="${1:-}" env_mode="${DEVIN_PERMISSION_MODE:-}" env_model="${DEVIN_MODEL:-}"
  local env_sandbox="${DEVIN_SANDBOX:-}"
  local -a toks=()
  read -ra toks <<<"$argv"
  local mode="" model="" sandbox=0 tok next i=0 n=${#toks[@]}
  while ((i < n)); do
    tok="${toks[i]}"
    next=""
    ((i + 1 < n)) && next="${toks[i + 1]}"
    case "$tok" in
      --) break ;;
      --permission-mode) mode="$next"; i=$((i + 1)) ;;
      --permission-mode=*) mode="${tok#*=}" ;;
      --model) model="$next"; i=$((i + 1)) ;;
      --model=*) model="${tok#*=}" ;;
      --sandbox) sandbox=1 ;;
    esac
    i=$((i + 1))
  done
  [[ -n "$mode" ]] || mode="$env_mode"
  [[ -n "$model" ]] || model="$env_model"
  [[ "$env_sandbox" == 1 || "$env_sandbox" == true ]] && sandbox=1
  local out=""
  _clinch_devin_safe_token "$mode" && out+=" --permission-mode $mode"
  _clinch_devin_safe_token "$model" && out+=" --model $model"
  ((sandbox)) && out+=" --sandbox"
  printf '%s' "$out"
}

# The argv of the `devin` process that owns this hook: the first ancestor carrying a flag worth
# restoring. A plain launch matches nothing, so its resume command stays plain.
# `WARP_AGENT_RESUME_FAKE_ARGV` (set, even to empty) overrides the walk for the tests.
_clinch_devin_argv() {
  if [[ -n "${WARP_AGENT_RESUME_FAKE_ARGV+x}" ]]; then
    printf '%s' "$WARP_AGENT_RESUME_FAKE_ARGV"
    return 0
  fi
  local pid="$PPID" args hops=0
  while [[ -n "$pid" && "$pid" -gt 1 && "$hops" -lt 8 ]]; do
    args="$(/bin/ps -ww -o args= -p "$pid" 2>/dev/null)"
    case " $args " in
      *" --permission-mode "*|*" --permission-mode="*|*" --model "*|*" --model="*|*" --sandbox "*)
        printf '%s' "$args"; return 0 ;;
    esac
    pid="$(/bin/ps -o ppid= -p "$pid" 2>/dev/null | tr -d '[:space:]')"
    hops=$((hops + 1))
  done
  return 0
}

# True when Clinch's prompt log shows session <id> took a real prompt. Devin's own session store
# isn't documented, so this log is what says a conversation exists to reopen.
_clinch_devin_has_conversation() {
  local id="$1" registry="$2"
  [[ -n "$id" ]] && grep -q '"prompt"' "$registry/prompts/devin/$id.jsonl" 2>/dev/null
}

# True when the pane's entry still points at a Devin conversation worth keeping. Until a
# machinery-spawned blank takes its first prompt, it must not replace that pointer.
_clinch_devin_entry_protected() {
  local entry_file="$1" registry="$2" old_sid
  [[ -f "$entry_file" ]] || return 1
  old_sid="$(sed -nE 's/.*(clinch|warp)_agent_resume_launch devin ([A-Za-z0-9-]+).*/\2/p' "$entry_file")"
  _clinch_devin_has_conversation "$old_sid" "$registry"
}

# Writes a status event straight to the pane. Devin may run hooks without a controlling terminal,
# so Clinch's own path to the pane's PTY is the fallback.
_clinch_devin_emit() {
  local body="$1" seq
  [[ -n "$body" && -n "${WARP_CLI_AGENT_PROTOCOL_VERSION:-}" && -n "${WARP_CLIENT_VERSION:-}" ]] || return 0
  seq="$(printf '\033]777;notify;warp://cli-agent;%s\007' "$body")"
  if ! ( printf '%s' "$seq" > /dev/tty ) 2>/dev/null; then
    if [[ -n "${WARP_TTY:-}" && -w "$WARP_TTY" ]]; then
      ( printf '%s' "$seq" > "$WARP_TTY" ) 2>/dev/null || true
    fi
  fi
}

_clinch_devin_write_entry() {
  local bin="$1" sid="$2" cwd="$3" owner_pid="$4" owner_tty="$5" extra
  extra="$(_clinch_devin_extract_flags "$(_clinch_devin_argv)")"
  "$bin/clinch-agent-resume" write "$WARP_TERMINAL_SESSION_UUID" \
    "clinch_agent_resume_launch devin $sid$extra" "$cwd" "" "$owner_pid" "$owner_tty" \
    >/dev/null 2>&1 || true
}

_clinch_devin_hook_main() {
  set -uo pipefail
  local event="${1:-}"
  [[ -n "${WARP_TERMINAL_SESSION_UUID:-}" ]] || return 0
  local BIN registry entry_file
  BIN="$(cd "$(dirname "$0")" && pwd)"
  registry="${WARP_AGENT_RESUME_DIR:-$HOME/.warp/agent-resume}"
  entry_file="$registry/$WARP_TERMINAL_SESSION_UUID.json"

  # A tool result needs nothing from the payload and this runs after every tool call, so it skips
  # the JSON helper. It only tells Clinch a permission prompt has been answered.
  if [[ "$event" == PostToolUse ]]; then
    cat >/dev/null
    _clinch_devin_emit '{"v":1,"agent":"devin","event":"tool_complete"}'
    return 0
  fi

  local payload fields sid64 cwd64 _event64 _pmode64 _model64 reason64 sid cwd
  payload="$(cat)"
  fields="$(printf '%s' "$payload" | "$BIN/agent-json" hook-fields 2>/dev/null)" || return 0
  IFS='|' read -r sid64 cwd64 _event64 _pmode64 _model64 reason64 <<<"$fields"
  sid="$(_clinch_devin_decode "$sid64")" || return 0
  cwd="$(_clinch_devin_decode "$cwd64")" || return 0
  [[ -n "$cwd" ]] || cwd="$PWD"
  [[ "$sid" =~ ^[A-Za-z0-9-]+$ ]] || return 0

  local nested=0 owner_fields owner_pid="" owner_tty64 owner_tty=""
  if owner_fields="$("$BIN/clinch-agent-resume" hook-owner-fields 2>/dev/null)"; then
    IFS='|' read -r owner_pid owner_tty64 <<<"$owner_fields"
    owner_tty="$(_clinch_devin_decode "$owner_tty64")" || return 0
  else
    # A Devin run started by another agent's tool shares this pane but must not replace the visible
    # agent's restore target or its status.
    nested=1
  fi

  # Status goes first: the bookkeeping below can take a second, and the tab should show the new
  # state as soon as Devin reports it.
  if ((!nested)); then
    _clinch_devin_emit "$(printf '%s' "$payload" \
      | "$BIN/agent-json" devin-event "$event" "$sid" "$cwd" 2>/dev/null)"
  fi

  case "$event" in
    SessionStart)
      ((nested)) && return 0
      if [[ -z "${WARP_AGENT_RESUME_STARTED_FRESH:-}" ]] \
         || _clinch_devin_has_conversation "$sid" "$registry" \
         || ! _clinch_devin_entry_protected "$entry_file" "$registry"; then
        _clinch_devin_write_entry "$BIN" "$sid" "$cwd" "$owner_pid" "$owner_tty"
      fi
      # A coordinator tab gets its role here because Devin has no system prompt flag. This costs no
      # turn and also runs when the coordinator is reopened after a restart.
      if [[ -n "${CLINCH_COORDINATOR_ROLE:-}" ]]; then
        "$BIN/agent-json" devin-session-context \
          "$(_clinch_devin_decode "$CLINCH_COORDINATOR_ROLE")" 2>/dev/null || true
      fi
      ;;
    UserPromptSubmit|Stop)
      # The log is keyed by session, so a nested run keeps its history without touching this pane.
      printf '%s' "$payload" | "$BIN/prompt-mirror.sh" devin "$event" >/dev/null 2>&1 || true
      ((nested)) && return 0
      _clinch_devin_write_entry "$BIN" "$sid" "$cwd" "$owner_pid" "$owner_tty"
      ;;
    PostCompaction)
      # Only the status event above: the conversation and its restore entry are unchanged.
      ;;
    SessionEnd)
      ((nested)) && return 0
      # A signal or PTY failure ends Devin too. Only an exit the user chose may drop the resume
      # pointer, and app shutdown keeps every entry so tabs reopen.
      [[ "$(_clinch_devin_decode "$reason64")" == other ]] && return 0
      "$BIN/clinch-agent-resume" app-terminating >/dev/null 2>&1 && return 0
      "$BIN/clinch-agent-resume" remove-if-matches \
        "$WARP_TERMINAL_SESSION_UUID" devin "$sid" "$owner_pid" "$owner_tty" >/dev/null 2>&1 || true
      ;;
  esac
  return 0
}

if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  _clinch_devin_hook_main "$@"
  exit 0
fi
