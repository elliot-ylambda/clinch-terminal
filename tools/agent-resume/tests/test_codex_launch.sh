#!/usr/bin/env zsh
# Clinch-owned Codex replay must preserve pane identity without changing older CLIs.
set -euo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/bin" "$TMP/home/.codex/sessions"
export CLINCH_CODEX_LAUNCH_TEST_DIR="$TMP"
cat > "$TMP/bin/codex" <<'EOF'
#!/bin/sh
if [ "${1:-}" = --help ]; then
  case "$(cat "$CLINCH_CODEX_LAUNCH_TEST_DIR/capability")" in
    modern) printf '%s\n' '  --no-daemon  Run without the shared background server' ;;
    old) printf '%s\n' 'Usage: codex [OPTIONS]' ;;
    failed) printf '%s\n' '--no-daemon' ; exit 1 ;;
  esac
  exit 0
fi
printf '%s\n' "$@" > "$CLINCH_CODEX_LAUNCH_TEST_DIR/argv"
printf '%s|%s\n' "$WARP_TERMINAL_SESSION_UUID" "$WARP_TTY" \
  > "$CLINCH_CODEX_LAUNCH_TEST_DIR/pane"
exit "${CLINCH_CODEX_LAUNCH_TEST_EXIT:-0}"
EOF
chmod +x "$TMP/bin/codex"
export PATH="$TMP/bin:$PATH"
export WARP_TERMINAL_SESSION_UUID=aa11 WARP_TTY=/dev/ttys-test
source "$HERE/claude.zsh"
printf '%s\n' '{"role":"user","content":"existing conversation"}' \
  > "$TMP/home/.codex/sessions/rollout-good-session.jsonl"
printf '%s\n' modern > "$TMP/capability"

# Existing conversations keep the exact ID, launch flags, and argument boundaries.
HOME="$TMP/home" clinch_agent_resume_launch codex good-session \
  --dangerously-bypass-approvals-and-sandbox --model 'two words' 'literal*'
[[ "$(< "$TMP/argv")" == $'--no-daemon\nresume\ngood-session\n--dangerously-bypass-approvals-and-sandbox\n--model\ntwo words\nliteral*' ]] \
  || { echo 'FAIL: modern resume arguments changed'; exit 1; }
[[ "$(< "$TMP/pane")" == 'aa11|/dev/ttys-test' ]] \
  || { echo 'FAIL: launch changed pane identity'; exit 1; }

# Missing conversations use the same pane-local launch for the fresh fallback. The standalone
# executable sources the updated runtime itself, even in an already-running restored shell.
HOME="$TMP/home" "$HERE/clinch_agent_resume_launch" codex missing-session --model model-a
[[ "$(< "$TMP/argv")" == $'--no-daemon\n--model\nmodel-a' ]] \
  || { echo 'FAIL: fresh fallback attached to the shared daemon'; exit 1; }

# The shared entrypoint handles fresh tasks and forks without changing their semantics.
"$HERE/clinch_codex_launch" -- 'prompt with "quotes" and $literal'
[[ "$(< "$TMP/argv")" == $'--no-daemon\n--\nprompt with "quotes" and $literal' ]] \
  || { echo 'FAIL: fresh entrypoint changed prompt'; exit 1; }
"$HERE/clinch_codex_launch" fork good-session --model fork-model
[[ "$(< "$TMP/argv")" == $'--no-daemon\nfork\ngood-session\n--model\nfork-model' ]] \
  || { echo 'FAIL: fork entrypoint changed arguments'; exit 1; }
"$HERE/clinch_codex_launch" --no-daemon resume good-session
[[ "$(< "$TMP/argv")" == $'--no-daemon\nresume\ngood-session' ]] \
  || { echo 'FAIL: explicit no-daemon flag duplicated'; exit 1; }
"$HERE/clinch_codex_launch" -- --no-daemon
[[ "$(< "$TMP/argv")" == $'--no-daemon\n--\n--no-daemon' ]] \
  || { echo 'FAIL: prompt mistaken for no-daemon option'; exit 1; }

# Explicit connections and the shared-server overview keep their native semantics.
"$HERE/clinch_codex_launch" --remote unix:///tmp/codex.sock
[[ "$(< "$TMP/argv")" == $'--remote\nunix:///tmp/codex.sock' ]] \
  || { echo 'FAIL: remote connection forced into a pane process'; exit 1; }
"$HERE/clinch_codex_launch" --model model-a agents
[[ "$(< "$TMP/argv")" == $'--model\nmodel-a\nagents' ]] \
  || { echo 'FAIL: agents overview lost shared-server access'; exit 1; }
"$HERE/clinch_codex_launch" --model agents -- 'agents'
[[ "$(< "$TMP/argv")" == $'--no-daemon\n--model\nagents\n--\nagents' ]] \
  || { echo 'FAIL: model or prompt mistaken for agents subcommand'; exit 1; }

"$HERE/clinch_codex_launch" resume good-session --remote unix:///tmp/codex.sock
[[ "$(< "$TMP/argv")" == $'resume\ngood-session\n--remote\nunix:///tmp/codex.sock' ]] \
  || { echo 'FAIL: remote resume forced into a pane process'; exit 1; }
"$HERE/clinch_codex_launch" fork good-session --remote=unix:///tmp/codex.sock
[[ "$(< "$TMP/argv")" == $'fork\ngood-session\n--remote=unix:///tmp/codex.sock' ]] \
  || { echo 'FAIL: remote fork forced into a pane process'; exit 1; }
"$HERE/clinch_codex_launch" resume good-session agents
[[ "$(< "$TMP/argv")" == $'--no-daemon\nresume\ngood-session\nagents' ]] \
  || { echo 'FAIL: resume prompt mistaken for agents subcommand'; exit 1; }
"$HERE/clinch_codex_launch" fork good-session -- --remote
[[ "$(< "$TMP/argv")" == $'--no-daemon\nfork\ngood-session\n--\n--remote' ]] \
  || { echo 'FAIL: fork prompt mistaken for remote option'; exit 1; }

for capability in old failed; do
  printf '%s\n' "$capability" > "$TMP/capability"
  HOME="$TMP/home" clinch_agent_resume_launch codex good-session --model old-model
  [[ "$(< "$TMP/argv")" == $'resume\ngood-session\n--model\nold-model' ]] \
    || { echo "FAIL: $capability capability probe changed legacy arguments"; exit 1; }
done

# Do not replace the user's command or swallow the provider's exit code.
[[ "$(whence -w codex)" == 'codex: command' ]] \
  || { echo 'FAIL: sourcing replay runtime replaced codex'; exit 1; }
printf '%s\n' modern > "$TMP/capability"
result=0
CLINCH_CODEX_LAUNCH_TEST_EXIT=23 HOME="$TMP/home" \
  clinch_agent_resume_launch codex good-session || result=$?
[[ "$result" == 23 ]] || { echo 'FAIL: provider exit status changed'; exit 1; }
echo PASS
