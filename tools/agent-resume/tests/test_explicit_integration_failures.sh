#!/usr/bin/env bash
set -euo pipefail

HERE="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
export HOME="$TMP/home"
mkdir -p "$HOME/.claude" "$HOME/.codex"

printf '%s\n' '{not valid json' > "$HOME/.claude/settings.json"
printf '%s\n' 'model = "keep-me"' > "$HOME/.codex/config.toml"
claude_before="$(shasum "$HOME/.claude/settings.json")"
codex_before="$(shasum "$HOME/.codex/config.toml")"

if bash "$HERE/install.sh" enable --quiet >/dev/null 2>&1; then
  echo "FAIL: invalid Claude JSON unexpectedly enabled the integration"
  exit 1
fi

[[ "$claude_before" == "$(shasum "$HOME/.claude/settings.json")" ]] \
  || { echo "FAIL: failed enable changed invalid Claude config"; exit 1; }
[[ "$codex_before" == "$(shasum "$HOME/.codex/config.toml")" ]] \
  || { echo "FAIL: failed enable changed Codex config"; exit 1; }
[[ ! -e "$HOME/.warp" ]] || { echo "FAIL: failed enable installed runtime files"; exit 1; }
[[ ! -e "$HOME/Library/Application Support/sh.clinch.Clinch/agent-integration/enabled" ]] \
  || { echo "FAIL: failed enable recorded consent"; exit 1; }

# A hand-edited or truncated managed block must fail closed before any file is changed. Without
# this check, an unmatched begin marker would cause every following Codex setting to be dropped.
printf '%s\n' '{}' > "$HOME/.claude/settings.json"
printf '%s\n' 'model = "keep-me"' '# >>> clinch agent-resume >>>' 'approval_policy = "never"' \
  > "$HOME/.codex/config.toml"
claude_before="$(shasum "$HOME/.claude/settings.json")"
codex_before="$(shasum "$HOME/.codex/config.toml")"
if bash "$HERE/install.sh" enable --quiet >/dev/null 2>&1; then
  echo "FAIL: unmatched Codex managed marker unexpectedly enabled the integration"
  exit 1
fi
[[ "$claude_before" == "$(shasum "$HOME/.claude/settings.json")" ]] \
  || { echo "FAIL: failed Codex validation changed Claude config"; exit 1; }
[[ "$codex_before" == "$(shasum "$HOME/.codex/config.toml")" ]] \
  || { echo "FAIL: failed Codex validation changed Codex config"; exit 1; }
[[ ! -e "$HOME/.warp" ]] || { echo "FAIL: failed Codex validation installed runtime files"; exit 1; }
[[ ! -e "$HOME/Library/Application Support/sh.clinch.Clinch/agent-integration/enabled" ]] \
  || { echo "FAIL: failed Codex validation recorded consent"; exit 1; }

echo "PASS"
