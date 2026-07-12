#!/usr/bin/env bash
# Installs Clinch's local agent-resume capture layer and wires Claude Code / Codex hooks.
#
# This script is bundled inside Clinch.app and is run idempotently on GUI launch. It uses
# only macOS system tools: no repository clone, jq/Homebrew dependency, shell rc edit, or
# shell restart is required. It is also safe to run directly while developing.
set -euo pipefail

QUIET=0
INSTALL_PLUGINS=0
for arg in "$@"; do
  case "$arg" in
    --quiet) QUIET=1 ;;
    --plugins) INSTALL_PLUGINS=1 ;;
    --help|-h)
      echo "usage: install.sh [--quiet] [--plugins]"
      exit 0
      ;;
    *) echo "error: unknown option: $arg" >&2; exit 2 ;;
  esac
done

log() { (( QUIET )) || printf '%s\n' "$*"; }
warn() { printf 'clinch agent setup warning: %s\n' "$*" >&2; }

SRC="$(cd "$(dirname "$0")" && pwd)"
BIN="$HOME/.warp/agent-resume-bin"
REG="$HOME/.warp/agent-resume"

mkdir -p "$BIN" "$REG"
chmod 700 "$BIN" "$REG"

install -m 0755 \
  "$SRC/agent-json" \
  "$SRC/clinch-agent-resume" \
  "$SRC/warp-agent-resume" \
  "$SRC/clinch_agent_resume_launch" \
  "$SRC/claude-capture.sh" \
  "$SRC/codex-session-start.sh" \
  "$SRC/codex-session-end.sh" \
  "$SRC/install-agent-plugins.sh" \
  "$SRC/wire-claude-hooks.sh" \
  "$BIN/"
install -m 0644 "$SRC/agent-json.js" "$SRC/claude.zsh" "$BIN/"

# An external compatibility entrypoint covers commands persisted by older builds even when
# the user's ~/.zshrc never sourced the old function definitions.
install -m 0755 "$SRC/clinch_agent_resume_launch" "$BIN/warp_agent_resume_launch"

# Remove the pre-rename capture script so a stale settings.json entry cannot run it.
rm -f "$BIN/claude-session-start.sh"

wire_codex_hooks() {
  local cfg="$HOME/.codex/config.toml" tmp mode
  mkdir -p "$(dirname "$cfg")"
  tmp="$(mktemp "$(dirname "$cfg")/.clinch-codex.XXXXXX")"

  if [[ -f "$cfg" ]]; then
    mode="$(stat -f '%Lp' "$cfg" 2>/dev/null || echo 600)"
    # Replace both Clinch and legacy Warp managed blocks. Everything outside those exact
    # markers is copied byte-for-line and remains user-owned.
    awk '
      function flush_blanks() {
        for (i = 0; i < pending_blanks; i++) print ""
        pending_blanks = 0
      }
      /^# >>> (clinch|warp) agent-resume >>>$/ { managed = 1; next }
      /^# <<< (clinch|warp) agent-resume <<<$/{ managed = 0; next }
      !managed && $0 == "" { pending_blanks++; next }
      !managed { flush_blanks(); print }
    ' "$cfg" > "$tmp"
  else
    mode=600
    : > "$tmp"
  fi

  # A leading newline safely separates this table array from any preceding TOML value.
  printf '\n# >>> clinch agent-resume >>>\n' >> "$tmp"
  printf '[[hooks.SessionStart]]\n' >> "$tmp"
  printf 'matcher = "startup|resume"\n' >> "$tmp"
  printf '[[hooks.SessionStart.hooks]]\n' >> "$tmp"
  printf 'type = "command"\n' >> "$tmp"
  printf 'command = "%s/codex-session-start.sh"\n\n' "$BIN" >> "$tmp"
  printf '[[hooks.SessionEnd]]\n' >> "$tmp"
  printf '[[hooks.SessionEnd.hooks]]\n' >> "$tmp"
  printf 'type = "command"\n' >> "$tmp"
  printf 'command = "%s/codex-session-end.sh"\n' "$BIN" >> "$tmp"
  printf '# <<< clinch agent-resume <<<\n' >> "$tmp"

  chmod "$mode" "$tmp"
  if [[ -f "$cfg" ]] && cmp -s "$tmp" "$cfg"; then
    rm -f "$tmp"
  else
    mv "$tmp" "$cfg"
  fi
}

wire_codex_hooks
log "Wired Codex capture hooks (SessionStart, SessionEnd)"

# Structural JSON merge: preserves unrelated Claude settings/hooks, removes stale managed
# entries, and leaves exactly one current hook on each supported lifecycle event.
if "$SRC/wire-claude-hooks.sh" "$HOME/.claude/settings.json" "$BIN"; then
  log "Wired Claude capture hooks (SessionStart, UserPromptSubmit, Stop)"
else
  warn "could not update ~/.claude/settings.json; fix invalid JSON and relaunch Clinch"
fi

# Notification plugins are optional because their marketplace commands can need network
# access. The app's built-in plugin manager also offers this as a one-click install/update.
if (( INSTALL_PLUGINS )); then
  source "$SRC/install-agent-plugins.sh" 2>/dev/null \
    && warp_install_agent_notification_plugins \
    || true
fi

log "Agent resume is ready. New Claude and Codex sessions are captured immediately."
