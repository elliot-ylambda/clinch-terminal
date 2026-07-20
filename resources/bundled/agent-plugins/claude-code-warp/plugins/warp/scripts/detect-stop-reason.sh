#!/bin/bash

# Prints the structured stop reason understood by Clinch, or an empty line
# for an ordinary completion. Match only provider-authored hard-limit phrases;
# a generic error must never authorize an automatic prompt.
detect_stop_reason() {
    local message
    message=$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]')
    if [[ "$message" == *"you've hit your"*"limit"* ]] \
        || [[ "$message" == *"usage limit reached"* ]] \
        || [[ "$message" == *"rate limit reached"* ]]; then
        printf '%s\n' "usage_limit"
    else
        printf '\n'
    fi
}
