#!/usr/bin/env zsh
# Tests the replay side in claude.zsh: resume a session only if it has a real conversation,
# otherwise start fresh (the stub/missing case must not error out with "No conversation found").
set -euo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d)"
mkdir -p "$TMP/bin"
# Fake `claude` that records the args it was called with (path baked in). Each call also
# appends to all_args so fallback chains are assertable, records whether the machinery
# fresh marker (WARP_AGENT_RESUME_STARTED_FRESH) was set, and the exit code is
# configurable via $TMP/claude_rc (defaults to 0).
cat > "$TMP/bin/claude" <<EOF
#!/usr/bin/env bash
echo "\$@" > "$TMP/last_args"
echo "\$@" >> "$TMP/all_args"
echo "\${WARP_AGENT_RESUME_STARTED_FRESH:-}" > "$TMP/last_fresh_marker"
exit "\$(cat "$TMP/claude_rc" 2>/dev/null || echo 0)"
EOF
chmod +x "$TMP/bin/claude"
export PATH="$TMP/bin:$PATH"
source "$HERE/claude.zsh"

# Run the pre-URL cases without a pane uuid so no cloud-URL registry lookup interferes
# (this test may itself run inside a Warp pane).
unset WARP_TERMINAL_SESSION_UUID

EHOME="$TMP/home"
mkdir -p "$EHOME/.claude/projects/-tmp-repo"
printf '{"type":"user","message":{}}\n' > "$EHOME/.claude/projects/-tmp-repo/good-1.jsonl"  # real turn
printf '{"type":"bridge-session"}\n'    > "$EHOME/.claude/projects/-tmp-repo/stub-1.jsonl"  # stub, 0 turns

HOME="$EHOME" warp_agent_resume_resumable claude good-1    || { echo "FAIL: good should be resumable"; exit 1; }
HOME="$EHOME" warp_agent_resume_resumable claude stub-1    && { echo "FAIL: stub should NOT be resumable"; exit 1; }
HOME="$EHOME" warp_agent_resume_resumable claude missing-1 && { echo "FAIL: missing should NOT be resumable"; exit 1; }

# Resumable -> resume that id.
rm -f "$TMP/last_args"
HOME="$EHOME" warp_agent_resume_launch claude good-1
grep -q -- '--resume good-1' "$TMP/last_args" || { echo "FAIL: resumable session should resume"; exit 1; }

# Not resumable -> start fresh (call claude with no --resume).
rm -f "$TMP/last_args"
HOME="$EHOME" warp_agent_resume_launch claude stub-1
[[ -f "$TMP/last_args" ]] || { echo "FAIL: fallback should launch claude"; exit 1; }
grep -q -- '--resume' "$TMP/last_args" && { echo "FAIL: fallback must not resume"; exit 1; }

# Extra launch flags (permission mode + model) are forwarded -- on resume...
rm -f "$TMP/last_args"
HOME="$EHOME" warp_agent_resume_launch claude good-1 --dangerously-skip-permissions --model opus
grep -q -- '--resume good-1' "$TMP/last_args"               || { echo "FAIL: resumable should still resume"; exit 1; }
grep -q -- '--dangerously-skip-permissions' "$TMP/last_args" || { echo "FAIL: skip-permissions not forwarded on resume"; exit 1; }
grep -q -- '--model opus' "$TMP/last_args"                  || { echo "FAIL: model not forwarded on resume"; exit 1; }

# ...and on the fresh fallback (so a non-resumable bypass session still restarts in bypass).
rm -f "$TMP/last_args"
HOME="$EHOME" warp_agent_resume_launch claude stub-1 --dangerously-skip-permissions
grep -q -- '--resume' "$TMP/last_args"                       && { echo "FAIL: fallback must not resume"; exit 1; }
grep -q -- '--dangerously-skip-permissions' "$TMP/last_args" || { echo "FAIL: skip-permissions not forwarded on fresh fallback"; exit 1; }

# --- Bridged sessions: teleport-first (cloud copy is authoritative) ---
export WARP_AGENT_RESUME_DIR="$TMP/reg"
export WARP_TERMINAL_SESSION_UUID="pane99"
mkdir -p "$WARP_AGENT_RESUME_DIR"
printf '{ "command": "warp_agent_resume_launch claude good-1", "cwd": "/tmp/repo", "bridge": "session_01XYZ" }\n' \
  > "$WARP_AGENT_RESUME_DIR/pane99.json"

# Bridge recorded -> teleport (with launch flags forwarded), even when a local jsonl with a
# real turn exists (a bridged session's local file is a stale husk).
rm -f "$TMP/last_args" "$TMP/all_args" "$TMP/claude_rc"
HOME="$EHOME" warp_agent_resume_launch claude good-1 --dangerously-skip-permissions
grep -q -- '--teleport session_01XYZ' "$TMP/last_args"       || { echo "FAIL: bridged session should teleport"; exit 1; }
grep -q -- '--dangerously-skip-permissions' "$TMP/last_args" || { echo "FAIL: flags not forwarded to teleport"; exit 1; }
grep -q -- '--resume' "$TMP/all_args"                        && { echo "FAIL: successful teleport must not also resume"; exit 1; }

# Teleport that fails FAST falls back to the local paths (resume here) and prints the
# cloud URL for manual recovery.
rm -f "$TMP/last_args" "$TMP/all_args"
echo 1 > "$TMP/claude_rc"
err="$(HOME="$EHOME" warp_agent_resume_launch claude good-1 2>&1 >/dev/null)" || true
grep -q -- '--teleport session_01XYZ' "$TMP/all_args" || { echo "FAIL: teleport not attempted"; exit 1; }
grep -q -- '--resume good-1' "$TMP/all_args"          || { echo "FAIL: fast-fail teleport should fall back to resume"; exit 1; }
[[ "$err" == *"https://claude.ai/code/session_01XYZ"* ]] || { echo "FAIL: fallback missing cloud URL"; exit 1; }

# ...and to a fresh session when nothing is locally resumable.
rm -f "$TMP/last_args" "$TMP/all_args"
printf '{ "command": "warp_agent_resume_launch claude stub-1", "cwd": "/tmp/repo", "bridge": "session_01XYZ" }\n' \
  > "$WARP_AGENT_RESUME_DIR/pane99.json"
HOME="$EHOME" warp_agent_resume_launch claude stub-1 || true
grep -q -- '--teleport' "$TMP/last_args" && { echo "FAIL: fallback fresh launch must not re-teleport"; exit 1; }
grep -q -- '--resume' "$TMP/last_args"   && { echo "FAIL: stub must not resume"; exit 1; }

# A non-zero exit AFTER a real run is the user quitting -- no relaunch on top.
# GRACE=-1 makes every run count as "real" without sleeping in the test.
rm -f "$TMP/last_args" "$TMP/all_args"
WARP_AGENT_RESUME_TELEPORT_GRACE=-1 HOME="$EHOME" warp_agent_resume_launch claude good-1 || true
(( $(wc -l < "$TMP/all_args") == 1 )) || { echo "FAIL: long-run teleport exit must not relaunch"; exit 1; }
echo 0 > "$TMP/claude_rc"

# Malformed bridge values are ignored -> normal resume path.
rm -f "$TMP/last_args" "$TMP/all_args"
printf '{ "command": "warp_agent_resume_launch claude good-1", "cwd": "/tmp/repo", "bridge": "garbage value" }\n' \
  > "$WARP_AGENT_RESUME_DIR/pane99.json"
HOME="$EHOME" warp_agent_resume_launch claude good-1
grep -q -- '--teleport' "$TMP/last_args"      && { echo "FAIL: malformed bridge must not teleport"; exit 1; }
grep -q -- '--resume good-1' "$TMP/last_args" || { echo "FAIL: malformed bridge should fall back to resume"; exit 1; }

# No bridge recorded -> normal resume path, no teleport.
printf '{ "command": "warp_agent_resume_launch claude good-1", "cwd": "/tmp/repo" }\n' \
  > "$WARP_AGENT_RESUME_DIR/pane99.json"
rm -f "$TMP/last_args" "$TMP/all_args"
HOME="$EHOME" warp_agent_resume_launch claude good-1
grep -q -- '--teleport' "$TMP/last_args"      && { echo "FAIL: teleport without bridge field"; exit 1; }
grep -q -- '--resume good-1' "$TMP/last_args" || { echo "FAIL: unbridged session should resume"; exit 1; }

# Outside a pane (no WARP_TERMINAL_SESSION_UUID): no teleport, no error.
unset WARP_TERMINAL_SESSION_UUID
rm -f "$TMP/last_args" "$TMP/all_args"
HOME="$EHOME" warp_agent_resume_launch claude good-1
grep -q -- '--teleport' "$TMP/last_args" && { echo "FAIL: teleport without pane uuid"; exit 1; }

# --- Registry-rot fallback: a dead id adopts the newest unclaimed session for this cwd ---
export WARP_AGENT_RESUME_DIR="$TMP/reg2"
mkdir -p "$WARP_AGENT_RESUME_DIR"
WORK="$TMP/work"; mkdir -p "$WORK"
PROJ="$EHOME/.claude/projects/-work"
mkdir -p "$PROJ/subagents"
printf '{"type":"user","cwd":"%s","message":{}}\n' "$WORK" > "$PROJ/lost-1.jsonl"
printf '{"type":"user","cwd":"%s","message":{}}\n' "$WORK" > "$PROJ/lost-2.jsonl"
printf '{"type":"user","cwd":"%s","message":{}}\n' "$WORK" > "$PROJ/subagents/side-1.jsonl"  # sidechain
printf '{"type":"bridge-session","cwd":"%s"}\n'    "$WORK" > "$PROJ/stub-9.jsonl"            # stub, 0 turns
printf '{"type":"user","cwd":"/somewhere/else","message":{}}\n' > "$PROJ/other-cwd.jsonl"
touch -t 202607010000 "$PROJ/lost-1.jsonl"
touch -t 202607020000 "$PROJ/lost-2.jsonl"           # newest REAL match for $WORK…
touch -t 202607030000 "$PROJ/subagents/side-1.jsonl" # …these three are newer but must be skipped
touch -t 202607040000 "$PROJ/stub-9.jsonl"
touch -t 202607050000 "$PROJ/other-cwd.jsonl"

# Dead id -> adopt the newest resumable session recorded for this cwd (sidechains, stubs,
# and other directories skipped), forwarding launch flags. Adoption is a resume, so the
# machinery fresh marker must NOT be set.
rm -f "$TMP/last_args"
( cd "$WORK" && HOME="$EHOME" warp_agent_resume_launch claude dead-1 --dangerously-skip-permissions )
grep -q -- '--resume lost-2' "$TMP/last_args" || { echo "FAIL: dead id should adopt newest cwd session"; exit 1; }
grep -q -- '--dangerously-skip-permissions' "$TMP/last_args" || { echo "FAIL: flags not forwarded on adoption"; exit 1; }
[[ -z "$(cat "$TMP/last_fresh_marker")" ]] || { echo "FAIL: adoption must not set the fresh marker"; exit 1; }

# A session claimed by another pane's registry entry is skipped -> next unclaimed wins
# (a pane whose id died must never steal a sibling pane's live session).
printf '{ "command": "warp_agent_resume_launch claude lost-2 --model opus", "cwd": "%s" }\n' "$WORK" \
  > "$WARP_AGENT_RESUME_DIR/other-pane.json"
rm -f "$TMP/last_args"
( cd "$WORK" && HOME="$EHOME" warp_agent_resume_launch claude dead-1 )
grep -q -- '--resume lost-1' "$TMP/last_args" || { echo "FAIL: claimed session must not be stolen"; exit 1; }

# Everything for this cwd claimed -> start fresh, tagged with the machinery marker so the
# capture hook knows this blank must not clobber a protected entry.
printf '{ "command": "warp_agent_resume_launch claude lost-1", "cwd": "%s" }\n' "$WORK" \
  > "$WARP_AGENT_RESUME_DIR/other-pane2.json"
rm -f "$TMP/last_args"
( cd "$WORK" && HOME="$EHOME" warp_agent_resume_launch claude dead-1 )
grep -q -- '--resume' "$TMP/last_args" && { echo "FAIL: fully-claimed cwd must start fresh"; exit 1; }
[[ "$(cat "$TMP/last_fresh_marker")" == "1" ]] || { echo "FAIL: machinery fresh launch must set the marker"; exit 1; }

echo "PASS"
