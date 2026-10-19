#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TMP="$(mktemp -d -t clinch-release-layout-test)"
trap 'rm -rf "$TMP"' EXIT

make -C "$ROOT" -n _bundle VERSION=v0.2099.01.02.0304 UPDATE_SEQUENCE=1 \
  > "$TMP/native.out"
grep -Fq -- '--arch aarch64' "$TMP/native.out"
grep -Fq -- '--arch x86_64 --dmg-name-suffix x86_64' "$TMP/native.out"

make -C "$ROOT" -n _bundle VERSION=v0.2099.01.02.0304 UPDATE_SEQUENCE=1 \
  UNIVERSAL=1 > "$TMP/universal.out"
! grep -Fq -- '--arch aarch64' "$TMP/universal.out"
! grep -Fq -- '--arch x86_64' "$TMP/universal.out"
grep -Fq './script/bundle -c stable --selfsign;' "$TMP/universal.out"
codesign_identity="$(openssl x509 -in "$ROOT/resources/release/clinch-codesign-cert.pem" \
  -noout -fingerprint -sha1 | cut -d= -f2 | tr -d :)"
grep -Fq "export CLINCH_CODESIGN_IDENTITY=\"$codesign_identity\"" "$TMP/native.out"

# Release bundles must be signed by the committed certificate; stub `security` so the gate runs
# without the release workstation's keychain.
mkdir -p "$TMP/bin"
cat > "$TMP/bin/security" <<'EOF'
#!/usr/bin/env bash
[[ "$1" == find-identity ]] && printf '  1) %s "Clinch Code Signing"\n' "${TRUSTED_IDENTITY:-}"
exit 0
EOF
chmod +x "$TMP/bin/security"
touch "$TMP/codesign.keychain-db" "$TMP/codesign-keychain-password"
codesign_args=(
  CLINCH_CODESIGN_KEYCHAIN="$TMP/codesign.keychain-db"
  CLINCH_CODESIGN_KEYCHAIN_PASSWORD_FILE="$TMP/codesign-keychain-password"
)
export PATH="$TMP/bin:$PATH"
export TRUSTED_IDENTITY="$codesign_identity"

failure_root="$TMP/failure-root"
mkdir -p "$failure_root/script" "$failure_root/resources/release"
cp "$ROOT/Makefile" "$failure_root/Makefile"
cp "$ROOT/resources/release/clinch-codesign-cert.pem" "$failure_root/resources/release/"
cat > "$failure_root/script/bundle" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "${BUNDLE_CALLS:?}"
exit 42
EOF
chmod +x "$failure_root/script/bundle"
export BUNDLE_CALLS="$TMP/bundle-calls"
if TRUSTED_IDENTITY= make -C "$failure_root" _bundle VERSION=v0.2099.01.02.0304 \
    UPDATE_SEQUENCE=1 "${codesign_args[@]}" > "$TMP/untrusted.out" 2>&1; then
  echo "FAIL: untrusted code-signing identity was accepted" >&2
  exit 1
fi
grep -Fq 'not trusted for code signing' "$TMP/untrusted.out"
[[ ! -e "$BUNDLE_CALLS" ]]
if make -C "$failure_root" _bundle VERSION=v0.2099.01.02.0304 UPDATE_SEQUENCE=1 \
    "${codesign_args[@]}" > "$TMP/failure.out" 2>&1; then
  echo "FAIL: architecture bundle failure was masked" >&2
  exit 1
fi
grep -Fq -- '--arch aarch64' "$BUNDLE_CALLS"
! grep -Fq -- '--arch x86_64' "$BUNDLE_CALLS"

checksum_root="$TMP/checksum-root"
checksum_dir="$checksum_root/target/release-lto/bundle/osx"
mkdir -p "$checksum_dir"
cp "$ROOT/Makefile" "$checksum_root/Makefile"
for artifact in \
    Clinch.app.zip \
    Clinch.dmg \
    Clinch-x86_64.app.zip \
    Clinch-x86_64.dmg; do
  printf '%s\n' "$artifact" > "$checksum_dir/$artifact"
done
make -C "$checksum_root" _write-release-checksums UNIVERSAL=0 \
  > "$TMP/checksums.out"
for artifact in \
    Clinch.app.zip \
    Clinch.dmg \
    Clinch-x86_64.app.zip \
    Clinch-x86_64.dmg; do
  (cd "$checksum_dir" && shasum -a 256 -c "$artifact.sha256")
done

if make -C "$ROOT" _validate-release-layout \
    UNIVERSAL=invalid > "$TMP/invalid.out" 2>&1; then
  echo "FAIL: invalid release layout was accepted" >&2
  exit 1
fi
grep -Fq 'UNIVERSAL must be 0 or 1' "$TMP/invalid.out"

echo "PASS"
