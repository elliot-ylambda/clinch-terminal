#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TMP="$(mktemp -d -t clinch-sequence-test)"
trap 'rm -rf "$TMP"' EXIT
FIXTURE="$TMP/repo"
mkdir -p "$FIXTURE/script" "$FIXTURE/resources/release" "$TMP/bin" "$TMP/source"
cp "$ROOT/script/next-clinch-update-sequence" "$FIXTURE/script/"

LATEST_VERSION=v0.2026.07.15.2000
LATEST_SEQUENCE=200
printf '{"tag_name":"%s"}\n' "$LATEST_VERSION" > "$TMP/source/latest.json"
printf '{"sequence":%s}\n' "$LATEST_SEQUENCE" > "$TMP/source/manifest.json"

ssh-keygen -q -t ed25519 -N '' -f "$TMP/release-key"
printf 'clinch-release %s\n' "$(awk '{print $1, $2}' "$TMP/release-key.pub")" \
  > "$FIXTURE/resources/release/clinch-release-allowed-signers"
ssh-keygen -Y sign -f "$TMP/release-key" -n clinch-install - \
  < "$TMP/source/manifest.json" > "$TMP/source/manifest.sshsig"

cat > "$TMP/bin/curl" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
output=
url=
while (( $# )); do
  case "$1" in
    -o)
      output="$2"
      shift 2
      ;;
    http://*|https://*)
      url="$1"
      shift
      ;;
    *)
      shift
      ;;
  esac
done
case "$url" in
  */releases/latest)
    [[ "${SEQUENCE_NO_RELEASE:-0}" != 1 ]] || exit 22
    cp "$SEQUENCE_FIXTURE_SOURCE/latest.json" "$output"
    ;;
  */Clinch.update.json) cp "$SEQUENCE_FIXTURE_SOURCE/manifest.json" "$output" ;;
  */Clinch.update.sshsig) cp "$SEQUENCE_FIXTURE_SOURCE/manifest.sshsig" "$output" ;;
  *) echo "unexpected URL: $url" >&2; exit 2 ;;
esac
STUB
chmod +x "$FIXTURE/script/next-clinch-update-sequence" "$TMP/bin/curl"

run_sequence() {
  (cd "$FIXTURE" && env \
    PATH="$TMP/bin:$PATH" \
    SEQUENCE_FIXTURE_SOURCE="$TMP/source" \
    ./script/next-clinch-update-sequence "$@")
}

output="$(run_sequence verify v0.2026.07.15.3000 201)"
grep -Fq 'sequence 201 is newer' <<< "$output"

if run_sequence verify v0.2026.07.15.3000 200 > "$TMP/stale-sequence.out" 2>&1; then
  echo "FAIL: stale sequence was accepted" >&2
  exit 1
fi
grep -Fq 'is not newer than 200' "$TMP/stale-sequence.out"

if run_sequence verify "$LATEST_VERSION" 201 > "$TMP/stale-version.out" 2>&1; then
  echo "FAIL: stale version was accepted" >&2
  exit 1
fi
grep -Fq 'is not newer than' "$TMP/stale-version.out"

next="$(run_sequence)"
[[ "$next" =~ ^[0-9]+$ && "$next" -gt "$LATEST_SEQUENCE" ]]

bootstrap="$(
  CLINCH_ALLOW_NO_PREVIOUS_RELEASE=1 SEQUENCE_NO_RELEASE=1 run_sequence
)"
[[ "$bootstrap" =~ ^[0-9]+$ && "$bootstrap" -gt 0 ]]
bootstrap_verify="$(
  CLINCH_ALLOW_NO_PREVIOUS_RELEASE=1 SEQUENCE_NO_RELEASE=1 \
    run_sequence verify v0.2026.07.17.2000 "$bootstrap"
)"
grep -Fq 'authenticated bootstrap release' <<< "$bootstrap_verify"

if SEQUENCE_NO_RELEASE=1 run_sequence > "$TMP/no-release.out" 2>&1; then
  echo "FAIL: missing release was accepted without bootstrap authorization" >&2
  exit 1
fi
grep -Fq 'could not resolve the latest release' "$TMP/no-release.out"

echo "PASS"
