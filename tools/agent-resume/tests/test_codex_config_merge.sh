#!/usr/bin/env bash
# Codex writes its own tables at the end of config.toml, which lands them inside Clinch's managed
# markers because the block ends with a comment. Refreshing that block must keep them: dropping
# the `[hooks.state]` trust records makes Codex re-prompt "N hooks are new or changed" on every
# launch, and dropping `[plugins.*]` silently disables the bundled notification plugin.
set -euo pipefail

HERE="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
export HOME="$TMP/home"
unset WARP_AGENT_RESUME_DIR CLINCH_AGENT_BIN_DIR CLINCH_AGENT_STATE_DIR \
  CLINCH_CLAUDE_SETTINGS CLINCH_CODEX_CONFIG
mkdir -p "$HOME/.claude" "$HOME/.codex"
printf '%s\n' '{}' > "$HOME/.claude/settings.json"
CFG="$HOME/.codex/config.toml"

# Everything Codex appended sits between the markers, exactly as a real config accumulates it.
seed_config() {
  local open="$1" close="$2"
  cat > "$CFG" <<EOF
model = "gpt-5.6-sol"

[projects."/work"]
trust_level = "trusted"

$open
[[hooks.SessionStart]]
matcher = "startup|resume"
[[hooks.SessionStart.hooks]]
type = "command"
command = "/stale/agent-resume-bin/codex-session-start.sh"

[[hooks.SessionEnd]]
[[hooks.SessionEnd.hooks]]
type = "command"
command = "/stale/agent-resume-bin/codex-session-end.sh"

[hooks.state]

[hooks.state."$HOME/.codex/config.toml:session_start:0:0"]
trusted_hash = "sha256:aaa"

[hooks.state."warp@clinch-codex-warp:hooks/hooks.json:stop:0:0"]
trusted_hash = "sha256:bbb"

[plugins."warp@clinch-codex-warp"]
enabled = true
$close
EOF
}

managed_region() {
  /usr/bin/awk '
    /^# >>> (clinch|warp) agent-resume >>>$/ { managed = 1; next }
    /^# <<< (clinch|warp) agent-resume <<<$/ { managed = 0; next }
    managed
  ' "$CFG"
}

# Grows once the test proves a hook trusted after the rescue is also carried through a refresh.
EXPECTED_HASHES="sha256:aaa sha256:bbb"

assert_state_survived() {
  local context="$1" hash
  for hash in $EXPECTED_HASHES; do
    grep -q "^trusted_hash = \"$hash\"\$" "$CFG" \
      || { echo "FAIL: $context dropped the $hash hook-trust record"; exit 1; }
  done
  grep -q '^\[plugins\."warp@clinch-codex-warp"\]$' "$CFG" \
    || { echo "FAIL: $context dropped the plugin enablement"; exit 1; }
  grep -q '^enabled = true$' "$CFG" \
    || { echo "FAIL: $context dropped the plugin enabled value"; exit 1; }
  grep -q '^model = "gpt-5.6-sol"$' "$CFG" \
    || { echo "FAIL: $context lost an unrelated Codex setting"; exit 1; }
  grep -q '^trust_level = "trusted"$' "$CFG" \
    || { echo "FAIL: $context lost an unrelated Codex table"; exit 1; }
  [[ "$(grep -c 'trusted_hash' "$CFG")" -eq "$(wc -w <<< "$EXPECTED_HASHES")" ]] \
    || { echo "FAIL: $context duplicated hook-trust records"; exit 1; }
  [[ "$(grep -c '^\[plugins\."warp@clinch-codex-warp"\]$' "$CFG")" -eq 1 ]] \
    || { echo "FAIL: $context duplicated the plugin table"; exit 1; }
}

# Rescued state must live outside the markers so the next refresh has nothing left to lose.
assert_state_outside_block() {
  local context="$1" region
  region="$(managed_region)"
  [[ "$region" != *trusted_hash* ]] \
    || { echo "FAIL: $context left hook-trust records inside the managed block"; exit 1; }
  [[ "$region" != *plugins* ]] \
    || { echo "FAIL: $context left plugin state inside the managed block"; exit 1; }
}

# TOML validity is the point of re-homing whole tables, so parse the result where a parser exists.
toml_python() {
  local candidate
  for candidate in python3 /opt/homebrew/bin/python3 /usr/local/bin/python3; do
    command -v "$candidate" >/dev/null 2>&1 || continue
    "$candidate" -c 'import tomllib' 2>/dev/null && { printf '%s\n' "$candidate"; return 0; }
  done
  return 1
}

assert_parses() {
  local context="$1" py
  py="$(toml_python)" || return 0
  "$py" - "$CFG" "$context" "$HOME/.warp/agent-resume-bin" "$EXPECTED_HASHES" <<'PY' || exit 1
import sys, tomllib

path, context, bin_dir, expected = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4].split()
with open(path, "rb") as handle:
    config = tomllib.load(handle)


def fail(message):
    print(f"FAIL: {context} {message}")
    sys.exit(1)


hashes = {
    entry["trusted_hash"]
    for entry in config.get("hooks", {}).get("state", {}).values()
    if isinstance(entry, dict)
}
if hashes != set(expected):
    fail(f"did not preserve every hook-trust record: {sorted(hashes)}")
if not config.get("plugins", {}).get("warp@clinch-codex-warp", {}).get("enabled"):
    fail("did not preserve the plugin enablement")
if config.get("model") != "gpt-5.6-sol":
    fail("did not preserve unrelated settings")

hooks = config.get("hooks", {})
if "# >>> clinch agent-resume >>>" in open(path, encoding="utf-8").read():
    for event in ("SessionStart", "UserPromptSubmit", "SessionEnd"):
        entries = hooks.get(event, [])
        if len(entries) != 1:
            fail(f"declared {len(entries)} {event} hooks")
        command = entries[0]["hooks"][0]["command"]
        if not command.startswith(bin_dir):
            fail(f"kept a stale {event} command: {command}")
elif any(event in hooks for event in ("SessionStart", "UserPromptSubmit", "SessionEnd")):
    fail("left managed hooks behind after disable")
PY
}

for markers in "clinch" "warp"; do
  seed_config "# >>> $markers agent-resume >>>" "# <<< $markers agent-resume <<<"
  EXPECTED_HASHES="sha256:aaa sha256:bbb"

  bash "$HERE/install.sh" enable --quiet
  assert_state_survived "enable ($markers markers)"
  assert_state_outside_block "enable ($markers markers)"
  assert_parses "enable ($markers markers)"
  [[ "$(grep -c '^# >>> clinch agent-resume >>>$' "$CFG")" -eq 1 ]] \
    || { echo "FAIL: enable ($markers markers) duplicated the managed block"; exit 1; }
  ! grep -q '/stale/agent-resume-bin' "$CFG" \
    || { echo "FAIL: enable ($markers markers) kept a stale managed hook command"; exit 1; }
  [[ "$(grep -c 'agent-resume-bin/codex-session-start.sh' "$CFG")" -eq 1 ]] \
    || { echo "FAIL: enable ($markers markers) did not rewrite the managed hooks"; exit 1; }

  # The refresh that runs on every launch must now be a no-op, not another round of data loss.
  before="$(shasum "$CFG")"
  bash "$HERE/install.sh" repair --quiet
  [[ "$before" == "$(shasum "$CFG")" ]] \
    || { echo "FAIL: repair after rescue ($markers markers) rewrote the config"; exit 1; }
  assert_state_survived "repair ($markers markers)"

  # Codex records a newly trusted hook at the end of the document, back inside the markers. The
  # next refresh has to rescue that one too.
  printf '\n[hooks.state."%s"]\ntrusted_hash = "sha256:ccc"\n' \
    "warp@clinch-codex-warp:hooks/hooks.json:post_tool_use:0:0" >> "$CFG"
  bash "$HERE/install.sh" repair --quiet
  EXPECTED_HASHES="$EXPECTED_HASHES sha256:ccc"
  grep -q '^trusted_hash = "sha256:ccc"$' "$CFG" \
    || { echo "FAIL: repair ($markers markers) dropped a newly trusted hook"; exit 1; }
  assert_state_outside_block "repair after a new trust decision ($markers markers)"

  bash "$HERE/install.sh" disable --quiet
  assert_state_survived "disable ($markers markers)"
  assert_parses "disable ($markers markers)"
  ! grep -q 'agent-resume >>>' "$CFG" \
    || { echo "FAIL: disable ($markers markers) kept the managed block"; exit 1; }
  ! grep -q 'agent-resume-bin/codex-session-start.sh' "$CFG" \
    || { echo "FAIL: disable ($markers markers) kept a managed hook"; exit 1; }
  rm -f "$HOME/Library/Application Support/sh.clinch.Clinch/agent-integration/disabled"
done

# A config Codex has never written to still round-trips to the same bytes.
seed_config "# >>> clinch agent-resume >>>" "# <<< clinch agent-resume <<<"
/usr/bin/sed -e '/^\[hooks\.state/,$d' -e '/^\[plugins/,$d' "$CFG" > "$CFG.trimmed"
printf '# <<< clinch agent-resume <<<\n' >> "$CFG.trimmed"
mv "$CFG.trimmed" "$CFG"
bash "$HERE/install.sh" enable --quiet
before="$(shasum "$CFG")"
bash "$HERE/install.sh" repair --quiet
[[ "$before" == "$(shasum "$CFG")" ]] \
  || { echo "FAIL: repair rewrote a config with no third-party block content"; exit 1; }

# Foreign content is never left behind in the provider directory as a stray staging file.
[[ -z "$(find "$HOME/.codex" -name '*.foreign' -print)" ]] \
  || { echo "FAIL: installer left a staging file in the Codex directory"; exit 1; }

echo "PASS"
