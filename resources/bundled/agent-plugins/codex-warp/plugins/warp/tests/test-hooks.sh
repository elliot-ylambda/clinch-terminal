#!/bin/bash
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../scripts" && pwd)"
source "$SCRIPT_DIR/detect-stop-reason.sh"

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/clinch-codex-hook-test.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT INT TERM

passed=0
failed=0
assert_eq() {
    if [ "$2" = "$3" ]; then
        passed=$((passed + 1))
    else
        printf 'FAIL: %s (expected %q, got %q)\n' "$1" "$2" "$3"
        failed=$((failed + 1))
    fi
}

assert_success() {
    local name="$1"
    shift
    if "$@"; then
        passed=$((passed + 1))
    else
        printf 'FAIL: %s (command exited %s)\n' "$name" "$?"
        failed=$((failed + 1))
    fi
}

run_prompt_hook() {
    local path="$1"
    local payload="$2"
    printf '%s\n' "$payload" | env \
        PATH="$path" \
        WARP_CLI_AGENT_PROTOCOL_VERSION=1 \
        WARP_CLIENT_VERSION=v0.test \
        WARP_TTY="$TMP_DIR/tty" \
        /bin/bash "$SCRIPT_DIR/on-prompt-submit.sh" >/dev/null 2>&1
}

assert_eq "Codex usage limit" "usage_limit" \
    "$(detect_stop_reason "You've hit your usage limit")"
assert_eq "Codex quota" "usage_limit" \
    "$(detect_stop_reason "Quota exceeded. Check your plan and billing details.")"
assert_eq "ordinary completion" "" \
    "$(detect_stop_reason "Implemented the requested changes")"
assert_eq "generic transient error" "" \
    "$(detect_stop_reason "Network request failed")"

PROMPT_PAYLOAD='{"session_id":"fresh-install","cwd":"/tmp/project","hook_event_name":"UserPromptSubmit","prompt":"hello"}'
: > "$TMP_DIR/tty"
assert_success "prompt hook accepts a valid payload" \
    run_prompt_hook "$PATH" "$PROMPT_PAYLOAD"

mkdir "$TMP_DIR/failing-bin"
printf '#!/bin/sh\nexit 1\n' > "$TMP_DIR/failing-bin/jq"
chmod 755 "$TMP_DIR/failing-bin/jq"
assert_success "prompt hook fails open when jq fails" \
    run_prompt_hook "$TMP_DIR/failing-bin:$PATH" "$PROMPT_PAYLOAD"
assert_success "prompt hook fails open for malformed input" \
    run_prompt_hook "$PATH" '{not-json'

# Run a hook from a copy of the scripts whose notifier records the payload instead of writing
# an OSC sequence, and print that payload (empty when the hook stayed silent).
CAPTURE_DIR="$TMP_DIR/capture-scripts"
cp -R "$SCRIPT_DIR" "$CAPTURE_DIR"
printf '#!/bin/bash\nprintf "%%s" "$2" > "%s/body"\n' "$TMP_DIR" > "$CAPTURE_DIR/warp-notify.sh"
run_captured_hook() {
    local hook="$1"
    local payload="$2"
    rm -f "$TMP_DIR/body"
    printf '%s\n' "$payload" | env \
        WARP_CLI_AGENT_PROTOCOL_VERSION=1 \
        WARP_CLIENT_VERSION=v0.test \
        /bin/bash "$CAPTURE_DIR/$hook" >/dev/null 2>&1
    cat "$TMP_DIR/body" 2>/dev/null || true
}

BODY=$(run_captured_hook on-pre-tool-use.sh \
    '{"session_id":"s1","cwd":"/tmp/project","tool_name":"request_user_input","tool_input":{"questions":[{"id":"color","question":"Red or blue?"}]}}')
assert_eq "request_user_input is a question" "question_asked" "$(echo "$BODY" | jq -r '.event')"
assert_eq "question summary is the question" "Red or blue?" "$(echo "$BODY" | jq -r '.summary')"

BODY=$(run_captured_hook on-pre-tool-use.sh \
    '{"session_id":"s1","cwd":"/tmp/project","tool_name":"request_user_input","tool_input":"{\"questions\":[{\"question\":\"Cats or dogs?\"}]}"}')
assert_eq "string tool_input is decoded" "Cats or dogs?" "$(echo "$BODY" | jq -r '.summary')"

BODY=$(run_captured_hook on-pre-tool-use.sh \
    '{"session_id":"s1","cwd":"/tmp/project","tool_name":"request_user_input","tool_input":{}}')
assert_eq "missing question falls back" "Codex has a question for you" "$(echo "$BODY" | jq -r '.summary')"

assert_eq "async questions end the turn instead" "" \
    "$(run_captured_hook on-pre-tool-use.sh '{"tool_name":"request_user_input_async","tool_input":{}}')"
assert_eq "other tools are ignored" "" \
    "$(run_captured_hook on-pre-tool-use.sh '{"tool_name":"shell","tool_input":{"command":"ls"}}')"
assert_success "pre-tool hook fails open for malformed input" \
    run_captured_hook on-pre-tool-use.sh '{not-json'

printf '%s passed, %s failed\n' "$passed" "$failed"
test "$failed" -eq 0
