#!/bin/bash
# Hook script for Codex SubagentStart events.
# Sends the stable subagent identity so Clinch can count running subagents and keep the
# parent session active.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/should-use-structured.sh"

# No legacy equivalent for this hook.
if ! should_use_structured; then
    exit 0
fi

source "$SCRIPT_DIR/build-payload.sh"

INPUT=$(cat)
SUBAGENT_ID=$(echo "$INPUT" | jq -r '.agent_id // empty' 2>/dev/null)

# An identity-less event cannot be paired with its counterpart.
if [ -z "$SUBAGENT_ID" ]; then
    exit 0
fi

BODY=$(build_payload "$INPUT" "subagent_start" \
    --arg subagent_id "$SUBAGENT_ID")

"$SCRIPT_DIR/warp-notify.sh" "warp://cli-agent" "$BODY"
