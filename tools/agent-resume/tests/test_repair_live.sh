#!/usr/bin/env bash
# Reconciles legacy/nested pane ownership before an update while process ancestry is live.
set -euo pipefail

HERE="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
export HOME="$TMP/home"
export WARP_AGENT_RESUME_DIR="$TMP/reg"
export WARP_AGENT_RESUME_CLAUDE_PROJECTS="$HOME/.claude/projects"
export WARP_AGENT_RESUME_CODEX_SESSIONS="$HOME/.codex/sessions"
mkdir -p "$WARP_AGENT_RESUME_DIR" "$WARP_AGENT_RESUME_CLAUDE_PROJECTS/p" \
  "$WARP_AGENT_RESUME_CODEX_SESSIONS/2026/07/12"
CLI="$HERE/clinch-agent-resume"

printf '%s\n' \
  pane-cross pane-same pane-codex pane-shell pane-live-owner pane-bridge-owner pane-bridge-leak \
  > "$WARP_AGENT_RESUME_DIR/active-panes"

# The visible root is Claude, but a nested Codex took over this pane. Journal history has
# the resumable outer Claude identity, which repair-live must restore.
"$CLI" write pane-cross 'clinch_agent_resume_launch claude parent-claude' /work
printf '%s\n' '{"type":"user","cwd":"/work"}' \
  > "$WARP_AGENT_RESUME_CLAUDE_PROJECTS/p/parent-claude.jsonl"
"$CLI" write pane-cross 'clinch_agent_resume_launch codex nested-codex' /work/child
printf '%s\n' '{"role":"user"}' \
  > "$WARP_AGENT_RESUME_CODEX_SESSIONS/2026/07/12/rollout-test-nested-codex.jsonl"

# A nested Claude can clobber an outer Claude without changing providers. The root
# process's cwd is the unambiguous discriminator: repair back to the newest durable
# history entry from that live directory, not the child's temporary checkout.
"$CLI" write pane-same 'clinch_agent_resume_launch claude parent-same' /work session_01SAME
"$CLI" write pane-same 'clinch_agent_resume_launch claude nested-same --model haiku' /tmp/child

# Exact Codex ownership is already healthy and must remain untouched.
"$CLI" write pane-codex 'clinch_agent_resume_launch codex exact-codex' /work
printf '%s\n' '{"role":"assistant"}' \
  > "$WARP_AGENT_RESUME_CODEX_SESSIONS/2026/07/12/rollout-test-exact-codex.jsonl"

# No root agent remains in this active pane: the stale entry should be removed so a shell
# does not unexpectedly relaunch an old agent.
"$CLI" write pane-shell 'clinch_agent_resume_launch claude exited-claude' /work

# If a future provider executable name is not recognized by the process scan, a live
# hook-recorded PID/tty is stronger evidence than the empty map and must prevent deletion.
test_tty="$(/bin/ps -o tty= -p $$ | tr -d '[:space:]')"
"$CLI" write pane-live-owner 'clinch_agent_resume_launch claude live-owner' /work "" $$ "$test_tty"

# One real local conversation owns a bridge leaked into a second pane. Keep the bridge only
# on the resumable owner so the other pane cannot teleport the same cloud conversation.
"$CLI" write pane-bridge-owner 'clinch_agent_resume_launch claude bridge-owner' \
  /work session_01DUP $$ "$test_tty"
printf '%s\n' '{"type":"assistant","cwd":"/work"}' \
  > "$WARP_AGENT_RESUME_CLAUDE_PROJECTS/p/bridge-owner.jsonl"
"$CLI" write pane-bridge-leak 'clinch_agent_resume_launch claude bridge-leak' /work session_01DUP

work64="$(printf /work | /usr/bin/base64 | tr -d '\n')"
{
  printf 'pane-cross\tclaude\t%s\n' "$work64"
  printf 'pane-same\tclaude\t%s\n' "$work64"
  printf 'pane-codex\tcodex\t%s\n' "$work64"
  printf 'pane-bridge-owner\tclaude\t%s\n' "$work64"
  printf 'pane-bridge-leak\tclaude\t%s\n' "$work64"
} > "$TMP/live-map"
export WARP_AGENT_RESUME_LIVE_MAP="$TMP/live-map"

out="$("$CLI" repair-live)"
[[ "$out" == 'repaired 2 pane mappings; removed 1 inactive mappings' ]] \
  || { echo "FAIL: unexpected repair summary: $out"; exit 1; }
grep -q 'claude parent-claude' "$WARP_AGENT_RESUME_DIR/pane-cross.json" \
  || { echo "FAIL: visible outer Claude was not restored"; exit 1; }
grep -q 'codex exact-codex' "$WARP_AGENT_RESUME_DIR/pane-codex.json" \
  || { echo "FAIL: healthy Codex mapping changed"; exit 1; }
grep -q 'claude parent-same' "$WARP_AGENT_RESUME_DIR/pane-same.json" \
  || { echo "FAIL: same-provider nested Claude mapping was not repaired"; exit 1; }
[[ ! -e "$WARP_AGENT_RESUME_DIR/pane-shell.json" ]] \
  || { echo "FAIL: exited agent mapping survived"; exit 1; }
grep -q 'claude live-owner' "$WARP_AGENT_RESUME_DIR/pane-live-owner.json" \
  || { echo "FAIL: live hook-recorded owner was removed after scan miss"; exit 1; }
grep -q '"bridge": "session_01DUP"' "$WARP_AGENT_RESUME_DIR/pane-bridge-owner.json" \
  || { echo "FAIL: real bridge owner lost bridge"; exit 1; }
grep -q "\"owner_pid\": \"$$\"" "$WARP_AGENT_RESUME_DIR/pane-bridge-owner.json" \
  || { echo "FAIL: duplicate bridge repair discarded live owner metadata"; exit 1; }
grep -q '"bridge"' "$WARP_AGENT_RESUME_DIR/pane-bridge-leak.json" \
  && { echo "FAIL: leaked duplicate bridge survived"; exit 1; }

echo "PASS"
