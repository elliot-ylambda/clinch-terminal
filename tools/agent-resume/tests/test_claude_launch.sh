#!/usr/bin/env zsh
# Tests the replay side in claude.zsh: resume a session only if it has a real conversation,
# otherwise start fresh (the stub/missing case must not error out with "No conversation found").
set -euo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d)"
mkdir -p "$TMP/bin"
# Fake `claude` that records the args and environment it was called with (path baked in).
# Each call also appends to all_args so fallback chains are assertable, records whether the
# machinery fresh marker (WARP_AGENT_RESUME_STARTED_FRESH) was set, and the exit code is
# configurable via $TMP/claude_rc (defaults to 0).
cat > "$TMP/bin/claude" <<EOF
#!/usr/bin/env bash
echo "\$@" > "$TMP/last_args"
echo "\$@" >> "$TMP/all_args"
printf '%s\n' "\$@" > "$TMP/last_argv"
env > "$TMP/last_env"
echo "\${WARP_AGENT_RESUME_STARTED_FRESH:-}" > "$TMP/last_fresh_marker"
exit "\$(cat "$TMP/claude_rc" 2>/dev/null || echo 0)"
EOF
chmod +x "$TMP/bin/claude"
export PATH="$TMP/bin:$PATH"
source "$HERE/claude.zsh"

# The interactive-shell wrapper strips only inherited Claude session identity, while
# preserving exact argv boundaries and user-selected behavior/unrelated environment.
export CLAUDE_CODE_SESSION_ID="stale-session"
export CLAUDE_CODE_BRIDGE_SESSION_ID="session_01STALE"
export CLAUDE_CODE_REMOTE_SESSION_ID="remote-stale"
export CLAUDE_CODE_CHILD_SESSION=1
export CLAUDECODE=1
export CLAUDE_CODE_ENTRYPOINT=child
export CLAUDE_CODE_EXECPATH=/tmp/stale-claude
export AI_AGENT=claude-code_stale
export CLAUDE_EFFORT=high
export WARP_AGENT_RESUME_TEST_UNRELATED=preserved
claude --teleport session_01WRAPPER --model "two words" 'literal*'
[[ "$(< "$TMP/last_argv")" == $'--teleport\nsession_01WRAPPER\n--model\ntwo words\nliteral*' ]] \
  || { echo "FAIL: claude wrapper changed argv boundaries"; exit 1; }
for name in CLAUDE_CODE_SESSION_ID CLAUDE_CODE_BRIDGE_SESSION_ID \
  CLAUDE_CODE_REMOTE_SESSION_ID CLAUDE_CODE_CHILD_SESSION CLAUDECODE \
  CLAUDE_CODE_ENTRYPOINT CLAUDE_CODE_EXECPATH AI_AGENT; do
  grep -q "^${name}=" "$TMP/last_env" \
    && { echo "FAIL: claude wrapper leaked $name"; exit 1; }
done
grep -q '^CLAUDE_EFFORT=high$' "$TMP/last_env" \
  || { echo "FAIL: claude wrapper stripped user behavior"; exit 1; }
grep -q '^WARP_AGENT_RESUME_TEST_UNRELATED=preserved$' "$TMP/last_env" \
  || { echo "FAIL: claude wrapper stripped unrelated env"; exit 1; }

# Run the pre-URL cases without a pane uuid so no cloud-URL registry lookup interferes
# (this test may itself run inside a Clinch pane).
unset WARP_TERMINAL_SESSION_UUID

EHOME="$TMP/home"
mkdir -p "$EHOME/.claude/projects/-tmp-repo"
printf '{"type":"user","message":{}}\n' > "$EHOME/.claude/projects/-tmp-repo/good-1.jsonl"  # real turn
printf '{"type":"bridge-session"}\n'    > "$EHOME/.claude/projects/-tmp-repo/stub-1.jsonl"  # stub, 0 turns

HOME="$EHOME" clinch_agent_resume_resumable claude good-1    || { echo "FAIL: good should be resumable"; exit 1; }
HOME="$EHOME" clinch_agent_resume_resumable claude stub-1    && { echo "FAIL: stub should NOT be resumable"; exit 1; }
HOME="$EHOME" clinch_agent_resume_resumable claude missing-1 && { echo "FAIL: missing should NOT be resumable"; exit 1; }

# Resumable -> resume that id.
rm -f "$TMP/last_args"
HOME="$EHOME" clinch_agent_resume_launch claude good-1
grep -q -- '--resume good-1' "$TMP/last_args" || { echo "FAIL: resumable session should resume"; exit 1; }

# Legacy registry commands still work, but all status text comes from the Clinch launcher.
rm -f "$TMP/last_args"
legacy_err="$(HOME="$EHOME" warp_agent_resume_launch claude missing-legacy 2>&1 >/dev/null)" || true
[[ -f "$TMP/last_args" ]] || { echo "FAIL: legacy launcher alias did not run"; exit 1; }
[[ "$legacy_err" == clinch:* ]] || { echo "FAIL: legacy launcher did not use Clinch status text"; exit 1; }
[[ "$legacy_err" != warp:* ]] || { echo "FAIL: legacy launcher leaked Warp status text"; exit 1; }

# Not resumable -> start fresh (call claude with no --resume).
rm -f "$TMP/last_args"
HOME="$EHOME" clinch_agent_resume_launch claude stub-1
[[ -f "$TMP/last_args" ]] || { echo "FAIL: fallback should launch claude"; exit 1; }
grep -q -- '--resume' "$TMP/last_args" && { echo "FAIL: fallback must not resume"; exit 1; }

# Extra launch flags (permission mode + model) are forwarded -- on resume...
rm -f "$TMP/last_args"
HOME="$EHOME" clinch_agent_resume_launch claude good-1 --dangerously-skip-permissions --model opus
grep -q -- '--resume good-1' "$TMP/last_args"               || { echo "FAIL: resumable should still resume"; exit 1; }
grep -q -- '--dangerously-skip-permissions' "$TMP/last_args" || { echo "FAIL: skip-permissions not forwarded on resume"; exit 1; }
grep -q -- '--model opus' "$TMP/last_args"                  || { echo "FAIL: model not forwarded on resume"; exit 1; }

# ...and on the fresh fallback (so a non-resumable bypass session still restarts in bypass).
rm -f "$TMP/last_args"
HOME="$EHOME" clinch_agent_resume_launch claude stub-1 --dangerously-skip-permissions
grep -q -- '--resume' "$TMP/last_args"                       && { echo "FAIL: fallback must not resume"; exit 1; }
grep -q -- '--dangerously-skip-permissions' "$TMP/last_args" || { echo "FAIL: skip-permissions not forwarded on fresh fallback"; exit 1; }

# --- Bridged sessions: teleport-first (cloud copy is authoritative) ---
export WARP_AGENT_RESUME_DIR="$TMP/reg"
export WARP_TERMINAL_SESSION_UUID="pane99"
mkdir -p "$WARP_AGENT_RESUME_DIR"
printf '{ "command": "clinch_agent_resume_launch claude good-1", "cwd": "/tmp/repo", "bridge": "session_01XYZ" }\n' \
  > "$WARP_AGENT_RESUME_DIR/pane99.json"

# Bridge recorded -> teleport (with launch flags forwarded), even when a local jsonl with a
# real turn exists (a bridged session's local file is a stale husk).
rm -f "$TMP/last_args" "$TMP/all_args" "$TMP/claude_rc"
HOME="$EHOME" clinch_agent_resume_launch claude good-1 --dangerously-skip-permissions
grep -q -- '--teleport session_01XYZ' "$TMP/last_args"       || { echo "FAIL: bridged session should teleport"; exit 1; }
grep -q -- '--dangerously-skip-permissions' "$TMP/last_args" || { echo "FAIL: flags not forwarded to teleport"; exit 1; }
grep -q -- '--resume' "$TMP/all_args"                        && { echo "FAIL: successful teleport must not also resume"; exit 1; }

# Teleport that fails FAST falls back to the local paths (resume here) and prints the
# cloud URL for manual recovery.
rm -f "$TMP/last_args" "$TMP/all_args"
echo 1 > "$TMP/claude_rc"
err="$(HOME="$EHOME" clinch_agent_resume_launch claude good-1 2>&1 >/dev/null)" || true
grep -q -- '--teleport session_01XYZ' "$TMP/all_args" || { echo "FAIL: teleport not attempted"; exit 1; }
grep -q -- '--resume good-1' "$TMP/all_args"          || { echo "FAIL: fast-fail teleport should fall back to resume"; exit 1; }
[[ "$err" == *"https://claude.ai/code/session_01XYZ"* ]] || { echo "FAIL: fallback missing cloud URL"; exit 1; }

# ...and to a fresh session when nothing is locally resumable.
rm -f "$TMP/last_args" "$TMP/all_args"
printf '{ "command": "clinch_agent_resume_launch claude stub-1", "cwd": "/tmp/repo", "bridge": "session_01XYZ" }\n' \
  > "$WARP_AGENT_RESUME_DIR/pane99.json"
HOME="$EHOME" clinch_agent_resume_launch claude stub-1 || true
grep -q -- '--teleport' "$TMP/last_args" && { echo "FAIL: fallback fresh launch must not re-teleport"; exit 1; }
grep -q -- '--resume' "$TMP/last_args"   && { echo "FAIL: stub must not resume"; exit 1; }

# A non-zero exit AFTER a real run is the user quitting -- no relaunch on top.
# GRACE=-1 makes every run count as "real" without sleeping in the test.
rm -f "$TMP/last_args" "$TMP/all_args"
WARP_AGENT_RESUME_TELEPORT_GRACE=-1 HOME="$EHOME" clinch_agent_resume_launch claude good-1 || true
(( $(wc -l < "$TMP/all_args") == 1 )) || { echo "FAIL: long-run teleport exit must not relaunch"; exit 1; }
echo 0 > "$TMP/claude_rc"

# Malformed bridge values are ignored -> normal resume path.
rm -f "$TMP/last_args" "$TMP/all_args"
printf '{ "command": "clinch_agent_resume_launch claude good-1", "cwd": "/tmp/repo", "bridge": "garbage value" }\n' \
  > "$WARP_AGENT_RESUME_DIR/pane99.json"
HOME="$EHOME" clinch_agent_resume_launch claude good-1
grep -q -- '--teleport' "$TMP/last_args"      && { echo "FAIL: malformed bridge must not teleport"; exit 1; }
grep -q -- '--resume good-1' "$TMP/last_args" || { echo "FAIL: malformed bridge should fall back to resume"; exit 1; }

# No bridge recorded -> normal resume path, no teleport.
printf '{ "command": "clinch_agent_resume_launch claude good-1", "cwd": "/tmp/repo" }\n' \
  > "$WARP_AGENT_RESUME_DIR/pane99.json"
rm -f "$TMP/last_args" "$TMP/all_args"
HOME="$EHOME" clinch_agent_resume_launch claude good-1
grep -q -- '--teleport' "$TMP/last_args"      && { echo "FAIL: teleport without bridge field"; exit 1; }
grep -q -- '--resume good-1' "$TMP/last_args" || { echo "FAIL: unbridged session should resume"; exit 1; }

# Outside a pane (no WARP_TERMINAL_SESSION_UUID): no teleport, no error.
unset WARP_TERMINAL_SESSION_UUID
rm -f "$TMP/last_args" "$TMP/all_args"
HOME="$EHOME" clinch_agent_resume_launch claude good-1
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
( cd "$WORK" && HOME="$EHOME" clinch_agent_resume_launch claude dead-1 --dangerously-skip-permissions )
grep -q -- '--resume lost-2' "$TMP/last_args" || { echo "FAIL: dead id should adopt newest cwd session"; exit 1; }
grep -q -- '--dangerously-skip-permissions' "$TMP/last_args" || { echo "FAIL: flags not forwarded on adoption"; exit 1; }
[[ -z "$(cat "$TMP/last_fresh_marker")" ]] || { echo "FAIL: adoption must not set the fresh marker"; exit 1; }

# A session claimed by another pane's registry entry is skipped -> next unclaimed wins
# (a pane whose id died must never steal a sibling pane's live session).
printf '{ "command": "clinch_agent_resume_launch claude lost-2 --model opus", "cwd": "%s" }\n' "$WORK" \
  > "$WARP_AGENT_RESUME_DIR/other-pane.json"
rm -f "$TMP/last_args"
( cd "$WORK" && HOME="$EHOME" clinch_agent_resume_launch claude dead-1 )
grep -q -- '--resume lost-1' "$TMP/last_args" || { echo "FAIL: claimed session must not be stolen"; exit 1; }

# Everything for this cwd claimed -> start fresh, tagged with the machinery marker so the
# capture hook knows this blank must not clobber a protected entry.
printf '{ "command": "clinch_agent_resume_launch claude lost-1", "cwd": "%s" }\n' "$WORK" \
  > "$WARP_AGENT_RESUME_DIR/other-pane2.json"
rm -f "$TMP/last_args"
( cd "$WORK" && HOME="$EHOME" clinch_agent_resume_launch claude dead-1 )
grep -q -- '--resume' "$TMP/last_args" && { echo "FAIL: fully-claimed cwd must start fresh"; exit 1; }
[[ "$(cat "$TMP/last_fresh_marker")" == "1" ]] || { echo "FAIL: machinery fresh launch must set the marker"; exit 1; }

# --- Adoption claims: simultaneous restores must not adopt the same session twice ---
# The fake claude never runs the SessionStart hook, so nothing re-captures the adopted id
# into the registry; only the atomic claim file can close that real restore-time race.
export WARP_AGENT_RESUME_DIR="$TMP/reg3"
mkdir -p "$WARP_AGENT_RESUME_DIR"
rm -f "$TMP/last_args"
( cd "$WORK" && HOME="$EHOME" clinch_agent_resume_launch claude dead-A )
grep -q -- '--resume lost-2' "$TMP/last_args" || { echo "FAIL: first pane should adopt newest"; exit 1; }
rm -f "$TMP/last_args"
( cd "$WORK" && HOME="$EHOME" clinch_agent_resume_launch claude dead-B )
grep -q -- '--resume lost-1' "$TMP/last_args" || { echo "FAIL: claimed session adopted twice"; exit 1; }

# A stale claim is reclaimable so a crashed restorer cannot block adoption forever.
rm -f "$TMP/last_args" "$WARP_AGENT_RESUME_DIR/.adopt-claim-lost-1"
touch -t 202601010000 "$WARP_AGENT_RESUME_DIR/.adopt-claim-lost-2"
( cd "$WORK" && HOME="$EHOME" clinch_agent_resume_launch claude dead-C )
grep -q -- '--resume lost-2' "$TMP/last_args" || { echo "FAIL: stale claim should be reclaimable"; exit 1; }

echo "PASS"
