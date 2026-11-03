#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
CHECK="$ROOT/script/check-ui-literals"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/app/src" "$TMP/script"
echo 'app/src/old.rs' > "$TMP/script/ui-literals-allowlist.txt"
echo 'let c = ColorU::new(1, 2, 3, 4);' > "$TMP/app/src/old.rs"

"$CHECK" "$TMP"

echo 'let r = Radius::Pixels(4.);' > "$TMP/app/src/bad_radius.rs"
if "$CHECK" "$TMP" 2>/dev/null; then echo "FAIL: literal radius passed" >&2; exit 1; fi
rm "$TMP/app/src/bad_radius.rs"

echo 'let r = Radius::Pixels(1.);' > "$TMP/app/src/hairline.rs"
"$CHECK" "$TMP"

echo 'let c = ColorU::new(9, 9, 9, 9);' > "$TMP/app/src/new_color.rs"
if "$CHECK" "$TMP" 2>/dev/null; then echo "FAIL: new color file passed" >&2; exit 1; fi
rm "$TMP/app/src/new_color.rs"

echo 'nothing here' > "$TMP/app/src/old.rs"
if "$CHECK" "$TMP" 2>/dev/null; then echo "FAIL: stale allowlist entry passed" >&2; exit 1; fi

echo "PASS"
