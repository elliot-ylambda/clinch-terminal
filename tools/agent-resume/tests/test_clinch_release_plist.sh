#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
PLIST="$TMP/Info.plist"

plutil -create xml1 "$PLIST"
plutil -insert CFBundleIdentifier -string sh.clinch.Clinch "$PLIST"
plutil -insert CFBundleName -string Clinch "$PLIST"
plutil -insert CFBundleShortVersionString -string 0.0.0 "$PLIST"
plutil -insert CFBundleVersion -string 0.0.0 "$PLIST"
plutil -insert NSMicrophoneUsageDescription -string stale "$PLIST"

(
  cd "$ROOT"
  WARP_PLIST_PATH="$PLIST" \
    WARP_PLIST_NO_FILE_TYPES=1 \
    WARP_SCHEME_NAME=clinch \
    GIT_RELEASE_TAG=v0.2026.07.13.1800 \
    CLINCH_UPDATE_SEQUENCE=1784000000 \
    MACOSX_DEPLOYMENT_TARGET=14.0 \
    bash script/update_plist >/dev/null
)

[[ "$(/usr/libexec/PlistBuddy -c 'Print WarpVersion' "$PLIST")" == v0.2026.07.13.1800 ]]
[[ "$(/usr/libexec/PlistBuddy -c 'Print CFBundleShortVersionString' "$PLIST")" == 0.2026.07.13.1800 ]]
[[ "$(/usr/libexec/PlistBuddy -c 'Print CFBundleVersion' "$PLIST")" == 1784000000 ]]
[[ "$(/usr/libexec/PlistBuddy -c 'Print ClinchUpdateSequence' "$PLIST")" == 1784000000 ]]
[[ "$(/usr/libexec/PlistBuddy -c 'Print LSMinimumSystemVersion' "$PLIST")" == 14.0 ]]
if /usr/libexec/PlistBuddy -c 'Print NSMicrophoneUsageDescription' "$PLIST" >/dev/null 2>&1; then
  echo "FAIL: Clinch plist retained an unused privacy permission description"
  exit 1
fi

echo "PASS"
