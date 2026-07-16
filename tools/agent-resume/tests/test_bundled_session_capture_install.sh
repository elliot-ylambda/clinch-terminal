#!/usr/bin/env bash
# Proves the prepared app resources contain every helper consumed by the session-capture installer.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

NO_LICENSES=1 SKIP_SETTINGS_SCHEMA=1 \
  "$ROOT/script/prepare_bundled_resources" "$TMP/resources" >/dev/null

BUNDLE="$TMP/resources/agent-resume"
for helper in prompt-mirror.sh codex-prompt-submit.sh; do
  [[ -x "$BUNDLE/$helper" ]] \
    || { echo "FAIL: prepared resources omitted $helper"; exit 1; }
done

export HOME="$TMP/home"
export CLINCH_AGENT_BIN_DIR="$TMP/runtime"
export WARP_AGENT_RESUME_DIR="$TMP/capture"
export CLINCH_AGENT_STATE_DIR="$TMP/state"
export CLINCH_CLAUDE_SETTINGS="$TMP/config/claude.json"
export CLINCH_CODEX_CONFIG="$TMP/config/codex.toml"

/bin/bash "$BUNDLE/install.sh" enable --quiet
[[ -f "$CLINCH_AGENT_STATE_DIR/enabled" ]] \
  || { echo "FAIL: bundled session capture did not enable"; exit 1; }
[[ -x "$CLINCH_AGENT_BIN_DIR/prompt-mirror.sh" ]] \
  || { echo "FAIL: bundled installer omitted prompt-mirror.sh"; exit 1; }
[[ -x "$CLINCH_AGENT_BIN_DIR/codex-prompt-submit.sh" ]] \
  || { echo "FAIL: bundled installer omitted codex-prompt-submit.sh"; exit 1; }

echo "PASS"
