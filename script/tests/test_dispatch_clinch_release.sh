#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TMP="$(mktemp -d -t clinch-release-dispatch-test)"
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/bin"

cat > "$TMP/bin/gh" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$GH_LOG"
case "${1:-} ${2:-}" in
  "auth status")
    exit 0
    ;;
  "api repos/elliot-ylambda/clinch-terminal/releases/latest")
    printf '%s\n' "$GH_LATEST_VERSION"
    ;;
  "api user")
    printf '%s\n' test-operator
    ;;
  "issue create")
    while (( $# )); do
      if [[ "$1" == --body-file ]]; then
        cp "$2" "$GH_ISSUE_BODY"
        break
      fi
      shift
    done
    printf '%s\n' https://github.com/elliot-ylambda/clinch-terminal/issues/999
    ;;
  "issue comment")
    exit 0
    ;;
  "workflow run")
    exit 0
    ;;
  *)
    echo "unexpected gh invocation: $*" >&2
    exit 2
    ;;
esac
STUB
chmod +x "$TMP/bin/gh"

run_dispatch() {
  env \
    PATH="$TMP/bin:$PATH" \
    GH_LOG="$TMP/gh.log" \
    GH_ISSUE_BODY="$TMP/issue.md" \
    GH_LATEST_VERSION=v0.2026.07.15.2000 \
    CLINCH_RELEASE_SKIP_REPOSITORY_CHECKS=1 \
    CLINCH_RELEASE_COMMIT_SHA=0123456789abcdef0123456789abcdef01234567 \
    QA_TESTED_MACOS_VERSIONS="macOS 15.5 (24F74)" \
    "$@"
}

: > "$TMP/gh.log"
output="$(run_dispatch \
  VERSION=v0.2026.07.15.1959 \
  CLINCH_AUTO_VERSION=1 \
  QA_CONFIRMED=true \
  QA_RECORD=auto \
  QA_INTEL_SMOKE=false \
  "$ROOT/script/dispatch-clinch-release")"
grep -Fq "Dispatched the gated release workflow for v0.2026.07.15.2001." <<< "$output"
grep -Fq "workflow run release.yml" "$TMP/gh.log"
grep -Fq -- "-f version=v0.2026.07.15.2001" "$TMP/gh.log"
grep -Fq -- "-f expected_commit=0123456789abcdef0123456789abcdef01234567" "$TMP/gh.log"
grep -Fq -- "-f qa_record=https://github.com/elliot-ylambda/clinch-terminal/issues/999" "$TMP/gh.log"
grep -Fq -- "-f qa_first_install=true" "$TMP/gh.log"
grep -Fq -- "-f qa_apple_silicon_smoke=true" "$TMP/gh.log"
grep -Fq -- "-f qa_intel_smoke=false" "$TMP/gh.log"
grep -Fq "Version: \`v0.2026.07.15.2001\`" "$TMP/issue.md"
grep -Fq 'The candidate update manifest and signatures authenticate' "$TMP/issue.md"
grep -Fq 'Capture purge lists and removes only' "$TMP/issue.md"
grep -Fq -- '- [x] PASS' "$TMP/issue.md"

: > "$TMP/gh.log"
run_dispatch \
  VERSION=v0.2026.07.15.3000 \
  CLINCH_AUTO_VERSION=0 \
  QA_CONFIRMED=true \
  QA_RECORD=https://example.test/qa/3000 \
  QA_INTEL_SMOKE=true \
  "$ROOT/script/dispatch-clinch-release" >/dev/null
grep -Fq -- "-f version=v0.2026.07.15.3000" "$TMP/gh.log"
grep -Fq -- "-f qa_record=https://example.test/qa/3000" "$TMP/gh.log"
grep -Fq -- "-f qa_intel_smoke=true" "$TMP/gh.log"
if grep -Fq "issue create" "$TMP/gh.log"; then
  echo "FAIL: explicit QA record unexpectedly created an issue" >&2
  exit 1
fi

if run_dispatch \
  VERSION=v0.2026.07.15.2000 \
  CLINCH_AUTO_VERSION=0 \
  QA_CONFIRMED=true \
  QA_RECORD=https://example.test/qa/stale \
  "$ROOT/script/dispatch-clinch-release" >"$TMP/stale.out" 2>"$TMP/stale.err"; then
  echo "FAIL: explicit stale version was accepted" >&2
  exit 1
fi
grep -Fq "is not newer than" "$TMP/stale.err"

if run_dispatch \
  VERSION=v0.2026.07.15.3001 \
  CLINCH_AUTO_VERSION=0 \
  QA_CONFIRMED=false \
  QA_RECORD=https://example.test/qa/unconfirmed \
  "$ROOT/script/dispatch-clinch-release" </dev/null >"$TMP/unconfirmed.out" \
  2>"$TMP/unconfirmed.err"; then
  echo "FAIL: noninteractive unconfirmed release was accepted" >&2
  exit 1
fi
grep -Fq "manual QA is not confirmed" "$TMP/unconfirmed.err"

echo "PASS"
