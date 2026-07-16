#!/usr/bin/env bash
# Wires the Claude capture hook (claude-capture.sh) into a Claude settings.json:
# SessionStart captures the live session; UserPromptSubmit and Stop keep the entry's
# permission-mode flags in sync; SessionEnd removes sessions users actually exited. Also removes entries left
# by the pre-rename install (claude-session-start.sh). Structural merge only -- never clobbers
# unrelated settings or hooks. Idempotent. This low-level helper does not persist the setting;
# normal callers should use `install.sh enable` instead.
#
# Usage: wire-claude-hooks.sh <settings.json> <installed-bin-dir>
set -euo pipefail
CFG="$1"; BIN="$2"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
mkdir -p "$(dirname "$CFG")"
tmp="$(mktemp "$(dirname "$CFG")/.clinch-settings.XXXXXX")"
trap 'rm -f "$tmp"' EXIT

if [[ -f "$CFG" ]]; then
  mode="$(stat -f '%Lp' "$CFG" 2>/dev/null || echo 600)"
  "$SCRIPT_DIR/agent-json" wire-claude \
    "$BIN/claude-session-start.sh" "$BIN/claude-capture.sh" < "$CFG" > "$tmp"
else
  mode=600
  printf '{}' | "$SCRIPT_DIR/agent-json" wire-claude \
    "$BIN/claude-session-start.sh" "$BIN/claude-capture.sh" > "$tmp"
fi

chmod "$mode" "$tmp"
if [[ -f "$CFG" ]] && cmp -s "$tmp" "$CFG"; then
  exit 0
fi
mv "$tmp" "$CFG"
trap - EXIT
