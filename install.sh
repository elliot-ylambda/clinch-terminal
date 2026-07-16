#!/bin/sh
# Authenticated installer for the unnotarized Clinch public preview.
# Primary installation path: curl -fsSL https://clinch.sh/install | sh
# Manual versioned DMG/ZIP downloads, verified per the README, remain available.
set -eu

REPO="elliot-ylambda/clinch-terminal"
APP_NAME="Clinch"
ASSET="Clinch.app.zip"
MANIFEST_ASSET="Clinch.update.json"
SSHSIG_ASSET="Clinch.update.sshsig"
EXPECTED_BUNDLE_ID="sh.clinch.Clinch"
EXPECTED_EXECUTABLE="stable"
EXPECTED_UPDATE_KEY_ID="a353cda3ad59f128"
RELEASE_SIGNER="clinch-release"
RELEASE_NAMESPACE="clinch-install"
RELEASE_PUBLIC_KEY="ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGr+qT8+Fx8TjATDpWlhzzfbL08AsS1EXbaaOUBi0wJp"
GITHUB_API="https://api.github.com/repos/$REPO"
GITHUB_RELEASES="https://github.com/$REPO/releases"

VERSION=""
INSTALL_DIR="${CLINCH_INSTALL_DIR:-}"
OPEN_APP=1

say() { printf '%s\n' "$*"; }
fail() {
    printf 'Error: %s\n' "$*" >&2
    exit 1
}
usage() {
    cat <<'EOF'
usage: install.sh [--version vX.Y.Z] [--install-dir DIRECTORY] [--no-open]

Downloads one exact Clinch GitHub release, authenticates its signed manifest with the
embedded Clinch release key, verifies the app, installs it, and opens it. It does not
change Gatekeeper, Claude Code, Codex, plugins, or preferences. Use --no-open to skip
launching Clinch after the install.
EOF
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --version)
            [ "$#" -ge 2 ] || fail "--version requires a value."
            VERSION="$2"
            shift 2
            ;;
        --version=*) VERSION=${1#*=}; shift ;;
        --install-dir)
            [ "$#" -ge 2 ] || fail "--install-dir requires a value."
            INSTALL_DIR="$2"
            shift 2
            ;;
        --install-dir=*) INSTALL_DIR=${1#*=}; shift ;;
        --no-open) OPEN_APP=0; shift ;;
        --help|-h) usage; exit 0 ;;
        *) fail "unknown option: $1" ;;
    esac
done

safe_tag() {
    value=${1#v}
    [ "$value" != "$1" ] || return 1
    case "$value" in *.*) ;; *) return 1 ;; esac
    case "$value" in *[!0-9.]*|*..*|.*|*.|'') return 1 ;; esac
    return 0
}

download() {
    output=$1
    url=$2
    "${CLINCH_CURL_BIN:-/usr/bin/curl}" --proto '=https' -fL --retry 3 \
        --connect-timeout 15 --max-time 300 --silent --show-error \
        -o "$output" "$url"
}

file_size() {
    /usr/bin/stat -f '%z' "$1" 2>/dev/null || printf '0\n'
}

version_at_least() {
    /usr/bin/awk -v actual="$1" -v required="$2" 'BEGIN {
        na = split(actual, a, "."); nr = split(required, r, ".");
        n = na > nr ? na : nr;
        for (i = 1; i <= n; i++) {
            av = (a[i] == "" ? 0 : a[i]) + 0;
            rv = (r[i] == "" ? 0 : r[i]) + 0;
            if (av > rv) exit 0;
            if (av < rv) exit 1;
        }
        exit 0;
    }'
}

app_is_running() {
    [ -n "$("${CLINCH_LSAPPINFO_BIN:-/usr/bin/lsappinfo}" \
        find "bundleID=$EXPECTED_BUNDLE_ID" 2>/dev/null)" ] && return 0
    "${CLINCH_PS_BIN:-/bin/ps}" -axo command= 2>/dev/null \
        | /usr/bin/grep -q '/[C]linch\.app/Contents/MacOS/'
}

read_latest_tag() {
    latest_json=$1
    /usr/bin/osascript -l JavaScript - "$latest_json" <<'JXA'
ObjC.import("Foundation");
function run(argv) {
    const text = $.NSString.stringWithContentsOfFileEncodingError(
        argv[0], $.NSUTF8StringEncoding, null
    );
    if (!text) throw new Error("could not read latest-release response");
    const value = JSON.parse(ObjC.unwrap(text));
    if (!value || typeof value.tag_name !== "string") {
        throw new Error("latest-release response has no tag_name");
    }
    return value.tag_name;
}
JXA
}

read_verified_manifest() {
    manifest=$1
    expected_tag=$2
    /usr/bin/osascript -l JavaScript - "$manifest" "$expected_tag" "$EXPECTED_UPDATE_KEY_ID" <<'JXA'
ObjC.import("Foundation");
function fail(message) { throw new Error(message); }
function run(argv) {
    const text = $.NSString.stringWithContentsOfFileEncodingError(
        argv[0], $.NSUTF8StringEncoding, null
    );
    if (!text) fail("could not read signed manifest");
    const m = JSON.parse(ObjC.unwrap(text));
    const tag = argv[1];
    const keyId = argv[2];
    const repository = "elliot-ylambda/clinch-terminal";
    const archiveName = "Clinch.app.zip";
    const archiveUrl = "https://github.com/" + repository +
        "/releases/download/" + tag + "/" + archiveName;
    const releaseUrl = "https://github.com/" + repository + "/releases/tag/" + tag;

    if (!m || Array.isArray(m) || typeof m !== "object") fail("manifest is not an object");
    if (m.schema_version !== 1) fail("unsupported manifest schema");
    if (m.repository !== repository) fail("manifest repository mismatch");
    if (m.version !== tag || m.tag !== tag) fail("manifest tag mismatch");
    if (m.bundle_id !== "sh.clinch.Clinch") fail("manifest bundle identifier mismatch");
    if (m.signing_key_id !== keyId) fail("manifest update-key identifier mismatch");
    if (m.notarized !== false) fail("manifest notarization status is invalid");
    if (m.release_url !== releaseUrl) fail("manifest release URL mismatch");
    if (!Number.isSafeInteger(m.sequence) || m.sequence < 1) fail("invalid release sequence");
    if (typeof m.minimum_macos_version !== "string" ||
        !/^[0-9]+(?:\.[0-9]+){0,2}$/.test(m.minimum_macos_version)) {
        fail("invalid minimum macOS version");
    }
    if (!Array.isArray(m.architectures) || m.architectures.length !== 2 ||
        m.architectures.slice().sort().join(",") !== "arm64,x86_64") {
        fail("manifest does not require a universal app");
    }
    if (!m.archive || typeof m.archive !== "object") fail("manifest archive is missing");
    if (m.archive.name !== archiveName || m.archive.url !== archiveUrl) {
        fail("manifest archive identity mismatch");
    }
    if (!Number.isSafeInteger(m.archive.size) || m.archive.size < 1 ||
        m.archive.size > 2147483648) fail("invalid archive size");
    if (typeof m.archive.sha256 !== "string" ||
        !/^[0-9a-f]{64}$/.test(m.archive.sha256)) fail("invalid archive SHA-256");

    return [
        m.archive.sha256,
        String(m.archive.size),
        m.minimum_macos_version,
        String(m.sequence),
        m.archive.url
    ].join("\n");
}
JXA
}

verify_bundle() {
    app=$1
    plist="$app/Contents/Info.plist"
    [ -f "$plist" ] || fail "the archive has no Clinch Info.plist."

    bundle_id=$(/usr/libexec/PlistBuddy -c 'Print CFBundleIdentifier' "$plist" 2>/dev/null || true)
    [ "$bundle_id" = "$EXPECTED_BUNDLE_ID" ] \
        || fail "unexpected app identity '$bundle_id'."
    executable=$(/usr/libexec/PlistBuddy -c 'Print CFBundleExecutable' "$plist" 2>/dev/null || true)
    [ "$executable" = "$EXPECTED_EXECUTABLE" ] \
        || fail "unexpected app executable '$executable'."
    bundle_tag=$(/usr/libexec/PlistBuddy -c 'Print WarpVersion' "$plist" 2>/dev/null || true)
    [ "$bundle_tag" = "$VERSION" ] || fail "app version does not match $VERSION."
    short_version=$(/usr/libexec/PlistBuddy -c 'Print CFBundleShortVersionString' "$plist" 2>/dev/null || true)
    [ "$short_version" = "${VERSION#v}" ] || fail "app display version does not match $VERSION."
    bundle_version=$(/usr/libexec/PlistBuddy -c 'Print CFBundleVersion' "$plist" 2>/dev/null || true)
    [ "$bundle_version" = "$UPDATE_SEQUENCE" ] \
        || fail "app build version does not match the authenticated release sequence."
    bundle_minimum=$(/usr/libexec/PlistBuddy -c 'Print LSMinimumSystemVersion' "$plist" 2>/dev/null || true)
    [ "$bundle_minimum" = "$MINIMUM_MACOS" ] \
        || fail "app and signed manifest disagree on minimum macOS version."

    binary="$app/Contents/MacOS/$EXPECTED_EXECUTABLE"
    [ -x "$binary" ] || fail "the app executable is missing or not executable."
    binary_description=$(/usr/bin/file -b "$binary")
    case "$binary_description" in *arm64*) ;; *) fail "the app is missing its Apple Silicon slice." ;; esac
    case "$binary_description" in *x86_64*) ;; *) fail "the app is missing its Intel slice." ;; esac
    /usr/bin/codesign --verify --deep --strict "$app" 2>/dev/null \
        || fail "the app's structural code signature is invalid."
}

verify_existing_destination() {
    app=$1
    plist="$app/Contents/Info.plist"
    [ ! -L "$app" ] && [ -f "$plist" ] \
        || fail "refusing to replace a symlink or invalid Clinch.app destination."
    bundle_id=$(/usr/libexec/PlistBuddy -c 'Print CFBundleIdentifier' "$plist" 2>/dev/null || true)
    executable=$(/usr/libexec/PlistBuddy -c 'Print CFBundleExecutable' "$plist" 2>/dev/null || true)
    [ "$bundle_id" = "$EXPECTED_BUNDLE_ID" ] && [ "$executable" = "$EXPECTED_EXECUTABLE" ] \
        || fail "refusing to replace an app that is not an existing Clinch installation."
}

TMP_DIR=""
STAGE_ROOT=""
STAGED=""
BACKUP_ROOT=""
BACKUP=""
DEST=""
cleanup() {
    if [ -n "$BACKUP" ] && [ -d "$BACKUP" ] && [ -n "$DEST" ] && [ ! -e "$DEST" ]; then
        /bin/mv "$BACKUP" "$DEST" 2>/dev/null || true
    fi
    [ -z "$STAGE_ROOT" ] || /bin/rm -rf "$STAGE_ROOT"
    if [ -n "$BACKUP_ROOT" ] && [ -d "$BACKUP_ROOT" ] && [ ! -e "$BACKUP" ]; then
        /bin/rmdir "$BACKUP_ROOT" 2>/dev/null || true
    fi
    [ -z "$TMP_DIR" ] || /bin/rm -rf "$TMP_DIR"
}
trap cleanup EXIT HUP INT TERM

[ "$(/usr/bin/uname -s)" = "Darwin" ] || fail "$APP_NAME only runs on macOS."
[ -x /usr/bin/ssh-keygen ] || fail "macOS ssh-keygen is required to authenticate this release."
if app_is_running; then
    fail "$APP_NAME is currently running. Quit it, then run the installer again."
fi

TMP_DIR="$(/usr/bin/mktemp -d -t clinch-install)"
/bin/chmod 700 "$TMP_DIR"

if [ -z "$VERSION" ]; then
    say "Resolving the latest Clinch release..."
    download "$TMP_DIR/latest.json" "$GITHUB_API/releases/latest" \
        || fail "could not resolve the latest GitHub release."
    [ "$(file_size "$TMP_DIR/latest.json")" -le 1048576 ] \
        || fail "latest-release response is unexpectedly large."
    VERSION="$(read_latest_tag "$TMP_DIR/latest.json" 2>/dev/null)" \
        || fail "GitHub returned an invalid latest-release response."
fi
safe_tag "$VERSION" || fail "unsafe release tag '$VERSION'."

ASSET_BASE="$GITHUB_RELEASES/download/$VERSION"
say "Authenticating Clinch $VERSION release metadata..."
download "$TMP_DIR/$MANIFEST_ASSET" "$ASSET_BASE/$MANIFEST_ASSET" \
    || fail "the signed manifest is missing for $VERSION."
download "$TMP_DIR/$SSHSIG_ASSET" "$ASSET_BASE/$SSHSIG_ASSET" \
    || fail "the release signature is missing for $VERSION."
[ "$(file_size "$TMP_DIR/$MANIFEST_ASSET")" -le 1048576 ] \
    || fail "the signed manifest is unexpectedly large."
[ "$(file_size "$TMP_DIR/$SSHSIG_ASSET")" -le 65536 ] \
    || fail "the release signature is unexpectedly large."
printf '%s %s\n' "$RELEASE_SIGNER" "$RELEASE_PUBLIC_KEY" > "$TMP_DIR/allowed_signers"
/bin/chmod 600 "$TMP_DIR/allowed_signers"
if ! /usr/bin/ssh-keygen -Y verify -f "$TMP_DIR/allowed_signers" \
    -I "$RELEASE_SIGNER" -n "$RELEASE_NAMESPACE" \
    -s "$TMP_DIR/$SSHSIG_ASSET" < "$TMP_DIR/$MANIFEST_ASSET" >/dev/null 2>&1; then
    fail "release signature verification failed; nothing was installed."
fi

MANIFEST_VALUES="$(read_verified_manifest "$TMP_DIR/$MANIFEST_ASSET" "$VERSION" 2>/dev/null)" \
    || fail "signed release metadata failed validation."
EXPECTED_SHA=$(printf '%s\n' "$MANIFEST_VALUES" | /usr/bin/sed -n '1p')
EXPECTED_SIZE=$(printf '%s\n' "$MANIFEST_VALUES" | /usr/bin/sed -n '2p')
MINIMUM_MACOS=$(printf '%s\n' "$MANIFEST_VALUES" | /usr/bin/sed -n '3p')
UPDATE_SEQUENCE=$(printf '%s\n' "$MANIFEST_VALUES" | /usr/bin/sed -n '4p')
ARCHIVE_URL=$(printf '%s\n' "$MANIFEST_VALUES" | /usr/bin/sed -n '5p')

CURRENT_MACOS=$(/usr/bin/sw_vers -productVersion)
version_at_least "$CURRENT_MACOS" "$MINIMUM_MACOS" \
    || fail "Clinch $VERSION requires macOS $MINIMUM_MACOS or later; this Mac runs $CURRENT_MACOS."

say "Downloading the exact $VERSION universal app..."
download "$TMP_DIR/$ASSET" "$ARCHIVE_URL" \
    || fail "archive download failed; nothing was installed."
ACTUAL_SIZE=$(file_size "$TMP_DIR/$ASSET")
[ "$ACTUAL_SIZE" = "$EXPECTED_SIZE" ] \
    || fail "archive size mismatch; nothing was installed."
ACTUAL_SHA=$(/usr/bin/shasum -a 256 "$TMP_DIR/$ASSET" | /usr/bin/awk '{print $1}')
[ "$ACTUAL_SHA" = "$EXPECTED_SHA" ] \
    || fail "archive SHA-256 mismatch; nothing was installed."

/usr/bin/unzip -Z1 "$TMP_DIR/$ASSET" > "$TMP_DIR/archive-list" 2>/dev/null \
    || fail "the downloaded ZIP is invalid."
[ -s "$TMP_DIR/archive-list" ] || fail "the downloaded ZIP is empty."
if ! /usr/bin/awk -F/ '
    /^\// { exit 1 }
    /(^|\/)\.\.($|\/)/ { exit 1 }
    $1 != "Clinch.app" { exit 1 }
' "$TMP_DIR/archive-list"; then
    fail "the archive contains an unsafe or unexpected path."
fi

/usr/bin/ditto -x -k "$TMP_DIR/$ASSET" "$TMP_DIR/extracted" \
    || fail "the verified archive could not be expanded."
APP_PATH="$TMP_DIR/extracted/$APP_NAME.app"
[ -d "$APP_PATH" ] || fail "the archive does not contain Clinch.app at its root."
verify_bundle "$APP_PATH"

if [ -z "$INSTALL_DIR" ]; then
    INSTALL_DIR="/Applications"
    if [ ! -w "$INSTALL_DIR" ]; then
        INSTALL_DIR="$HOME/Applications"
        say "/Applications is not writable; using $INSTALL_DIR."
    fi
fi
/bin/mkdir -p "$INSTALL_DIR"
[ -d "$INSTALL_DIR" ] || fail "installation directory is not a directory: $INSTALL_DIR"
[ ! -L "$INSTALL_DIR" ] || fail "refusing a symbolic-link installation directory."
DEST="$INSTALL_DIR/$APP_NAME.app"
[ ! -L "$DEST" ] || fail "refusing to replace a symbolic-link app destination."
[ ! -e "$DEST" ] || verify_existing_destination "$DEST"

STAGE_ROOT="$(/usr/bin/mktemp -d "$INSTALL_DIR/.Clinch.app.install.XXXXXX")" \
    || fail "could not create a private staging directory in $INSTALL_DIR."
/bin/chmod 700 "$STAGE_ROOT"
STAGED="$STAGE_ROOT/$APP_NAME.app"
/usr/bin/ditto "$APP_PATH" "$STAGED" || fail "could not stage the app in $INSTALL_DIR."
verify_bundle "$STAGED"

if [ -d "$DEST" ]; then
    BACKUP_ROOT="$(/usr/bin/mktemp -d "$INSTALL_DIR/.Clinch.app.backup.XXXXXX")" \
        || fail "could not reserve a rollback directory in $INSTALL_DIR."
    /bin/chmod 700 "$BACKUP_ROOT"
    BACKUP="$BACKUP_ROOT/$APP_NAME.app"
    /bin/mv "$DEST" "$BACKUP" || fail "could not preserve the existing app."
    verify_existing_destination "$BACKUP"
fi
if ! /bin/mv "$STAGED" "$DEST"; then
    [ ! -d "$BACKUP" ] || /bin/mv "$BACKUP" "$DEST" 2>/dev/null || true
    fail "could not activate the verified app; the previous app was restored."
fi
STAGED=""
if [ -d "$STAGE_ROOT" ]; then
    /bin/rmdir "$STAGE_ROOT"
fi
STAGE_ROOT=""
if [ -d "$BACKUP" ]; then
    /bin/rm -rf "$BACKUP"
fi
BACKUP=""
if [ -d "$BACKUP_ROOT" ]; then
    /bin/rmdir "$BACKUP_ROOT"
fi
BACKUP_ROOT=""

say ""
say "Clinch $VERSION is installed at $DEST"
say "SHA-256: $ACTUAL_SHA"
say "Release sequence: $UPDATE_SEQUENCE"
say ""
say "This public preview is ad-hoc signed and not notarized by Apple."
say "The installer did not remove quarantine data or change Gatekeeper settings."
say "Command-line downloads are not quarantined, so no Gatekeeper approval is needed."
say "Session restore is enabled on first launch; you can turn capture off in Clinch Settings."
say "Updates are manual for this preview; rerun this installer for a signed newer version."

if [ "$OPEN_APP" = 1 ]; then
    say ""
    if /usr/bin/open "$DEST" 2>/dev/null; then
        say "Opening Clinch. If it was already running, quit and reopen it to use $VERSION."
    else
        say "Could not open Clinch automatically; open it from $INSTALL_DIR."
        say "If macOS blocks the launch, use System Settings → Privacy & Security → Open Anyway."
    fi
fi
