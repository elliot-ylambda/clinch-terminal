#!/usr/bin/env zsh
# Tests the replay side in claude.zsh: resume a session only if it has a real conversation,
# otherwise start fresh (the stub/missing case must not error out with "No conversation found").
set -euo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d)"
mkdir -p "$TMP/bin"
# Fake `claude` that records the args it was called with (path baked in).
cat > "$TMP/bin/claude" <<EOF
#!/usr/bin/env bash
echo "\$@" > "$TMP/last_args"
exit 0
EOF
chmod +x "$TMP/bin/claude"
export PATH="$TMP/bin:$PATH"
source "$HERE/claude.zsh"

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

echo "PASS"
