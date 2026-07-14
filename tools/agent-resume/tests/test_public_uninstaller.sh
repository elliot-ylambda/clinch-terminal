#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
export HOME="$TMP/home"
mkdir -p "$TMP/bin"
cat > "$TMP/bin/not-running" <<'EOF'
#!/bin/sh
exit 0
EOF
chmod +x "$TMP/bin/not-running"
export CLINCH_LSAPPINFO_BIN="$TMP/bin/not-running"
export CLINCH_PS_BIN="$TMP/bin/not-running"

APP="$HOME/Applications/Clinch.app"

make_app() {
  local app="$1" bundle_id="${2:-sh.clinch.Clinch}"
  mkdir -p "$app/Contents/Resources"
  plutil -create xml1 "$app/Contents/Info.plist"
  plutil -insert CFBundleIdentifier -string "$bundle_id" "$app/Contents/Info.plist"
  cp -R "$ROOT/tools/agent-resume" "$app/Contents/Resources/agent-resume"
  chmod +x "$app/Contents/Resources/agent-resume/install.sh"
}

mkdir -p \
  "$HOME/Library/Application Support/sh.clinch.Clinch" \
  "$HOME/Library/Caches/sh.clinch.Clinch" \
  "$HOME/Library/Logs" \
  "$HOME/Library/Saved Application State/sh.clinch.Clinch.savedState" \
  "$HOME/.warp/agent-resume" \
  "$HOME/.claude/projects/keep" \
  "$HOME/Library/Keychains"
printf 'state\n' > "$HOME/Library/Application Support/sh.clinch.Clinch/state"
printf 'cache\n' > "$HOME/Library/Caches/sh.clinch.Clinch/cache"
printf 'log\n' > "$HOME/Library/Logs/clinch.log.old.0"
printf 'window\n' > "$HOME/Library/Saved Application State/sh.clinch.Clinch.savedState/windows.plist"
printf 'capture\n' > "$HOME/.warp/agent-resume/keep"
printf 'transcript\n' > "$HOME/.claude/projects/keep/session.jsonl"
printf 'keychain\n' > "$HOME/Library/Keychains/keep.keychain-db"

# Default removal deletes only the exact verified app bundle.
make_app "$APP"
bash "$ROOT/uninstall.sh" --app "$APP" >/dev/null
[[ ! -e "$APP" ]] || { echo "FAIL: default uninstall retained the app"; exit 1; }
[[ -f "$HOME/Library/Application Support/sh.clinch.Clinch/state" ]] \
  || { echo "FAIL: default uninstall deleted app state"; exit 1; }
[[ -f "$HOME/.warp/agent-resume/keep" ]] \
  || { echo "FAIL: default uninstall deleted capture data"; exit 1; }

# Purging app state disables managed hooks first, but keeps capture data and provider-owned data.
make_app "$APP"
HOME="$HOME" bash "$APP/Contents/Resources/agent-resume/install.sh" enable --quiet
bash "$ROOT/uninstall.sh" --app "$APP" --keep-app --purge-app-data >/dev/null
[[ -d "$APP" ]] || { echo "FAIL: --keep-app removed the app"; exit 1; }
[[ ! -e "$HOME/Library/Application Support/sh.clinch.Clinch" ]] \
  || { echo "FAIL: --purge-app-data retained app state"; exit 1; }
[[ ! -e "$HOME/Library/Logs/clinch.log.old.0" \
  && ! -e "$HOME/Library/Saved Application State/sh.clinch.Clinch.savedState" ]] \
  || { echo "FAIL: --purge-app-data retained Clinch logs or saved state"; exit 1; }
! grep -q 'agent-resume-bin/claude-capture.sh' "$HOME/.claude/settings.json" \
  || { echo "FAIL: app-data purge left Claude integration enabled"; exit 1; }
! grep -q '^# >>> clinch agent-resume >>>$' "$HOME/.codex/config.toml" \
  || { echo "FAIL: app-data purge left Codex integration enabled"; exit 1; }
[[ -f "$HOME/.warp/agent-resume/keep" ]] \
  || { echo "FAIL: app-data purge deleted capture data"; exit 1; }
[[ -f "$HOME/.claude/projects/keep/session.jsonl" ]] \
  || { echo "FAIL: app-data purge deleted a provider transcript"; exit 1; }
[[ -f "$HOME/Library/Keychains/keep.keychain-db" ]] \
  || { echo "FAIL: app-data purge deleted Keychain data"; exit 1; }

# Capture deletion is a separate explicit operation.
HOME="$HOME" bash "$APP/Contents/Resources/agent-resume/install.sh" enable --quiet
bash "$ROOT/uninstall.sh" --app "$APP" --keep-app --purge-capture >/dev/null
[[ ! -e "$HOME/.warp/agent-resume" ]] \
  || { echo "FAIL: --purge-capture retained captured metadata"; exit 1; }
[[ -f "$HOME/.claude/projects/keep/session.jsonl" ]] \
  || { echo "FAIL: capture purge deleted a provider transcript"; exit 1; }

# A lookalike directory with another bundle identity must never be recursively removed.
LOOKALIKE="$TMP/lookalike/Clinch.app"
make_app "$LOOKALIKE" com.example.NotClinch
if bash "$ROOT/uninstall.sh" --app "$LOOKALIKE" >/dev/null 2>&1; then
  echo "FAIL: uninstaller accepted another bundle identifier"
  exit 1
fi
[[ -d "$LOOKALIKE" ]] || { echo "FAIL: invalid app bundle was removed"; exit 1; }

echo "PASS"
