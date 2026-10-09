#!/bin/bash
# Hook script for Codex PreToolUse event (request_user_input only).
# Codex fires no PermissionRequest when it asks the user a question, so the blocking
# `request_user_input` tool call is the only signal that the turn is waiting on the user.
# The matching PostToolUse (tool_complete) resumes the session once the user answers.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/should-use-structured.sh"

if ! should_use_structured; then
    exit 0
fi

source "$SCRIPT_DIR/build-payload.sh"

INPUT=$(cat)
TOOL_NAME=$(echo "$INPUT" | jq -r '.tool_name // empty' 2>/dev/null || true)

# The hook matcher is a regex, so it also matches `request_user_input_async`, which returns
# immediately and ends the turn (Stop covers that case).
if [ "$TOOL_NAME" != "request_user_input" ]; then
    exit 0
fi

QUESTION=$(echo "$INPUT" | jq -r '
    (.tool_input | if type == "string" then (try fromjson catch {}) else . end) as $input
    | ($input.questions[0].question // $input.question // empty)
' 2>/dev/null || true)
[ -z "$QUESTION" ] && QUESTION="Codex has a question for you"
if [ ${#QUESTION} -gt 120 ]; then
    QUESTION="${QUESTION:0:117}..."
fi

BODY=$(build_payload "$INPUT" "question_asked" \
    --arg summary "$QUESTION" \
    --arg tool_name "$TOOL_NAME")

"$SCRIPT_DIR/warp-notify.sh" "warp://cli-agent" "$BODY"
