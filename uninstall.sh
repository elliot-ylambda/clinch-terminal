#!/usr/bin/env bash
# Selective Clinch uninstaller. Provider transcripts, Keychain items, and unrelated ~/.warp data
# are never removed.
set -euo pipefail

usage() {
  cat <<'EOF'
usage: uninstall.sh [options]

  --app PATH              Remove this verified sh.clinch.Clinch app bundle.
  --disable-integration   Remove Clinch-managed Claude/Codex hooks and helpers.
  --purge-capture         Also delete Clinch's captured session metadata (implies disable).
  --purge-app-data        Remove Clinch preferences, caches, logs, and application state
                          (also disables managed integration hooks first).
  --remove-plugins        Ask Claude/Codex CLIs to remove the known Warp notification plugins.
  --keep-app              Do not remove an app bundle.
  --help                  Show this help.

With no options, the script removes the installed Clinch.app only. It does not remove session
capture, provider plugins, local metadata, preferences, provider transcripts, or credentials.
EOF
}

APP_PATH=""
REMOVE_APP=1
DISABLE_INTEGRATION=0
PURGE_CAPTURE=0
PURGE_APP_DATA=0
REMOVE_PLUGINS=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --app)
      [[ $# -ge 2 ]] || { echo "error: --app requires a path" >&2; exit 2; }
      APP_PATH="$2"; shift 2 ;;
    --app=*) APP_PATH="${1#*=}"; shift ;;
    --disable-integration) DISABLE_INTEGRATION=1; shift ;;
    --purge-capture) PURGE_CAPTURE=1; DISABLE_INTEGRATION=1; shift ;;
    --purge-app-data) PURGE_APP_DATA=1; DISABLE_INTEGRATION=1; shift ;;
    --remove-plugins) REMOVE_PLUGINS=1; shift ;;
    --keep-app) REMOVE_APP=0; shift ;;
    --help|-h) usage; exit 0 ;;
    *) echo "error: unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
done

if [[ -z "$APP_PATH" ]]; then
  candidates=()
  [[ -d /Applications/Clinch.app ]] && candidates+=(/Applications/Clinch.app)
  [[ -d "$HOME/Applications/Clinch.app" ]] && candidates+=("$HOME/Applications/Clinch.app")
  if (( ${#candidates[@]} == 1 )); then
    APP_PATH="${candidates[0]}"
  elif (( ${#candidates[@]} > 1 )); then
    echo "error: Clinch exists in both /Applications and ~/Applications; use --app" >&2
    exit 1
  fi
fi

app_is_running() {
  [[ -n "$("${CLINCH_LSAPPINFO_BIN:-/usr/bin/lsappinfo}" \
    find 'bundleID=sh.clinch.Clinch' 2>/dev/null)" ]] && return 0
  "${CLINCH_PS_BIN:-/bin/ps}" -axo command= 2>/dev/null \
    | /usr/bin/grep -q '/[C]linch\.app/Contents/MacOS/'
}

verify_app_path() {
  local app="$1" bundle_id
  [[ "$app" == */Clinch.app ]] || {
    echo "error: refusing an app path that does not end in Clinch.app: $app" >&2
    return 1
  }
  [[ ! -L "$app" && -f "$app/Contents/Info.plist" ]] || {
    echo "error: refusing a symlink or invalid app bundle: $app" >&2
    return 1
  }
  bundle_id="$(/usr/libexec/PlistBuddy -c 'Print CFBundleIdentifier' \
    "$app/Contents/Info.plist" 2>/dev/null || true)"
  [[ "$bundle_id" == "sh.clinch.Clinch" ]] || {
    echo "error: refusing to remove bundle identifier '$bundle_id'" >&2
    return 1
  }
}

integration_installer() {
  local script_dir candidate
  if [[ -n "$APP_PATH" ]]; then
    candidate="$APP_PATH/Contents/Resources/agent-resume/install.sh"
    [[ -x "$candidate" ]] && { printf '%s\n' "$candidate"; return 0; }
  fi
  script_dir="$(cd "$(dirname "$0")" && pwd)"
  candidate="$script_dir/tools/agent-resume/install.sh"
  [[ -x "$candidate" ]] && { printf '%s\n' "$candidate"; return 0; }
  return 1
}

if (( REMOVE_APP || DISABLE_INTEGRATION || PURGE_APP_DATA )); then
  app_is_running && {
    echo "error: Clinch is running. Quit it before changing the app, integration, or app data." >&2
    exit 1
  }
fi

if (( DISABLE_INTEGRATION )); then
  installer="$(integration_installer)" || {
    echo "error: session-capture removal needs Clinch.app or a repository checkout" >&2
    exit 1
  }
  if (( PURGE_CAPTURE )); then
    /bin/bash "$installer" purge
  else
    /bin/bash "$installer" disable
  fi
fi

if (( REMOVE_PLUGINS )); then
  if command -v claude >/dev/null 2>&1; then
    claude plugin uninstall warp@clinch-claude-code-warp </dev/null || true
    claude plugin marketplace remove clinch-claude-code-warp </dev/null || true
    claude plugin uninstall warp@claude-code-warp </dev/null || true
  else
    echo "Claude Code is not on PATH; its plugin was not changed."
  fi
  if command -v codex >/dev/null 2>&1; then
    codex plugin remove warp@clinch-codex-warp </dev/null || true
    codex plugin marketplace remove clinch-codex-warp </dev/null || true
    codex plugin remove warp@codex-warp </dev/null || true
  else
    echo "Codex is not on PATH; its plugin was not changed."
  fi
fi

if (( REMOVE_APP )); then
  [[ -n "$APP_PATH" && -e "$APP_PATH" ]] || {
    echo "error: no installed Clinch.app was found; use --app or --keep-app" >&2
    exit 1
  }
  verify_app_path "$APP_PATH"
  /bin/rm -rf "$APP_PATH"
  echo "Removed $APP_PATH"
fi

if (( PURGE_APP_DATA )); then
  /bin/rm -rf \
    "$HOME/Library/Application Support/sh.clinch.Clinch" \
    "$HOME/Library/Caches/sh.clinch.Clinch" \
    "$HOME/Library/HTTPStorages/sh.clinch.Clinch" \
    "$HOME/Library/Logs/Clinch" \
    "$HOME/Library/Saved Application State/sh.clinch.Clinch.savedState" \
    "$HOME/Library/WebKit/sh.clinch.Clinch"
  shopt -s nullglob
  log_files=(
    "$HOME/Library/Logs/clinch.log"
    "$HOME/Library/Logs/clinch.log.recovery"
    "$HOME/Library/Logs/clinch.log.in_session."*
    "$HOME/Library/Logs/clinch.log.old."*
  )
  /bin/rm -f "${log_files[@]}" \
    "$HOME/Library/Preferences/sh.clinch.Clinch.plist" \
    "$HOME/Library/Cookies/sh.clinch.Clinch.binarycookies"
  shopt -u nullglob
  /usr/bin/defaults delete sh.clinch.Clinch >/dev/null 2>&1 || true
  echo "Removed Clinch-owned preferences, caches, logs, and application state."
fi

echo "Provider transcripts, Keychain credentials, and unrelated ~/.warp files were kept."
