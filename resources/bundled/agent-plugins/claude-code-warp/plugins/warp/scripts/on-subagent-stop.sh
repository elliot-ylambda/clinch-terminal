#!/bin/bash
# Hook script for Claude Code SubagentStop events.
# Completes the matching subagent lifecycle in Clinch.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/should-use-structured.sh"

# No legacy equivalent for this hook.
if ! should_use_structured; then
    exit 0
fi

source "$SCRIPT_DIR/build-payload.sh"

INPUT=$(cat)
SUBAGENT_ID=$(echo "$INPUT" | jq -r '.agent_id // empty' 2>/dev/null)

# Ignore orphaned events without an identity instead of decrementing a lossy counter.
if [ -z "$SUBAGENT_ID" ]; then
    exit 0
fi

BODY=$(build_payload "$INPUT" "subagent_stop" \
    --arg subagent_id "$SUBAGENT_ID")

"$SCRIPT_DIR/warp-notify.sh" "warp://cli-agent" "$BODY"
