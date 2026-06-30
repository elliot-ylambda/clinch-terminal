#!/usr/bin/env bash
# Installs Warp's CLI-agent notification plugins into Claude Code / Codex so that running
# them inside Clinch emits the OSC-777 `warp://cli-agent` status events Clinch already
# consumes (badge on tabs + desktop notifications). Best-effort: never aborts, never blocks.
#
# Sourcing this file only defines the function (so the tests can exercise it); the install
# runs only when the file is executed directly.

warp_install_agent_notification_plugins() {
  # Claude
  if command -v claude >/dev/null 2>&1; then
    echo "Installing Claude notification plugin (warp@claude-code-warp)..."
    claude plugin marketplace add warpdotdev/claude-code-warp </dev/null >/dev/null 2>&1 \
      || echo "  warn: 'claude plugin marketplace add' failed (offline?) -- skipping"
    claude plugin install warp@claude-code-warp </dev/null >/dev/null 2>&1 \
      || echo "  warn: 'claude plugin install' failed -- skipping"
  else
    echo "claude not on PATH -- skipping Claude notification plugin"
  fi
  # Codex
  if command -v codex >/dev/null 2>&1; then
    echo "Installing Codex notification plugin (warp@codex-warp)..."
    codex plugin marketplace add warpdotdev/codex-warp </dev/null >/dev/null 2>&1 \
      || echo "  warn: 'codex plugin marketplace add' failed (offline?) -- skipping"
    codex plugin add warp@codex-warp </dev/null >/dev/null 2>&1 \
      || echo "  warn: 'codex plugin add' failed -- skipping"
  else
    echo "codex not on PATH -- skipping Codex notification plugin"
  fi
  return 0
}

# Run only when executed directly; sourcing (tests) just loads the function.
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  warp_install_agent_notification_plugins
fi
