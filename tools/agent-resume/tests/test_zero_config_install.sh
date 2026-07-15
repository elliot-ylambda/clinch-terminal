#!/usr/bin/env bash
# Proves session capture is an explicit, reversible opt-in that needs no jq, rcfile edit,
# repository clone, or shell restart.
set -euo pipefail

HERE="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
export HOME="$TMP/home"
unset WARP_AGENT_RESUME_DIR CLINCH_AGENT_BIN_DIR CLINCH_AGENT_STATE_DIR \
  CLINCH_CLAUDE_SETTINGS CLINCH_CODEX_CONFIG
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

# No command prints help and must not create the consent/runtime/data directories.
before_noop="$(shasum "$HOME/.claude/settings.json" "$HOME/.codex/config.toml")"
bash "$HERE/install.sh" >/dev/null
after_noop="$(shasum "$HOME/.claude/settings.json" "$HOME/.codex/config.toml")"
[[ "$before_noop" == "$after_noop" ]] || { echo "FAIL: no-argument call changed config"; exit 1; }
[[ ! -e "$HOME/.warp" ]] || { echo "FAIL: no-argument call created ~/.warp"; exit 1; }
[[ ! -e "$HOME/Library/Application Support/sh.clinch.Clinch/agent-integration/enabled" ]] \
  || { echo "FAIL: no-argument call recorded consent"; exit 1; }

# A symlink is not a durable consent record and must not authorize startup repair.
STATE="$HOME/Library/Application Support/sh.clinch.Clinch/agent-integration"
mkdir -p "$STATE"
ln -s "$HOME/.claude/settings.json" "$STATE/enabled"
bash "$HERE/install.sh" repair --quiet
[[ "$(bash "$HERE/install.sh" status)" == "disabled" ]] \
  || { echo "FAIL: symlink marker was treated as consent"; exit 1; }
[[ ! -e "$HOME/.warp" ]] || { echo "FAIL: symlink marker installed runtime files"; exit 1; }
rm -f "$STATE/enabled"

bash "$HERE/install.sh" enable --quiet
before="$(shasum "$HOME/.claude/settings.json" "$HOME/.codex/config.toml")"
bash "$HERE/install.sh" repair --quiet # idempotence after explicit consent
after="$(shasum "$HOME/.claude/settings.json" "$HOME/.codex/config.toml")"
[[ "$before" == "$after" ]] || { echo "FAIL: rerun changed managed config"; exit 1; }

BIN="$HOME/.warp/agent-resume-bin"
for file in agent-json agent-json.js clinch-agent-resume clinch_agent_resume_launch \
  claude-capture.sh prompt-mirror.sh claude.zsh codex-session-start.sh \
  codex-prompt-submit.sh codex-session-end.sh; do
  [[ -f "$BIN/$file" ]] || { echo "FAIL: installer omitted $file"; exit 1; }
done
[[ ! -e "$BIN/install-agent-plugins.sh" ]] \
  || { echo "FAIL: integration bundled a floating plugin installer"; exit 1; }

[[ -f "$STATE/enabled" && -f "$STATE/receipt" ]] \
  || { echo "FAIL: explicit enable did not record consent and receipt"; exit 1; }
[[ "$(stat -f '%Lp' "$STATE/enabled")" == "600" ]] \
  || { echo "FAIL: consent marker permissions are not 600"; exit 1; }
[[ "$(stat -f '%Lp' "$STATE/receipt")" == "600" ]] \
  || { echo "FAIL: receipt permissions are not 600"; exit 1; }
grep -q '^owner=sh\.clinch\.Clinch$' "$STATE/receipt" \
  || { echo "FAIL: receipt omitted its owner"; exit 1; }
grep -q "^runtime_dir=$BIN$" "$STATE/receipt" \
  || { echo "FAIL: receipt omitted its runtime directory"; exit 1; }
grep -q "^capture_data_dir=$HOME/.warp/agent-resume$" "$STATE/receipt" \
  || { echo "FAIL: receipt omitted its capture data directory"; exit 1; }
grep -q "^claude_post_mode=$(stat -f '%Lp' "$HOME/.claude/settings.json")$" "$STATE/receipt" \
  || { echo "FAIL: receipt omitted the Claude post-change mode"; exit 1; }
grep -q "^codex_post_mode=$(stat -f '%Lp' "$HOME/.codex/config.toml")$" "$STATE/receipt" \
  || { echo "FAIL: receipt omitted the Codex post-change mode"; exit 1; }

[[ ! -e "$HOME/.zshrc" ]] || { echo "FAIL: clean install edited ~/.zshrc"; exit 1; }
grep -q '"model": "opus"' "$HOME/.claude/settings.json" \
  || { echo "FAIL: unrelated Claude setting was lost"; exit 1; }
grep -q '"command": "/keep/me.sh"' "$HOME/.claude/settings.json" \
  || { echo "FAIL: unrelated Claude hook was lost"; exit 1; }
[[ "$(grep -c 'agent-resume-bin/claude-capture.sh' "$HOME/.claude/settings.json")" -eq 4 ]] \
  || { echo "FAIL: Claude managed hooks missing or duplicated"; exit 1; }
grep -q '^model = "gpt-5"$' "$HOME/.codex/config.toml" \
  || { echo "FAIL: unrelated Codex setting was lost"; exit 1; }
[[ "$(grep -c '^# >>> clinch agent-resume >>>$' "$HOME/.codex/config.toml")" -eq 1 ]] \
  || { echo "FAIL: Codex managed block duplicated"; exit 1; }
[[ "$(grep -c 'codex-prompt-submit.sh' "$HOME/.codex/config.toml")" -eq 1 ]] \
  || { echo "FAIL: Codex prompt hook missing or duplicated"; exit 1; }

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

# Disable removes only managed hooks/runtime and retains user settings plus captured metadata.
printf '%s\n' '{"keep":true}' > "$HOME/.warp/agent-resume/keep.json"
bash "$HERE/install.sh" disable --quiet
[[ "$(bash "$HERE/install.sh" status)" == "disabled" ]] \
  || { echo "FAIL: disable left consent enabled"; exit 1; }
[[ ! -e "$STATE/enabled" ]] || { echo "FAIL: disable kept consent marker"; exit 1; }
[[ ! -e "$BIN/claude-capture.sh" ]] || { echo "FAIL: disable kept owned runtime"; exit 1; }
[[ ! -e "$BIN/codex-prompt-submit.sh" ]] || { echo "FAIL: disable kept Codex prompt helper"; exit 1; }
[[ -f "$HOME/.warp/agent-resume/keep.json" ]] \
  || { echo "FAIL: disable deleted captured metadata"; exit 1; }
grep -q '"model": "opus"' "$HOME/.claude/settings.json" \
  || { echo "FAIL: disable lost unrelated Claude setting"; exit 1; }
grep -q '"command": "/keep/me.sh"' "$HOME/.claude/settings.json" \
  || { echo "FAIL: disable lost unrelated Claude hook"; exit 1; }
! grep -q 'agent-resume-bin/claude-capture.sh' "$HOME/.claude/settings.json" \
  || { echo "FAIL: disable kept a Claude managed hook"; exit 1; }
grep -q '^model = "gpt-5"$' "$HOME/.codex/config.toml" \
  || { echo "FAIL: disable lost unrelated Codex setting"; exit 1; }
! grep -q '^# >>> clinch agent-resume >>>$' "$HOME/.codex/config.toml" \
  || { echo "FAIL: disable kept the Codex managed block"; exit 1; }

# Repair without consent is a no-op. Purge requires an explicit command and removes only capture data.
before_disabled="$(shasum "$HOME/.claude/settings.json" "$HOME/.codex/config.toml")"
bash "$HERE/install.sh" repair --quiet
after_disabled="$(shasum "$HOME/.claude/settings.json" "$HOME/.codex/config.toml")"
[[ "$before_disabled" == "$after_disabled" ]] \
  || { echo "FAIL: repair without consent changed config"; exit 1; }
bash "$HERE/install.sh" enable --quiet
printf '%s\n' '{"purge":true}' > "$HOME/.warp/agent-resume/purge.json"
purge_output="$(bash "$HERE/install.sh" purge)"
[[ "$purge_output" == *"Removing Clinch capture metadata from: $HOME/.warp/agent-resume"* ]] \
  || { echo "FAIL: purge did not identify the directory it removed"; exit 1; }
[[ ! -e "$HOME/.warp/agent-resume" ]] \
  || { echo "FAIL: explicit purge retained capture data"; exit 1; }

echo "PASS"
