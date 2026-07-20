#!/bin/bash
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../scripts" && pwd)"
source "$SCRIPT_DIR/detect-stop-reason.sh"

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

assert_eq "Codex usage limit" "usage_limit" \
    "$(detect_stop_reason "You've hit your usage limit")"
assert_eq "Codex quota" "usage_limit" \
    "$(detect_stop_reason "Quota exceeded. Check your plan and billing details.")"
assert_eq "ordinary completion" "" \
    "$(detect_stop_reason "Implemented the requested changes")"
assert_eq "generic transient error" "" \
    "$(detect_stop_reason "Network request failed")"

printf '%s passed, %s failed\n' "$passed" "$failed"
test "$failed" -eq 0
