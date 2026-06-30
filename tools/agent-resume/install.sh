#!/usr/bin/env bash
# Installs the agent-resume capture layer (claude wrapper + codex hooks + registry CLI)
# into your shell so that running `claude`/`codex` inside Warp records a resumable
# session per pane. The Rust side of Warp reads ~/.warp/agent-resume/<pane_uuid>.json
# on restore and re-runs the captured command.
#
# Safe to re-run (idempotent). macOS, zsh.
set -euo pipefail

SRC="$(cd "$(dirname "$0")" && pwd)"
BIN="$HOME/.warp/agent-resume-bin"
REG="$HOME/.warp/agent-resume"

mkdir -p "$BIN" "$REG"
chmod 700 "$REG"

install -m 0755 "$SRC/warp-agent-resume" "$SRC/claude-session-start.sh" \
  "$SRC/codex-session-start.sh" "$SRC/codex-session-end.sh" \
  "$SRC/install-agent-plugins.sh" "$BIN/"
install -m 0644 "$SRC/claude.zsh" "$BIN/claude.zsh"

# Wire ~/.zshrc (PATH for the CLI + source the replay functions) once.
marker="# >>> warp agent-resume >>>"
if ! grep -qF "$marker" "$HOME/.zshrc" 2>/dev/null; then
  {
    echo ""
    echo "$marker"
    echo "export PATH=\"\$HOME/.warp/agent-resume-bin:\$PATH\""
    echo "source \"\$HOME/.warp/agent-resume-bin/claude.zsh\""
    echo "# <<< warp agent-resume <<<"
  } >> "$HOME/.zshrc"
  echo "Added agent-resume block to ~/.zshrc"
else
  echo "~/.zshrc already wired (skipping)"
fi

# Wire ~/.codex/config.toml hooks once (paths point at the installed bin).
CODEX_CFG="$HOME/.codex/config.toml"
if [[ -f "$CODEX_CFG" ]] && grep -qF "agent-resume-bin/codex-session-start.sh" "$CODEX_CFG"; then
  echo "~/.codex/config.toml already wired (skipping)"
else
  mkdir -p "$HOME/.codex"
  cat >> "$CODEX_CFG" <<EOF

# >>> warp agent-resume >>>
[[hooks.SessionStart]]
matcher = "startup|resume"
[[hooks.SessionStart.hooks]]
type = "command"
command = "$BIN/codex-session-start.sh"

[[hooks.SessionEnd]]
[[hooks.SessionEnd.hooks]]
type = "command"
command = "$BIN/codex-session-end.sh"
# <<< warp agent-resume <<<
EOF
  echo "Added agent-resume hooks to ~/.codex/config.toml"
fi

# Wire the Claude SessionStart hook into ~/.claude/settings.json once (jq merge so we
# never clobber existing settings/hooks). This is what captures the live Claude session.
CLAUDE_CFG="$HOME/.claude/settings.json"
HOOK_CMD="$BIN/claude-session-start.sh"
if [[ -f "$CLAUDE_CFG" ]] && jq -e --arg c "$HOOK_CMD" \
     '.hooks.SessionStart[]?.hooks[]? | select(.command==$c)' "$CLAUDE_CFG" >/dev/null 2>&1; then
  echo "~/.claude/settings.json SessionStart hook already wired (skipping)"
else
  command -v jq >/dev/null || { echo "error: jq is required to wire the Claude hook" >&2; exit 1; }
  mkdir -p "$HOME/.claude"
  [[ -f "$CLAUDE_CFG" ]] || echo '{}' > "$CLAUDE_CFG"
  tmp="$(mktemp)"
  jq --arg c "$HOOK_CMD" '
    .hooks = (.hooks // {})
    | .hooks.SessionStart = ((.hooks.SessionStart // [])
        + [ { "hooks": [ { "type": "command", "command": $c } ] } ])
  ' "$CLAUDE_CFG" > "$tmp" && mv "$tmp" "$CLAUDE_CFG"
  echo "Added Claude SessionStart hook to ~/.claude/settings.json"
fi

# Install the CLI-agent notification plugins (best-effort) so Claude/Codex emit the
# status events that drive tab badges + desktop notifications in Clinch.
source "$SRC/install-agent-plugins.sh" 2>/dev/null && warp_install_agent_notification_plugins || true

echo ""
echo "Done. Requirements: jq, uuidgen (uuidgen is preinstalled on macOS; 'brew install jq' if missing)."
echo "Restart your shell (or 'source ~/.zshrc') so the replay functions load."
echo "Capture is via Claude/Codex SessionStart hooks; they only record inside a Warp pane"
echo "(WARP_TERMINAL_SESSION_UUID set). New Claude sessions are captured immediately; no restart needed for that."
