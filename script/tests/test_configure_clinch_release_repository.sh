#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TMP="$(mktemp -d -t clinch-repository-config-test)"
trap 'rm -rf "$TMP"' EXIT
FIXTURE="$TMP/repo"
BIN="$TMP/bin"
mkdir -p "$FIXTURE/script" "$FIXTURE/resources/release" "$BIN"
cp "$ROOT/script/configure-clinch-release-repository" "$FIXTURE/script/"

ssh-keygen -q -t ed25519 -N '' -f "$TMP/release-key"
ssh-keygen -q -t ed25519 -N '' -f "$TMP/wrong-release-key"
printf 'clinch-release %s\n' "$(awk '{print $1, $2}' "$TMP/release-key.pub")" \
  > "$FIXTURE/resources/release/clinch-release-allowed-signers"
printf 'fixture update key\n' > "$TMP/update-key.pem"

cat > "$FIXTURE/script/clinch-update-manifest" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf 'manifest %s\n' "$*" >> "$CONFIG_LOG"
[[ "${1:-}" == verify-key && -f "${2:-}" && "${UPDATE_KEY_FAIL:-0}" != 1 ]]
STUB

cat > "$BIN/gh" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf 'gh %s\n' "$*" >> "$CONFIG_LOG"
case "${1:-} ${2:-}" in
  'auth status')
    exit 0
    ;;
  'api repos/elliot-ylambda/clinch-terminal/environments/public-release')
    [[ "${ENVIRONMENT_EXISTS:-1}" == 1 ]]
    ;;
  'api --method')
    if [[ " $* " == *' DELETE repos/elliot-ylambda/clinch-terminal/environments/public-release '* ]]; then
      touch "$ENVIRONMENT_DELETED_STATE"
    fi
    ;;
  'secret list')
    printf '%s\n' CLINCH_RELEASE_SIGNING_KEY CLINCH_UPDATE_SIGNING_KEY UNRELATED_FIXTURE
    ;;
  'secret delete')
    printf '%s\n' "$3" >> "$DELETED_SECRETS_LOG"
    ;;
  *)
    echo "unexpected gh invocation: $*" >&2
    exit 2
    ;;
esac
STUB
chmod +x "$FIXTURE/script/"* "$BIN/gh"

run_configure() {
  (cd "$FIXTURE" && env \
    PATH="$BIN:$PATH" \
    CONFIG_LOG="$TMP/config.log" \
    DELETED_SECRETS_LOG="$TMP/deleted-secrets.log" \
    ENVIRONMENT_DELETED_STATE="$TMP/environment-deleted" \
    CLINCH_UPDATE_SIGNING_KEY="$TMP/update-key.pem" \
    CLINCH_RELEASE_SIGNING_KEY="$TMP/release-key" \
    "$@" \
    ./script/configure-clinch-release-repository)
}

reset_state() {
  : > "$TMP/config.log"
  : > "$TMP/deleted-secrets.log"
  rm -f "$TMP/environment-deleted"
}

assert_no_release_credentials_deleted() {
  [[ ! -s "$TMP/deleted-secrets.log" && ! -e "$TMP/environment-deleted" ]]
}

reset_state
run_configure >/dev/null
grep -Fxq CLINCH_RELEASE_SIGNING_KEY "$TMP/deleted-secrets.log"
grep -Fxq CLINCH_UPDATE_SIGNING_KEY "$TMP/deleted-secrets.log"
if grep -Fxq UNRELATED_FIXTURE "$TMP/deleted-secrets.log"; then
  echo "FAIL: an unrelated environment secret was explicitly deleted" >&2
  exit 1
fi
[[ -e "$TMP/environment-deleted" ]]

reset_state
if run_configure CLINCH_UPDATE_SIGNING_KEY="$TMP/missing-update-key.pem" \
    > "$TMP/missing-update.out" 2>&1; then
  echo "FAIL: environment deletion accepted a missing update key" >&2
  exit 1
fi
assert_no_release_credentials_deleted

reset_state
if run_configure CLINCH_RELEASE_SIGNING_KEY="$TMP/wrong-release-key" \
    > "$TMP/wrong-release.out" 2>&1; then
  echo "FAIL: environment deletion accepted the wrong release key" >&2
  exit 1
fi
assert_no_release_credentials_deleted

reset_state
if run_configure UPDATE_KEY_FAIL=1 > "$TMP/wrong-update.out" 2>&1; then
  echo "FAIL: environment deletion accepted the wrong update key" >&2
  exit 1
fi
assert_no_release_credentials_deleted

reset_state
run_configure ENVIRONMENT_EXISTS=0 >/dev/null
assert_no_release_credentials_deleted

echo "PASS"
