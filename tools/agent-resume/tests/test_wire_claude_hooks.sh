#!/usr/bin/env bash
# Tests the settings.json hook wiring: adds claude-capture.sh to the three events,
# removes stale pre-rename entries, preserves unrelated hooks, and is idempotent.
set -euo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d)"
CFG="$TMP/settings.json"
BIN="/fake/bin"
NEW="$BIN/claude-capture.sh"
OLD="$BIN/claude-session-start.sh"

fail() { echo "FAIL: $1"; exit 1; }
count() { jq -r --arg c "$NEW" "[.hooks.$1[]?.hooks[]? | select(.command == \$c)] | length" "$CFG"; }

# 1. Missing file: created with all three events wired.
bash "$HERE/wire-claude-hooks.sh" "$CFG" "$BIN"
for ev in SessionStart UserPromptSubmit Stop; do
  [[ "$(count "$ev")" == 1 ]] || fail "$ev not wired on fresh file"
done

# 2. Idempotent: run again, still exactly one entry per event.
bash "$HERE/wire-claude-hooks.sh" "$CFG" "$BIN"
for ev in SessionStart UserPromptSubmit Stop; do
  [[ "$(count "$ev")" == 1 ]] || fail "$ev duplicated on re-run"
done

# 3. Migration: stale pre-rename entry removed; unrelated hooks preserved.
cat > "$CFG" <<EOF
{
  "model": "opus",
  "hooks": {
    "SessionStart": [ { "hooks": [ { "type": "command", "command": "$OLD" } ] } ],
    "PostToolUse": [ { "matcher": "Bash", "hooks": [ { "type": "command", "command": "/keep/me.sh" } ] } ]
  }
}
EOF
bash "$HERE/wire-claude-hooks.sh" "$CFG" "$BIN"
[[ "$(jq -r --arg c "$OLD" '[.hooks[][]?.hooks[]? | select(.command == $c)] | length' "$CFG")" == 0 ]] || fail "stale entry not removed"
[[ "$(jq -r '[.hooks.PostToolUse[]?.hooks[]? | select(.command == "/keep/me.sh")] | length' "$CFG")" == 1 ]] || fail "unrelated hook clobbered"
[[ "$(jq -r '.model' "$CFG")" == "opus" ]] || fail "unrelated settings clobbered"
for ev in SessionStart UserPromptSubmit Stop; do
  [[ "$(count "$ev")" == 1 ]] || fail "$ev not wired after migration"
done

echo "PASS"
