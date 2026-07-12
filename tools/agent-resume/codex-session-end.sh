#!/usr/bin/env bash
set -euo pipefail
[[ -n "${WARP_TERMINAL_SESSION_UUID:-}" ]] || exit 0
# Absolute sibling path: hooks do not reliably inherit the shell PATH.
BIN="$(cd "$(dirname "$0")" && pwd)"
"$BIN/clinch-agent-resume" is-nested-agent >/dev/null 2>&1 && exit 0
"$BIN/clinch-agent-resume" app-terminating >/dev/null 2>&1 && exit 0
payload="$(cat)"
fields="$(printf '%s' "$payload" | "$BIN/agent-json" hook-fields 2>/dev/null)" || exit 0
IFS='|' read -r sid64 _rest <<<"$fields"
sid="$(printf '%s' "$sid64" | /usr/bin/base64 -D 2>/dev/null)" || exit 0
[[ "$sid" =~ ^[A-Za-z0-9-]+$ ]] || exit 0
"$BIN/clinch-agent-resume" remove-if-matches "$WARP_TERMINAL_SESSION_UUID" codex "$sid"
