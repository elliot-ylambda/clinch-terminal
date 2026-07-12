#!/usr/bin/env bash
set -euo pipefail
[[ -n "${WARP_TERMINAL_SESSION_UUID:-}" ]] || exit 0
# Absolute sibling path: hooks do not reliably inherit the shell PATH.
BIN="$(cd "$(dirname "$0")" && pwd)"
"$BIN/clinch-agent-resume" remove "$WARP_TERMINAL_SESSION_UUID"
