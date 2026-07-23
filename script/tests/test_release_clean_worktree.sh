#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TMP="$(mktemp -d -t clinch-clean-worktree-test)"
trap 'rm -rf "$TMP"' EXIT
FIXTURE="$TMP/repo"
mkdir -p "$FIXTURE/script"

git -C "$TMP" init -q -b main repo
git -C "$FIXTURE" config user.name fixture
git -C "$FIXTURE" config user.email fixture@example.com
cp "$ROOT/script/release-from-clean-worktree" "$FIXTURE/script/"
cat > "$FIXTURE/script/require-latest-main" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
[[ "$(git rev-parse --abbrev-ref HEAD)" == main ]]
[[ -z "$(git status --porcelain)" ]]
STUB
cat > "$FIXTURE/script/dispatch-clinch-release" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
[[ "$CLINCH_RELEASE_ISOLATED_WORKTREE" == 1 ]]
[[ "$CLINCH_RELEASE_PINNED_COMMIT" == "$(git rev-parse HEAD)" ]]
[[ "$(git rev-parse --abbrev-ref HEAD)" == HEAD ]]
[[ -L target ]]
grep -Fq shared-cache target/cache-marker
touch "$CLINCH_RELEASE_CALLER_ROOT/concurrent-edit"
[[ -z "$(git status --porcelain)" ]]
printf '%s\n' "$CLINCH_RELEASE_PINNED_COMMIT" > "$CLINCH_RELEASE_CALLER_ROOT/released-commit"
STUB
chmod +x "$FIXTURE/script/"*
printf '/target\n/released-commit\n' > "$FIXTURE/.gitignore"
printf 'fixture\n' > "$FIXTURE/tracked"
git -C "$FIXTURE" add .
git -C "$FIXTURE" commit -q -m fixture
mkdir -p "$FIXTURE/target"
printf 'shared-cache\n' > "$FIXTURE/target/cache-marker"

"$FIXTURE/script/release-from-clean-worktree" > "$TMP/output.log"

grep -Fq "$(git -C "$FIXTURE" rev-parse HEAD)" "$FIXTURE/released-commit"
[[ -f "$FIXTURE/concurrent-edit" ]]
[[ "$(git -C "$FIXTURE" worktree list --porcelain | grep -c '^worktree ')" == 1 ]]
grep -Fq 'from an isolated clean worktree' "$TMP/output.log"

echo "PASS"
