#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
INSTALLER="$ROOT/resources/bundled/agent-plugins/install.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

mkdir -p "$TMP/bin" "$TMP/home" "$TMP/claude" "$TMP/codex"
LOG="$TMP/commands.log"

for cli in claude codex; do
  cat >"$TMP/bin/$cli" <<'EOF'
#!/usr/bin/env bash
printf '%s %s\n' "$(basename "$0")" "$*" >>"$CLINCH_PLUGIN_TEST_LOG"
EOF
  chmod +x "$TMP/bin/$cli"
done

HOME="$TMP/home" \
CLAUDE_CONFIG_DIR="$TMP/claude" \
CODEX_HOME="$TMP/codex" \
CLINCH_PLUGIN_TEST_LOG="$LOG" \
PATH="$TMP/bin:/usr/bin:/bin" \
  bash "$INSTALLER"

CLAUDE_ROOT="$ROOT/resources/bundled/agent-plugins/claude-code-warp"
CODEX_ROOT="$ROOT/resources/bundled/agent-plugins/codex-warp"
grep -Fxq "claude plugin marketplace remove clinch-claude-code-warp" "$LOG"
grep -Fxq "claude plugin uninstall warp@claude-code-warp --scope user" "$LOG"
grep -Fxq "claude plugin marketplace add $CLAUDE_ROOT" "$LOG"
grep -Fxq "claude plugin install warp@clinch-claude-code-warp --scope user" "$LOG"
grep -Fxq "codex plugin marketplace remove clinch-codex-warp" "$LOG"
grep -Fxq "codex plugin remove warp@codex-warp" "$LOG"
grep -Fxq "codex plugin marketplace add $CODEX_ROOT" "$LOG"
grep -Fxq "codex plugin add warp@clinch-codex-warp" "$LOG"
if grep -Fxq "claude plugin marketplace remove claude-code-warp" "$LOG"; then
  echo "installer removed the user's upstream Claude marketplace" >&2
  exit 1
fi
if grep -Fxq "codex plugin marketplace remove codex-warp" "$LOG"; then
  echo "installer removed the user's upstream Codex marketplace" >&2
  exit 1
fi

# Current provider state must make a later launch a true no-op.
mkdir -p \
  "$TMP/claude/plugins" \
  "$TMP/codex/plugins/cache/clinch-codex-warp/warp/0.5.0/.codex-plugin"
cat >"$TMP/claude/plugins/installed_plugins.json" <<'EOF'
{
  "version": 2,
  "plugins": {
    "warp@clinch-claude-code-warp": [
      { "scope": "user", "version": "2.3.0" }
    ]
  }
}
EOF
cat >"$TMP/codex/config.toml" <<'EOF'
[plugins."warp@clinch-codex-warp"]
enabled = true
EOF
cat >"$TMP/codex/plugins/cache/clinch-codex-warp/warp/0.5.0/.codex-plugin/plugin.json" <<'EOF'
{ "name": "warp", "version": "0.5.0" }
EOF
: >"$LOG"

HOME="$TMP/home" \
CLAUDE_CONFIG_DIR="$TMP/claude" \
CODEX_HOME="$TMP/codex" \
CLINCH_PLUGIN_TEST_LOG="$LOG" \
PATH="$TMP/bin:/usr/bin:/bin" \
  bash "$INSTALLER"

[[ ! -s "$LOG" ]] || {
  echo "installer reran provider commands for current plugins" >&2
  cat "$LOG" >&2
  exit 1
}

# A newer plugin installed from Warp's regular marketplace is also left untouched.
rm -rf "$TMP/codex/plugins/cache/clinch-codex-warp"
mkdir -p "$TMP/codex/plugins/cache/codex-warp/warp/9.0.0/.codex-plugin"
cat >"$TMP/claude/plugins/installed_plugins.json" <<'EOF'
{
  "version": 2,
  "plugins": {
    "warp@claude-code-warp": [
      { "scope": "user", "version": "9.0.0" }
    ]
  }
}
EOF
cat >"$TMP/codex/config.toml" <<'EOF'
[plugins."warp@codex-warp"]
enabled = true
EOF
cat >"$TMP/codex/plugins/cache/codex-warp/warp/9.0.0/.codex-plugin/plugin.json" <<'EOF'
{ "name": "warp", "version": "9.0.0" }
EOF
: >"$LOG"

HOME="$TMP/home" \
CLAUDE_CONFIG_DIR="$TMP/claude" \
CODEX_HOME="$TMP/codex" \
CLINCH_PLUGIN_TEST_LOG="$LOG" \
PATH="$TMP/bin:/usr/bin:/bin" \
  bash "$INSTALLER"

[[ ! -s "$LOG" ]] || {
  echo "installer replaced current upstream plugins" >&2
  cat "$LOG" >&2
  exit 1
}

echo "bundled agent plugin installer tests passed"
