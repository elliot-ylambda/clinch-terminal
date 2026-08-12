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

if make -C "$ROOT" _validate-release-layout \
    UNIVERSAL=invalid > "$TMP/invalid.out" 2>&1; then
  echo "FAIL: invalid release layout was accepted" >&2
  exit 1
fi
grep -Fq 'UNIVERSAL must be 0 or 1' "$TMP/invalid.out"

echo "PASS"
