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

printf '%s passed, %s failed\n' "$passed" "$failed"
test "$failed" -eq 0
