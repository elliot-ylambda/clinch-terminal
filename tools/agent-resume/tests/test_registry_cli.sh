#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
export WARP_AGENT_RESUME_DIR="$(mktemp -d)/agent-resume"
CLI="$HERE/clinch-agent-resume"
LEGACY_CLI="$HERE/warp-agent-resume"

"$CLI" write deadbeef "claude --resume abc-123" "/tmp/proj"
f="$WARP_AGENT_RESUME_DIR/deadbeef.json"
[[ -f "$f" ]] || { echo "FAIL: file not created"; exit 1; }
grep -q '"command": "claude --resume abc-123"' "$f" || { echo "FAIL: command missing"; exit 1; }
grep -q '"cwd": "/tmp/proj"' "$f" || { echo "FAIL: cwd missing"; exit 1; }
perms="$(stat -f '%Lp' "$f")"; [[ "$perms" == "600" ]] || { echo "FAIL: file perms $perms"; exit 1; }
dperms="$(stat -f '%Lp' "$WARP_AGENT_RESUME_DIR")"; [[ "$dperms" == "700" ]] || { echo "FAIL: dir perms $dperms"; exit 1; }

# Optional 4th arg records the claude.ai bridge id in a "bridge" field.
"$CLI" write deadbeef "claude --resume abc-123" "/tmp/proj" "session_01TESTBRIDGE"
grep -q '"bridge": "session_01TESTBRIDGE"' "$f" || { echo "FAIL: bridge missing"; exit 1; }

# Empty/omitted bridge writes no field (and a rewrite drops a stale one).
"$CLI" write deadbeef "claude --resume abc-123" "/tmp/proj" ""
grep -q '"bridge"' "$f" && { echo "FAIL: empty bridge must not write field"; exit 1; }
"$CLI" write deadbeef "claude --resume abc-123" "/tmp/proj"
grep -q '"bridge"' "$f" && { echo "FAIL: omitted bridge must not write field"; exit 1; }

"$CLI" remove deadbeef
[[ ! -f "$f" ]] || { echo "FAIL: file not removed"; exit 1; }
"$CLI" remove deadbeef   # must be idempotent / no error
"$LEGACY_CLI" remove deadbeef # compatibility shim for pre-rebrand scripts

# scrub-bridge strips only entries recording the exact leaked bridge, preserving escaped
# fields and journaling an explicit clear so discovery does not resurrect the bad URL.
"$CLI" write paneA 'clinch_agent_resume_launch claude ccc' '/tmp/a"quoted' session_01LEAK
"$CLI" write paneB 'clinch_agent_resume_launch claude ddd' /tmp/b session_01REAL
"$CLI" write paneC 'clinch_agent_resume_launch claude eee' /tmp/c
out="$("$CLI" scrub-bridge session_01LEAK)"
[[ "$out" == "scrubbed bridge from 1 entries" ]] || { echo "FAIL: scrub-bridge count ($out)"; exit 1; }
jq -e 'has("bridge") | not' "$WARP_AGENT_RESUME_DIR/paneA.json" >/dev/null \
  || { echo "FAIL: scrub left the poisoned bridge"; exit 1; }
[[ "$(jq -r .cwd "$WARP_AGENT_RESUME_DIR/paneA.json")" == '/tmp/a"quoted' ]] \
  || { echo "FAIL: scrub damaged escaped entry fields"; exit 1; }
[[ "$(jq -r .bridge "$WARP_AGENT_RESUME_DIR/paneB.json")" == "session_01REAL" ]] \
  || { echo "FAIL: scrub hit a different bridge"; exit 1; }
grep -q '"op":"scrub-bridge".*"bridge":"session_01LEAK"' "$WARP_AGENT_RESUME_DIR/journal.jsonl" \
  || { echo "FAIL: scrub not journaled"; exit 1; }
out="$("$CLI" list --json)"
[[ "$(printf '%s' "$out" | jq -r '.[] | select(.session_id == "ccc") | .bridge')" == "null" ]] \
  || { echo "FAIL: discovery resurrected scrubbed bridge"; echo "$out"; exit 1; }

# Empty bridge ids are rejected.
"$CLI" scrub-bridge "" 2>/dev/null && { echo "FAIL: empty bridge id must error"; exit 1; }
echo "PASS"
