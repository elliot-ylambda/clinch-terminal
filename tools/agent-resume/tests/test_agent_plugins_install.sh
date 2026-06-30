#!/usr/bin/env bash
# Verifies warp_install_agent_notification_plugins runs the right plugin CLI commands
# when claude/codex are present, skips cleanly when they're absent, and never aborts.
set -uo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
source "$HERE/install-agent-plugins.sh"   # source-guard: defines the fn, runs nothing

fail() { echo "FAIL: $1"; exit 1; }
TMP="$(mktemp -d)"; mkdir -p "$TMP/bin"; LOG="$TMP/calls.log"; : > "$LOG"

# Fake claude/codex that record their argv (one line per invocation), exit 0.
for tool in claude codex; do
  cat > "$TMP/bin/$tool" <<EOF
#!/usr/bin/env bash
echo "$tool \$*" >> "$LOG"
EOF
  chmod +x "$TMP/bin/$tool"
done

# Case A: both present -> all four commands recorded.
PATH="$TMP/bin:$PATH" warp_install_agent_notification_plugins >/dev/null 2>&1 \
  || fail "function returned non-zero with both tools present"
grep -qx "claude plugin marketplace add warpdotdev/claude-code-warp" "$LOG" || fail "missing claude marketplace add"
grep -qx "claude plugin install warp@claude-code-warp"               "$LOG" || fail "missing claude install"
grep -qx "codex plugin marketplace add warpdotdev/codex-warp"        "$LOG" || fail "missing codex marketplace add"
grep -qx "codex plugin add warp@codex-warp"                          "$LOG" || fail "missing codex add"

# Case B: a tool that fails must not abort the function (best-effort).
cat > "$TMP/bin/claude" <<'EOF'
#!/usr/bin/env bash
exit 3
EOF
chmod +x "$TMP/bin/claude"
PATH="$TMP/bin:$PATH" warp_install_agent_notification_plugins >/dev/null 2>&1 \
  || fail "function aborted when a plugin command failed"

# Case C: tools absent -> still exits 0, records nothing new.
: > "$LOG"
PATH="$TMP/empty:$PATH" warp_install_agent_notification_plugins >/dev/null 2>&1 \
  || fail "function aborted when tools absent"
[[ -s "$LOG" ]] && fail "recorded calls when no tools on PATH"

echo "PASS"
