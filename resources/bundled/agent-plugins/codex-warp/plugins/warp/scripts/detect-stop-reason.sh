#!/bin/bash

# Prints the structured stop reason understood by Clinch, or an empty line
# for an ordinary completion. Match only Codex's hard-limit phrases.
detect_stop_reason() {
    local message
    message=$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]')
    if [[ "$message" == *"you've hit your usage limit"* ]] \
        || [[ "$message" == *"usage limit reached"* ]] \
        || [[ "$message" == *"quota exceeded"* ]]; then
        printf '%s\n' "usage_limit"
    else
        printf '\n'
    fi
}
