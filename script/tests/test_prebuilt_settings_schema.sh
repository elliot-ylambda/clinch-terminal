#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TMP="$(mktemp -d -t clinch-prebuilt-schema-test)"
trap 'rm -rf "$TMP"' EXIT
BIN="$TMP/bin"
mkdir -p "$BIN"

cat > "$BIN/uname" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' Linux
STUB
cat > "$BIN/cargo" <<'STUB'
#!/usr/bin/env bash
echo "unexpected cargo invocation: $*" >&2
exit 99
STUB
cat > "$TMP/generate-settings-schema" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" > "$GENERATOR_LOG"
output="${!#}"
printf '{"generated":true}\n' > "$output"
STUB
chmod +x "$BIN/uname" "$BIN/cargo" "$TMP/generate-settings-schema"

PATH="$BIN:$PATH" \
NO_LICENSES=1 \
GENERATOR_LOG="$TMP/generator.log" \
SETTINGS_SCHEMA_GENERATOR="$TMP/generate-settings-schema" \
  "$ROOT/script/prepare_bundled_resources" \
    "$TMP/resources" stable release-lto > "$TMP/output.log"

grep -Fq "Using prebuilt settings schema generator $TMP/generate-settings-schema" \
  "$TMP/output.log"
grep -Fq -- '--channel stable' "$TMP/generator.log"
grep -Fq '"generated":true' "$TMP/resources/settings_schema.json"

echo "PASS"
