#!/usr/bin/env bash
# Tests the append-only registry journal and `list`: every write/remove is journaled, so
# an overwritten pane entry's (session, bridge, cwd) tuple stays recoverable forever --
# the data-loss class behind the 2026-07-08 and 2026-07-09 incidents.
set -euo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
export WARP_AGENT_RESUME_DIR="$(mktemp -d)/agent-resume"
CLI="$HERE/warp-agent-resume"
J="$WARP_AGENT_RESUME_DIR/journal.jsonl"

# A write journals op/pane/command/cwd, with bridge always present (empty when unbridged).
"$CLI" write pane-1 "warp_agent_resume_launch claude sid-first" "/tmp/projA"
[[ -f "$J" ]] || { echo "FAIL: journal not created on write"; exit 1; }
grep -q '"op":"write"' "$J" || { echo "FAIL: op missing"; exit 1; }
grep -q '"pane":"pane-1"' "$J" || { echo "FAIL: pane missing"; exit 1; }
grep -q '"command":"warp_agent_resume_launch claude sid-first"' "$J" || { echo "FAIL: command missing"; exit 1; }
grep -q '"cwd":"/tmp/projA"' "$J" || { echo "FAIL: cwd missing"; exit 1; }
grep -q '"bridge":""' "$J" || { echo "FAIL: unbridged write must journal an empty bridge"; exit 1; }
jq -e . < "$J" >/dev/null || { echo "FAIL: journal line is not valid JSON"; exit 1; }
perms="$(stat -f '%Lp' "$J")"; [[ "$perms" == "600" ]] || { echo "FAIL: journal perms $perms"; exit 1; }

# Overwriting the pane appends a second line -- the first stays greppable (the point).
"$CLI" write pane-1 "warp_agent_resume_launch claude sid-second" "/tmp/projA" "session_01BRIDGE"
[[ "$(wc -l < "$J")" -eq 2 ]] || { echo "FAIL: overwrite did not append"; exit 1; }
grep -q 'sid-first' "$J" || { echo "FAIL: overwritten pointer lost from journal"; exit 1; }
grep -q '"bridge":"session_01BRIDGE"' "$J" || { echo "FAIL: bridge id not journaled"; exit 1; }

# remove journals (before deleting); an idempotent re-remove adds no line.
"$CLI" remove pane-1
grep -q '"op":"remove".*"pane":"pane-1"' "$J" || { echo "FAIL: remove not journaled"; exit 1; }
[[ ! -f "$WARP_AGENT_RESUME_DIR/pane-1.json" ]] || { echo "FAIL: entry not removed"; exit 1; }
n="$(wc -l < "$J")"
"$CLI" remove pane-1
[[ "$(wc -l < "$J")" -eq "$n" ]] || { echo "FAIL: re-remove of missing entry must not journal"; exit 1; }

# Journal write failure must not fail the CLI: the entry still lands (fail-open).
chmod 000 "$J"
"$CLI" write pane-2 "warp_agent_resume_launch claude sid-x" "/tmp/projB" || { echo "FAIL: write failed when journal unwritable"; exit 1; }
[[ -f "$WARP_AGENT_RESUME_DIR/pane-2.json" ]] || { echo "FAIL: entry missing after journal failure"; exit 1; }
chmod 600 "$J"

# Quotes/backslashes in fields stay valid JSON in the journal.
"$CLI" write pane-3 'warp_agent_resume_launch claude sid-q' '/tmp/we"ird\path'
tail -n 1 "$J" | jq -e . >/dev/null || { echo "FAIL: escaped journal line is not valid JSON"; exit 1; }

# --- list ---
# Hand-crafted journal + mirrors so ordering is deterministic (real writes can share a
# second). Three conversations: bridged, local, and mirror-only (nested, never registered).
rm -f "$J" "$WARP_AGENT_RESUME_DIR"/pane-*.json
P="$WARP_AGENT_RESUME_DIR/prompts"; mkdir -p "$P"
cat > "$J" <<'EOF'
{"ts":"2026-07-09T10:00:00Z","op":"write","pane":"pane-a","command":"warp_agent_resume_launch claude sid-oldest-aaa","cwd":"/tmp/projA","bridge":""}
{"ts":"2026-07-09T11:00:00Z","op":"write","pane":"pane-b","command":"warp_agent_resume_launch claude sid-bridged-bbb","cwd":"/tmp/projB","bridge":""}
{"ts":"2026-07-09T11:30:00Z","op":"write","pane":"pane-b","command":"warp_agent_resume_launch claude sid-bridged-bbb --model opus","cwd":"/tmp/projB","bridge":"session_01LISTBRIDGE"}
{"ts":"2026-07-09T11:45:00Z","op":"remove","pane":"pane-a"}
EOF
printf '{"ts":"2026-07-09T11:00:05Z","cwd":"/tmp/projB","bridge":"","prompt":"fix the flaky test in ci"}\n' > "$P/sid-bridged-bbb.jsonl"
printf '{"ts":"2026-07-09T12:00:00Z","cwd":"/tmp/projC","bridge":"","prompt":"nested run prompt"}\n' > "$P/sid-nested-ccc.jsonl"

out="$("$CLI" list)"
# Newest first: nested (12:00) > bridged (11:00) > oldest (10:00).
[[ "$(printf '%s\n' "$out" | sed -n 1p)" == 2026-07-09T12:00:00Z* ]] || { echo "FAIL: list not newest-first"; echo "$out"; exit 1; }
[[ "$(printf '%s\n' "$out" | sed -n 3p)" == 2026-07-09T10:00:00Z* ]] || { echo "FAIL: oldest not last"; echo "$out"; exit 1; }
# Bridged row shows the cloud URL (from its LATEST bridge value) and its first prompt.
printf '%s\n' "$out" | grep -q 'sid-brid.*https://claude.ai/code/session_01LISTBRIDGE.*"fix the flaky test in ci"' \
  || { echo "FAIL: bridged row wrong"; echo "$out"; exit 1; }
# Unbridged journal-only row says "local"; mirror-only (nested) session appears at all.
printf '%s\n' "$out" | grep -q 'sid-olde.*/tmp/projA.*local' || { echo "FAIL: local row wrong"; echo "$out"; exit 1; }
printf '%s\n' "$out" | grep -q 'sid-nest.*/tmp/projC.*"nested run prompt"' || { echo "FAIL: mirror-only session missing"; echo "$out"; exit 1; }

# --cwd filters.
out="$("$CLI" list --cwd /tmp/projB)"
[[ "$(printf '%s\n' "$out" | wc -l | tr -d ' ')" == "1" ]] || { echo "FAIL: --cwd filter row count"; echo "$out"; exit 1; }
printf '%s\n' "$out" | grep -q 'sid-brid' || { echo "FAIL: --cwd kept wrong row"; echo "$out"; exit 1; }

echo "PASS"
