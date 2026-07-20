#!/bin/bash
# Hook script for Claude Code StopFailure events. Current Claude Code reports
# API rate limits here instead of through the ordinary Stop hook.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/should-use-structured.sh"

if ! should_use_structured; then
    exit 0
fi

source "$SCRIPT_DIR/build-payload.sh"
source "$SCRIPT_DIR/detect-stop-reason.sh"

INPUT=$(cat)
ERROR_TYPE=$(echo "$INPUT" | jq -r '.error // empty' 2>/dev/null)
ERROR_DETAILS=$(echo "$INPUT" | jq -r '.error_details // empty' 2>/dev/null)
RESPONSE=$(echo "$INPUT" | jq -r '.last_assistant_message // empty' 2>/dev/null)

if [ "$ERROR_TYPE" = "rate_limit" ]; then
    STOP_REASON="usage_limit"
else
    STOP_REASON=$(detect_stop_reason "$RESPONSE")
fi

if [ -n "$RESPONSE" ] && [ ${#RESPONSE} -gt 200 ]; then
    RESPONSE="${RESPONSE:0:197}..."
fi

BODY=$(build_payload "$INPUT" "stop_failure" \
    --arg response "$RESPONSE" \
    --arg summary "$ERROR_DETAILS" \
    --arg stop_reason "$STOP_REASON")

# Claude Code ignores stdout for StopFailure hooks, including structured hook
# output, so deliver this event directly to the pane PTY.
WARP_FORCE_DIRECT_TTY=1 "$SCRIPT_DIR/warp-notify.sh" "warp://cli-agent" "$BODY"
