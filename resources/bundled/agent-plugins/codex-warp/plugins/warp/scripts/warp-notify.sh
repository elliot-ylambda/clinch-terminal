#!/bin/bash
# Warp notification utility using OSC escape sequences.
# Usage: warp-notify.sh <title> <body>
#
# For structured Warp notifications, title should be "warp://cli-agent"
# and body should be a JSON string matching the cli-agent notification schema.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/should-use-structured.sh"

# Only emit notifications when we've confirmed the Warp build can render them.
if ! should_use_structured; then
    exit 0
fi

TITLE="${1:-Notification}"
BODY="${2:-}"

# OSC 777 format: \033]777;notify;<title>;<body>\007
#
# Codex spawns hook processes without a controlling terminal, so `/dev/tty`
# usually fails to open. Warp exports the pane's PTY device path as WARP_TTY;
# writing to it by path works without a controlling terminal.
SEQ=$(printf '\033]777;notify;%s;%s\007' "$TITLE" "$BODY")
# The subshell keeps the shell's own "cannot open /dev/tty" redirection error
# quiet too, not just printf's stderr.
if ! ( printf '%s' "$SEQ" > /dev/tty ) 2>/dev/null; then
    if [ -n "${WARP_TTY:-}" ] && [ -w "$WARP_TTY" ]; then
        ( printf '%s' "$SEQ" > "$WARP_TTY" ) 2>/dev/null || true
    fi
fi
