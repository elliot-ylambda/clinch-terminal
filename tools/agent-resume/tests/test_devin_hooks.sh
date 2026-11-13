#!/usr/bin/env bash
# Devin's hook, restore launcher and config wiring, driven with the payloads its docs describe.
set -euo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
export WARP_AGENT_RESUME_DIR="$TMP/reg"
export WARP_TERMINAL_SESSION_UUID="dd44"
# What Devin 3000.11 really runs: hooks sit under a `devin acp` child of the `devin` screen.
export WARP_AGENT_RESUME_FAKE_ANCESTRY="devin,devin,zsh"
export WARP_AGENT_RESUME_FAKE_OWNER_PID=1000
export WARP_AGENT_RESUME_FAKE_ARGV=""
unset CLINCH_CONTROL_PID WARP_CLI_AGENT_PROTOCOL_VERSION WARP_CLIENT_VERSION CLINCH_COORDINATOR_ROLE
unset DEVIN_PERMISSION_MODE DEVIN_MODEL DEVIN_SANDBOX DEVIN_PROJECT_DIR
HOOK="$HERE/devin-hook.sh"
f="$WARP_AGENT_RESUME_DIR/dd44.json"
fail() { echo "FAIL: $1"; exit 1; }

# Session start records the exact conversation. Devin's payload, as 3000.11 sends it, has no cwd.
echo '{"hook_event_name":"SessionStart","source":"startup","session_id":"brisk-otter"}' | (cd "$TMP" && bash "$HOOK" SessionStart)
grep -q '"command": "clinch_agent_resume_launch devin brisk-otter"' "$f" || fail "start"
grep -q "\"cwd\": \"$(cd "$TMP" && pwd)\"" "$f" || fail "cwd falls back to the hook's directory"

# A hook prints nothing unless it has a decision to make: Devin would read it as one.
out="$(echo '{"session_id":"brisk-otter"}' | bash "$HOOK" PermissionRequest)"
[[ -z "$out" ]] || fail "permission hook printed: $out"
out="$(echo '{"session_id":"brisk-otter","prompt":"hi"}' | bash "$HOOK" UserPromptSubmit)"
[[ -z "$out" ]] || fail "prompt hook printed: $out"

# Launch flags come back with the session, from argv first and Devin's env vars otherwise.
echo '{"session_id":"brisk-otter"}' | WARP_AGENT_RESUME_FAKE_ARGV="devin --permission-mode accept-edits --model opus --sandbox -- fix it" bash "$HOOK" SessionStart
grep -q 'launch devin brisk-otter --permission-mode accept-edits --model opus --sandbox"' "$f" || fail "argv flags"
echo '{"session_id":"brisk-otter"}' | DEVIN_PERMISSION_MODE=smart DEVIN_MODEL=gpt DEVIN_SANDBOX=1 bash "$HOOK" SessionStart
grep -q 'launch devin brisk-otter --permission-mode smart --model gpt --sandbox"' "$f" || fail "env flags"
# Words in a prompt aren't flags, and shell syntax never reaches the restore command.
echo '{"session_id":"brisk-otter"}' | WARP_AGENT_RESUME_FAKE_ARGV='devin -- --model evil' bash "$HOOK" SessionStart
grep -q 'launch devin brisk-otter"' "$f" || fail "prompt text read as a flag"
echo '{"session_id":"brisk-otter"}' | WARP_AGENT_RESUME_FAKE_ARGV='devin --model $(rm)' bash "$HOOK" SessionStart
grep -q 'launch devin brisk-otter"' "$f" || fail "unsafe model kept"

# An unsafe session id records nothing.
rm -f "$f"
echo '{"session_id":"a;b"}' | bash "$HOOK" SessionStart
[[ ! -f "$f" ]] || fail "unsafe id recorded"
echo '{"session_id":"brisk-otter"}' | bash "$HOOK" SessionStart

# Prompts and final replies are logged, exact and multi-line, with the event from the command line.
log="$WARP_AGENT_RESUME_DIR/prompts/devin/brisk-otter.jsonl"
rm -f "$log"
printf '%s' '{"session_id":"brisk-otter","prompt":"line one\nline two"}' | bash "$HOOK" UserPromptSubmit
grep -q '"prompt":"line one\\nline two"' "$log" || fail "prompt log"
printf '%s' '{"session_id":"brisk-otter","stop_hook_active":false,"last_assistant_message":"all done"}' | bash "$HOOK" Stop
tail -1 "$log" | grep -q '"stop":true' || fail "turn boundary"
tail -1 "$log" | grep -q '"response":"all done"' || fail "final reply"

# Claude's log stays as small as it was.
printf '%s' '{"session_id":"cl-1","hook_event_name":"Stop","last_assistant_message":"secret"}' \
  | bash "$HERE/prompt-mirror.sh" claude
if grep -q 'secret' "$WARP_AGENT_RESUME_DIR/prompts/claude/cl-1.jsonl"; then fail "Claude reply logged"; fi

# Devin also runs Claude Code's hooks by default. They must stay out of the way.
rm -rf "$WARP_AGENT_RESUME_DIR/prompts/claude"
before="$(cat "$f")"
printf '%s' '{"session_id":"brisk-otter","cwd":"/tmp/x","hook_event_name":"UserPromptSubmit","prompt":"p"}' \
  | DEVIN_PROJECT_DIR=/tmp/x WARP_AGENT_RESUME_FAKE_ANCESTRY="claude" bash "$HERE/claude-capture.sh"
[[ ! -e "$WARP_AGENT_RESUME_DIR/prompts/claude" ]] || fail "Claude hook logged a Devin prompt"
[[ "$(cat "$f")" == "$before" ]] || fail "Claude hook replaced the restore target"

# A Devin run under another agent keeps its history but leaves the pane to the visible agent.
printf '%s' '{"session_id":"nested-1","prompt":"child"}' \
  | WARP_AGENT_RESUME_FAKE_ANCESTRY="devin,devin,zsh,claude" bash "$HOOK" UserPromptSubmit
grep -q 'brisk-otter' "$f" || fail "nested Devin replaced the owner"
[[ -f "$WARP_AGENT_RESUME_DIR/prompts/devin/nested-1.jsonl" ]] || fail "nested prompt lost"
echo '{"session_id":"brisk-otter","reason":"prompt_input_exit"}' \
  | WARP_AGENT_RESUME_FAKE_ANCESTRY="devin,devin,zsh,claude" bash "$HOOK" SessionEnd
grep -q 'brisk-otter' "$f" || fail "nested Devin end removed the owner"
# Devin started from inside Devin runs through a shell, so it is nested too, not the same session.
echo '{"session_id":"inner-1","source":"startup"}' \
  | WARP_AGENT_RESUME_FAKE_ANCESTRY="devin,devin,zsh,devin,devin,zsh" bash "$HOOK" SessionStart
grep -q 'brisk-otter' "$f" || fail "Devin inside Devin replaced the owner"

# A blank session the restore machinery started can't replace a pointer to a real conversation.
echo '{"session_id":"blank-1"}' | WARP_AGENT_RESUME_STARTED_FRESH=1 bash "$HOOK" SessionStart
grep -q 'brisk-otter' "$f" || fail "machinery blank took the entry"
printf '%s' '{"session_id":"blank-1","prompt":"first real prompt"}' | WARP_AGENT_RESUME_STARTED_FRESH=1 bash "$HOOK" UserPromptSubmit
grep -q 'blank-1' "$f" || fail "first prompt did not take over"

# /clear starts a new session id in the same process: the next prompt follows it.
printf '%s' '{"session_id":"after-clear","prompt":"again"}' | bash "$HOOK" UserPromptSubmit
grep -q 'launch devin after-clear' "$f" || fail "new session id not followed"

# An exit Devin didn't choose and an app shutdown both keep the entry. A real exit removes it.
echo '{"session_id":"after-clear","reason":"other"}' | bash "$HOOK" SessionEnd
[[ -f "$f" ]] || fail "signal exit removed the entry"
printf '%s\n' "$$" > "$WARP_AGENT_RESUME_DIR/.app-terminating"
echo '{"session_id":"after-clear","reason":"prompt_input_exit"}' | bash "$HOOK" SessionEnd
[[ -f "$f" ]] || fail "app shutdown removed the entry"
rm -f "$WARP_AGENT_RESUME_DIR/.app-terminating"
echo '{"session_id":"some-other","reason":"prompt_input_exit"}' | bash "$HOOK" SessionEnd
[[ -f "$f" ]] || fail "another session's end removed the entry"
echo '{"session_id":"after-clear","reason":"prompt_input_exit"}' | bash "$HOOK" SessionEnd
[[ ! -f "$f" ]] || fail "real exit kept the entry"

# Outside a Clinch pane the hook does nothing.
env -u WARP_TERMINAL_SESSION_UUID bash "$HOOK" SessionStart <<<'{"session_id":"outside"}'
[[ ! -f "$WARP_AGENT_RESUME_DIR/outside.json" ]] || fail "acted outside a pane"

# Status events, through the same path a pane uses when /dev/tty can't be opened.
export WARP_CLI_AGENT_PROTOCOL_VERSION=1 WARP_CLIENT_VERSION=test
export WARP_TTY="$TMP/tty"
: > "$WARP_TTY"
emitted() {
  : > "$WARP_TTY"
  printf '%s' "$2" | /usr/bin/python3 -I -c '
import os,sys,subprocess
try:
    os.setsid()
except OSError:
    pass
sys.exit(subprocess.call(["bash", sys.argv[1], sys.argv[2]]))' "$HOOK" "$1"
  cat "$WARP_TTY"
}
payload="$(emitted SessionStart '{"session_id":"s1"}')"
[[ "$payload" == $'\033]777;notify;warp://cli-agent;'*'"event":"session_start"'* ]] || fail "session_start: $payload"
[[ "$payload" == *'"agent":"devin"'* && "$payload" == *'"session_id":"s1"'* ]] || fail "identity: $payload"
payload="$(emitted UserPromptSubmit '{"session_id":"s1","prompt":"build it"}')"
[[ "$payload" == *'"event":"prompt_submit"'*'"query":"build it"'* ]] || fail "prompt_submit: $payload"
payload="$(emitted Stop '{"session_id":"s1","last_assistant_message":"finished"}')"
[[ "$payload" == *'"event":"stop"'*'"response":"finished"'* ]] || fail "stop: $payload"
payload="$(emitted PermissionRequest '{"session_id":"s1","tool_name":"exec","tool_input":{"command":"rm x"}}')"
[[ "$payload" == *'"event":"permission_request"'*'"tool_name":"exec"'*'"summary":"Wants to run exec: rm x"'* ]] || fail "permission: $payload"
payload="$(emitted PostToolUse '{"session_id":"s1","tool_name":"exec"}')"
[[ "$payload" == *'"event":"tool_complete"'* ]] || fail "tool_complete: $payload"
# `/compact` fires no prompt hooks, so its end is reported the way Claude Code reports one.
before="$(cat "$f")"
payload="$(emitted PostCompaction '{"hook_event_name":"PostCompaction","summary":"s","session_id":"s1","prompt_id":"p1"}')"
[[ "$payload" == *'"event":"session_start"'*'"session_id":"s1"'* ]] || fail "compaction: $payload"
[[ "$(cat "$f")" == "$before" ]] || fail "compaction changed the restore entry"
# Long or hostile text can't overrun the event or end its escape sequence early.
long="$(printf 'x%.0s' {1..3000})"
payload="$(emitted UserPromptSubmit "{\"session_id\":\"s1\",\"prompt\":\"$long\"}")"
(( ${#payload} < 600 )) || fail "long prompt not bounded: ${#payload}"
payload="$(emitted UserPromptSubmit '{"session_id":"s1","prompt":"a\u009cb\u0007c\u001bd"}')"
stripped="${payload#$'\033]777;notify;warp://cli-agent;'}"
stripped="${stripped%$'\007'}"
[[ "$stripped" != *$'\033'* && "$stripped" != *$'\007'* && "$stripped" != *$'\xc2\x9c'* ]] || fail "control characters leaked"
unset WARP_TTY

# A coordinator gets its role through SessionStart, and only then.
out="$(echo '{"session_id":"c1"}' | CLINCH_COORDINATOR_ROLE="$(printf '%s' 'be the "coordinator"' | /usr/bin/base64)" WARP_CLI_AGENT_PROTOCOL_VERSION='' bash "$HOOK" SessionStart)"
[[ "$out" == '{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"be the \"coordinator\""}}' ]] || fail "role context: $out"
out="$(echo '{"session_id":"c2"}' | WARP_CLI_AGENT_PROTOCOL_VERSION='' bash "$HOOK" SessionStart)"
[[ -z "$out" ]] || fail "role context without a role: $out"

# --- Restore launcher, with a stand-in `devin` that records how it was called.
STUB="$TMP/stub"
mkdir -p "$STUB"
cat > "$STUB/devin" <<'EOF'
#!/bin/sh
printf '%s\n' "$*" >> "$DEVIN_CALLS"
[ -z "${DEVIN_STUB_REFUSE:-}" ] || case " $* " in *" -r "*) exit 1 ;; esac
printf 'fresh=%s\n' "${WARP_AGENT_RESUME_STARTED_FRESH:-}" >> "$DEVIN_CALLS"
EOF
chmod +x "$STUB/devin"
export DEVIN_CALLS="$TMP/calls"
launch() { : > "$DEVIN_CALLS"; PATH="$STUB:/usr/bin:/bin" /bin/zsh "$HERE/clinch_agent_resume_launch" "$@" 2>"$TMP/err"; }
mkdir -p "$WARP_AGENT_RESUME_DIR/prompts/devin"
printf '{"ts":"t","prompt":"p"}\n' > "$WARP_AGENT_RESUME_DIR/prompts/devin/real-1.jsonl"

launch devin real-1 --permission-mode smart --model opus
grep -q '^-r real-1 --permission-mode smart --model opus$' "$DEVIN_CALLS" || fail "resume args: $(cat "$DEVIN_CALLS")"
grep -q '^fresh=$' "$DEVIN_CALLS" || fail "resume marked fresh"

# A session that never took a prompt starts fresh with its flags.
launch devin never-used --model opus
grep -q '^--model opus$' "$DEVIN_CALLS" || fail "fresh args: $(cat "$DEVIN_CALLS")"
grep -q '^fresh=1$' "$DEVIN_CALLS" || fail "fresh not marked"

# Devin refusing a known session (deleted with /rm-session) falls through to a fresh start.
DEVIN_STUB_REFUSE=1 launch devin real-1
grep -q '^-r real-1$' "$DEVIN_CALLS" || fail "refused resume not tried"
grep -q '^fresh=1$' "$DEVIN_CALLS" || fail "refused resume did not start fresh"
grep -q 'could not reopen session real-1' "$TMP/err" || fail "refusal not explained"
# An explicit "this conversation or nothing" restore never substitutes a new one.
: > "$DEVIN_CALLS"
if DEVIN_STUB_REFUSE=1 CLINCH_AGENT_RESTORE_EXACT=1 PATH="$STUB:/usr/bin:/bin" /bin/zsh "$HERE/clinch_agent_resume_launch" devin real-1 2>/dev/null; then
  fail "exact restore reported success"
fi
if grep -q '^fresh=1$' "$DEVIN_CALLS"; then fail "exact restore started fresh"; fi

# Registry helpers accept Devin's entries.
export WARP_TERMINAL_SESSION_UUID="dd55"
"$HERE/clinch-agent-resume" write dd55 "clinch_agent_resume_launch devin real-1" "/tmp/x"
"$HERE/clinch-agent-resume" remove-if-matches dd55 devin real-1
[[ ! -f "$WARP_AGENT_RESUME_DIR/dd55.json" ]] || fail "remove-if-matches rejected devin"
"$HERE/clinch-agent-resume" write dd55 "clinch_agent_resume_launch devin real-1" "/tmp/x"
"$HERE/clinch-agent-resume" list --json | grep -q '"session_id": "real-1"' || fail "list omits devin"

# --- Config wiring.
export HOME="$TMP/home"
export CLINCH_AGENT_STATE_DIR="$TMP/state" CLINCH_AGENT_BIN_DIR="$TMP/bin"
mkdir -p "$HOME"
CFG="$HOME/.config/devin/config.json"
bash "$HERE/install.sh" enable --quiet
for event in SessionStart UserPromptSubmit PermissionRequest PostToolUse PostCompaction Stop SessionEnd; do
  grep -q "devin-hook.sh' $event\"" "$CFG" || fail "$event not wired"
done
first="$(shasum "$CFG")"
bash "$HERE/install.sh" enable --quiet
[[ "$first" == "$(shasum "$CFG")" ]] || fail "second enable changed the config"
[[ "$(grep -c 'devin-hook.sh' "$CFG")" -eq 7 ]] || fail "hooks duplicated"
[[ -x "$CLINCH_AGENT_BIN_DIR/devin-hook.sh" ]] || fail "runtime missing"
grep -q '^devin_config=' "$CLINCH_AGENT_STATE_DIR/receipt" || fail "receipt omits Devin"

# Someone's own settings and hooks survive enabling and disabling.
cat > "$CFG" <<'EOF'
{"agent":{"model":"opus"},"hooks":{"Stop":[{"hooks":[{"type":"command","command":"./mine.sh"}]}]}}
EOF
bash "$HERE/install.sh" repair --quiet
grep -q '"model": "opus"' "$CFG" || fail "setting lost on repair"
grep -q 'mine.sh' "$CFG" || fail "user hook lost on repair"
bash "$HERE/install.sh" disable --quiet
grep -q '"model": "opus"' "$CFG" || fail "setting lost on disable"
grep -q 'mine.sh' "$CFG" || fail "user hook lost on disable"
if grep -q 'devin-hook.sh' "$CFG"; then fail "disable kept the hook"; fi

# A file only Clinch had anything in is removed entirely.
bash "$HERE/install.sh" enable --quiet
rm "$CFG"
bash "$HERE/install.sh" repair --quiet
bash "$HERE/install.sh" disable --quiet
[[ ! -e "$CFG" ]] || fail "disable left an empty config"

# A config with comments is left alone, and Claude and Codex are still set up.
mkdir -p "$(dirname "$CFG")"
printf '// mine\n{"agent":{}}\n' > "$CFG"
sum="$(shasum "$CFG")"
rm -f "$HOME/.claude/settings.json" "$HOME/.codex/config.toml"
err="$(bash "$HERE/install.sh" enable --quiet 2>&1)" || fail "commented config broke enable: $err"
[[ "$err" == *"isn't plain JSON"* ]] || fail "no warning: $err"
[[ "$sum" == "$(shasum "$CFG")" ]] || fail "commented config rewritten"
grep -q 'claude-capture.sh' "$HOME/.claude/settings.json" || fail "Claude not set up"
grep -q 'codex-session-start.sh' "$HOME/.codex/config.toml" || fail "Codex not set up"
bash "$HERE/install.sh" disable --quiet || fail "commented config broke disable"
[[ "$sum" == "$(shasum "$CFG")" ]] || fail "disable rewrote the commented config"

echo "PASS"
