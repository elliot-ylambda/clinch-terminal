#!/usr/bin/env bash
# Tests the Claude SessionStart capture hook: it records the live session id (fresh,
# --resume, picker, or --continue all deliver session_id in the payload) keyed by pane uuid.
set -euo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d)"
export WARP_AGENT_RESUME_DIR="$TMP/reg"
# The hook calls warp-agent-resume as a sibling; put both in one bin and run from there.
BIN="$TMP/bin"; mkdir -p "$BIN"
install -m 0755 "$HERE/warp-agent-resume" "$HERE/claude-session-start.sh" "$BIN/"

export WARP_TERMINAL_SESSION_UUID="cc33"
f="$WARP_AGENT_RESUME_DIR/cc33.json"

# Pin the launch-flag detection off for the plain cases so they are deterministic regardless
# of how this test was launched (a real claude ancestor must not leak its flags in).
export WARP_AGENT_RESUME_FAKE_ARGV=""

# Fresh/startup: session_id recorded via the launcher form.
echo '{"session_id":"sess-aaa","cwd":"/tmp/repo","source":"startup"}' | "$BIN/claude-session-start.sh"
grep -q '"command": "warp_agent_resume_launch claude sess-aaa"' "$f" || { echo "FAIL: startup not recorded"; exit 1; }
grep -q '"cwd": "/tmp/repo"' "$f" || { echo "FAIL: cwd"; exit 1; }

# Resume/picker: the resumed id must OVERWRITE the pane entry (this is the bug being fixed).
echo '{"session_id":"sess-bbb","cwd":"/tmp/repo","source":"resume"}' | "$BIN/claude-session-start.sh"
grep -q '"command": "warp_agent_resume_launch claude sess-bbb"' "$f" || { echo "FAIL: resume did not overwrite stale entry"; exit 1; }

# Launched in bypass mode with a model override (e.g. the `CA` alias): the recorded resume
# command carries those flags through so restore reopens the session the same way.
WARP_AGENT_RESUME_FAKE_ARGV="node /x/claude-code/cli.js --dangerously-skip-permissions --model opus" \
  bash -c 'echo "{\"session_id\":\"sess-ccc\",\"cwd\":\"/tmp/repo\"}" | "$0"' "$BIN/claude-session-start.sh"
grep -q '"command": "warp_agent_resume_launch claude sess-ccc --dangerously-skip-permissions --model opus"' "$f" \
  || { echo "FAIL: launch flags not carried into resume command"; exit 1; }

# Missing session_id: no-op (don't write garbage).
rm -f "$f"
echo '{"cwd":"/tmp/repo","source":"startup"}' | "$BIN/claude-session-start.sh"
[[ ! -f "$f" ]] || { echo "FAIL: wrote with no session_id"; exit 1; }

# Outside a Warp pane: no-op.
unset WARP_TERMINAL_SESSION_UUID
echo '{"session_id":"x","cwd":"/tmp"}' | "$BIN/claude-session-start.sh"
[[ -z "$(ls -A "$WARP_AGENT_RESUME_DIR" 2>/dev/null)" ]] || { echo "FAIL: wrote outside pane"; exit 1; }

echo "PASS"
