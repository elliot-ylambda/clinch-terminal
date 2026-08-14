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

failure_root="$TMP/failure-root"
mkdir -p "$failure_root/script"
cp "$ROOT/Makefile" "$failure_root/Makefile"
cat > "$failure_root/script/bundle" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "${BUNDLE_CALLS:?}"
exit 42
EOF
chmod +x "$failure_root/script/bundle"
export BUNDLE_CALLS="$TMP/bundle-calls"
if make -C "$failure_root" _bundle VERSION=v0.2099.01.02.0304 UPDATE_SEQUENCE=1 \
    > "$TMP/failure.out" 2>&1; then
  echo "FAIL: architecture bundle failure was masked" >&2
  exit 1
fi
grep -Fq -- '--arch aarch64' "$BUNDLE_CALLS"
! grep -Fq -- '--arch x86_64' "$BUNDLE_CALLS"

if make -C "$ROOT" _validate-release-layout \
    UNIVERSAL=invalid > "$TMP/invalid.out" 2>&1; then
  echo "FAIL: invalid release layout was accepted" >&2
  exit 1
fi
grep -Fq 'UNIVERSAL must be 0 or 1' "$TMP/invalid.out"

echo "PASS"
