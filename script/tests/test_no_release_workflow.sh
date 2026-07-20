#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DISPATCHER="$ROOT/script/dispatch-clinch-release"
CONFIGURE="$ROOT/script/configure-clinch-release-repository"

[[ ! -e "$ROOT/.github/workflows/release.yml" ]]
if grep -REn 'gh workflow run|workflow_dispatch|runs-on:' \
    "$DISPATCHER" "$ROOT/.github/workflows" 2>/dev/null; then
  echo "FAIL: release publication still invokes GitHub Actions" >&2
  exit 1
fi

grep -Fq "gh_with_retry release download \"\$version\"" "$DISPATCHER"
grep -Fq "./script/verify-clinch-release-stage \"\$publish_tmp/dist\"" "$DISPATCHER"
grep -Fq './script/next-clinch-update-sequence verify' "$DISPATCHER"
grep -Fq "gh_with_retry release edit \"\$version\" --repo \"\$REPO\"" "$DISPATCHER"
grep -Fq 'clinch-update-manifest verify-key' "$CONFIGURE"
grep -Fq "gh secret delete \"\$secret\"" "$CONFIGURE"
grep -Fq "environments/\$RELEASE_ENVIRONMENT" "$CONFIGURE"

echo "PASS"
