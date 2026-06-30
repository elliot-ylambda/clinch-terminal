#!/usr/bin/env bash
# Tests the process-ancestry walk that finds the live `claude` argv in production (the part the
# other tests stub out via WARP_AGENT_RESUME_FAKE_ARGV). We fake `ps` to model a small process
# tree so the walk is deterministic and free of the real session's processes.
set -uo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
source "$HERE/claude-session-start.sh"
unset WARP_AGENT_RESUME_FAKE_ARGV 2>/dev/null || true

TMP="$(mktemp -d)"
export PSTREE_DIR="$TMP/tree"; mkdir -p "$PSTREE_DIR"
# Minimal fake `ps` supporting `ps -ww -o args= -p PID` and `ps -o ppid= -p PID`.
mkdir -p "$TMP/bin"
cat > "$TMP/bin/ps" <<'EOF'
#!/usr/bin/env bash
field=""; pid=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    -o) field="$2"; shift 2 ;;
    -p) pid="$2"; shift 2 ;;
    *)  shift ;;
  esac
done
case "$field" in
  args=) cat "$PSTREE_DIR/$pid.args" 2>/dev/null ;;
  ppid=) cat "$PSTREE_DIR/$pid.ppid" 2>/dev/null ;;
esac
EOF
chmod +x "$TMP/bin/ps"
export PATH="$TMP/bin:$PATH"

fail() { echo "FAIL: $1"; exit 1; }
node() { printf '%s' "$1" > "$PSTREE_DIR/$2.args"; printf '%s' "$3" > "$PSTREE_DIR/$2.ppid"; }

# Tree: hook(500) -> sh -c wrapper(400) -> claude(300) -> zsh(200) -> init(1).
# Only the real claude carries the launch flags; the wrapper paths must not be mistaken for it.
node "bash /opt/agent-resume/claude-session-start.sh" 500 400
node "/bin/sh -c /opt/agent-resume/claude-session-start.sh" 400 300
node "node /opt/claude-code/cli.js --dangerously-skip-permissions --model opus" 300 200
node "-zsh" 200 1

argv="$(_warp_agent_resume_claude_argv 500)"
[[ "$argv" == *"--dangerously-skip-permissions"* && "$argv" == *"--model opus"* ]] \
  || fail "walk did not find the flag-bearing claude ancestor (got: '$argv')"
flags="$(_warp_agent_resume_extract_flags "$argv")"
[[ "$flags" == *"--dangerously-skip-permissions"* && "$flags" == *"--model opus"* ]] \
  || fail "extracted flags wrong (got: '$flags')"

# A plain claude launch (no carry-over flags anywhere) yields empty -> resume stays plain.
node "node /opt/claude-code/cli.js" 300 200
argv="$(_warp_agent_resume_claude_argv 500)"
[[ -z "$argv" ]] || fail "expected empty walk for plain launch (got: '$argv')"

echo "PASS"
