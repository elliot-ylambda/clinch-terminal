#!/usr/bin/env bash
# Manages Clinch's local Claude Code / Codex session-capture integration.
# No command means no mutation. The app invokes `repair --quiet` only after a user has enabled it.
set -euo pipefail

usage() {
  cat <<'EOF'
usage: install.sh <enable|repair|disable|status|purge> [--quiet]

  enable   Install Clinch-owned helpers and add managed Claude/Codex hooks.
  repair   Refresh managed files only when the durable consent marker exists.
  disable  Remove managed hooks and helpers; keep captured conversation metadata.
  status   Print enabled or disabled without changing anything.
  purge    Disable the integration and delete Clinch's captured conversation metadata.

Notification plugins are separate and are never installed by this command.
EOF
}

COMMAND="${1:-}"
if [[ -z "$COMMAND" || "$COMMAND" == "--help" || "$COMMAND" == "-h" ]]; then
  usage
  exit 0
fi
shift

QUIET=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --quiet) QUIET=1 ;;
    --help|-h) usage; exit 0 ;;
    *) echo "error: unknown option: $1" >&2; exit 2 ;;
  esac
  shift
done

case "$COMMAND" in
  enable|repair|disable|status|purge) ;;
  *) echo "error: unknown command: $COMMAND" >&2; usage >&2; exit 2 ;;
esac

log() { (( QUIET )) || printf '%s\n' "$*"; }
warn() { printf 'clinch session-capture warning: %s\n' "$*" >&2; }

SRC="$(cd "$(dirname "$0")" && pwd)"
BIN="${CLINCH_AGENT_BIN_DIR:-$HOME/.warp/agent-resume-bin}"
REG="${WARP_AGENT_RESUME_DIR:-$HOME/.warp/agent-resume}"
STATE="${CLINCH_AGENT_STATE_DIR:-$HOME/Library/Application Support/sh.clinch.Clinch/agent-integration}"
CONSENT="$STATE/enabled"
RECEIPT="$STATE/receipt"
CLAUDE_CFG="${CLINCH_CLAUDE_SETTINGS:-$HOME/.claude/settings.json}"
CODEX_CFG="${CLINCH_CODEX_CONFIG:-$HOME/.codex/config.toml}"

sha_or_absent() {
  if [[ -f "$1" ]]; then
    /usr/bin/shasum -a 256 "$1" | /usr/bin/awk '{print $1}'
  else
    printf 'absent\n'
  fi
}

mode_or_absent() {
  if [[ -f "$1" ]]; then
    /usr/bin/stat -f '%Lp' "$1" 2>/dev/null || printf '600\n'
  else
    printf 'absent\n'
  fi
}

install_runtime() {
  mkdir -p "$BIN" "$REG"
  chmod 700 "$BIN" "$REG"
  install -m 0755 \
    "$SRC/agent-json" \
    "$SRC/clinch-agent-resume" \
    "$SRC/warp-agent-resume" \
    "$SRC/clinch_agent_resume_launch" \
    "$SRC/claude-capture.sh" \
    "$SRC/prompt-mirror.sh" \
    "$SRC/codex-session-start.sh" \
    "$SRC/codex-prompt-submit.sh" \
    "$SRC/codex-session-end.sh" \
    "$BIN/"
  install -m 0644 "$SRC/agent-json.js" "$SRC/claude.zsh" "$BIN/"
  install -m 0755 "$SRC/clinch_agent_resume_launch" "$BIN/warp_agent_resume_launch"
  rm -f "$BIN/claude-session-start.sh" "$BIN/install-agent-plugins.sh"
}

remove_runtime() {
  local name
  for name in \
    agent-json agent-json.js clinch-agent-resume warp-agent-resume \
    clinch_agent_resume_launch warp_agent_resume_launch claude-capture.sh \
    prompt-mirror.sh claude-session-start.sh claude.zsh codex-session-start.sh \
    codex-prompt-submit.sh codex-session-end.sh \
    install-agent-plugins.sh wire-claude-hooks.sh unwire-claude-hooks.sh; do
    rm -f "$BIN/$name"
  done
  rmdir "$BIN" 2>/dev/null || true
}

strip_codex_managed_blocks() {
  local source="$1" output="$2"
  /usr/bin/awk '
      function flush_blanks() {
        for (i = 0; i < pending_blanks; i++) print ""
        pending_blanks = 0
      }
      /^# >>> (clinch|warp) agent-resume >>>$/ {
        if (managed) exit 42
        managed = 1
        next
      }
      /^# <<< (clinch|warp) agent-resume <<<$/{
        if (!managed) exit 42
        managed = 0
        next
      }
      !managed && $0 == "" { pending_blanks++; next }
      !managed { flush_blanks(); print }
      END { if (managed) exit 42 }
    ' "$source" > "$output"
}

write_codex_config() {
  local output="$1"
  if [[ -f "$CODEX_CFG" ]]; then
    strip_codex_managed_blocks "$CODEX_CFG" "$output"
  else
    : > "$output"
  fi
  {
    printf '\n# >>> clinch agent-resume >>>\n'
    printf '[[hooks.SessionStart]]\n'
    printf 'matcher = "startup|resume"\n'
    printf '[[hooks.SessionStart.hooks]]\n'
    printf 'type = "command"\n'
    printf 'command = "%s/codex-session-start.sh"\n\n' "$BIN"
    printf '[[hooks.UserPromptSubmit]]\n'
    printf '[[hooks.UserPromptSubmit.hooks]]\n'
    printf 'type = "command"\n'
    printf 'command = "%s/codex-prompt-submit.sh"\n\n' "$BIN"
    printf '[[hooks.SessionEnd]]\n'
    printf '[[hooks.SessionEnd.hooks]]\n'
    printf 'type = "command"\n'
    printf 'command = "%s/codex-session-end.sh"\n' "$BIN"
    printf '# <<< clinch agent-resume <<<\n'
  } >> "$output"
}

write_codex_without_managed_block() {
  local output="$1"
  strip_codex_managed_blocks "$CODEX_CFG" "$output"
}

apply_staged_file() {
  local staged="$1" destination="$2" mode="$3"
  mkdir -p "$(dirname "$destination")"
  [[ "$mode" != "absent" ]] || mode=600
  chmod "$mode" "$staged"
  if [[ -f "$destination" ]] && cmp -s "$staged" "$destination"; then
    rm -f "$staged"
  else
    mv "$staged" "$destination"
  fi
}

write_receipt() {
  local claude_pre_sha="$1" claude_pre_mode="$2" codex_pre_sha="$3" codex_pre_mode="$4"
  local tmp="$STATE/.receipt.$$"
  umask 077
  {
    printf 'schema=1\n'
    printf 'owner=sh.clinch.Clinch\n'
    printf 'runtime_dir=%s\n' "$BIN"
    printf 'capture_data_dir=%s\n' "$REG"
    printf 'claude_settings=%s\n' "$CLAUDE_CFG"
    printf 'claude_pre_sha256=%s\n' "$claude_pre_sha"
    printf 'claude_pre_mode=%s\n' "$claude_pre_mode"
    printf 'claude_post_sha256=%s\n' "$(sha_or_absent "$CLAUDE_CFG")"
    printf 'claude_post_mode=%s\n' "$(mode_or_absent "$CLAUDE_CFG")"
    printf 'codex_config=%s\n' "$CODEX_CFG"
    printf 'codex_pre_sha256=%s\n' "$codex_pre_sha"
    printf 'codex_pre_mode=%s\n' "$codex_pre_mode"
    printf 'codex_post_sha256=%s\n' "$(sha_or_absent "$CODEX_CFG")"
    printf 'codex_post_mode=%s\n' "$(mode_or_absent "$CODEX_CFG")"
  } > "$tmp"
  chmod 600 "$tmp"
  mv "$tmp" "$RECEIPT"
}

configure_integration() {
  local is_first_enable="$1"
  local tx claude_mode codex_mode claude_pre_sha codex_pre_sha
  tx="$(mktemp -d -t clinch-agent-enable)"
  trap 'rm -rf "$tx"' RETURN

  claude_mode="$(mode_or_absent "$CLAUDE_CFG")"
  codex_mode="$(mode_or_absent "$CODEX_CFG")"
  claude_pre_sha="$(sha_or_absent "$CLAUDE_CFG")"
  codex_pre_sha="$(sha_or_absent "$CODEX_CFG")"

  if [[ -f "$CLAUDE_CFG" ]]; then
    "$SRC/agent-json" wire-claude \
      "$BIN/claude-session-start.sh" "$BIN/claude-capture.sh" \
      < "$CLAUDE_CFG" > "$tx/claude.json"
  else
    printf '{}' | "$SRC/agent-json" wire-claude \
      "$BIN/claude-session-start.sh" "$BIN/claude-capture.sh" \
      > "$tx/claude.json"
  fi
  write_codex_config "$tx/codex.toml"

  install_runtime
  apply_staged_file "$tx/claude.json" "$CLAUDE_CFG" "$claude_mode"
  apply_staged_file "$tx/codex.toml" "$CODEX_CFG" "$codex_mode"

  mkdir -p "$STATE"
  chmod 700 "$STATE"
  if [[ "$is_first_enable" == "1" || ! -f "$RECEIPT" ]]; then
    write_receipt "$claude_pre_sha" "$claude_mode" "$codex_pre_sha" "$codex_mode"
  fi
  printf '1\n' > "$STATE/.enabled.$$"
  chmod 600 "$STATE/.enabled.$$"
  mv "$STATE/.enabled.$$" "$CONSENT"
  trap - RETURN
  rm -rf "$tx"
}

disable_integration() {
  local failures=0 tmp mode
  rm -f "$CONSENT"

  if [[ -f "$CLAUDE_CFG" ]]; then
    tmp="$(mktemp "$(dirname "$CLAUDE_CFG")/.clinch-settings.XXXXXX")"
    mode="$(mode_or_absent "$CLAUDE_CFG")"
    if "$SRC/agent-json" unwire-claude \
      "$BIN/claude-session-start.sh" "$BIN/claude-capture.sh" \
      < "$CLAUDE_CFG" > "$tmp"; then
      apply_staged_file "$tmp" "$CLAUDE_CFG" "$mode"
    else
      rm -f "$tmp"
      warn "could not remove hooks from $CLAUDE_CFG; the JSON is invalid"
      failures=1
    fi
  fi

  if [[ -f "$CODEX_CFG" ]]; then
    tmp="$(mktemp "$(dirname "$CODEX_CFG")/.clinch-codex.XXXXXX")"
    mode="$(mode_or_absent "$CODEX_CFG")"
    if write_codex_without_managed_block "$tmp"; then
      apply_staged_file "$tmp" "$CODEX_CFG" "$mode"
    else
      rm -f "$tmp"
      warn "could not remove hooks from $CODEX_CFG; managed block markers are invalid"
      failures=1
    fi
  fi

  remove_runtime
  if (( failures )); then
    return 1
  fi
}

case "$COMMAND" in
  status)
    if [[ -f "$CONSENT" && ! -L "$CONSENT" ]]; then
      printf 'enabled\n'
    else
      printf 'disabled\n'
    fi
    ;;
  enable)
    if [[ -f "$CONSENT" && ! -L "$CONSENT" ]]; then
      configure_integration 0
      log "Clinch session capture was already enabled; managed files were refreshed."
    else
      if (( ! QUIET )); then
        cat <<EOF
Clinch will add clearly marked hooks to:
  $CLAUDE_CFG
  $CODEX_CFG
and install helper commands in:
  $BIN
Captured session metadata will be stored in:
  $REG
Consent and the non-secret change receipt will be stored in:
  $STATE
No notification plugin is installed.
EOF
      fi
      configure_integration 1
      log "Clinch session capture is enabled."
    fi
    ;;
  repair)
    if [[ ! -f "$CONSENT" || -L "$CONSENT" ]]; then
      exit 0
    fi
    configure_integration 0
    log "Clinch session capture files were refreshed."
    ;;
  disable)
    disable_integration
    log "Clinch session capture is disabled. Captured metadata was kept in $REG."
    ;;
  purge)
    disable_integration
    case "$REG" in
      "$HOME/.warp/agent-resume"|"${WARP_AGENT_RESUME_DIR:-}")
        log "Removing Clinch capture metadata from: $REG"
        rm -rf "$REG"
        ;;
      *) warn "refusing to purge unexpected data directory: $REG"; exit 1 ;;
    esac
    log "Clinch session capture is disabled and its captured metadata was removed."
    ;;
esac
