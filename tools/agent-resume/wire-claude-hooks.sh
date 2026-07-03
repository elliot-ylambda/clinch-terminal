#!/usr/bin/env bash
# Wires the Claude capture hook (claude-capture.sh) into a Claude settings.json:
# SessionStart captures the live session; UserPromptSubmit and Stop keep the entry's
# permission-mode flags in sync with the session's live mode. Also removes entries left
# by the pre-rename install (claude-session-start.sh). jq-merge only -- never clobbers
# unrelated settings or hooks. Idempotent.
#
# Usage: wire-claude-hooks.sh <settings.json> <installed-bin-dir>
set -euo pipefail
CFG="$1"; BIN="$2"
command -v jq >/dev/null || { echo "error: jq is required to wire the Claude hooks" >&2; exit 1; }
mkdir -p "$(dirname "$CFG")"
[[ -f "$CFG" ]] || echo '{}' > "$CFG"
tmp="$(mktemp)"
jq --arg old "$BIN/claude-session-start.sh" --arg c "$BIN/claude-capture.sh" '
  .hooks = (.hooks // {})
  | .hooks |= with_entries(
      .value |= (map(.hooks = ((.hooks // []) | map(select(.command != $old))))
                 | map(select((.hooks | length) > 0)))
    )
  | reduce ("SessionStart", "UserPromptSubmit", "Stop") as $ev (.;
      if ([.hooks[$ev][]?.hooks[]? | select(.command == $c)] | length) > 0 then .
      else .hooks[$ev] = ((.hooks[$ev] // []) + [{ "hooks": [{ "type": "command", "command": $c }] }])
      end)
' "$CFG" > "$tmp"
mv "$tmp" "$CFG"
