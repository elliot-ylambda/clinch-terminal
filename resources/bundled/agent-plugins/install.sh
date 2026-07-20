#!/usr/bin/env bash

# Installs the Warp notification plugins bundled with Clinch into the provider-owned plugin
# stores. This is deliberately best-effort: an unavailable or policy-restricted provider must
# never prevent Clinch from starting, and a failed provider is retried on the next launch.

set -u

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CLAUDE_PLUGIN_VERSION="2.3.0"
CODEX_PLUGIN_VERSION="0.5.0"
CLAUDE_MARKETPLACE_NAME="clinch-claude-code-warp"
CLAUDE_BUNDLED_PLUGIN_KEY="warp@clinch-claude-code-warp"
CLAUDE_UPSTREAM_PLUGIN_KEY="warp@claude-code-warp"
CODEX_MARKETPLACE_NAME="clinch-codex-warp"
CODEX_BUNDLED_PLUGIN_KEY="warp@clinch-codex-warp"
CODEX_UPSTREAM_PLUGIN_KEY="warp@codex-warp"

# Startup provisioning runs before Clinch forwards a second launch to the existing process.
# Serialize provider mutations across concurrently starting application processes.
LOCK_DIR="${TMPDIR:-/tmp}/clinch-agent-plugin-install-${UID}"
if ! mkdir "$LOCK_DIR" 2>/dev/null; then
  if [[ -f "$LOCK_DIR/pid" ]]; then
    lock_pid="$(cat "$LOCK_DIR/pid" 2>/dev/null || true)"
    if [[ "$lock_pid" =~ ^[0-9]+$ ]] && kill -0 "$lock_pid" 2>/dev/null; then
      exit 0
    fi
  fi
  rm -rf "$LOCK_DIR"
  mkdir "$LOCK_DIR" 2>/dev/null || exit 0
fi
printf '%s\n' "$$" >"$LOCK_DIR/pid"
trap 'rm -rf "$LOCK_DIR"' EXIT INT TERM

# Finder-launched apps inherit a minimal PATH. Cover the supported native installers and common
# JavaScript toolchains without sourcing an interactive shell startup file.
for candidate in \
  "$HOME/.local/bin" \
  "$HOME/.cargo/bin" \
  "$HOME/.volta/bin" \
  "$HOME/.bun/bin" \
  /opt/homebrew/bin \
  /usr/local/bin \
  "$HOME"/.nvm/versions/node/*/bin; do
  if [[ -d "$candidate" ]]; then
    PATH="$PATH:$candidate"
  fi
done
export PATH

version_is_at_least() {
  local actual="$1"
  local wanted="$2"
  awk -v actual="$actual" -v wanted="$wanted" '
    BEGIN {
      sub(/^v/, "", actual)
      sub(/^v/, "", wanted)
      actual_count = split(actual, actual_parts, ".")
      wanted_count = split(wanted, wanted_parts, ".")
      count = actual_count > wanted_count ? actual_count : wanted_count
      for (i = 1; i <= count; i++) {
        actual_part = actual_parts[i]
        wanted_part = wanted_parts[i]
        sub(/[^0-9].*$/, "", actual_part)
        sub(/[^0-9].*$/, "", wanted_part)
        actual_part = actual_part == "" ? 0 : actual_part + 0
        wanted_part = wanted_part == "" ? 0 : wanted_part + 0
        if (actual_part > wanted_part) exit 0
        if (actual_part < wanted_part) exit 1
      }
      exit 0
    }
  '
}

# A registered bundle path can vanish after installation (a cleaned dev build, a moved or
# reinstalled app). The version check alone still reports the plugin as current then, so the
# dead registration is never repaired and every provider hook fails until it is. Treat a
# registered-but-missing marketplace directory as stale to force the reinstall that fixes it.
claude_bundled_marketplace_is_stale() {
  local claude_dir="${CLAUDE_CONFIG_DIR:-$HOME/.claude}"
  local known="$claude_dir/plugins/known_marketplaces.json"
  local location
  [[ -f "$known" ]] || return 1
  location="$(awk -v name="$CLAUDE_MARKETPLACE_NAME" '
    index($0, "\"" name "\"") { in_entry = 1; next }
    in_entry && /"installLocation"[[:space:]]*:/ {
      sub(/^.*"installLocation"[[:space:]]*:[[:space:]]*"/, "")
      sub(/".*$/, "")
      print
      exit
    }
  ' "$known")"
  [[ -n "$location" && ! -d "$location" ]]
}

claude_plugin_is_current() {
  local claude_dir="${CLAUDE_CONFIG_DIR:-$HOME/.claude}"
  local installed="$claude_dir/plugins/installed_plugins.json"
  local version
  claude_bundled_marketplace_is_stale && return 1
  [[ -f "$installed" ]] || return 1

  # installed_plugins.json groups every install beneath its plugin id. Limit the version lookup
  # to the Warp entry so another plugin with the same version cannot produce a false positive.
  while IFS= read -r version; do
    if version_is_at_least "$version" "$CLAUDE_PLUGIN_VERSION"; then
      return 0
    fi
  done < <(awk \
    -v bundled_key="$CLAUDE_BUNDLED_PLUGIN_KEY" \
    -v upstream_key="$CLAUDE_UPSTREAM_PLUGIN_KEY" '
    index($0, "\"" bundled_key "\"") || index($0, "\"" upstream_key "\"") {
      in_plugin = 1
      next
    }
    in_plugin && /"version"[[:space:]]*:/ {
      version = $0
      sub(/^.*"version"[[:space:]]*:[[:space:]]*"/, "", version)
      sub(/".*$/, "", version)
      print version
    }
    in_plugin && /^[[:space:]]*][,]?[[:space:]]*$/ { in_plugin = 0 }
  ' "$installed")
  return 1
}

codex_plugin_is_enabled() {
  local config="$1"
  local plugin_key="$2"
  awk -v plugin_key="$plugin_key" '
    $0 == "[plugins.\"" plugin_key "\"]" { in_plugin = 1; next }
    in_plugin && /^\[/ { in_plugin = 0 }
    in_plugin && /^[[:space:]]*enabled[[:space:]]*=[[:space:]]*true[[:space:]]*$/ { found = 1 }
    END { exit !found }
  ' "$config"
}

codex_marketplace_has_current_version() {
  local codex_dir="$1"
  local marketplace_name="$2"
  local manifest
  local version
  for manifest in "$codex_dir"/plugins/cache/"$marketplace_name"/warp/*/.codex-plugin/plugin.json; do
    [[ -f "$manifest" ]] || continue
    version="$(awk '
      /"version"[[:space:]]*:/ {
        version = $0
        sub(/^.*"version"[[:space:]]*:[[:space:]]*"/, "", version)
        sub(/".*$/, "", version)
        print version
        exit
      }
    ' "$manifest")"
    if version_is_at_least "$version" "$CODEX_PLUGIN_VERSION"; then
      return 0
    fi
  done
  return 1
}

# Same self-heal as claude_bundled_marketplace_is_stale: Codex records the bundled marketplace
# source path in config.toml, and a dead path must force a reinstall rather than pass the
# version check.
codex_bundled_marketplace_is_stale() {
  local codex_dir="${CODEX_HOME:-$HOME/.codex}"
  local config="$codex_dir/config.toml"
  local location
  [[ -f "$config" ]] || return 1
  location="$(awk -v section="[marketplaces.$CODEX_MARKETPLACE_NAME]" '
    $0 == section { in_section = 1; next }
    in_section && /^\[/ { exit }
    in_section && /^[[:space:]]*source[[:space:]]*=[[:space:]]*"/ {
      sub(/^[[:space:]]*source[[:space:]]*=[[:space:]]*"/, "")
      sub(/".*$/, "")
      print
      exit
    }
  ' "$config")"
  [[ -n "$location" && ! -d "$location" ]]
}

codex_plugin_is_current() {
  local codex_dir="${CODEX_HOME:-$HOME/.codex}"
  local config="$codex_dir/config.toml"
  [[ -f "$config" ]] || return 1

  codex_bundled_marketplace_is_stale && return 1
  if codex_plugin_is_enabled "$config" "$CODEX_BUNDLED_PLUGIN_KEY" &&
    codex_marketplace_has_current_version "$codex_dir" "$CODEX_MARKETPLACE_NAME"; then
    return 0
  fi
  codex_plugin_is_enabled "$config" "$CODEX_UPSTREAM_PLUGIN_KEY" &&
    codex_marketplace_has_current_version "$codex_dir" codex-warp
}

install_claude_plugin() {
  command -v claude >/dev/null 2>&1 || return 0
  claude_plugin_is_current && return 0

  # Use a Clinch-owned marketplace id so provisioning the notification plugin never removes the
  # Oz plugin or a development override from Warp's upstream marketplace. If an outdated upstream
  # notification plugin exists, remove only that plugin to avoid duplicate hooks.
  claude plugin marketplace remove "$CLAUDE_MARKETPLACE_NAME" </dev/null >/dev/null 2>&1 || true
  claude plugin uninstall "$CLAUDE_UPSTREAM_PLUGIN_KEY" --scope user </dev/null >/dev/null 2>&1 || true
  if ! claude plugin marketplace add "$ROOT/claude-code-warp" </dev/null >/dev/null 2>&1; then
    echo "clinch: could not register the bundled Claude Code marketplace" >&2
    return 1
  fi
  if ! claude plugin install "$CLAUDE_BUNDLED_PLUGIN_KEY" --scope user </dev/null >/dev/null 2>&1; then
    echo "clinch: could not install the bundled Claude Code Warp plugin" >&2
    return 1
  fi
}

install_codex_plugin() {
  command -v codex >/dev/null 2>&1 || return 0
  codex_plugin_is_current && return 0

  codex plugin marketplace remove "$CODEX_MARKETPLACE_NAME" </dev/null >/dev/null 2>&1 || true
  codex plugin remove "$CODEX_UPSTREAM_PLUGIN_KEY" </dev/null >/dev/null 2>&1 || true
  if ! codex plugin marketplace add "$ROOT/codex-warp" </dev/null >/dev/null 2>&1; then
    echo "clinch: could not register the bundled Codex marketplace" >&2
    return 1
  fi
  if ! codex plugin add "$CODEX_BUNDLED_PLUGIN_KEY" </dev/null >/dev/null 2>&1; then
    echo "clinch: could not install the bundled Codex Warp plugin" >&2
    return 1
  fi
}

# Keep the two providers independent: one failure must not suppress the other installation.
install_claude_plugin || true
install_codex_plugin || true
exit 0
