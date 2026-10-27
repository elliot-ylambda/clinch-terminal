#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
export CODEX_HOME="$TMP/codex"
mkdir -p "$CODEX_HOME/sessions" "$TMP/bin"
transcript="$CODEX_HOME/sessions/rollout-child-id.jsonl"
cat > "$transcript" <<'JSON'
{"type":"session_meta","payload":{"id":"child-id","forked_from_id":"parent-id"}}
{"type":"turn_context","payload":{"model":"old-model","effort":"low","approval_policy":"never","sandbox_policy":{"type":"danger-full-access"}}}
{"type":"event_msg","payload":{"type":"thread_settings_applied","thread_id":"parent-id","thread_settings":{"model":"wrong-parent-model","approval_policy":"never","permission_profile":{"type":"disabled"}}}}
JSON
flags="$($HERE/agent-json codex-resume-flags "$transcript" child-id)"
[[ "$flags" == *$'--model\nold-model'* && "$flags" != *wrong-parent-model* ]] || { echo 'FAIL: fork inherited a foreign settings event'; exit 1; }
cat >> "$transcript" <<'JSON'
{"type":"event_msg","payload":{"type":"thread_settings_applied","thread_id":"child-id","thread_settings":{"model":"gpt-6-astra","reasoning_effort":"xhigh","approval_policy":"on-request","approvals_reviewer":"auto_review","active_permission_profile":{"id":":workspace"},"cwd":"/tmp/repo","runtime_workspace_roots":["/tmp/repo"],"permission_profile":{"type":"managed","network":"restricted","file_system":{"type":"restricted","entries":[{"path":{"type":"special","value":{"kind":"root"}},"access":"read"},{"path":{"type":"special","value":{"kind":"slash_tmp"}},"access":"write"},{"path":{"type":"special","value":{"kind":"tmpdir"}},"access":"write"},{"path":{"type":"path","path":"/tmp/repo"},"access":"write"}]}}}}}
JSON
flags="$($HERE/agent-json codex-resume-flags "$transcript" child-id)"
[[ "$flags" == $'--approve-for-me\n--model\ngpt-6-astra\n-c\nmodel_reasoning_effort=xhigh' ]] || { echo "FAIL: latest permission/model selection lost: $flags"; exit 1; }

# Real launcher replaces stale overrides for resume AND fork, retaining argument boundaries.
cat > "$TMP/bin/codex" <<'MOCK'
#!/bin/sh
if [ "${1:-}" = --help ]; then echo "--no-daemon --approve-for-me"; exit 0; fi
printf '%s\n' "$@" > "$CLINCH_TEST_ARGV"
MOCK
chmod +x "$TMP/bin/codex"
export PATH="$TMP/bin:$PATH" CLINCH_TEST_ARGV="$TMP/argv"
for operation in resume fork; do
  "$HERE/clinch_codex_launch" "$operation" child-id --model stale \
    --dangerously-bypass-approvals-and-sandbox -c model_reasoning_effort=low \
    --config 'untouched=two words' -- 'literal --model prompt'
  settings_args=""
  if [[ "$operation" == fork ]]; then settings_args=$'--approve-for-me\n--model\ngpt-6-astra\n-c\nmodel_reasoning_effort=xhigh\n'; fi
  [[ "$(cat "$TMP/argv")" == "--no-daemon
$operation
child-id
--config
untouched=two words
${settings_args}--
literal --model prompt" ]] || { echo "FAIL: $operation launch did not preserve exact settings/args"; cat "$TMP/argv"; exit 1; }
done

# Never-ask does NOT imply full access. Keep a read-only sandbox and human reviewer.
cat >> "$transcript" <<'JSON'
{"type":"turn_context","payload":{"model":"model-b","approval_policy":"never","approvals_reviewer":"user","sandbox_policy":{"type":"read-only"}}}
JSON
flags="$($HERE/agent-json codex-resume-flags "$transcript" child-id)"
[[ "$flags" == *$'--sandbox\nread-only\n--ask-for-approval\nnever'* && "$flags" != *danger* ]] || { echo 'FAIL: read-only policy widened'; exit 1; }
# Partial trailing writes are ignored, preserving the latest complete settings record.
printf '{"type":' >> "$transcript"
[[ "$($HERE/agent-json codex-resume-flags "$transcript" child-id)" == "$flags" ]] || { echo 'FAIL: partial tail lost settings'; exit 1; }
if "$HERE/agent-json" codex-resume-flags "$transcript" different-id >/dev/null 2>&1; then echo 'FAIL: wrong session accepted'; exit 1; fi
printf '\n' >> "$transcript"
cat >> "$transcript" <<'JSON'
{"type":"turn_context","payload":{"model":"model-b","approval_policy":"on-request","permission_profile":{"type":"managed"},"active_permission_profile":{"id":"custom-production"}}}
JSON
if "$HERE/clinch_codex_launch" fork child-id >/dev/null 2>&1; then echo 'FAIL: custom profile silently changed'; exit 1; fi

# Native resume preserves the entire custom profile by omitting direct policy overrides.
"$HERE/clinch_codex_launch" resume child-id --model stale --sandbox danger-full-access
[[ "$(cat "$TMP/argv")" == $'--no-daemon\nresume\nchild-id' ]] || { echo 'FAIL: native custom profile overridden'; exit 1; }
# Unsupported fork settings must never prevent capture of the actual new child UUID.
export WARP_AGENT_RESUME_DIR="$TMP/registry" WARP_TERMINAL_SESSION_UUID=aa11
export WARP_AGENT_RESUME_FAKE_ANCESTRY=codex WARP_AGENT_RESUME_FAKE_OWNER_PID=1000
jq -cn --arg path "$transcript" '{session_id:"child-id",cwd:"/tmp/repo",transcript_path:$path}' | bash "$HERE/codex-session-start.sh"
grep -q 'clinch_agent_resume_launch codex child-id' "$WARP_AGENT_RESUME_DIR/aa11.json" || { echo 'FAIL: unsupported profile lost child identity'; exit 1; }

# Startup restore never substitutes another conversation or creates a blank replacement.
if CLINCH_AGENT_RESTORE_EXACT=1 "$HERE/clinch_agent_resume_launch" codex missing-child >/dev/null 2>&1; then echo 'FAIL: missing child silently replaced'; exit 1; fi
# Hook shorthand is lossy and must never grant full access on its own.
flags="$(printf '%s\n' '{"session_id":"child-id","permission_mode":"bypassPermissions","model":"model-c"}' | "$HERE/agent-json" codex-hook-flags)"
[[ "$flags" == $'--model\nmodel-c' ]] || { echo 'FAIL: lossy hook mode widened access'; exit 1; }
echo PASS
