#!/bin/bash
# Hook script for Codex SessionStart event.
# Emits plugin version via warp://cli-agent so Warp can track the session.

set -euo pipefail

# Bump on every bundled release; keep in sync with the marketplace manifest and startup installer.
PLUGIN_VERSION="0.5.1"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/should-use-structured.sh"

if ! should_use_structured; then
    exit 0
fi

source "$SCRIPT_DIR/build-payload.sh"

INPUT=$(cat)
BODY=$(build_payload "$INPUT" "session_start" \
    --arg plugin_version "$PLUGIN_VERSION")
"$SCRIPT_DIR/warp-notify.sh" "warp://cli-agent" "$BODY"
