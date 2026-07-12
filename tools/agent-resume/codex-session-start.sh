#!/usr/bin/env bash
# Codex SessionStart hook: record the live session for this Clinch pane so it can be
# resumed on restore. The payload also carries permission_mode and model, so the
# recorded resume command reopens the session the way it is currently running.
# Unknown or absent fields degrade to a plain resume (fail-safe).
set -euo pipefail
[[ -n "${WARP_TERMINAL_SESSION_UUID:-}" ]] || exit 0
payload="$(cat)"
# Absolute sibling path: hooks do not reliably inherit the shell PATH.
BIN="$(cd "$(dirname "$0")" && pwd)"
# Direct and detached nested agents inherit the pane UUID but must not replace the visible
# outer target. The owner fields combine ancestry with the already-recorded live PID/tty.
owner_fields="$("$BIN/clinch-agent-resume" hook-owner-fields 2>/dev/null)" || exit 0
IFS='|' read -r owner_pid owner_tty64 <<<"$owner_fields"
fields="$(printf '%s' "$payload" | "$BIN/agent-json" hook-fields 2>/dev/null)" || exit 0
IFS='|' read -r sid64 cwd64 _event64 pmode64 model64 <<<"$fields"
decode() { printf '%s' "$1" | /usr/bin/base64 -D 2>/dev/null; }
sid="$(decode "$sid64")" || exit 0
cwd="$(decode "$cwd64")" || exit 0
pmode="$(decode "$pmode64")" || exit 0
model="$(decode "$model64")" || exit 0
owner_tty="$(decode "$owner_tty64")" || exit 0
[[ "$sid" =~ ^[A-Za-z0-9-]+$ ]] || exit 0
extra=""
[[ "$pmode" == "bypassPermissions" ]] && extra+=" --dangerously-bypass-approvals-and-sandbox"
[[ "$model" =~ ^[A-Za-z0-9._:/-]+$ ]] && extra+=" --model $model"
"$BIN/clinch-agent-resume" write "$WARP_TERMINAL_SESSION_UUID" \
  "clinch_agent_resume_launch codex $sid$extra" "$cwd" "" "$owner_pid" "$owner_tty"
