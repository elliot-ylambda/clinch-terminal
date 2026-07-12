#!/usr/bin/env bash
# Tests the append-only registry journal and `list`: every write/remove is journaled, so
# an overwritten pane entry's (session, bridge, cwd) tuple stays recoverable forever --
# the data-loss class behind the 2026-07-08 and 2026-07-09 incidents.
set -euo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
export WARP_AGENT_RESUME_DIR="$(mktemp -d)/agent-resume"
CLI="$HERE/clinch-agent-resume"
J="$WARP_AGENT_RESUME_DIR/journal.jsonl"

# A write journals op/pane/command/cwd, with bridge always present (empty when unbridged).
"$CLI" write pane-1 "clinch_agent_resume_launch claude sid-first" "/tmp/projA"
[[ -f "$J" ]] || { echo "FAIL: journal not created on write"; exit 1; }
grep -q '"op":"write"' "$J" || { echo "FAIL: op missing"; exit 1; }
grep -q '"pane":"pane-1"' "$J" || { echo "FAIL: pane missing"; exit 1; }
grep -q '"command":"clinch_agent_resume_launch claude sid-first"' "$J" || { echo "FAIL: command missing"; exit 1; }
grep -q '"cwd":"/tmp/projA"' "$J" || { echo "FAIL: cwd missing"; exit 1; }
grep -q '"bridge":""' "$J" || { echo "FAIL: unbridged write must journal an empty bridge"; exit 1; }
jq -e . < "$J" >/dev/null || { echo "FAIL: journal line is not valid JSON"; exit 1; }
perms="$(stat -f '%Lp' "$J")"; [[ "$perms" == "600" ]] || { echo "FAIL: journal perms $perms"; exit 1; }

# Overwriting the pane appends a second line -- the first stays greppable (the point).
"$CLI" write pane-1 "clinch_agent_resume_launch claude sid-second" "/tmp/projA" "session_01BRIDGE"
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

# Shutdown suppression is process-scoped. It is active while the marked app PID lives,
# then self-cleans instead of suppressing SessionEnd indefinitely after a crash/quit.
printf '%s\n' "$$" > "$WARP_AGENT_RESUME_DIR/.app-terminating"
"$CLI" app-terminating || { echo "FAIL: live app-termination marker was ignored"; exit 1; }
printf '%s\n' 999999999 > "$WARP_AGENT_RESUME_DIR/.app-terminating"
if "$CLI" app-terminating; then
  echo "FAIL: dead app-termination marker remained active"
  exit 1
fi
[[ ! -e "$WARP_AGENT_RESUME_DIR/.app-terminating" ]] \
  || { echo "FAIL: stale app-termination marker was not removed"; exit 1; }

# Journal write failure must not fail the CLI: the entry still lands (fail-open).
chmod 000 "$J"
"$CLI" write pane-2 "clinch_agent_resume_launch claude sid-x" "/tmp/projB" || { echo "FAIL: write failed when journal unwritable"; exit 1; }
[[ -f "$WARP_AGENT_RESUME_DIR/pane-2.json" ]] || { echo "FAIL: entry missing after journal failure"; exit 1; }
"$CLI" remove pane-2 || { echo "FAIL: remove failed when journal unwritable"; exit 1; }
[[ -f "$WARP_AGENT_RESUME_DIR/tombstones/pane-2" ]] \
  || { echo "FAIL: durable removal tombstone missing after journal failure"; exit 1; }
[[ "$(stat -f '%Lp' "$WARP_AGENT_RESUME_DIR/tombstones/pane-2")" == "600" ]] \
  || { echo "FAIL: removal tombstone is not private"; exit 1; }
chmod 600 "$J"
"$CLI" write pane-2 "clinch_agent_resume_launch claude sid-new" "/tmp/projB"
[[ ! -e "$WARP_AGENT_RESUME_DIR/tombstones/pane-2" ]] \
  || { echo "FAIL: new owner did not clear old removal tombstone"; exit 1; }

# Quotes/backslashes in fields stay valid JSON in the journal.
"$CLI" write pane-3 'clinch_agent_resume_launch claude sid-q' '/tmp/we"ird\path'
tail -n 1 "$J" | jq -e . >/dev/null || { echo "FAIL: escaped journal line is not valid JSON"; exit 1; }

# --- list ---
# Hand-crafted journal + mirrors so ordering is deterministic (real writes can share a
# second). Three conversations: bridged, local, and mirror-only (nested, never registered).
rm -f "$J" "$WARP_AGENT_RESUME_DIR"/pane-*.json
P="$WARP_AGENT_RESUME_DIR/prompts"; mkdir -p "$P"
cat > "$J" <<'EOF'
{"ts":"2026-07-09T10:00:00Z","op":"write","pane":"pane-a","command":"warp_agent_resume_launch claude sid-oldest-aaa","cwd":"/tmp/projA","bridge":""}
{"ts":"2026-07-09T11:00:00Z","op":"write","pane":"pane-b","command":"clinch_agent_resume_launch claude sid-bridged-bbb","cwd":"/tmp/projB","bridge":""}
{"ts":"2026-07-09T11:30:00Z","op":"write","pane":"pane-b","command":"clinch_agent_resume_launch claude sid-bridged-bbb --model opus","cwd":"/tmp/projB","bridge":"session_01LISTBRIDGE"}
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
# Unbridged legacy journal rows remain discoverable and say "local"; mirror-only (nested)
# sessions appear too.
printf '%s\n' "$out" | grep -q 'sid-olde.*/tmp/projA.*local' || { echo "FAIL: local row wrong"; echo "$out"; exit 1; }
printf '%s\n' "$out" | grep -q 'sid-nest.*/tmp/projC.*"nested run prompt"' || { echo "FAIL: mirror-only session missing"; echo "$out"; exit 1; }

# --cwd filters.
out="$("$CLI" list --cwd /tmp/projB)"
[[ "$(printf '%s\n' "$out" | wc -l | tr -d ' ')" == "1" ]] || { echo "FAIL: --cwd filter row count"; echo "$out"; exit 1; }
printf '%s\n' "$out" | grep -q 'sid-brid' || { echo "FAIL: --cwd kept wrong row"; echo "$out"; exit 1; }

# --- list --json ---
# Machine-readable variant of the same aggregation. A fourth, mirror-only session with a
# newline + quotes in its prompt proves first_prompt round-trips exactly (the human
# format truncates and flattens; JSON must not).
printf '{"ts":"2026-07-09T13:00:00Z","cwd":"/tmp/projD","bridge":"","prompt":"line1\\nline2 \\"quoted\\""}\n' > "$P/sid-tricky-ddd.jsonl"

out="$("$CLI" list --json)"
printf '%s' "$out" | jq -e 'type == "array" and length == 4' >/dev/null \
  || { echo "FAIL: --json must emit a 4-element array"; echo "$out"; exit 1; }
# Newest-first with FULL session ids: tricky (13:00) > nested (12:00) > bridged (11:00) > oldest (10:00).
[[ "$(printf '%s' "$out" | jq -r '.[0].session_id')" == "sid-tricky-ddd" ]] || { echo "FAIL: --json not newest-first"; echo "$out"; exit 1; }
[[ "$(printf '%s' "$out" | jq -r '.[3].session_id')" == "sid-oldest-aaa" ]] || { echo "FAIL: --json oldest not last"; echo "$out"; exit 1; }
# The bridged row carries every field: start ts, cwd, bridge id, cloud URL, first prompt.
row="$(printf '%s' "$out" | jq -c '.[2]')"
[[ "$(printf '%s' "$row" | jq -r '.ts')" == "2026-07-09T11:00:00Z" ]] || { echo "FAIL: --json ts wrong"; echo "$row"; exit 1; }
[[ "$(printf '%s' "$row" | jq -r '.cwd')" == "/tmp/projB" ]] || { echo "FAIL: --json cwd wrong"; echo "$row"; exit 1; }
[[ "$(printf '%s' "$row" | jq -r '.bridge')" == "session_01LISTBRIDGE" ]] || { echo "FAIL: --json bridge wrong"; echo "$row"; exit 1; }
[[ "$(printf '%s' "$row" | jq -r '.url')" == "https://claude.ai/code/session_01LISTBRIDGE" ]] || { echo "FAIL: --json url wrong"; echo "$row"; exit 1; }
[[ "$(printf '%s' "$row" | jq -r '.first_prompt')" == "fix the flaky test in ci" ]] || { echo "FAIL: --json first_prompt wrong"; echo "$row"; exit 1; }
# Unbridged rows have null bridge/url; a session with no mirror has null first_prompt.
[[ "$(printf '%s' "$out" | jq -r '.[3].bridge')" == "null" ]] || { echo "FAIL: --json unbridged bridge must be null"; echo "$out"; exit 1; }
[[ "$(printf '%s' "$out" | jq -r '.[3].url')" == "null" ]] || { echo "FAIL: --json unbridged url must be null"; echo "$out"; exit 1; }
[[ "$(printf '%s' "$out" | jq -r '.[3].first_prompt')" == "null" ]] || { echo "FAIL: --json missing mirror must be null prompt"; echo "$out"; exit 1; }
# first_prompt is the exact text -- newline and quotes intact, no 80-char flattening.
[[ "$(printf '%s' "$out" | jq -r '.[0].first_prompt')" == $'line1\nline2 "quoted"' ]] \
  || { echo "FAIL: --json first_prompt must round-trip exactly"; echo "$out"; exit 1; }
# --cwd composes with --json.
out="$("$CLI" list --json --cwd /tmp/projB)"
printf '%s' "$out" | jq -e 'length == 1 and .[0].session_id == "sid-bridged-bbb"' >/dev/null \
  || { echo "FAIL: --json --cwd filter wrong"; echo "$out"; exit 1; }
# An empty/missing registry emits an empty array, not an error.
out="$(WARP_AGENT_RESUME_DIR="$(mktemp -d)/never-created" "$CLI" list --json)"
[[ "$(printf '%s' "$out" | jq -c .)" == "[]" ]] || { echo "FAIL: --json empty registry must emit []"; echo "$out"; exit 1; }
# Human output is unchanged by the --json work (still short sid + quoted prompt).
out="$("$CLI" list)"
printf '%s\n' "$out" | grep -q 'sid-brid.*https://claude.ai/code/session_01LISTBRIDGE.*"fix the flaky test in ci"' \
  || { echo "FAIL: human list output regressed"; echo "$out"; exit 1; }

echo "PASS"
