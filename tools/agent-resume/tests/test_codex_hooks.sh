#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d)"
export WARP_AGENT_RESUME_DIR="$TMP/reg"
# No PATH export: the hooks must find warp-agent-resume as a sibling of the script,
# because the real hook environment does not inherit the shell PATH.
export WARP_TERMINAL_SESSION_UUID="bb22"

echo '{"session_id":"sess-77","cwd":"/tmp/repo","source":"startup"}' | bash "$HERE/codex-session-start.sh"
f="$WARP_AGENT_RESUME_DIR/bb22.json"
grep -q '"command": "warp_agent_resume_launch codex sess-77"' "$f" || { echo "FAIL: start"; exit 1; }
grep -q '"cwd": "/tmp/repo"' "$f" || { echo "FAIL: cwd"; exit 1; }

# Bypass + model from the payload are carried into the resume command.
echo '{"session_id":"sess-88","cwd":"/tmp/repo","permission_mode":"bypassPermissions","model":"gpt-5.3-codex"}' | bash "$HERE/codex-session-start.sh"
grep -q '"command": "warp_agent_resume_launch codex sess-88 --dangerously-bypass-approvals-and-sandbox --model gpt-5.3-codex"' "$f" || { echo "FAIL: codex bypass+model"; exit 1; }

# Non-bypass modes carry only the model (conservative mapping).
echo '{"session_id":"sess-99","cwd":"/tmp/repo","permission_mode":"acceptEdits","model":"gpt-5.3-codex"}' | bash "$HERE/codex-session-start.sh"
grep -q '"command": "warp_agent_resume_launch codex sess-99 --model gpt-5.3-codex"' "$f" || { echo "FAIL: codex non-bypass carries model only"; exit 1; }

echo '{"session_id":"sess-77","cwd":"/tmp/repo"}' | bash "$HERE/codex-session-end.sh"
[[ ! -f "$f" ]] || { echo "FAIL: end did not remove"; exit 1; }

# No-op outside a Warp pane.
unset WARP_TERMINAL_SESSION_UUID
echo '{"session_id":"x","cwd":"/tmp"}' | bash "$HERE/codex-session-start.sh"
[[ -z "$(ls -A "$WARP_AGENT_RESUME_DIR" 2>/dev/null)" ]] || { echo "FAIL: wrote outside pane"; exit 1; }
echo "PASS"
