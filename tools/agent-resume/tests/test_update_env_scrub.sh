#!/usr/bin/env bash
# Tests that the app-relaunch boundary removes the session environment `open` would
# otherwise forward into every pane (the 2026-07-09 transcript-loss incident).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# Sourcing defines helpers only; it must not attempt an update.
source "$ROOT/script/update-installed-clinch"

# Fake `open`: capture the exact child environment and argv.
cat > "$TMP/open" <<EOF
#!/usr/bin/env bash
env > "$TMP/captured_env"
printf '%s\n' "\$@" > "$TMP/captured_args"
EOF
chmod +x "$TMP/open"
export CLINCH_OPEN_BIN="$TMP/open"

export CLAUDE_CODE_SESSION_ID="stale-session"
export CLAUDE_CODE_BRIDGE_SESSION_ID="session_01STALE"
export CLAUDE_CODE_FUTURE_ID="future-name-proves-dynamic-scrub"
export CLAUDECODE=1
export CLAUDE_EFFORT=xhigh
export AI_AGENT=claude-code_stale
export MAKEFLAGS=n
export MFLAGS=-n
export MAKELEVEL=2
export SKIP_SYNC=1
export CLINCH_SCRUB_TEST_UNRELATED=preserved

clinch_scrubbed_open "/Applications/Fake App.app"

for name in CLAUDE_CODE_SESSION_ID CLAUDE_CODE_BRIDGE_SESSION_ID \
  CLAUDE_CODE_FUTURE_ID CLAUDECODE CLAUDE_EFFORT AI_AGENT \
  MAKEFLAGS MFLAGS MAKELEVEL SKIP_SYNC; do
  grep -q "^${name}=" "$TMP/captured_env" \
    && { echo "FAIL: relaunch leaked $name"; exit 1; }
done
grep -q '^CLINCH_SCRUB_TEST_UNRELATED=preserved$' "$TMP/captured_env" \
  || { echo "FAIL: relaunch stripped unrelated env"; exit 1; }
[[ "$(< "$TMP/captured_args")" == "/Applications/Fake App.app" ]] \
  || { echo "FAIL: relaunch changed bundle argv"; exit 1; }

echo "PASS"
