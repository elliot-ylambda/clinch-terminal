#!/usr/bin/env bash
# Codex SessionStart hook: record the live session for this Warp pane so it can be
# resumed on restore. The payload also carries permission_mode and model, so the
# recorded resume command reopens the session the way it is currently running.
# Unknown or absent fields degrade to a plain resume (fail-safe).
set -euo pipefail
[[ -n "${WARP_TERMINAL_SESSION_UUID:-}" ]] || exit 0
payload="$(cat)"
sid="$(printf '%s' "$payload" | jq -r '.session_id // empty')"
cwd="$(printf '%s' "$payload" | jq -r '.cwd // empty')"
pmode="$(printf '%s' "$payload" | jq -r '.permission_mode // empty')"
model="$(printf '%s' "$payload" | jq -r '.model // empty')"
[[ -n "$sid" ]] || exit 0
extra=""
[[ "$pmode" == "bypassPermissions" ]] && extra+=" --dangerously-bypass-approvals-and-sandbox"
[[ -n "$model" ]] && extra+=" --model $model"
# Absolute sibling path: hooks do not reliably inherit the shell PATH.
BIN="$(cd "$(dirname "$0")" && pwd)"
"$BIN/warp-agent-resume" write "$WARP_TERMINAL_SESSION_UUID" "warp_agent_resume_launch codex $sid$extra" "$cwd"
