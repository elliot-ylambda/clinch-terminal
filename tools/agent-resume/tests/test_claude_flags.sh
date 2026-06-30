#!/usr/bin/env bash
# Tests the pure flag-extraction helper used by the capture hook. From the live `claude`
# process argv it carries forward the permission mode (--dangerously-skip-permissions or
# --permission-mode <mode>) and the --model, so a restored session resumes the way it was
# launched (e.g. the `CA` alias = `claude --dangerously-skip-permissions`).
set -uo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"

# Source the hook for its functions only; sourcing must NOT run the capture body.
source "$HERE/claude-session-start.sh"

fail() { echo "FAIL: $1"; exit 1; }

# 1. --dangerously-skip-permissions (boolean) is carried through.
out="$(_warp_agent_resume_extract_flags "node /x/claude-code/cli.js --dangerously-skip-permissions")"
[[ "$out" == *"--dangerously-skip-permissions"* ]] || fail "skip-permissions not carried (got: '$out')"

# 2. --permission-mode <mode> (space form) carried with its value.
out="$(_warp_agent_resume_extract_flags "claude --permission-mode plan")"
[[ "$out" == *"--permission-mode plan"* ]] || fail "permission-mode space form (got: '$out')"

# 3. --permission-mode=<mode> (equals form) normalized to space form.
out="$(_warp_agent_resume_extract_flags "claude --permission-mode=acceptEdits")"
[[ "$out" == *"--permission-mode acceptEdits"* ]] || fail "permission-mode equals form (got: '$out')"

# 4. --model carried (both space and equals forms), alongside the mode.
out="$(_warp_agent_resume_extract_flags "claude --dangerously-skip-permissions --model opus")"
[[ "$out" == *"--dangerously-skip-permissions"* && "$out" == *"--model opus"* ]] || fail "model space form (got: '$out')"
out="$(_warp_agent_resume_extract_flags "claude --model=sonnet")"
[[ "$out" == *"--model sonnet"* ]] || fail "model equals form (got: '$out')"

# 5. No relevant flags -> empty.
out="$(_warp_agent_resume_extract_flags "node /x/claude-code/cli.js")"
[[ -z "$out" ]] || fail "expected empty for no flags (got: '$out')"

# 6. Unrelated flags (incl. a stale --resume) are ignored: we only carry mode + model.
out="$(_warp_agent_resume_extract_flags "claude --verbose --resume old-id")"
[[ -z "$out" ]] || fail "unrelated flags must be ignored (got: '$out')"

echo "PASS"
