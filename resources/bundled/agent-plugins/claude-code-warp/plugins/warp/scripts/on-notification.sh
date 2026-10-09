#!/bin/bash
# Hook script for Claude Code Notification event (idle_prompt and elicitation_dialog)
# Sends a structured Warp notification when Claude has been idle, or when an MCP server is
# waiting on the user to fill in an elicitation form

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/should-use-structured.sh"

# Legacy fallback for old Warp versions
if ! should_use_structured; then
    [ "$TERM_PROGRAM" = "WarpTerminal" ] && exec "$SCRIPT_DIR/legacy/on-notification.sh"
    exit 0
fi

source "$SCRIPT_DIR/build-payload.sh"

# Read hook input from stdin
INPUT=$(cat)

# Extract notification-specific fields
NOTIF_TYPE=$(echo "$INPUT" | jq -r '.notification_type // "unknown"' 2>/dev/null)
MSG=$(echo "$INPUT" | jq -r '.message // "Input needed"' 2>/dev/null)
[ -z "$MSG" ] && MSG="Input needed"

# An MCP elicitation form blocks the turn until the user answers it.
EVENT="$NOTIF_TYPE"
[ "$NOTIF_TYPE" = "elicitation_dialog" ] && EVENT="question_asked"

BODY=$(build_payload "$INPUT" "$EVENT" \
    --arg summary "$MSG")

"$SCRIPT_DIR/warp-notify.sh" "warp://cli-agent" "$BODY"
