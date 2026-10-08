#!/usr/bin/env bash
# Old shutdown hooks must not erase either a saved owner or its replacement.
set -euo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
export WARP_AGENT_RESUME_DIR="$TMP/reg"
export WARP_TERMINAL_SESSION_UUID=aa11
export WARP_AGENT_RESUME_FAKE_ARGV=""
export WARP_AGENT_RESUME_FAKE_RECORDED_OWNER_ACTIVE=0
unset CLAUDE_CODE_BRIDGE_SESSION_ID

hook() {
  local event="$1" owner="$2" script
  if [[ "$provider" == claude ]]; then
    script="$HERE/claude-capture.sh"
  elif [[ "$event" == SessionStart ]]; then
    script="$HERE/codex-session-start.sh"
  else
    script="$HERE/codex-session-end.sh"
  fi
  printf '{"session_id":"same-session","cwd":"/tmp/repo","hook_event_name":"%s"}\n' "$event" \
    | WARP_AGENT_RESUME_FAKE_OWNER_PID="$owner" bash "$script"
}

for provider in claude codex; do
  export WARP_AGENT_RESUME_FAKE_ANCESTRY="$provider"
  hook SessionStart 111111
  entry="$WARP_AGENT_RESUME_DIR/aa11.json"
  mkdir -p "$WARP_AGENT_RESUME_DIR/shutdown-owners"
  cp "$entry" "$WARP_AGENT_RESUME_DIR/shutdown-owners/aa11.json"

  # Simulate an app restart clearing its global guard, long past the old grace window.
  printf '999999999\n' > "$WARP_AGENT_RESUME_DIR/.app-terminating"
  touch -t 202601010000 "$WARP_AGENT_RESUME_DIR/.app-terminating" \
    "$WARP_AGENT_RESUME_DIR/shutdown-owners/aa11.json"
  if "$HERE/clinch-agent-resume" app-terminating; then
    echo 'FAIL: legacy guard should have expired'; exit 1
  fi
  hook SessionEnd 111111
  [[ -f "$entry" && ! -e "$WARP_AGENT_RESUME_DIR/tombstones/aa11" ]] \
    || { echo "FAIL: $provider late shutdown erased saved owner"; exit 1; }

  # A new process may resume the identical session. Its old predecessor cannot erase it.
  WARP_AGENT_RESUME_FAKE_RECORDED_OWNER_ACTIVE=1 hook SessionStart 222222
  grep -q '"owner_pid": "222222"' "$entry" \
    || { echo "FAIL: $provider shutting-down owner blocked replacement capture"; exit 1; }
  hook SessionEnd 111111
  [[ -f "$entry" ]] || { echo "FAIL: $provider stale hook erased replacement"; exit 1; }
  hook SessionEnd 222222
  [[ ! -f "$entry" && -f "$WARP_AGENT_RESUME_DIR/tombstones/aa11" ]] \
    || { echo "FAIL: $provider replacement's intentional exit was suppressed"; exit 1; }

  # Older and repair-live entries have no recorded process owner. Their genuine exit
  # must retain the legacy provider/session check rather than resurrecting forever.
  "$HERE/clinch-agent-resume" write aa11 "clinch_agent_resume_launch $provider same-session" /tmp/repo
  hook SessionEnd 222222
  [[ ! -f "$entry" ]] || { echo "FAIL: $provider ownerless entry ignored intentional exit"; exit 1; }
done
echo 'delayed Claude/Codex shutdown tests passed'
