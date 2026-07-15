#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
TMP="$(mktemp -d -t clinch-update-manifest-test)"
trap 'rm -rf "$TMP"' EXIT

find_openssl3() {
  local candidate
  for candidate in "$(command -v openssl 2>/dev/null || true)" \
      /opt/homebrew/opt/openssl@3/bin/openssl /usr/local/opt/openssl@3/bin/openssl; do
    [[ -x "$candidate" ]] || continue
    "$candidate" version | grep -q '^OpenSSL 3\.' && { echo "$candidate"; return; }
  done
  return 1
}

OPENSSL3="$(find_openssl3)"
KEY="$TMP/key.pem"
PUBLIC="$TMP/public.json"
APP="$TMP/Clinch.app"
ZIP="$TMP/Clinch.app.zip"
MANIFEST="$TMP/Clinch.update.json"
SIGNATURE="$TMP/Clinch.update.sig"
VERSION="v0.2099.01.02.0304"
SEQUENCE="4070991840"

"$OPENSSL3" genpkey -algorithm ED25519 -out "$KEY"
RAW_PUBLIC="$($OPENSSL3 pkey -in "$KEY" -pubout -outform DER | tail -c 32 | base64 | tr -d '\n')"
printf '{"key_id":"test-key","ed25519_public_key":"%s"}\n' "$RAW_PUBLIC" > "$PUBLIC"
mkdir -p "$APP/Contents"
/usr/bin/plutil -create xml1 "$APP/Contents/Info.plist"
/usr/bin/plutil -insert CFBundleIdentifier -string sh.clinch.Clinch "$APP/Contents/Info.plist"
/usr/bin/plutil -insert WarpVersion -string "$VERSION" "$APP/Contents/Info.plist"
/usr/bin/plutil -insert ClinchUpdateSequence -string "$SEQUENCE" "$APP/Contents/Info.plist"
/usr/bin/plutil -insert LSMinimumSystemVersion -string 14.0 "$APP/Contents/Info.plist"
printf 'authenticated archive fixture\n' > "$ZIP"

CLINCH_OPENSSL_BIN="$OPENSSL3" \
CLINCH_UPDATE_SIGNING_KEY="$KEY" \
CLINCH_UPDATE_PUBLIC_KEY_JSON="$PUBLIC" \
RELEASE_NOTES="Signed updater fixture" \
  "$ROOT/script/clinch-update-manifest" generate \
    "$APP" "$ZIP" "$VERSION" "$SEQUENCE" "$MANIFEST" "$SIGNATURE"

CLINCH_OPENSSL_BIN="$OPENSSL3" \
  "$ROOT/script/clinch-update-manifest" verify "$MANIFEST" "$SIGNATURE" "$PUBLIC"
/usr/bin/python3 - "$MANIFEST" "$SEQUENCE" <<'PY'
import json, sys
with open(sys.argv[1], encoding="utf-8") as source:
    manifest = json.load(source)
assert manifest["schema_version"] == 1
assert manifest["sequence"] == int(sys.argv[2])
assert manifest["archive"]["name"] == "Clinch.app.zip"
assert manifest["release_notes"] == "Signed updater fixture"
PY

printf ' ' >> "$MANIFEST"
if CLINCH_OPENSSL_BIN="$OPENSSL3" \
    "$ROOT/script/clinch-update-manifest" verify "$MANIFEST" "$SIGNATURE" "$PUBLIC" \
    >/dev/null 2>&1; then
  echo "tampered update manifest unexpectedly verified" >&2
  exit 1
fi

echo "clinch signed update manifest tests passed"
