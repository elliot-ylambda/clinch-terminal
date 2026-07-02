#!/bin/sh
#
# Clinch installer — https://clinch.sh
#
# Usage:
#   curl -fsSL https://clinch.sh/install | sh
#
# What this does (and all it does):
#   1. Downloads Clinch.app.zip from the latest GitHub Release of
#      https://github.com/elliot-ylambda/clinch-terminal
#   2. Prints its SHA-256 so you can compare against the digest GitHub
#      shows on the release page
#   3. Extracts it and moves Clinch.app into /Applications
#      (or ~/Applications if /Applications isn't writable)
#   4. Opens Clinch
#
# Because the download happens via curl (not a browser), macOS never sets
# the com.apple.quarantine flag, so Gatekeeper doesn't block the app.
# Clinch is open source (AGPL-3.0); the most trustworthy install is still
# building from source — see the repo README.

set -eu

REPO="elliot-ylambda/clinch-terminal"
APP_NAME="Clinch"
ASSET="Clinch.app.zip"
DOWNLOAD_URL="https://github.com/$REPO/releases/latest/download/$ASSET"

say() { printf '%s\n' "$*"; }
fail() {
    printf 'Error: %s\n' "$*" >&2
    exit 1
}

[ "$(uname -s)" = "Darwin" ] || fail "$APP_NAME only runs on macOS."

# Refuse to clobber a running app: replacing the bundle under a live
# process leads to crashes on relaunch.
if pgrep -qf "$APP_NAME.app/Contents/MacOS" 2>/dev/null; then
    fail "$APP_NAME is currently running. Quit it, then re-run this installer."
fi

TMP_DIR="$(mktemp -d -t clinch-install)"
trap 'rm -rf "$TMP_DIR"' EXIT INT TERM

# The first redirect hop names the release tag:
#   .../releases/latest/download/X -> .../releases/download/<tag>/X
# (the final hop is an opaque CDN URL, so url_effective is useless here).
VERSION="$(curl --proto '=https' -fsSI -o /dev/null -w '%{redirect_url}' \
    "$DOWNLOAD_URL" | sed -n 's|.*/download/\([^/]*\)/.*|\1|p' || true)"

say "Downloading $APP_NAME${VERSION:+ $VERSION}..."
# --proto '=https' pins every request (including redirects) to HTTPS.
curl --proto '=https' -fL --retry 3 --progress-bar \
    -o "$TMP_DIR/$ASSET" "$DOWNLOAD_URL" \
    || fail "download failed. Check your connection, or grab the DMG from
       https://github.com/$REPO/releases/latest"

say "SHA-256 (compare with the digest on the release page if you like):"
say "  $(shasum -a 256 "$TMP_DIR/$ASSET" | awk '{print $1}')"

# ditto preserves symlinks, permissions, and extended attributes that
# unzip can mangle inside .app bundles.
ditto -x -k "$TMP_DIR/$ASSET" "$TMP_DIR/extracted"
APP_PATH="$TMP_DIR/extracted/$APP_NAME.app"
[ -d "$APP_PATH" ] || fail "unexpected archive layout: $APP_NAME.app not found in $ASSET."

# Arch check via `file` (always present, unlike lipo which needs the
# Xcode Command Line Tools). Universal binaries list every slice.
EXECUTABLE="$(/usr/libexec/PlistBuddy -c 'Print CFBundleExecutable' \
    "$APP_PATH/Contents/Info.plist" 2>/dev/null || printf 'stable')"
BINARY="$APP_PATH/Contents/MacOS/$EXECUTABLE"
MACHINE_ARCH="$(uname -m)"
if [ -f "$BINARY" ] && ! file -b "$BINARY" | grep -q "$MACHINE_ARCH"; then
    if [ "$MACHINE_ARCH" = "arm64" ]; then
        say "Note: this build is Intel-only; it will run under Rosetta 2."
    else
        fail "this $APP_NAME build is Apple Silicon-only and won't run on an
       Intel Mac. You can build from source instead:
       https://github.com/$REPO#readme"
    fi
fi

# Set CLINCH_INSTALL_DIR to install somewhere other than /Applications.
if [ -n "${CLINCH_INSTALL_DIR:-}" ]; then
    INSTALL_DIR="$CLINCH_INSTALL_DIR"
    mkdir -p "$INSTALL_DIR"
else
    INSTALL_DIR="/Applications"
    if [ ! -w "$INSTALL_DIR" ]; then
        INSTALL_DIR="$HOME/Applications"
        mkdir -p "$INSTALL_DIR"
        say "/Applications isn't writable; installing to $INSTALL_DIR instead."
    fi
fi
DEST="$INSTALL_DIR/$APP_NAME.app"

if [ -d "$DEST" ]; then
    say "Replacing the existing $DEST..."
    rm -rf "$DEST"
fi
mv "$APP_PATH" "$DEST"

# curl downloads never get the quarantine flag, but strip it anyway in
# case this zip was ever staged through a browser or AirDrop.
xattr -dr com.apple.quarantine "$DEST" 2>/dev/null || true

say ""
say "✓ $APP_NAME${VERSION:+ $VERSION} installed to $DEST"
open "$DEST"
