#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DISPATCHER="$ROOT/script/dispatch-clinch-release"
CONFIGURE="$ROOT/script/configure-clinch-release-repository"

[[ ! -e "$ROOT/.github/workflows/release.yml" ]]
if grep -En 'gh workflow run|workflow_dispatch|runs-on:' "$DISPATCHER"; then
  echo "FAIL: release publication still invokes GitHub Actions" >&2
  exit 1
fi
# Workflows may check pull requests, but never build, sign, or publish a release, see secrets,
# or run with write access.
if grep -REn 'workflow_dispatch|pull_request_target|secrets\.|: *write|gh release|make (release|candidate)|dispatch-clinch-release|release-from-clean-worktree' \
    "$ROOT/.github/workflows" 2>/dev/null; then
  echo "FAIL: a GitHub Actions workflow can publish a release or read secrets" >&2
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
