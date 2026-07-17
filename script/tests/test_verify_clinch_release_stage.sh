#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TMP="$(mktemp -d -t clinch-release-stage-test)"
trap 'rm -rf "$TMP"' EXIT
FIXTURE="$TMP/repo"
RELEASE_DIR="$FIXTURE/target/release-lto/bundle/osx"
mkdir -p \
  "$FIXTURE/script" \
  "$FIXTURE/resources/release" \
  "$FIXTURE/resources/update" \
  "$RELEASE_DIR/Clinch.app" \
  "$TMP/bin"
cp \
  "$ROOT/script/assemble-clinch-release-stage" \
  "$ROOT/script/verify-clinch-release-stage" \
  "$ROOT/script/clinch-update-manifest" \
  "$FIXTURE/script/"
cp "$ROOT/Cargo.lock" "$FIXTURE/Cargo.lock"
printf '#!/bin/sh\n' > "$FIXTURE/install.sh"
printf '#!/bin/sh\n' > "$FIXTURE/uninstall.sh"

VERSION=v0.2099.01.02.0304
COMMIT=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
SEQUENCE=4102441440

find_openssl3() {
  local candidate
  for candidate in "${CLINCH_OPENSSL_BIN:-}" "$(command -v openssl 2>/dev/null || true)" \
      /opt/homebrew/opt/openssl@3/bin/openssl /usr/local/opt/openssl@3/bin/openssl; do
    [[ -n "$candidate" && -x "$candidate" ]] || continue
    if "$candidate" version 2>/dev/null | grep -q '^OpenSSL 3\.'; then
      printf '%s\n' "$candidate"
      return
    fi
  done
  echo "SKIP: OpenSSL 3 is unavailable" >&2
  exit 0
}

OPENSSL3="$(find_openssl3)"
export CLINCH_OPENSSL_BIN="$OPENSSL3"

"$OPENSSL3" genpkey -algorithm Ed25519 -out "$TMP/update-key.pem"
update_public="$($OPENSSL3 pkey -in "$TMP/update-key.pem" -pubout -outform DER \
  | tail -c 32 | base64 | tr -d '\n')"
printf '{"ed25519_public_key":"%s","key_id":"fixture"}\n' "$update_public" \
  > "$FIXTURE/resources/update/clinch-update-public-key.json"
"$FIXTURE/script/clinch-update-manifest" verify-key \
  "$TMP/update-key.pem" "$FIXTURE/resources/update/clinch-update-public-key.json" >/dev/null
"$OPENSSL3" genpkey -algorithm Ed25519 -out "$TMP/wrong-update-key.pem"
if "$FIXTURE/script/clinch-update-manifest" verify-key \
    "$TMP/wrong-update-key.pem" "$FIXTURE/resources/update/clinch-update-public-key.json" \
    > "$TMP/wrong-update-key.out" 2>&1; then
  echo "FAIL: mismatched update signing key was accepted" >&2
  exit 1
fi

ssh-keygen -q -t ed25519 -N '' -f "$TMP/release-key"
printf 'clinch-release %s\n' "$(awk '{print $1, $2}' "$TMP/release-key.pub")" \
  > "$FIXTURE/resources/release/clinch-release-allowed-signers"

printf 'fixture zip\n' > "$RELEASE_DIR/Clinch.app.zip"
printf 'fixture dmg\n' > "$RELEASE_DIR/Clinch.dmg"
(cd "$RELEASE_DIR" && shasum -a 256 Clinch.app.zip > Clinch.app.zip.sha256)
(cd "$RELEASE_DIR" && shasum -a 256 Clinch.dmg > Clinch.dmg.sha256)

/usr/bin/python3 - "$RELEASE_DIR" "$VERSION" "$SEQUENCE" <<'PY'
import hashlib
import json
import pathlib
import sys

release_dir = pathlib.Path(sys.argv[1])
version = sys.argv[2]
sequence = int(sys.argv[3])
archive = release_dir / "Clinch.app.zip"
digest = hashlib.sha256(archive.read_bytes()).hexdigest()
manifest = {
    "architectures": ["arm64", "x86_64"],
    "archive": {
        "name": archive.name,
        "sha256": digest,
        "size": archive.stat().st_size,
        "url": (
            "https://github.com/elliot-ylambda/clinch-terminal/releases/download/"
            f"{version}/{archive.name}"
        ),
    },
    "bundle_id": "sh.clinch.Clinch",
    "minimum_macos_version": "14.0",
    "notarized": False,
    "release_notes": "Fixture release notes.",
    "release_url": (
        "https://github.com/elliot-ylambda/clinch-terminal/releases/tag/"
        f"{version}"
    ),
    "repository": "elliot-ylambda/clinch-terminal",
    "rollback": False,
    "schema_version": 1,
    "sequence": sequence,
    "signing_key_id": "fixture",
    "tag": version,
    "version": version,
}
(release_dir / "Clinch.update.json").write_text(
    json.dumps(manifest, sort_keys=True, separators=(",", ":")) + "\n",
    encoding="utf-8",
)
PY

"$OPENSSL3" pkeyutl -sign -rawin -inkey "$TMP/update-key.pem" \
  -in "$RELEASE_DIR/Clinch.update.json" -out "$TMP/update-signature.bin"
base64 < "$TMP/update-signature.bin" | tr -d '\n' > "$RELEASE_DIR/Clinch.update.sig"
printf '\n' >> "$RELEASE_DIR/Clinch.update.sig"
ssh-keygen -Y sign -f "$TMP/release-key" -n clinch-install - \
  < "$RELEASE_DIR/Clinch.update.json" > "$RELEASE_DIR/Clinch.update.sshsig"

cat > "$TMP/bin/git" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
[[ "$*" == 'rev-parse HEAD' ]] || exit 2
printf '%s\n' "$FIXTURE_COMMIT"
STUB

cat > "$TMP/bin/sw_vers" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
[[ "${1:-}" == -productVersion ]] || exit 2
printf '%s\n' '99.0'
STUB

cat > "$TMP/bin/syft" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
[[ "$2" == -o && "$3" == cyclonedx-json=* ]] || exit 2
output="${3#cyclonedx-json=}"
printf '{"bomFormat":"CycloneDX","specVersion":"1.6"}\n' > "$output"
STUB
chmod +x "$FIXTURE/script/"* "$TMP/bin/"*

assemble_output="$(cd "$FIXTURE" && env \
  PATH="$TMP/bin:$PATH" \
  FIXTURE_COMMIT="$COMMIT" \
  CLINCH_RELEASE_SIGNING_KEY="$TMP/release-key" \
  ./script/assemble-clinch-release-stage \
    "$VERSION" "$COMMIT" "$SEQUENCE")"
grep -Fq "target/release-stage/$VERSION" <<< "$assemble_output"

DIST="$FIXTURE/target/release-stage/$VERSION/dist"
[[ -f "$FIXTURE/target/release-stage/$VERSION/release-notes.md" ]]
/usr/bin/python3 - "$DIST/Clinch.release-validation.json" <<'PY'
import json
import pathlib
import sys

validation = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
assert validation["schema_version"] == 3
assert "manual_qa" not in validation
PY

verify() {
  (cd "$FIXTURE" && ./script/verify-clinch-release-stage "$@")
}

expect_failure() {
  local label="$1"
  shift
  if "$@" >"$TMP/$label.out" 2>"$TMP/$label.err"; then
    echo "FAIL: $label was accepted" >&2
    exit 1
  fi
}

verify "$DIST" "$VERSION" "$COMMIT" "$SEQUENCE" >/dev/null

cp "$DIST/Clinch.app.zip" "$TMP/Clinch.app.zip"
printf 'tampered\n' >> "$DIST/Clinch.app.zip"
expect_failure tampered verify "$DIST" "$VERSION" "$COMMIT" "$SEQUENCE"
cp "$TMP/Clinch.app.zip" "$DIST/Clinch.app.zip"

printf 'extra\n' > "$DIST/unexpected.txt"
expect_failure extra verify "$DIST" "$VERSION" "$COMMIT" "$SEQUENCE"
rm "$DIST/unexpected.txt"

mkdir "$DIST/unexpected-directory"
expect_failure extra_directory verify "$DIST" "$VERSION" "$COMMIT" "$SEQUENCE"
rmdir "$DIST/unexpected-directory"

mv "$DIST/uninstall.sh" "$TMP/uninstall.sh"
expect_failure missing verify "$DIST" "$VERSION" "$COMMIT" "$SEQUENCE"
mv "$TMP/uninstall.sh" "$DIST/uninstall.sh"

expect_failure wrong_version verify "$DIST" v0.2099.01.02.0305 "$COMMIT" "$SEQUENCE"
expect_failure wrong_commit verify "$DIST" "$VERSION" bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb "$SEQUENCE"
expect_failure stale_sequence verify "$DIST" "$VERSION" "$COMMIT" "$((SEQUENCE - 1))"

cp "$DIST/Clinch.build-provenance.sshsig" "$TMP/provenance.sshsig"
printf 'invalid\n' > "$DIST/Clinch.build-provenance.sshsig"
expect_failure invalid_signature verify "$DIST" "$VERSION" "$COMMIT" "$SEQUENCE"
cp "$TMP/provenance.sshsig" "$DIST/Clinch.build-provenance.sshsig"

echo "PASS"
