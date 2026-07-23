#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TMP="$(mktemp -d -t clinch-release-dispatch-test)"
trap 'rm -rf "$TMP"' EXIT
FIXTURE="$TMP/repo"
BIN="$TMP/bin"
mkdir -p "$FIXTURE/script" "$FIXTURE/resources/release" "$BIN"
cp "$ROOT/script/dispatch-clinch-release" "$FIXTURE/script/"

COMMIT=0123456789abcdef0123456789abcdef01234567
LATEST=v0.2026.07.15.2000
VERSION=v0.2026.07.15.3000
SEQUENCE=4102441440

ssh-keygen -q -t ed25519 -N '' -f "$TMP/release-key"
printf 'clinch-release %s\n' "$(awk '{print $1, $2}' "$TMP/release-key.pub")" \
  > "$FIXTURE/resources/release/clinch-release-allowed-signers"
printf 'fixture\n' > "$TMP/update-key.pem"

cat > "$FIXTURE/script/require-latest-main" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' 'require-latest-main' >> "$OPS_LOG"
STUB

cat > "$FIXTURE/script/clinch-update-manifest" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
[[ "${1:-}" == verify-key ]] || exit 2
printf '%s\n' 'verify-update-key' >> "$OPS_LOG"
STUB

cat > "$FIXTURE/script/next-clinch-update-sequence" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" == verify ]]; then
  printf '%s\n' 'verify-sequence' >> "$OPS_LOG"
  [[ "${MUTATE_DRAFT_BEFORE_PUBLISH:-0}" != 1 ]] || touch "$DRAFT_MUTATED_STATE"
  [[ "${MUTATE_TAG_BEFORE_PUBLISH:-0}" != 1 ]] || touch "$TAG_MUTATED_STATE"
  [[ "${MUTATE_MAIN_BEFORE_PUBLISH:-0}" != 1 ]] || touch "$MAIN_MUTATED_STATE"
  exit 0
fi
printf '%s\n' 'derive-sequence' >> "$OPS_LOG"
printf '%s\n' "$FIXTURE_SEQUENCE"
STUB

cat > "$FIXTURE/script/assemble-clinch-release-stage" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
version="$1"
stage="target/release-stage/$version"
dist="$stage/dist"
mkdir -p "$dist"
files=(
  Clinch.app.zip Clinch.app.zip.sha256 Clinch.source.tar.gz
  Clinch.source.tar.gz.sha256 Clinch.build-provenance.json
  Clinch.build-provenance.sshsig Clinch.checksums.sshsig Clinch.checksums.txt
  Clinch.dmg Clinch.dmg.sha256 Clinch.release-validation.json Clinch.sbom.cdx.json
  Clinch.update.json Clinch.update.sig Clinch.update.sshsig
  clinch-release-allowed-signers install.sh uninstall.sh
)
for file in "${files[@]}"; do
  printf 'fixture %s\n' "$file" > "$dist/$file"
done
printf 'fixture notes\n' > "$stage/release-notes.md"
printf '%s\n' 'assemble-stage' >> "$OPS_LOG"
printf '%s\n' "$stage"
STUB

cat > "$FIXTURE/script/verify-clinch-release-stage" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf 'verify-stage %s %s %s\n' "$2" "$3" "$4" >> "$OPS_LOG"
if [[ "$1" == *clinch-release-publish* && "${VERIFY_REMOTE_STAGE_FAIL:-0}" == 1 ]]; then
  exit 1
fi
[[ "${VERIFY_STAGE_FAIL:-0}" != 1 ]]
STUB

cat > "$BIN/make" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf 'make %s\n' "$*" >> "$OPS_LOG"
if [[ "${MAKE_FAIL_ON:-}" == "${1:-}" ]]; then
  exit 37
fi
STUB

cat > "$BIN/git" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf 'git %s\n' "$*" >> "$GIT_LOG"
args=("$@")
while [[ "${args[0]:-}" == -c ]]; do
  args=("${args[@]:2}")
done
command="${args[0]:-}"
case "$command" in
  remote)
    [[ "${args[1]:-}" == get-url ]] || exit 2
    printf '%s\n' 'git@github.com:elliot-ylambda/clinch-terminal.git'
    ;;
  status)
    ;;
  rev-parse)
    case "${args[*]:1}" in
      '--abbrev-ref HEAD') printf '%s\n' main ;;
      'HEAD') printf '%s\n' "$FIXTURE_COMMIT" ;;
      '--short HEAD') printf '%s\n' "${FIXTURE_COMMIT:0:7}" ;;
      'clinch/main') printf '%s\n' "$FIXTURE_COMMIT" ;;
      'refs/tags/'*) printf '%s\n' "$FIXTURE_TAG_OID" ;;
      '-q --verify refs/tags/'*)
        [[ -f "$LOCAL_TAG_STATE" ]]
        ;;
      *) echo "unexpected git rev-parse: ${args[*]}" >&2; exit 2 ;;
    esac
    ;;
  ls-remote)
    if [[ "${args[*]}" == *'--tags --refs'* ]]; then
      printf '%s\trefs/tags/%s\n' \
        aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa "$FIXTURE_LATEST"
    elif [[ "${args[*]}" == *'--tags'* ]]; then
      [[ -f "$REMOTE_TAG_STATE" ]] || exit 2
      if [[ -f "$TAG_MUTATED_STATE" ]]; then
        tag_oid=dddddddddddddddddddddddddddddddddddddddd
      else
        tag_oid="$FIXTURE_TAG_OID"
      fi
      printf '%s\trefs/tags/%s\n' "$tag_oid" "$FIXTURE_VERSION"
    else
      if [[ -f "$MAIN_MUTATED_STATE" ]]; then
        printf '%s\trefs/heads/main\n' bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
      else
        printf '%s\trefs/heads/main\n' "$FIXTURE_COMMIT"
      fi
    fi
    ;;
  fetch)
    touch "$LOCAL_TAG_STATE"
    ;;
  tag)
    touch "$LOCAL_TAG_STATE"
    printf '%s\n' 'git tag' >> "$OPS_LOG"
    ;;
  verify-tag)
    [[ -f "$LOCAL_TAG_STATE" && "${GIT_VERIFY_TAG_FAIL:-0}" != 1 ]]
    ;;
  rev-list)
    printf '%s\n' "${TAG_COMMIT:-$FIXTURE_COMMIT}"
    ;;
  push)
    touch "$REMOTE_TAG_STATE"
    printf '%s\n' 'git push-tag' >> "$OPS_LOG"
    ;;
  *)
    echo "unexpected git invocation: ${args[*]}" >&2
    exit 2
    ;;
esac
STUB

cat > "$BIN/gh" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf 'gh %s\n' "$*" >> "$GH_LOG"
if [[ -n "${GH_TRANSIENT_MATCH:-}" && "$*" == *"$GH_TRANSIENT_MATCH"* ]]; then
  transient_count=0
  [[ ! -f "$GH_TRANSIENT_STATE" ]] || transient_count="$(< "$GH_TRANSIENT_STATE")"
  if (( transient_count < ${GH_TRANSIENT_FAILURES:-1} )); then
    printf '%s\n' "$((transient_count + 1))" > "$GH_TRANSIENT_STATE"
    echo 'gh: No server is currently available to service your request. (HTTP 503)' >&2
    exit 1
  fi
fi
case "${1:-} ${2:-}" in
  'auth status')
    exit 0
    ;;
  'api repos/elliot-ylambda/clinch-terminal/releases?per_page=100')
    [[ "${FIXTURE_NO_PREVIOUS_RELEASE:-0}" == 1 ]] || printf '%s\n' "$FIXTURE_LATEST"
    ;;
  'release view')
    if [[ ! -f "$DRAFT_STATE" ]]; then
      echo 'release not found' >&2
      exit 1
    fi
    if [[ -f "$DRAFT_MUTATED_STATE" ]]; then
      draft_body=mutated
    else
      draft_body=fixture
    fi
    draft_value="${DRAFT_VALUE:-true}"
    [[ ! -f "$PUBLISHED_STATE" ]] || draft_value=false
    printf '{"databaseId":1,"isDraft":%s,"tagName":"%s",' \
      "$draft_value" "$FIXTURE_VERSION"
    printf '"name":"fixture","body":"%s","targetCommitish":"%s","assets":[' \
      "$draft_body" "$FIXTURE_COMMIT"
    separator=
    for name in \
      Clinch.app.zip Clinch.app.zip.sha256 Clinch.source.tar.gz \
      Clinch.source.tar.gz.sha256 Clinch.build-provenance.json \
      Clinch.build-provenance.sshsig Clinch.checksums.sshsig Clinch.checksums.txt \
      Clinch.dmg Clinch.dmg.sha256 Clinch.release-validation.json Clinch.sbom.cdx.json \
      Clinch.update.json Clinch.update.sig Clinch.update.sshsig \
      clinch-release-allowed-signers install.sh uninstall.sh; do
      printf '%s{"id":"fixture-%s","name":"%s","size":1,' \
        "$separator" "$name" "$name"
      printf '"updatedAt":"fixture","digest":"sha256:fixture"}'
      separator=,
    done
    if [[ -n "${GH_EXTRA_ASSET:-}" ]]; then
      printf '%s{"id":"fixture-extra","name":"%s","size":1,' \
        "$separator" "$GH_EXTRA_ASSET"
      printf '"updatedAt":"fixture","digest":"sha256:fixture"}'
    fi
    printf ']}\n'
    ;;
  'release create')
    if [[ "${GH_CREATE_AMBIGUOUS_ONCE:-0}" == 1 && ! -f "$GH_CREATE_AMBIGUOUS_STATE" ]]; then
      touch "$DRAFT_STATE" "$GH_CREATE_AMBIGUOUS_STATE"
      printf '%s\n' 'gh release-create-ambiguous' >> "$OPS_LOG"
      echo 'gh: No server is currently available to service your request. (HTTP 503)' >&2
      exit 1
    fi
    if [[ -f "$DRAFT_STATE" ]]; then
      echo 'gh: Validation Failed (HTTP 422)' >&2
      exit 1
    fi
    touch "$DRAFT_STATE"
    printf '%s\n' 'gh release-create' >> "$OPS_LOG"
    printf 'https://github.com/elliot-ylambda/clinch-terminal/releases/tag/%s\n' \
      "$FIXTURE_VERSION"
    ;;
  'release edit')
    if [[ " $* " == *' --draft=false '* ]]; then
      printf '%s\n' 'gh release-publish' >> "$OPS_LOG"
      if [[ "${GH_PUBLISH_AMBIGUOUS_ONCE:-0}" == 1 && ! -f "$GH_PUBLISH_AMBIGUOUS_STATE" ]]; then
        touch "$PUBLISHED_STATE" "$GH_PUBLISH_AMBIGUOUS_STATE"
        echo 'gh: No server is currently available to service your request. (HTTP 503)' >&2
        exit 1
      fi
      if [[ -f "$GH_PUBLISH_AMBIGUOUS_STATE" ]]; then
        echo 'gh: Validation Failed (HTTP 422)' >&2
        exit 1
      fi
      [[ "${GH_PUBLISH_FAIL:-0}" != 1 ]] || exit 42
      touch "$PUBLISHED_STATE"
    else
      printf '%s\n' 'gh release-edit' >> "$OPS_LOG"
    fi
    ;;
  'release upload')
    printf '%s\n' 'gh release-upload' >> "$OPS_LOG"
    ;;
  'release download')
    destination=
    while (( $# )); do
      if [[ "$1" == --dir ]]; then
        destination="$2"
        break
      fi
      shift
    done
    [[ -n "$destination" ]] || exit 2
    cp target/release-stage/"$FIXTURE_VERSION"/dist/* "$destination/"
    printf '%s\n' 'gh release-download' >> "$OPS_LOG"
    [[ "${MUTATE_DRAFT_ON_DOWNLOAD:-0}" != 1 ]] || touch "$DRAFT_MUTATED_STATE"
    ;;
  'issue comment')
    ;;
  *)
    echo "unexpected gh invocation: $*" >&2
    exit 2
    ;;
esac
STUB

for command in cargo cargo-nextest cargo-deny cargo-bundle cargo-about syft hdiutil; do
  cat > "$BIN/$command" <<'STUB'
#!/usr/bin/env bash
exit 0
STUB
done
chmod +x "$FIXTURE/script/"* "$BIN/"*

BASE_ENV=(
  PATH="$BIN:$PATH"
  OPS_LOG="$TMP/ops.log"
  GIT_LOG="$TMP/git.log"
  GH_LOG="$TMP/gh.log"
  LOCAL_TAG_STATE="$TMP/local-tag"
  REMOTE_TAG_STATE="$TMP/remote-tag"
  DRAFT_STATE="$TMP/draft"
  DRAFT_MUTATED_STATE="$TMP/draft-mutated"
  TAG_MUTATED_STATE="$TMP/tag-mutated"
  MAIN_MUTATED_STATE="$TMP/main-mutated"
  PUBLISHED_STATE="$TMP/published"
  GH_TRANSIENT_STATE="$TMP/gh-transient-count"
  GH_CREATE_AMBIGUOUS_STATE="$TMP/gh-create-ambiguous"
  GH_PUBLISH_AMBIGUOUS_STATE="$TMP/gh-publish-ambiguous"
  FIXTURE_COMMIT="$COMMIT"
  FIXTURE_TAG_OID=cccccccccccccccccccccccccccccccccccccccc
  FIXTURE_LATEST="$LATEST"
  FIXTURE_VERSION="$VERSION"
  FIXTURE_SEQUENCE="$SEQUENCE"
  CLINCH_UPDATE_SIGNING_KEY="$TMP/update-key.pem"
  CLINCH_RELEASE_SIGNING_KEY="$TMP/release-key"
  VERSION="$VERSION"
  CLINCH_AUTO_VERSION=0
)

reset_state() {
  rm -f \
    "$TMP/local-tag" "$TMP/remote-tag" "$TMP/draft" "$TMP/draft-mutated" \
    "$TMP/tag-mutated" "$TMP/main-mutated" "$TMP/published"
  rm -f \
    "$TMP/gh-transient-count" "$TMP/gh-create-ambiguous" "$TMP/gh-publish-ambiguous"
  : > "$TMP/ops.log"
  : > "$TMP/git.log"
  : > "$TMP/gh.log"
  rm -rf "$FIXTURE/target"
}

run_release() {
  (cd "$FIXTURE" && env "${BASE_ENV[@]}" "$@" ./script/dispatch-clinch-release)
}

run_release_tty() {
  local confirmation="$1"
  shift
  (cd "$FIXTURE" && env "${BASE_ENV[@]}" "$@" \
    RELEASE_CONFIRMATION="$confirmation" \
    RELEASE_SCRIPT="$FIXTURE/script/dispatch-clinch-release" \
    /usr/bin/python3 - <<'PY'
import errno
import fcntl
import os
import pty

pid, master = pty.fork()
if pid == 0:
    if os.environ.get("NONBLOCKING_STDIN") == "1":
        flags = fcntl.fcntl(0, fcntl.F_GETFL)
        fcntl.fcntl(0, fcntl.F_SETFL, flags | os.O_NONBLOCK)
    script = os.environ["RELEASE_SCRIPT"]
    os.execve(script, [script], os.environ)

prompt = b"Type exactly: PUBLISH "
recent = b""
confirmation_sent = False
while True:
    try:
        chunk = os.read(master, 4096)
    except OSError as error:
        if error.errno == errno.EIO:
            break
        raise
    if not chunk:
        break
    os.write(1, chunk)
    if not confirmation_sent:
        recent = (recent + chunk)[-4096:]
        if prompt in recent:
            os.write(master, (os.environ["RELEASE_CONFIRMATION"] + "\n").encode())
            confirmation_sent = True

_, status = os.waitpid(pid, 0)
raise SystemExit(os.waitstatus_to_exitcode(status))
PY
  )
}

assert_no_remote_mutation() {
  if grep -Eq 'git (tag|push-tag)|gh (release-|workflow-run)' "$TMP/ops.log"; then
    echo "FAIL: remote release state changed" >&2
    cat "$TMP/ops.log" >&2
    exit 1
  fi
}

reset_state
run_release_tty "PUBLISH $VERSION ${COMMIT:0:12}" NONBLOCKING_STDIN=1 > "$TMP/happy.out"
grep -Fq "Locally built, verified, signed, and published $VERSION" "$TMP/happy.out"
grep -Fq 'make release-check' "$TMP/ops.log"
grep -Fq "make _verify VERSION=$VERSION UPDATE_SEQUENCE=$SEQUENCE UNIVERSAL=1" "$TMP/ops.log"
grep -Fq 'git tag' "$TMP/ops.log"
grep -Fq 'git push-tag' "$TMP/ops.log"
grep -Fq 'gh release-create' "$TMP/ops.log"
grep -Fq 'gh release-download' "$TMP/ops.log"
grep -Fq 'verify-sequence' "$TMP/ops.log"
grep -Fq 'gh release-publish' "$TMP/ops.log"
grep -Fq "gh release view $VERSION" "$TMP/gh.log"
[[ -f "$TMP/published" ]]
if grep -Fq 'workflow run' "$TMP/gh.log"; then
  echo "FAIL: local release dispatched GitHub Actions" >&2
  exit 1
fi

reset_state
run_release_tty "PUBLISH $VERSION ${COMMIT:0:12}" \
  FIXTURE_NO_PREVIOUS_RELEASE=1 > "$TMP/bootstrap.out"
grep -Fq "Locally built, verified, signed, and published $VERSION" "$TMP/bootstrap.out"
grep -Fq 'gh release-publish' "$TMP/ops.log"

reset_state
run_release_tty "PUBLISH $VERSION ${COMMIT:0:12}" \
  GH_TRANSIENT_MATCH='releases?per_page=100' GH_TRANSIENT_FAILURES=2 \
  CLINCH_GH_RETRY_DELAY_SECONDS=0 > "$TMP/transient-read.out" 2>&1
grep -Fq "Locally built, verified, signed, and published $VERSION" "$TMP/transient-read.out"
[[ "$(grep -Fc 'gh api repos/elliot-ylambda/clinch-terminal/releases?per_page=100' \
  "$TMP/gh.log")" == 3 ]]

reset_state
run_release_tty "PUBLISH $VERSION ${COMMIT:0:12}" \
  GH_CREATE_AMBIGUOUS_ONCE=1 CLINCH_GH_RETRY_DELAY_SECONDS=0 \
  > "$TMP/ambiguous-create.out" 2>&1
grep -Fq "Locally built, verified, signed, and published $VERSION" \
  "$TMP/ambiguous-create.out"
grep -Fq 'gh release-create-ambiguous' "$TMP/ops.log"
grep -Fq 'gh release-upload' "$TMP/ops.log"
grep -Fq 'gh release-publish' "$TMP/ops.log"

reset_state
run_release_tty "PUBLISH $VERSION ${COMMIT:0:12}" \
  GH_PUBLISH_AMBIGUOUS_ONCE=1 CLINCH_GH_RETRY_DELAY_SECONDS=0 \
  > "$TMP/ambiguous-publish.out" 2>&1
grep -Fq "Locally built, verified, signed, and published $VERSION" \
  "$TMP/ambiguous-publish.out"
[[ -f "$TMP/gh-publish-ambiguous" ]]
[[ -f "$TMP/published" ]]

reset_state
if run_release_tty 'PUBLISH wrong' > "$TMP/wrong-confirm.out" 2>&1; then
  echo "FAIL: incorrect remote-staging confirmation was accepted" >&2
  exit 1
fi
assert_no_remote_mutation

reset_state
if run_release MAKE_FAIL_ON=release-check > "$TMP/gate-fail.out" 2>&1; then
  echo "FAIL: source-gate failure was accepted" >&2
  exit 1
fi
assert_no_remote_mutation

reset_state
if run_release MAKE_FAIL_ON=_verify > "$TMP/package-fail.out" 2>&1; then
  echo "FAIL: candidate verification failure was accepted" >&2
  exit 1
fi
assert_no_remote_mutation

reset_state
if run_release VERIFY_STAGE_FAIL=1 > "$TMP/stage-fail.out" 2>&1; then
  echo "FAIL: staged-asset verification failure was accepted" >&2
  exit 1
fi
assert_no_remote_mutation
: > "$TMP/ops.log"
: > "$TMP/git.log"
: > "$TMP/gh.log"
run_release_tty "PUBLISH $VERSION ${COMMIT:0:12}" > "$TMP/candidate-resume.out"
grep -Fq "Resuming $VERSION" "$TMP/candidate-resume.out"
grep -Fq "make _verify-existing VERSION=$VERSION UPDATE_SEQUENCE=$SEQUENCE UNIVERSAL=1" \
  "$TMP/ops.log"
if grep -Fq 'make release-check' "$TMP/ops.log" \
    || grep -Fq "make _verify VERSION=$VERSION" "$TMP/ops.log"; then
  echo "FAIL: candidate resume reran the source gate or candidate build" >&2
  exit 1
fi
[[ ! -e "$FIXTURE/target/release-resume/$COMMIT/state.json" ]]

reset_state
if run_release VERSION="$LATEST" > "$TMP/stale.out" 2>&1; then
  echo "FAIL: stale explicit version was accepted" >&2
  exit 1
fi
grep -Fq 'is not newer than' "$TMP/stale.out"
assert_no_remote_mutation

reset_state
if run_release < /dev/null > "$TMP/noninteractive.out" 2>&1; then
  echo "FAIL: noninteractive publication was accepted" >&2
  exit 1
fi
grep -Fq 'publication requires an interactive terminal' "$TMP/noninteractive.out"
grep -Fq 'assemble-stage' "$TMP/ops.log"
assert_no_remote_mutation
: > "$TMP/ops.log"
: > "$TMP/git.log"
: > "$TMP/gh.log"
run_release_tty "PUBLISH $VERSION ${COMMIT:0:12}" > "$TMP/stage-resume.out"
grep -Fq "from phase stage_verified" "$TMP/stage-resume.out"
grep -Fq 'Verifying commit-keyed staged assets' "$TMP/stage-resume.out"
if grep -Fq 'make release-check' "$TMP/ops.log" \
    || grep -Fq 'make _verify' "$TMP/ops.log" \
    || grep -Fq 'assemble-stage' "$TMP/ops.log"; then
  echo "FAIL: staged release resume repeated local build work" >&2
  exit 1
fi
[[ ! -e "$FIXTURE/target/release-resume/$COMMIT/state.json" ]]

reset_state
touch "$TMP/remote-tag" "$TMP/draft"
run_release_tty "PUBLISH $VERSION ${COMMIT:0:12}" > "$TMP/retry.out"
grep -Fq 'gh release-edit' "$TMP/ops.log"
grep -Fq 'gh release-upload' "$TMP/ops.log"
grep -Fq 'gh release-download' "$TMP/ops.log"
grep -Fq 'gh release-publish' "$TMP/ops.log"
if grep -Eq 'git (tag|push-tag)|gh release-create' "$TMP/ops.log"; then
  echo "FAIL: matching retry recreated remote state" >&2
  exit 1
fi

reset_state
touch "$TMP/remote-tag"
if run_release_tty "PUBLISH $VERSION ${COMMIT:0:12}" \
    TAG_COMMIT=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb \
    > "$TMP/wrong-tag.out" 2>&1; then
  echo "FAIL: mismatched existing tag was accepted" >&2
  exit 1
fi
grep -Fq 'points to another commit' "$TMP/wrong-tag.out"
if grep -Eq 'gh (release-|workflow-run)' "$TMP/ops.log"; then
  echo "FAIL: draft changed after mismatched tag" >&2
  exit 1
fi

reset_state
touch "$TMP/remote-tag" "$TMP/draft"
if run_release_tty "PUBLISH $VERSION ${COMMIT:0:12}" DRAFT_VALUE=false \
    > "$TMP/published-release.out" 2>&1; then
  echo "FAIL: an existing public release was accepted as staging" >&2
  exit 1
fi
grep -Fq 'is not a private draft' "$TMP/published-release.out"
if grep -Eq 'gh (release-edit|release-upload|workflow-run)' "$TMP/ops.log"; then
  echo "FAIL: existing public release was mutated" >&2
  exit 1
fi

reset_state
touch "$TMP/remote-tag" "$TMP/draft"
if run_release_tty "PUBLISH $VERSION ${COMMIT:0:12}" GH_EXTRA_ASSET=unexpected.txt \
    > "$TMP/extra-draft-asset.out" 2>&1; then
  echo "FAIL: a draft with an unexpected asset was accepted" >&2
  exit 1
fi
grep -Fq 'contains unexpected assets' "$TMP/extra-draft-asset.out"
if grep -Eq 'gh (release-edit|release-upload|workflow-run)' "$TMP/ops.log"; then
  echo "FAIL: draft with unexpected assets was mutated" >&2
  exit 1
fi

reset_state
touch "$TMP/remote-tag"
if run_release_tty "PUBLISH $VERSION ${COMMIT:0:12}" GIT_VERIFY_TAG_FAIL=1 \
    > "$TMP/invalid-tag-signature.out" 2>&1; then
  echo "FAIL: a tag with an invalid signature was accepted" >&2
  exit 1
fi
if grep -Eq 'gh (release-|workflow-run)' "$TMP/ops.log"; then
  echo "FAIL: draft changed after invalid tag signature" >&2
  exit 1
fi

reset_state
if run_release_tty "PUBLISH $VERSION ${COMMIT:0:12}" VERIFY_REMOTE_STAGE_FAIL=1 \
    > "$TMP/remote-verification-fail.out" 2>&1; then
  echo "FAIL: failed verification of downloaded assets was accepted" >&2
  exit 1
fi
grep -Fq 'gh release-download' "$TMP/ops.log"
if grep -Fq 'gh release-publish' "$TMP/ops.log"; then
  echo "FAIL: draft published after downloaded-asset verification failure" >&2
  exit 1
fi

reset_state
if run_release_tty "PUBLISH $VERSION ${COMMIT:0:12}" MUTATE_DRAFT_ON_DOWNLOAD=1 \
    > "$TMP/draft-download-mutation.out" 2>&1; then
  echo "FAIL: draft mutation during download was accepted" >&2
  exit 1
fi
grep -Fq 'changed while its assets were downloading' "$TMP/draft-download-mutation.out"
if grep -Fq 'gh release-publish' "$TMP/ops.log"; then
  echo "FAIL: mutated draft was published" >&2
  exit 1
fi

reset_state
if run_release_tty "PUBLISH $VERSION ${COMMIT:0:12}" MUTATE_DRAFT_BEFORE_PUBLISH=1 \
    > "$TMP/draft-final-mutation.out" 2>&1; then
  echo "FAIL: post-verification draft mutation was accepted" >&2
  exit 1
fi
grep -Fq 'changed after cryptographic verification' "$TMP/draft-final-mutation.out"
if grep -Fq 'gh release-publish' "$TMP/ops.log"; then
  echo "FAIL: post-verification mutated draft was published" >&2
  exit 1
fi

reset_state
if run_release_tty "PUBLISH $VERSION ${COMMIT:0:12}" MUTATE_TAG_BEFORE_PUBLISH=1 \
    > "$TMP/tag-final-mutation.out" 2>&1; then
  echo "FAIL: post-verification tag mutation was accepted" >&2
  exit 1
fi
grep -Fq 'changed after verification' "$TMP/tag-final-mutation.out"
if grep -Fq 'gh release-publish' "$TMP/ops.log"; then
  echo "FAIL: draft published after tag mutation" >&2
  exit 1
fi

reset_state
if run_release_tty "PUBLISH $VERSION ${COMMIT:0:12}" MUTATE_MAIN_BEFORE_PUBLISH=1 \
    > "$TMP/main-final-mutation.out" 2>&1; then
  echo "FAIL: post-verification main mutation was accepted" >&2
  exit 1
fi
grep -Fq 'changed during the local release' "$TMP/main-final-mutation.out"
if grep -Fq 'gh release-publish' "$TMP/ops.log"; then
  echo "FAIL: draft published after main mutation" >&2
  exit 1
fi

echo "PASS"
