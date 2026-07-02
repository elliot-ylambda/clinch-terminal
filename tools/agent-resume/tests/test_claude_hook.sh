#!/usr/bin/env bash
# Tests the Claude SessionStart capture hook: it records the live session id (fresh,
# --resume, picker, or --continue all deliver session_id in the payload) keyed by pane uuid.
set -euo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d)"
export WARP_AGENT_RESUME_DIR="$TMP/reg"
# The hook calls warp-agent-resume as a sibling; put both in one bin and run from there.
BIN="$TMP/bin"; mkdir -p "$BIN"
install -m 0755 "$HERE/warp-agent-resume" "$HERE/claude-capture.sh" "$BIN/"

export WARP_TERMINAL_SESSION_UUID="cc33"
f="$WARP_AGENT_RESUME_DIR/cc33.json"

# Pin the launch-flag detection off for the plain cases so they are deterministic regardless
# of how this test was launched (a real claude ancestor must not leak its flags in).
export WARP_AGENT_RESUME_FAKE_ARGV=""

# Fresh/startup: session_id recorded via the launcher form.
echo '{"session_id":"sess-aaa","cwd":"/tmp/repo","source":"startup"}' | "$BIN/claude-capture.sh"
grep -q '"command": "warp_agent_resume_launch claude sess-aaa"' "$f" || { echo "FAIL: startup not recorded"; exit 1; }
grep -q '"cwd": "/tmp/repo"' "$f" || { echo "FAIL: cwd"; exit 1; }

# Resume/picker: the resumed id must OVERWRITE the pane entry (this is the bug being fixed).
echo '{"session_id":"sess-bbb","cwd":"/tmp/repo","source":"resume"}' | "$BIN/claude-capture.sh"
grep -q '"command": "warp_agent_resume_launch claude sess-bbb"' "$f" || { echo "FAIL: resume did not overwrite stale entry"; exit 1; }

# Launched in bypass mode with a model override (e.g. the `CA` alias): the recorded resume
# command carries those flags through so restore reopens the session the same way.
WARP_AGENT_RESUME_FAKE_ARGV="node /x/claude-code/cli.js --dangerously-skip-permissions --model opus" \
  bash -c 'echo "{\"session_id\":\"sess-ccc\",\"cwd\":\"/tmp/repo\"}" | "$0"' "$BIN/claude-capture.sh"
grep -q '"command": "warp_agent_resume_launch claude sess-ccc --dangerously-skip-permissions --model opus"' "$f" \
  || { echo "FAIL: launch flags not carried into resume command"; exit 1; }

# Missing session_id: no-op (don't write garbage).
rm -f "$f"
echo '{"cwd":"/tmp/repo","source":"startup"}' | "$BIN/claude-capture.sh"
[[ ! -f "$f" ]] || { echo "FAIL: wrote with no session_id"; exit 1; }

# --- Live-mode updater (UserPromptSubmit / Stop) ---
# The payload's permission_mode is authoritative for the mode; --model still comes from
# the live argv. `default` strips the mode flag; unknown values fall back to argv.

# Toggled to bypass mid-session (entry owned by this sid): entry rewritten with the flag.
echo '{"session_id":"sess-ddd","cwd":"/tmp/repo"}' | "$BIN/claude-capture.sh"
echo '{"session_id":"sess-ddd","cwd":"/tmp/repo","hook_event_name":"UserPromptSubmit","permission_mode":"bypassPermissions"}' | "$BIN/claude-capture.sh"
grep -q '"command": "warp_agent_resume_launch claude sess-ddd --dangerously-skip-permissions"' "$f" || { echo "FAIL: updater did not add bypass flag"; exit 1; }

# Toggled back to default (via Stop): the mode flag must be stripped again.
echo '{"session_id":"sess-ddd","cwd":"/tmp/repo","hook_event_name":"Stop","permission_mode":"default"}' | "$BIN/claude-capture.sh"
grep -q '"command": "warp_agent_resume_launch claude sess-ddd"' "$f" || { echo "FAIL: default did not strip mode flag"; exit 1; }

# plan maps to --permission-mode plan.
echo '{"session_id":"sess-ddd","cwd":"/tmp/repo","hook_event_name":"UserPromptSubmit","permission_mode":"plan"}' | "$BIN/claude-capture.sh"
grep -q '"command": "warp_agent_resume_launch claude sess-ddd --permission-mode plan"' "$f" || { echo "FAIL: plan mode not carried"; exit 1; }

# Model from the live argv is kept alongside the payload mode.
WARP_AGENT_RESUME_FAKE_ARGV="node /x/cli.js --model opus" \
  bash -c 'echo "{\"session_id\":\"sess-ddd\",\"cwd\":\"/tmp/repo\",\"hook_event_name\":\"UserPromptSubmit\",\"permission_mode\":\"bypassPermissions\"}" | "$0"' "$BIN/claude-capture.sh"
grep -q '"command": "warp_agent_resume_launch claude sess-ddd --dangerously-skip-permissions --model opus"' "$f" || { echo "FAIL: model not kept with payload mode"; exit 1; }

# Unknown permission_mode falls back to argv-derived flags (mode + model).
WARP_AGENT_RESUME_FAKE_ARGV="node /x/cli.js --permission-mode acceptEdits" \
  bash -c 'echo "{\"session_id\":\"sess-ddd\",\"cwd\":\"/tmp/repo\",\"hook_event_name\":\"UserPromptSubmit\",\"permission_mode\":\"weird\"}" | "$0"' "$BIN/claude-capture.sh"
grep -q '"command": "warp_agent_resume_launch claude sess-ddd --permission-mode acceptEdits"' "$f" || { echo "FAIL: unknown mode did not fall back to argv"; exit 1; }

# Session-id guard: an updater event from a DIFFERENT session must not clobber the entry…
echo '{"session_id":"sess-intruder","cwd":"/tmp/other","hook_event_name":"UserPromptSubmit","permission_mode":"bypassPermissions"}' | "$BIN/claude-capture.sh"
grep -q 'sess-ddd' "$f" || { echo "FAIL: foreign session clobbered the pane entry"; exit 1; }

# …but a missing entry is (re)created — this heals pre-flag registry entries.
rm -f "$f"
echo '{"session_id":"sess-eee","cwd":"/tmp/repo","hook_event_name":"UserPromptSubmit","permission_mode":"bypassPermissions"}' | "$BIN/claude-capture.sh"
grep -q '"command": "warp_agent_resume_launch claude sess-eee --dangerously-skip-permissions"' "$f" || { echo "FAIL: missing entry not healed"; exit 1; }

# Unknown events are ignored.
echo '{"session_id":"sess-eee","cwd":"/tmp/repo","hook_event_name":"PreCompact","permission_mode":"default"}' | "$BIN/claude-capture.sh"
grep -q -- '--dangerously-skip-permissions' "$f" || { echo "FAIL: unknown event must not rewrite the entry"; exit 1; }
rm -f "$f"   # clear residue so the empty-dir check below only sees new writes

# Outside a Warp pane: no-op.
unset WARP_TERMINAL_SESSION_UUID
echo '{"session_id":"x","cwd":"/tmp"}' | "$BIN/claude-capture.sh"
[[ -z "$(ls -A "$WARP_AGENT_RESUME_DIR" 2>/dev/null)" ]] || { echo "FAIL: wrote outside pane"; exit 1; }

echo "PASS"
