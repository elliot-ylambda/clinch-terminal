#!/usr/bin/env bash
# Tests the pure flag-extraction helper used by the capture hook. From the live `claude`
# process argv it carries forward the permission mode (--dangerously-skip-permissions or
# --permission-mode <mode>) and the --model, so a restored session resumes the way it was
# launched (e.g. the `CA` alias = `claude --dangerously-skip-permissions`).
set -uo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"

# Source the hook for its functions only; sourcing must NOT run the capture body.
source "$HERE/claude-capture.sh"

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

# 7. _warp_agent_resume_extract_model carries only --model, never the mode flags.
out="$(_warp_agent_resume_extract_model "node /x/cli.js --dangerously-skip-permissions --model opus")"
[[ "$out" == " --model opus" ]] || fail "extract_model carries only the model (got: '$out')"
out="$(_warp_agent_resume_extract_model "claude --model=sonnet")"
[[ "$out" == " --model sonnet" ]] || fail "extract_model equals form (got: '$out')"
out="$(_warp_agent_resume_extract_model "claude --permission-mode plan")"
[[ -z "$out" ]] || fail "extract_model must ignore mode flags (got: '$out')"

# 8. Payload permission_mode -> flags mapping. `default` maps to no flags (still rc 0);
# unknown/empty values return nonzero so the caller falls back to argv detection.
out="$(_warp_agent_resume_mode_flags_from_payload "bypassPermissions")" || fail "bypassPermissions must map"
[[ "$out" == " --dangerously-skip-permissions" ]] || fail "bypassPermissions mapping (got: '$out')"
out="$(_warp_agent_resume_mode_flags_from_payload "plan")" || fail "plan must map"
[[ "$out" == " --permission-mode plan" ]] || fail "plan mapping (got: '$out')"
out="$(_warp_agent_resume_mode_flags_from_payload "acceptEdits")" || fail "acceptEdits must map"
[[ "$out" == " --permission-mode acceptEdits" ]] || fail "acceptEdits mapping (got: '$out')"
out="$(_warp_agent_resume_mode_flags_from_payload "default")" || fail "default must map (to nothing)"
[[ -z "$out" ]] || fail "default maps to no flags (got: '$out')"
_warp_agent_resume_mode_flags_from_payload "somethingNew" >/dev/null && fail "unknown mode must return nonzero"
_warp_agent_resume_mode_flags_from_payload "" >/dev/null && fail "empty mode must return nonzero"

echo "PASS"
