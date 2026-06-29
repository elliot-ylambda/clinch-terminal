#!/usr/bin/env bash
# Builds the OSS-channel Warp client (with the Clinch agent-resume feature compiled
# in), rebrands it to "Clinch", and installs it to /Applications as a distinct,
# co-installable app. Built with the `skip_login` Cargo feature so it runs fully
# local: no sign-in screen, and authenticated backend calls hard-fail — it never
# phones home to Warp. Download → use immediately.
#
# Clinch is a fork of Warp (https://github.com/warpdotdev/warp), AGPL-3.0. The
# functional changes vs. upstream are agent-session resume on restart and the
# local-only (no-login) build.
#
# Why this is safe to run alongside the production (downloaded) Warp:
#   - The build keeps the OSS *channel* but is rebranded to bundle id
#     `sh.clinch.Clinch` (production is `dev.warp.Warp-Stable`), so macOS treats them
#     as different apps.
#   - Its data dir is `~/.warp-oss` (channel-derived, so the rebrand doesn't move it),
#     so the two never share session/restore state.
#
# Usage:
#   ./tools/agent-resume/build-app.sh                 # name it "Clinch"
#   CLINCH_NAME="My Build" ./tools/agent-resume/build-app.sh
#   CLINCH_REBRAND=0 ./tools/agent-resume/build-app.sh        # keep "WarpOss"
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

# `--features skip_login`: boots straight to the terminal (no login screen) and makes
# every authenticated backend request hard-fail by design — see
# crates/warp_server_client/src/auth/session.rs ("skip_login enabled; failing all
# authenticated requests"). ./script/run appends this to the oss feature set and
# builds `cargo bundle --bin warp-oss --features gui,skip_login`.
echo "==> Building + bundling WarpOss (oss channel, skip_login = local-only)…"
WARP_SKIP_COMMON_SKILLS_INSTALL=1 ./script/run --dont-open --features skip_login

APP="$(find target -maxdepth 5 -type d -name 'WarpOss.app' | head -1)"
[[ -n "$APP" ]] || { echo "error: WarpOss.app not produced" >&2; exit 1; }
echo "==> Built: $APP"

NAME="${CLINCH_NAME:-Clinch}"
if [[ "${CLINCH_REBRAND:-1}" = "1" ]]; then
  DEST="/Applications/$NAME.app"
else
  DEST="/Applications/WarpOss.app"
fi
rm -rf "$DEST"
cp -R "$APP" "$DEST"
# Remove a previous default-named install so we don't leave a duplicate behind.
[[ "$DEST" != "/Applications/WarpOss.app" ]] && rm -rf "/Applications/WarpOss.app"

if [[ "${CLINCH_REBRAND:-1}" = "1" ]]; then
  # Rebrand the display name (Finder/Dock/Launchpad/menu bar/system dialogs) AND the
  # bundle id, so nothing user-visible carries the Warp mark. The oss *channel* is
  # unchanged, so the data dir stays ~/.warp-oss and co-install with production Warp
  # still holds (bundle id differs from dev.warp.Warp-Stable). The debug entitlements
  # don't reference the bundle id, so the re-sign below is unaffected.
  #
  # NOTE: changing the bundle id makes macOS treat this as a new app, so the first
  # launch re-prompts for TCC permissions (expected). Verify a built Clinch.app still
  # signs in and restores sessions before distributing a binary.
  /usr/bin/plutil -replace CFBundleDisplayName -string "$NAME"            "$DEST/Contents/Info.plist"
  /usr/bin/plutil -replace CFBundleName        -string "$NAME"            "$DEST/Contents/Info.plist"
  /usr/bin/plutil -replace CFBundleIdentifier  -string "sh.clinch.Clinch" "$DEST/Contents/Info.plist"

  # Swap the Warp icon for the Clinch icon. The oss build ships a classic
  # WarpOss.icns (the adaptive .icon format is skipped for the oss channel), so we
  # replace that file and repoint CFBundleIconFile. CFBundleIconName is removed so
  # macOS doesn't look for a (nonexistent) "Clinch" entry in an asset catalog.
  # Regenerate the .icns from the SVG with branding/build-icon.sh.
  ICON_SRC="tools/agent-resume/branding/Clinch.icns"
  if [[ -f "$ICON_SRC" ]]; then
    rm -f "$DEST/Contents/Resources/WarpOss.icns"
    cp "$ICON_SRC" "$DEST/Contents/Resources/Clinch.icns"
    /usr/bin/plutil -replace CFBundleIconFile -string "Clinch" "$DEST/Contents/Info.plist"
    /usr/bin/plutil -remove  CFBundleIconName "$DEST/Contents/Info.plist" 2>/dev/null || true
    touch "$DEST"   # nudge macOS to refresh the cached icon
    echo "==> Icon: swapped in Clinch.icns"
  else
    echo "==> WARNING: $ICON_SRC missing; keeping the Warp icon." >&2
    echo "    Build it with ./tools/agent-resume/branding/build-icon.sh" >&2
  fi

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
  echo "==> Named: $NAME"
fi

echo "==> Installed: $DEST"
echo "Launch it from /Applications or Launchpad. It runs independently of your downloaded Warp."
echo "Mode: local-only (skip_login) — no sign-in, no backend calls to Warp."
echo "Data dir: ~/.warp-oss (separate from production ~/.warp)."
