#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
FIXTURES="$ROOT/tools/agent-resume/tests/fixtures"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

cat > "$TMP/curl" <<'SH'
#!/bin/sh
set -eu
output=""
previous=""
url=""
for argument in "$@"; do
  if [ "$previous" = "-o" ]; then
    output=$argument
  fi
  previous=$argument
  url=$argument
done
[ -n "$output" ] || exit 2
printf '%s\n' "$url" >> "$CLINCH_TEST_REQUEST_LOG"
case "$url" in
  */Clinch.update.json) cp "$CLINCH_TEST_MANIFEST" "$output" ;;
  */Clinch.update.sshsig) cp "$CLINCH_TEST_SIGNATURE" "$output" ;;
  */Clinch.app.zip) exit 22 ;;
  *) exit 22 ;;
esac
SH
cat > "$TMP/lsappinfo" <<'SH'
#!/bin/sh
exit 0
SH
cat > "$TMP/ps" <<'SH'
#!/bin/sh
exit 0
SH
chmod +x "$TMP/curl" "$TMP/lsappinfo" "$TMP/ps"

run_installer() {
  local manifest="$1" signature="$2" output="$3"
  : > "$TMP/requests"
  if HOME="$TMP/home" \
      CLINCH_CURL_BIN="$TMP/curl" \
      CLINCH_LSAPPINFO_BIN="$TMP/lsappinfo" \
      CLINCH_PS_BIN="$TMP/ps" \
      CLINCH_TEST_REQUEST_LOG="$TMP/requests" \
      CLINCH_TEST_MANIFEST="$manifest" \
      CLINCH_TEST_SIGNATURE="$signature" \
      "$ROOT/install.sh" --version v0.0.99 >"$output" 2>&1; then
    echo "FAIL: fixture unexpectedly completed an install"
    exit 1
  fi
}

# A correct signature and manifest advance to the archive request.
run_installer \
  "$FIXTURES/installer-manifest.json" \
  "$FIXTURES/installer-manifest.sshsig" \
  "$TMP/valid-output"
grep -q '/v0.0.99/Clinch.app.zip$' "$TMP/requests" \
  || { echo "FAIL: valid authenticated metadata did not reach the exact archive"; cat "$TMP/valid-output"; exit 1; }
grep -q 'archive download failed' "$TMP/valid-output" \
  || { echo "FAIL: valid fixture failed before the expected archive stub"; cat "$TMP/valid-output"; exit 1; }

# Any manifest byte change must fail before an archive is requested.
cp "$FIXTURES/installer-manifest.json" "$TMP/tampered.json"
printf ' ' >> "$TMP/tampered.json"
run_installer \
  "$TMP/tampered.json" \
  "$FIXTURES/installer-manifest.sshsig" \
  "$TMP/tampered-output"
! grep -q 'Clinch.app.zip' "$TMP/requests" \
  || { echo "FAIL: tampered metadata triggered an archive request"; exit 1; }
grep -q 'release signature verification failed' "$TMP/tampered-output" \
  || { echo "FAIL: tampered metadata did not report signature failure"; exit 1; }

for kind in wrong-namespace wrong-key; do
  run_installer \
    "$FIXTURES/installer-manifest.json" \
    "$FIXTURES/installer-manifest-$kind.sshsig" \
    "$TMP/$kind-output"
  ! grep -q 'Clinch.app.zip' "$TMP/requests" \
    || { echo "FAIL: $kind signature triggered an archive request"; exit 1; }
  grep -q 'release signature verification failed' "$TMP/$kind-output" \
    || { echo "FAIL: $kind signature did not fail closed"; exit 1; }
done

echo "PASS"
