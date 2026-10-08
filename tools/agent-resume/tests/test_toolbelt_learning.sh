#!/usr/bin/env bash
# Exercises the local, bounded learner that feeds typed toolbelt suggestions.
set -euo pipefail

HERE="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

export WARP_AGENT_RESUME_DIR="$TMP/registry"
export WARP_TERMINAL_SESSION_UUID="pane-learning-test"
STORE="$WARP_AGENT_RESUME_DIR/toolbelt-learning.json"

submit() {
  local provider="$1" session_id="$2" prompt="$3"
  jq -cn --arg sid "$session_id" --arg prompt "$prompt" \
    '{session_id:$sid,cwd:"/tmp/project",hook_event_name:"UserPromptSubmit",prompt:$prompt}' \
    | "$HERE/prompt-mirror.sh" "$provider"
}

submit codex first-session "Run the local documentation server"
[[ -f "$STORE" ]] || { echo "FAIL: first eligible prompt did not create learning store"; exit 1; }
[[ "$(jq '.patterns | length' "$STORE")" -eq 1 ]] \
  || { echo "FAIL: first prompt did not create exactly one fingerprint"; exit 1; }
[[ "$(jq '.patterns[0].sessions | length' "$STORE")" -eq 1 ]] \
  || { echo "FAIL: first prompt has wrong session count"; exit 1; }
[[ "$(jq '.patterns[0] | has("text") | not' "$STORE")" == true ]] \
  || { echo "FAIL: singleton retained duplicate prompt text"; exit 1; }

# Repeating inside one conversation is not cross-conversation evidence.
submit codex first-session "run   THE local documentation server"
[[ "$(jq '.patterns[0].sessions | length' "$STORE")" -eq 1 ]] \
  || { echo "FAIL: same-session repeat increased conversation count"; exit 1; }
[[ "$(jq '.patterns[0] | has("text") | not' "$STORE")" == true ]] \
  || { echo "FAIL: same-session repeat made candidate eligible"; exit 1; }

# A second distinct provider-scoped session qualifies the exact latest text.
submit claude second-session "Run the local documentation server"
[[ "$(jq '.patterns[0].sessions | length' "$STORE")" -eq 2 ]] \
  || { echo "FAIL: distinct session did not qualify candidate"; exit 1; }
[[ "$(jq -r '.patterns[0].text' "$STORE")" == "Run the local documentation server" ]] \
  || { echo "FAIL: eligible candidate text is wrong"; exit 1; }
[[ "$(jq -r '.patterns[0].providers | sort | join(",")' "$STORE")" == "claude,codex" ]] \
  || { echo "FAIL: provider aggregation is wrong"; exit 1; }
[[ "$(stat -f '%Lp' "$STORE")" == "600" ]] \
  || { echo "FAIL: learning store is not owner-only"; exit 1; }
[[ ! -e "$WARP_AGENT_RESUME_DIR/.toolbelt-learning.lock" ]] \
  || { echo "FAIL: learning lock leaked after update"; exit 1; }

# A killed hook cannot permanently disable learning: an empty stale lock is reclaimed.
mkdir "$WARP_AGENT_RESUME_DIR/.toolbelt-learning.lock"
touch -t 202001010000 "$WARP_AGENT_RESUME_DIR/.toolbelt-learning.lock"
submit codex stale-lock-session "Start the reusable preview server"
[[ ! -e "$WARP_AGENT_RESUME_DIR/.toolbelt-learning.lock" ]] \
  || { echo "FAIL: stale learning lock was not reclaimed"; exit 1; }

before="$(jq '.patterns | length' "$STORE")"
submit codex secret-session "Use api_key=sk-proj-abcdefghijklmnopqrstuvwxyz123456"
submit codex destructive-session "Please run rm -rf ./build-cache"
submit codex oneoff-session "Open task 123e4567-e89b-42d3-a456-426614174000 now"
[[ "$(jq '.patterns | length' "$STORE")" -eq "$before" ]] \
  || { echo "FAIL: unsafe or one-off prompt entered learning store"; exit 1; }

# One update prunes an oversized pre-existing store to the newest 512 patterns.
jq -n '{schema:1,patterns:[range(0;513) as $i | {
  id:"00000000-0000-4000-8000-000000000001",
  fingerprint:("fixture-" + ($i|tostring)), sessions:["codex:fixture"], providers:["codex"],
  first_seen:"2026-01-01T00:00:00Z", last_seen:("2026-01-01T00:00:" + (($i % 60)|tostring) + "Z")
}]}' > "$STORE"
submit codex prune-session "Start the reusable preview server"
[[ "$(jq '.patterns | length' "$STORE")" -eq 512 ]] \
  || { echo "FAIL: learning store was not capped at 512 patterns"; exit 1; }

echo "PASS"
