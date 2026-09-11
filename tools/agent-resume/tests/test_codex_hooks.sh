#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d)"
export WARP_AGENT_RESUME_DIR="$TMP/reg"
# No PATH export: the hooks must find clinch-agent-resume as a sibling of the script,
# because the real hook environment does not inherit the shell PATH.
export WARP_TERMINAL_SESSION_UUID="bb22"
export WARP_AGENT_RESUME_FAKE_ANCESTRY="codex"

echo '{"session_id":"sess-77","cwd":"/tmp/repo","source":"startup"}' | bash "$HERE/codex-session-start.sh"
f="$WARP_AGENT_RESUME_DIR/bb22.json"
grep -q '"command": "clinch_agent_resume_launch codex sess-77"' "$f" || { echo "FAIL: start"; exit 1; }
grep -q '"cwd": "/tmp/repo"' "$f" || { echo "FAIL: cwd"; exit 1; }

# Bypass + model from the payload are carried into the resume command.
echo '{"session_id":"sess-88","cwd":"/tmp/repo","permission_mode":"bypassPermissions","model":"gpt-5.3-codex"}' | bash "$HERE/codex-session-start.sh"
grep -q '"command": "clinch_agent_resume_launch codex sess-88 --dangerously-bypass-approvals-and-sandbox --model gpt-5.3-codex"' "$f" || { echo "FAIL: codex bypass+model"; exit 1; }

# Non-bypass modes carry only the model (conservative mapping).
echo '{"session_id":"sess-99","cwd":"/tmp/repo","permission_mode":"acceptEdits","model":"gpt-5.3-codex"}' | bash "$HERE/codex-session-start.sh"
grep -q '"command": "clinch_agent_resume_launch codex sess-99 --model gpt-5.3-codex"' "$f" || { echo "FAIL: codex non-bypass carries model only"; exit 1; }

# Nested Codex must not replace or remove an outer pane owner.
echo '{"session_id":"sess-nested","cwd":"/tmp/child"}' \
  | WARP_AGENT_RESUME_FAKE_ANCESTRY="codex,zsh,claude" bash "$HERE/codex-session-start.sh"
grep -q 'sess-99' "$f" || { echo "FAIL: nested Codex replaced outer owner"; exit 1; }
echo '{"session_id":"sess-nested","cwd":"/tmp/child"}' \
  | WARP_AGENT_RESUME_FAKE_ANCESTRY="codex,zsh,claude" bash "$HERE/codex-session-end.sh"
grep -q 'sess-99' "$f" || { echo "FAIL: nested Codex end removed outer owner"; exit 1; }

# A mismatched end is a no-op; app termination preserves; a normal matching end removes.
echo '{"session_id":"sess-77","cwd":"/tmp/repo"}' | bash "$HERE/codex-session-end.sh"
grep -q 'sess-99' "$f" || { echo "FAIL: mismatched Codex end removed owner"; exit 1; }
printf '%s\n' "$$" > "$WARP_AGENT_RESUME_DIR/.app-terminating"
echo '{"session_id":"sess-99","cwd":"/tmp/repo"}' | bash "$HERE/codex-session-end.sh"
grep -q 'sess-99' "$f" || { echo "FAIL: app shutdown removed Codex owner"; exit 1; }
rm -f "$WARP_AGENT_RESUME_DIR/.app-terminating"
echo '{"session_id":"sess-99","cwd":"/tmp/repo"}' | bash "$HERE/codex-session-end.sh"
[[ ! -f "$f" ]] || { echo "FAIL: end did not remove"; exit 1; }

# Codex CLI 0.144.3 UserPromptSubmit fixture. The supported schema includes session/turn ids,
# optional agent fields, transcript path, cwd, event, model, permission mode, and exact prompt.
# The prompt-only hook must preserve multiline content without changing the pane registry.
echo '{"session_id":"owner-stays","cwd":"/tmp/repo"}' | bash "$HERE/codex-session-start.sh"
prompt_in='fix "quotes" and \
keep this line'
jq -cn --arg p "$prompt_in" '{
  session_id:"sess-prompt", turn_id:"turn-1", agent_id:null, agent_type:null,
  transcript_path:"/tmp/rollout-sess-prompt.jsonl", cwd:"/tmp/repo",
  hook_event_name:"UserPromptSubmit", model:"gpt-5.3-codex",
  permission_mode:"bypassPermissions", prompt:$p
}' | bash "$HERE/codex-prompt-submit.sh"
mf="$WARP_AGENT_RESUME_DIR/prompts/codex/sess-prompt.jsonl"
[[ -f "$mf" ]] || { echo "FAIL: Codex prompt not mirrored"; exit 1; }
[[ "$(jq -r '.prompt' "$mf")" == "$prompt_in" ]] || { echo "FAIL: Codex prompt text mangled"; exit 1; }
[[ "$(jq -r '.cwd' "$mf")" == "/tmp/repo" ]] || { echo "FAIL: Codex mirror cwd wrong"; exit 1; }
grep -q 'owner-stays' "$f" || { echo "FAIL: prompt hook rewrote pane registry"; exit 1; }
[[ "$(stat -f '%Lp' "$mf")" == "600" ]] || { echo "FAIL: Codex mirror not private"; exit 1; }
[[ "$(stat -f '%Lp' "$(dirname "$mf")")" == "700" ]] || { echo "FAIL: Codex mirror dir not private"; exit 1; }

# Prompt capture is auxiliary and must fail open even when a hook runner omits HOME or a
# first-launch helper refresh is incomplete.
printf '%s\n' '{"session_id":"no-home","cwd":"/tmp","hook_event_name":"UserPromptSubmit","prompt":"secret"}' \
  | env -u HOME -u WARP_AGENT_RESUME_DIR \
      WARP_TERMINAL_SESSION_UUID="bb22" bash "$HERE/codex-prompt-submit.sh"
failing_bin="$TMP/failing-prompt-bin"
mkdir -p "$failing_bin"
cp "$HERE/codex-prompt-submit.sh" "$failing_bin/"
printf '#!/bin/sh\nexit 1\n' > "$failing_bin/prompt-mirror.sh"
chmod 755 "$failing_bin/prompt-mirror.sh"
printf '%s\n' '{"session_id":"helper-fails","hook_event_name":"UserPromptSubmit","prompt":"secret"}' \
  | bash "$failing_bin/codex-prompt-submit.sh"

# Repeated identical messages are separate turns; Stop appends a boundary and empty prompts append
# nothing. The boundary lets the reader preserve an intentional repeat after a completed turn.
jq -cn --arg p "$prompt_in" '{session_id:"sess-prompt",cwd:"/tmp/repo",hook_event_name:"UserPromptSubmit",prompt:$p}' \
  | bash "$HERE/codex-prompt-submit.sh"
before="$(wc -l < "$mf")"
echo '{"session_id":"sess-prompt","cwd":"/tmp/repo","hook_event_name":"Stop","prompt":"ignored"}' \
  | bash "$HERE/codex-prompt-submit.sh"
after_stop="$(wc -l < "$mf")"
echo '{"session_id":"sess-prompt","cwd":"/tmp/repo","hook_event_name":"UserPromptSubmit","prompt":""}' \
  | bash "$HERE/codex-prompt-submit.sh"
printf '%s\n' '{"session_id":"sess-prompt","cwd":"/tmp/repo","hook_event_name":"UserPromptSubmit","prompt":"  \n  "}' \
  | bash "$HERE/codex-prompt-submit.sh"
[[ "$before" -eq 2 && "$after_stop" -eq 3 && "$(wc -l < "$mf")" -eq 3 \
  && "$(tail -n 1 "$mf" | jq -r '.stop')" == true ]] \
  || { echo "FAIL: Codex repeat/ignored event semantics wrong"; exit 1; }

# A pre-existing symlink provider directory must never redirect sensitive prompt writes.
symlink_reg="$TMP/symlink-reg"; outside="$TMP/outside"; mkdir -p "$symlink_reg/prompts" "$outside"
ln -s "$outside" "$symlink_reg/prompts/codex"
echo '{"session_id":"escape","cwd":"/tmp","hook_event_name":"UserPromptSubmit","prompt":"secret"}' \
  | WARP_AGENT_RESUME_DIR="$symlink_reg" bash "$HERE/codex-prompt-submit.sh"
[[ -z "$(find "$outside" -type f -print)" ]] || { echo "FAIL: provider symlink redirected prompt"; exit 1; }
root_symlink_reg="$TMP/root-symlink-reg"; mkdir -p "$root_symlink_reg"; ln -s "$outside" "$root_symlink_reg/prompts"
echo '{"session_id":"root-escape","cwd":"/tmp","hook_event_name":"UserPromptSubmit","prompt":"secret"}' \
  | WARP_AGENT_RESUME_DIR="$root_symlink_reg" bash "$HERE/codex-prompt-submit.sh"
[[ -z "$(find "$outside" -type f -print)" ]] || { echo "FAIL: prompts-root symlink redirected prompt"; exit 1; }
rm -f "$f"

# No-op outside a Clinch pane. (The registry dir legitimately holds journal.jsonl from the
# writes above, so assert on pane entries, not an empty dir.)
unset WARP_TERMINAL_SESSION_UUID
echo '{"session_id":"x","cwd":"/tmp"}' | bash "$HERE/codex-session-start.sh"
entries="$(find "$WARP_AGENT_RESUME_DIR" -name '*.json' \
  ! -name 'toolbelt-learning.json' ! -name 'toolbelt-learning-resolutions.json' 2>/dev/null)"
[[ -z "$entries" ]] || { echo "FAIL: wrote outside pane"; exit 1; }
echo '{"session_id":"outside","cwd":"/tmp","hook_event_name":"UserPromptSubmit","prompt":"secret"}' \
  | bash "$HERE/codex-prompt-submit.sh"
[[ ! -e "$WARP_AGENT_RESUME_DIR/prompts/codex/outside.jsonl" ]] \
  || { echo "FAIL: prompt mirrored outside Clinch pane"; exit 1; }
echo "PASS"
