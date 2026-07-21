#!/bin/bash
# Hook script for Codex UserPromptSubmit event.
# Sends a structured Warp notification when the user submits a prompt.

# This hook is observational: notification failures must never affect prompt submission.
# In particular, a missing/broken jq or a partially refreshed plugin cache should quietly skip
# the notification instead of surfacing a Codex UserPromptSubmit failure.
set -o pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/should-use-structured.sh" 2>/dev/null || exit 0

if ! should_use_structured; then
    exit 0
fi

source "$SCRIPT_DIR/build-payload.sh" 2>/dev/null || exit 0
command -v jq >/dev/null 2>&1 || exit 0

INPUT=$(cat) || exit 0

QUERY=$(printf '%s' "$INPUT" | jq -r '.prompt // empty' 2>/dev/null) || exit 0
if [ -n "$QUERY" ] && [ ${#QUERY} -gt 200 ]; then
    QUERY="${QUERY:0:197}..."
fi

BODY=$(build_payload "$INPUT" "prompt_submit" \
    --arg query "$QUERY") || exit 0

"$SCRIPT_DIR/warp-notify.sh" "warp://cli-agent" "$BODY" || true
exit 0
