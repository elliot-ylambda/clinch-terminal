#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d)"
export WARP_AGENT_RESUME_DIR="$TMP/reg"
# No PATH export: the hooks must find clinch-agent-resume as a sibling of the script,
# because the real hook environment does not inherit the shell PATH.
export WARP_TERMINAL_SESSION_UUID="bb22"
export WARP_AGENT_RESUME_FAKE_ANCESTRY="codex"

echo '{"session_id":"sess-77","cwd":"/tmp/repo","source":"startup"}' | bash "$HERE/codex-session-start.sh"
f="$WARP_AGENT_RESUME_DIR/bb22.json"
grep -q '"command": "clinch_agent_resume_launch codex sess-77"' "$f" || { echo "FAIL: start"; exit 1; }
grep -q '"cwd": "/tmp/repo"' "$f" || { echo "FAIL: cwd"; exit 1; }

# Bypass + model from the payload are carried into the resume command.
echo '{"session_id":"sess-88","cwd":"/tmp/repo","permission_mode":"bypassPermissions","model":"gpt-5.3-codex"}' | bash "$HERE/codex-session-start.sh"
grep -q '"command": "clinch_agent_resume_launch codex sess-88 --dangerously-bypass-approvals-and-sandbox --model gpt-5.3-codex"' "$f" || { echo "FAIL: codex bypass+model"; exit 1; }

# Non-bypass modes carry only the model (conservative mapping).
echo '{"session_id":"sess-99","cwd":"/tmp/repo","permission_mode":"acceptEdits","model":"gpt-5.3-codex"}' | bash "$HERE/codex-session-start.sh"
grep -q '"command": "clinch_agent_resume_launch codex sess-99 --model gpt-5.3-codex"' "$f" || { echo "FAIL: codex non-bypass carries model only"; exit 1; }

# Nested Codex must not replace or remove an outer pane owner.
echo '{"session_id":"sess-nested","cwd":"/tmp/child"}' \
  | WARP_AGENT_RESUME_FAKE_ANCESTRY="codex,zsh,claude" bash "$HERE/codex-session-start.sh"
grep -q 'sess-99' "$f" || { echo "FAIL: nested Codex replaced outer owner"; exit 1; }
echo '{"session_id":"sess-nested","cwd":"/tmp/child"}' \
  | WARP_AGENT_RESUME_FAKE_ANCESTRY="codex,zsh,claude" bash "$HERE/codex-session-end.sh"
grep -q 'sess-99' "$f" || { echo "FAIL: nested Codex end removed outer owner"; exit 1; }

# A mismatched end is a no-op; app termination preserves; a normal matching end removes.
echo '{"session_id":"sess-77","cwd":"/tmp/repo"}' | bash "$HERE/codex-session-end.sh"
grep -q 'sess-99' "$f" || { echo "FAIL: mismatched Codex end removed owner"; exit 1; }
printf '%s\n' "$$" > "$WARP_AGENT_RESUME_DIR/.app-terminating"
echo '{"session_id":"sess-99","cwd":"/tmp/repo"}' | bash "$HERE/codex-session-end.sh"
grep -q 'sess-99' "$f" || { echo "FAIL: app shutdown removed Codex owner"; exit 1; }
rm -f "$WARP_AGENT_RESUME_DIR/.app-terminating"
echo '{"session_id":"sess-99","cwd":"/tmp/repo"}' | bash "$HERE/codex-session-end.sh"
[[ ! -f "$f" ]] || { echo "FAIL: end did not remove"; exit 1; }

# No-op outside a Clinch pane. (The registry dir legitimately holds journal.jsonl from the
# writes above, so assert on pane entries, not an empty dir.)
unset WARP_TERMINAL_SESSION_UUID
echo '{"session_id":"x","cwd":"/tmp"}' | bash "$HERE/codex-session-start.sh"
entries="$(find "$WARP_AGENT_RESUME_DIR" -name '*.json' 2>/dev/null)"
[[ -z "$entries" ]] || { echo "FAIL: wrote outside pane"; exit 1; }
echo "PASS"
