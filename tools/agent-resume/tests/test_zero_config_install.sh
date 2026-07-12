#!/usr/bin/env bash
# Proves a clean macOS account can install and replay without jq, a repository clone,
# a ~/.zshrc edit, or a shell restart.
set -euo pipefail

HERE="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
export HOME="$TMP/home"
mkdir -p "$HOME/.claude" "$HOME/.codex" "$TMP/bin"

# Any accidental jq use is a hard failure even if the developer has jq installed.
cat > "$TMP/bin/jq" <<'EOF'
#!/bin/sh
echo "jq must not be used by the runtime installer" >&2
exit 99
EOF
chmod +x "$TMP/bin/jq"
export PATH="$TMP/bin:/usr/bin:/bin"

# Seed unrelated user configuration that the managed merges must preserve.
printf '%s\n' '{"model":"opus","hooks":{"PostToolUse":[{"hooks":[{"type":"command","command":"/keep/me.sh"}]}]}}' \
  > "$HOME/.claude/settings.json"
printf '%s\n' 'model = "gpt-5"' > "$HOME/.codex/config.toml"

bash "$HERE/install.sh" --quiet
before="$(shasum "$HOME/.claude/settings.json" "$HOME/.codex/config.toml")"
bash "$HERE/install.sh" --quiet # idempotence
after="$(shasum "$HOME/.claude/settings.json" "$HOME/.codex/config.toml")"
[[ "$before" == "$after" ]] || { echo "FAIL: rerun changed managed config"; exit 1; }

BIN="$HOME/.warp/agent-resume-bin"
for file in agent-json agent-json.js clinch-agent-resume clinch_agent_resume_launch \
  claude-capture.sh claude.zsh codex-session-start.sh codex-session-end.sh; do
  [[ -f "$BIN/$file" ]] || { echo "FAIL: installer omitted $file"; exit 1; }
done

[[ ! -e "$HOME/.zshrc" ]] || { echo "FAIL: clean install edited ~/.zshrc"; exit 1; }
grep -q '"model": "opus"' "$HOME/.claude/settings.json" \
  || { echo "FAIL: unrelated Claude setting was lost"; exit 1; }
grep -q '"command": "/keep/me.sh"' "$HOME/.claude/settings.json" \
  || { echo "FAIL: unrelated Claude hook was lost"; exit 1; }
[[ "$(grep -c 'agent-resume-bin/claude-capture.sh' "$HOME/.claude/settings.json")" -eq 3 ]] \
  || { echo "FAIL: Claude managed hooks missing or duplicated"; exit 1; }
grep -q '^model = "gpt-5"$' "$HOME/.codex/config.toml" \
  || { echo "FAIL: unrelated Codex setting was lost"; exit 1; }
[[ "$(grep -c '^# >>> clinch agent-resume >>>$' "$HOME/.codex/config.toml")" -eq 1 ]] \
  || { echo "FAIL: Codex managed block duplicated"; exit 1; }

# The executable launcher sources its own runtime and resumes immediately; no rcfile source
# or new interactive shell is involved.
mkdir -p "$HOME/.claude/projects/test"
printf '%s\n' '{"type":"user","message":{}}' \
  > "$HOME/.claude/projects/test/session-ready.jsonl"
cat > "$TMP/bin/claude" <<EOF
#!/bin/sh
printf '%s\n' "\$*" > "$TMP/claude-args"
EOF
chmod +x "$TMP/bin/claude"
"$BIN/clinch_agent_resume_launch" claude session-ready
grep -q -- '--resume session-ready' "$TMP/claude-args" \
  || { echo "FAIL: standalone launcher did not resume the session"; exit 1; }

echo "PASS"
