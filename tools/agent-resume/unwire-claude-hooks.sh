#!/usr/bin/env bash
# Removes only Clinch/legacy Warp agent-resume hooks from Claude settings.
# Unrelated settings and hooks are preserved.
set -euo pipefail

CFG="${1:?usage: unwire-claude-hooks.sh <settings.json> <installed-bin-dir>}"
BIN="${2:?missing installed-bin-dir}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

[[ -f "$CFG" ]] || exit 0
tmp="$(mktemp "$(dirname "$CFG")/.clinch-settings.XXXXXX")"
trap 'rm -f "$tmp"' EXIT
mode="$(stat -f '%Lp' "$CFG" 2>/dev/null || echo 600)"

"$SCRIPT_DIR/agent-json" unwire-claude \
  "$BIN/claude-session-start.sh" "$BIN/claude-capture.sh" < "$CFG" > "$tmp"
chmod "$mode" "$tmp"
if cmp -s "$tmp" "$CFG"; then
  exit 0
fi
mv "$tmp" "$CFG"
trap - EXIT
