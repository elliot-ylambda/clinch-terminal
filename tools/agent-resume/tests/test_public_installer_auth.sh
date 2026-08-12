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
printf '%s\n' "$*" >> "$CLINCH_TEST_CURL_ARGS_LOG"
case "$url" in
  */Clinch.update.json) cp "$CLINCH_TEST_MANIFEST" "$output" ;;
  */Clinch.update.sshsig) cp "$CLINCH_TEST_SIGNATURE" "$output" ;;
  */Clinch.app.zip|*/Clinch-x86_64.app.zip) exit 22 ;;
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
cat > "$TMP/uname" <<'SH'
#!/bin/sh
set -eu
case "${1:-}" in
  -s) printf '%s\n' Darwin ;;
  -m) printf '%s\n' "${CLINCH_TEST_UNAME_MACHINE:-arm64}" ;;
  *) exit 2 ;;
esac
SH
cat > "$TMP/sysctl" <<'SH'
#!/bin/sh
set -eu
[ "$*" = '-in hw.optional.arm64' ] || exit 2
printf '%s\n' "${CLINCH_TEST_ARM64_SUPPORTED:-1}"
SH
chmod +x "$TMP/curl" "$TMP/lsappinfo" "$TMP/ps" "$TMP/uname" "$TMP/sysctl"

run_installer() {
  local manifest="$1" signature="$2" output="$3"
  local installer="${4:-$ROOT/install.sh}"
  : > "$TMP/requests"
  : > "$TMP/curl-args"
  if HOME="$TMP/home" \
      CLINCH_CURL_BIN="$TMP/curl" \
      CLINCH_LSAPPINFO_BIN="$TMP/lsappinfo" \
      CLINCH_PS_BIN="$TMP/ps" \
      CLINCH_UNAME_BIN="$TMP/uname" \
      CLINCH_SYSCTL_BIN="$TMP/sysctl" \
      CLINCH_TEST_REQUEST_LOG="$TMP/requests" \
      CLINCH_TEST_CURL_ARGS_LOG="$TMP/curl-args" \
      CLINCH_TEST_MANIFEST="$manifest" \
      CLINCH_TEST_SIGNATURE="$signature" \
      "$installer" --version v0.0.99 >"$output" 2>&1; then
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
grep 'Clinch.app.zip' "$TMP/curl-args" | grep -q -- '--progress-bar' \
  || { echo "FAIL: archive download did not enable visible progress"; cat "$TMP/curl-args"; exit 1; }
grep 'Clinch.app.zip' "$TMP/curl-args" | grep -q -- '--max-time 1800' \
  || { echo "FAIL: archive download retained the short metadata timeout"; cat "$TMP/curl-args"; exit 1; }
grep -q 'Downloading the exact v0.0.99 universal app (1 MiB)' "$TMP/valid-output" \
  || { echo "FAIL: archive download did not report its authenticated size"; cat "$TMP/valid-output"; exit 1; }

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

# New manifests bind one thin archive per architecture. The hardware probe wins
# even when an Apple Silicon Mac is running an x86_64 process under Rosetta.
ssh-keygen -q -t ed25519 -N '' -f "$TMP/native-release-key"
native_public="$(awk '{print $1, $2}' "$TMP/native-release-key.pub")"
/usr/bin/sed \
  "s|^RELEASE_PUBLIC_KEY=.*|RELEASE_PUBLIC_KEY=\"$native_public\"|" \
  "$ROOT/install.sh" > "$TMP/native-install.sh"
/bin/chmod +x "$TMP/native-install.sh"
/usr/bin/python3 - "$TMP/native-manifest.json" <<'PY'
import json
import pathlib

version = "v0.0.99"
base = (
    "https://github.com/elliot-ylambda/clinch-terminal/releases/download/"
    f"{version}/"
)
arm = {
    "name": "Clinch.app.zip",
    "sha256": "0" * 64,
    "size": 1,
    "url": base + "Clinch.app.zip",
}
manifest = {
    "architectures": ["arm64", "x86_64"],
    "archive": arm,
    "archives": {
        "arm64": arm,
        "x86_64": {
            "name": "Clinch-x86_64.app.zip",
            "sha256": "1" * 64,
            "size": 1,
            "url": base + "Clinch-x86_64.app.zip",
        },
    },
    "bundle_id": "sh.clinch.Clinch",
    "minimum_macos_version": "14.0",
    "notarized": False,
    "release_notes": "Native installer fixture.",
    "release_url": (
        "https://github.com/elliot-ylambda/clinch-terminal/releases/tag/" + version
    ),
    "repository": "elliot-ylambda/clinch-terminal",
    "rollback": False,
    "schema_version": 1,
    "sequence": 1,
    "signing_key_id": "a353cda3ad59f128",
    "tag": version,
    "version": version,
}
pathlib.Path(__import__("sys").argv[1]).write_text(
    json.dumps(manifest, sort_keys=True, separators=(",", ":")) + "\n",
    encoding="utf-8",
)
PY
ssh-keygen -Y sign -f "$TMP/native-release-key" -n clinch-install - \
  < "$TMP/native-manifest.json" > "$TMP/native-manifest.sshsig"

CLINCH_TEST_UNAME_MACHINE=x86_64 CLINCH_TEST_ARM64_SUPPORTED=1 \
  run_installer "$TMP/native-manifest.json" "$TMP/native-manifest.sshsig" \
    "$TMP/native-arm-output" "$TMP/native-install.sh"
grep -q '/v0.0.99/Clinch.app.zip$' "$TMP/requests"
! grep -q '/v0.0.99/Clinch-x86_64.app.zip$' "$TMP/requests"
grep -q 'v0.0.99 Apple Silicon app (1 MiB)' "$TMP/native-arm-output"

CLINCH_TEST_UNAME_MACHINE=x86_64 CLINCH_TEST_ARM64_SUPPORTED=0 \
  run_installer "$TMP/native-manifest.json" "$TMP/native-manifest.sshsig" \
    "$TMP/native-intel-output" "$TMP/native-install.sh"
grep -q '/v0.0.99/Clinch-x86_64.app.zip$' "$TMP/requests"
! grep -q '/v0.0.99/Clinch.app.zip$' "$TMP/requests"
grep -q 'v0.0.99 Intel app (1 MiB)' "$TMP/native-intel-output"

echo "PASS"
