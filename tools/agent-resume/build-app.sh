#!/usr/bin/env bash
# Builds the OSS-channel Warp app (with the agent-resume feature compiled in),
# optionally rebrands its display name, and installs it to /Applications as a
# distinct, co-installable app.
#
# Why this is safe to run alongside the production (downloaded) Warp:
#   - The OSS channel uses bundle id `dev.warp.WarpOss` (production is
#     `dev.warp.Warp-Stable`), so macOS treats them as different apps.
#   - Its data dir is `~/.warp-oss` + `~/Library/Application Support/dev.warp.WarpOss`,
#     so the two never share session/restore state.
#
# Usage:
#   ./tools/agent-resume/build-app.sh                 # name it "Warp (Elliot)"
#   WARP_ELLIOT_NAME="My Warp" ./tools/agent-resume/build-app.sh
#   WARP_ELLIOT_REBRAND=0 ./tools/agent-resume/build-app.sh   # keep "WarpOss"
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

echo "==> Building + bundling WarpOss (oss channel)…"
WARP_SKIP_COMMON_SKILLS_INSTALL=1 ./script/run --dont-open

APP="$(find target -maxdepth 5 -type d -name 'WarpOss.app' | head -1)"
[[ -n "$APP" ]] || { echo "error: WarpOss.app not produced" >&2; exit 1; }
echo "==> Built: $APP"

DEST="/Applications/WarpOss.app"
rm -rf "$DEST"
cp -R "$APP" "$DEST"

if [[ "${WARP_ELLIOT_REBRAND:-1}" = "1" ]]; then
  NAME="${WARP_ELLIOT_NAME:-Warp (Elliot)}"
  # Display name only — bundle id and channel are unchanged, so data isolation holds.
  /usr/bin/plutil -replace CFBundleDisplayName -string "$NAME" "$DEST/Contents/Info.plist"
  # Editing Info.plist invalidates the signature, so we must re-sign. Use a STABLE
  # identity (the same Apple Development cert script/macos/bundle uses), NOT ad-hoc:
  # macOS keys persisted TCC permission grants on the signing identity, so an ad-hoc
  # signature makes the OS re-prompt for permissions on EVERY launch. Matches
  # script/macos/bundle:696.
  IDENTITY="$(security find-identity -v -p codesigning | grep 'Apple Development' | head -1 | awk '{print $2}')"
  if [[ -n "$IDENTITY" ]]; then
    codesign --force --deep --options runtime --sign "$IDENTITY" \
      --entitlements script/Debug-Entitlements.plist "$DEST" >/dev/null
    echo "==> Re-signed with stable identity ($IDENTITY); macOS will remember permission grants."
  else
    codesign --force --deep --sign - "$DEST" >/dev/null 2>&1 || true
    echo "==> WARNING: no 'Apple Development' identity in keychain; signed ad-hoc."
    echo "    macOS will re-prompt for permissions every launch. Create a signing cert to fix."
  fi
  echo "==> Display name set to: $NAME"
fi

echo "==> Installed: $DEST"
echo "Launch it from /Applications or Launchpad. It runs independently of your downloaded Warp."
echo "Data dir: ~/.warp-oss (separate from production ~/.warp)."
