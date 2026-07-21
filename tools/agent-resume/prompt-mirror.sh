#!/usr/bin/env bash
# Append one provider UserPromptSubmit or Stop payload to Clinch's private prompt mirror. Stop
# records are turn boundaries, allowing repeated in-flight submissions to coalesce without
# erasing an intentional identical prompt after an answer. Prompt text is accepted only through
# stdin; argv contains the provider name and never user-authored content.

_clinch_prompt_mirror_decode() {
  printf '%s' "$1" | /usr/bin/base64 -D 2>/dev/null
}

_clinch_prompt_mirror_main() {
  set -uo pipefail
  [[ -n "${WARP_TERMINAL_SESSION_UUID:-}" ]] || return 0
  local provider="${1:-}" payload fields sid64 cwd64 event64 _
  local sid cwd event registry root dir file ts size line bin bridge=""
  case "$provider" in claude|codex) ;; *) return 0 ;; esac

  payload="$(cat)"
  bin="$(cd "$(dirname "$0")" && pwd)"
  fields="$(printf '%s' "$payload" | "$bin/agent-json" hook-fields 2>/dev/null)" || return 0
  IFS='|' read -r sid64 cwd64 event64 _ <<<"$fields"
  sid="$(_clinch_prompt_mirror_decode "$sid64")" || return 0
  cwd="$(_clinch_prompt_mirror_decode "$cwd64")" || return 0
  event="$(_clinch_prompt_mirror_decode "$event64")" || return 0
  [[ "$event" == "UserPromptSubmit" || "$event" == "Stop" ]] || return 0
  [[ "$sid" =~ ^[A-Za-z0-9-]+$ ]] || return 0

  ts="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  [[ "$provider" == "claude" ]] && bridge="${CLAUDE_CODE_BRIDGE_SESSION_ID:-}"
  line="$(printf '%s' "$payload" | "$bin/agent-json" prompt-line "$ts" "$cwd" \
    "$bridge" 2>/dev/null)" || return 0
  [[ -n "$line" ]] || return 0

  if [[ -n "${WARP_AGENT_RESUME_DIR:-}" ]]; then
    registry="$WARP_AGENT_RESUME_DIR"
  elif [[ -n "${HOME:-}" ]]; then
    registry="$HOME/.warp/agent-resume"
  else
    return 0
  fi
  root="$registry/prompts"
  dir="$root/$provider"
  file="$dir/$sid.jsonl"
  [[ ! -L "$root" && ! -L "$dir" ]] || return 0
  mkdir -p "$dir" 2>/dev/null || return 0
  chmod 700 "$registry" "$root" "$dir" 2>/dev/null || true
  [[ ! -L "$file" ]] || return 0

  # Past five MiB append exactly one marker, then stop. The reader applies the same bound and
  # surfaces either the marker or an over-limit file as partial history.
  size="$(stat -f %z "$file" 2>/dev/null || echo 0)"
  if (( size > 5242880 )); then
    [[ "$(tail -n 1 "$file" 2>/dev/null)" == *'"truncated":true'* ]] && return 0
    ( umask 077; printf '{"ts":"%s","truncated":true}\n' "$ts" >> "$file" ) \
      2>/dev/null || true
    chmod 600 "$file" 2>/dev/null || true
    return 0
  fi
  ( umask 077; printf '%s\n' "$line" >> "$file" ) 2>/dev/null || true
  chmod 600 "$file" 2>/dev/null || true
  return 0
}

_clinch_prompt_mirror_main "$@"
