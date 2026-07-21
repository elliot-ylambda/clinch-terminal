#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
INSTALLER="$ROOT/resources/bundled/agent-plugins/install.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

mkdir -p "$TMP/bin" "$TMP/home" "$TMP/claude" "$TMP/codex"
LOG="$TMP/commands.log"

cat >"$TMP/bin/claude" <<'EOF'
#!/usr/bin/env bash
printf '%s %s\n' "$(basename "$0")" "$*" >>"$CLINCH_PLUGIN_TEST_LOG"
EOF
chmod +x "$TMP/bin/claude"

# Codex's real CLI owns config.toml formatting and plugin registration. This fixture emulates only
# those config edits so the installer can exercise its isolated-CODEX_HOME transaction without
# touching the developer's provider state.
cat >"$TMP/bin/codex" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

printf '%s %s\n' "$(basename "$0")" "$*" >>"$CLINCH_PLUGIN_TEST_LOG"
config="${CODEX_HOME:?}/config.toml"
mkdir -p "$(dirname "$config")"
touch "$config"

remove_section() {
  local section="$1"
  local replacement="$config.clinch-test.$$"
  awk -v section="$section" '
    $0 == section { skipping = 1; next }
    skipping && /^\[/ { skipping = 0 }
    !skipping { print }
  ' "$config" >"$replacement"
  mv "$replacement" "$config"
}

if [[ "$*" == "plugin marketplace remove clinch-codex-warp" ]]; then
  remove_section '[marketplaces.clinch-codex-warp]'
elif [[ "$*" == "plugin remove warp@codex-warp" ]]; then
  remove_section '[plugins."warp@codex-warp"]'
elif [[ "$1 $2 $3" == "plugin marketplace add" ]]; then
  remove_section '[marketplaces.clinch-codex-warp]'
  printf '\n[marketplaces.clinch-codex-warp]\nsource_type = "local"\nsource = "%s"\n' "$4" >>"$config"
elif [[ "$*" == "plugin add warp@clinch-codex-warp" ]]; then
  remove_section '[plugins."warp@clinch-codex-warp"]'
  printf '\n[plugins."warp@clinch-codex-warp"]\nenabled = true\n' >>"$config"
fi
EOF
chmod +x "$TMP/bin/codex"

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
grep -Fq "source = \"$CODEX_ROOT\"" "$TMP/codex/config.toml"
grep -Fxq '[plugins."warp@clinch-codex-warp"]' "$TMP/codex/config.toml"
[[ -f "$TMP/codex/plugins/cache/clinch-codex-warp/warp/0.5.1/.codex-plugin/plugin.json" ]]

# Current provider state must make a later launch a true no-op.
mkdir -p \
  "$TMP/claude/plugins" \
  "$TMP/codex/plugins/cache/clinch-codex-warp/warp/0.5.1/.codex-plugin"
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
cat >"$TMP/codex/config.toml" <<EOF
[marketplaces.clinch-codex-warp]
source_type = "local"
source = "$CODEX_ROOT"

[plugins."warp@clinch-codex-warp"]
enabled = true
EOF
cat >"$TMP/codex/plugins/cache/clinch-codex-warp/warp/0.5.1/.codex-plugin/plugin.json" <<'EOF'
{ "name": "warp", "version": "0.5.1" }
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

# The fail-open prompt-hook patch must upgrade the previously bundled 0.5.0 snapshot without
# deleting the old generation that a running Codex process may still reference.
mv \
  "$TMP/codex/plugins/cache/clinch-codex-warp/warp/0.5.1" \
  "$TMP/codex/plugins/cache/clinch-codex-warp/warp/0.5.0"
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
  echo "installer used destructive provider commands for a bundled plugin upgrade" >&2
  cat "$LOG" >&2
  exit 1
}
[[ -f "$TMP/codex/plugins/cache/clinch-codex-warp/warp/0.5.0/.codex-plugin/plugin.json" ]]
[[ -f "$TMP/codex/plugins/cache/clinch-codex-warp/warp/0.5.1/.codex-plugin/plugin.json" ]]
diff -qr \
  "$CODEX_ROOT/plugins/warp" \
  "$TMP/codex/plugins/cache/clinch-codex-warp/warp/0.5.1"

# A live but obsolete marketplace source must be re-registered transactionally. Preserve its old
# cache generation while publishing the current bundle and switching the config to the new path.
rm -rf "$TMP/codex/plugins/cache/clinch-codex-warp/warp/0.5.1"
old_codex_marketplace="$TMP/old-codex-marketplace"
mkdir -p "$old_codex_marketplace"
cat >"$TMP/codex/config.toml" <<EOF
[marketplaces.clinch-codex-warp]
source_type = "local"
source = "$old_codex_marketplace"

[plugins."warp@clinch-codex-warp"]
enabled = true
EOF
: >"$LOG"

HOME="$TMP/home" \
CLAUDE_CONFIG_DIR="$TMP/claude" \
CODEX_HOME="$TMP/codex" \
CLINCH_PLUGIN_TEST_LOG="$LOG" \
PATH="$TMP/bin:/usr/bin:/bin" \
  bash "$INSTALLER"

grep -Fxq "codex plugin marketplace remove clinch-codex-warp" "$LOG"
grep -Fxq "codex plugin marketplace add $CODEX_ROOT" "$LOG"
grep -Fxq "codex plugin add warp@clinch-codex-warp" "$LOG"
grep -Fq "source = \"$CODEX_ROOT\"" "$TMP/codex/config.toml"
[[ -f "$TMP/codex/plugins/cache/clinch-codex-warp/warp/0.5.0/.codex-plugin/plugin.json" ]]
[[ -f "$TMP/codex/plugins/cache/clinch-codex-warp/warp/0.5.1/.codex-plugin/plugin.json" ]]

# A migration from an older upstream plugin leaves that marketplace's cache intact for any live
# session, while atomically disabling it and enabling the bundled plugin for future sessions.
rm -rf "$TMP/codex/plugins/cache/clinch-codex-warp"
mkdir -p "$TMP/codex/plugins/cache/codex-warp/warp/0.4.1/.codex-plugin"
cat >"$TMP/codex/config.toml" <<'EOF'
[plugins."warp@codex-warp"]
enabled = true
EOF
cat >"$TMP/codex/plugins/cache/codex-warp/warp/0.4.1/.codex-plugin/plugin.json" <<'EOF'
{ "name": "warp", "version": "0.4.1" }
EOF
: >"$LOG"

HOME="$TMP/home" \
CLAUDE_CONFIG_DIR="$TMP/claude" \
CODEX_HOME="$TMP/codex" \
CLINCH_PLUGIN_TEST_LOG="$LOG" \
PATH="$TMP/bin:/usr/bin:/bin" \
  bash "$INSTALLER"

grep -Fxq "codex plugin marketplace remove clinch-codex-warp" "$LOG"
grep -Fxq "codex plugin remove warp@codex-warp" "$LOG"
grep -Fxq "codex plugin marketplace add $CODEX_ROOT" "$LOG"
grep -Fxq "codex plugin add warp@clinch-codex-warp" "$LOG"
[[ -f "$TMP/codex/plugins/cache/codex-warp/warp/0.4.1/.codex-plugin/plugin.json" ]]
[[ -f "$TMP/codex/plugins/cache/clinch-codex-warp/warp/0.5.1/.codex-plugin/plugin.json" ]]
grep -Fxq '[plugins."warp@clinch-codex-warp"]' "$TMP/codex/config.toml"
if grep -Fxq '[plugins."warp@codex-warp"]' "$TMP/codex/config.toml"; then
  echo "installer left the outdated upstream Codex plugin enabled" >&2
  exit 1
fi

# A newer plugin installed from Warp's regular marketplace is also left untouched.
rm -rf "$TMP/codex/plugins/cache/clinch-codex-warp"
rm -rf "$TMP/codex/plugins/cache/codex-warp"
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
