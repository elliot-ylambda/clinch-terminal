#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TMP="$(mktemp -d -t clinch-reclaim-test)"
trap 'rm -rf "$TMP"' EXIT

RECLAIM="$ROOT/script/reclaim-build-space"
TARGET="$TMP/target"

# The prune refuses to run while a build is in flight. This machine routinely
# has one, so every case controls the process check instead of inheriting the
# real one: without this the suite passes or fails depending on whether some
# other session happens to be compiling.
mkdir -p "$TMP/bin-idle" "$TMP/bin-busy"
cat > "$TMP/bin-idle/pgrep" <<'STUB'
#!/usr/bin/env bash
exit 1
STUB
cat > "$TMP/bin-busy/pgrep" <<'STUB'
#!/usr/bin/env bash
[[ "${2:-}" == cargo ]] && exit 0
exit 1
STUB
chmod +x "$TMP/bin-idle/pgrep" "$TMP/bin-busy/pgrep"

IDLE_PATH="$TMP/bin-idle:$PATH"
BUSY_PATH="$TMP/bin-busy:$PATH"

# An age far enough back that any reasonable --days threshold treats it as cold.
OLD=202601010000

build_fixture() {
  rm -rf "$TARGET"

  # Regenerable caches, all cold.
  mkdir -p "$TARGET/debug/incremental/warp-1a2b"
  mkdir -p "$TARGET/aarch64-apple-darwin/release-lto/incremental"
  mkdir -p "$TARGET/release-worktree-cache/debug/incremental"
  mkdir -p "$TARGET/release-worktree-cache/release-stage"
  mkdir -p "$TARGET/release-resume/0123456789ab"

  # Where a parallel two-architecture release build puts the secondary
  # architecture: five levels down, deeper than a naive walk reaches.
  mkdir -p \
    "$TARGET/release-worktree-cache/parallel-arch-cache/x86_64-apple-darwin/debug/incremental"

  # Expensive artifacts that must survive: these are what keep a release fast.
  mkdir -p "$TARGET/debug/deps"
  mkdir -p "$TARGET/debug/build"
  mkdir -p "$TARGET/release-worktree-cache/debug/deps"
  mkdir -p "$TARGET/aarch64-apple-darwin/release-lto/deps"
  printf 'artifact\n' > "$TARGET/debug/deps/libwarp.rlib"
  printf 'artifact\n' > "$TARGET/release-worktree-cache/debug/deps/libwarp.rlib"
  printf 'artifact\n' > "$TARGET/aarch64-apple-darwin/release-lto/deps/libwarp.rlib"

  # A warm cache, touched just now, that the default age threshold must keep.
  mkdir -p "$TARGET/warm/incremental"

  touch -t "$OLD" \
    "$TARGET/debug/incremental" \
    "$TARGET/aarch64-apple-darwin/release-lto/incremental" \
    "$TARGET/release-worktree-cache/debug/incremental" \
    "$TARGET/release-worktree-cache/release-stage" \
    "$TARGET/release-resume/0123456789ab" \
    "$TARGET/release-worktree-cache/parallel-arch-cache/x86_64-apple-darwin/debug/incremental"
}

# --- cold caches are removed, warm caches and real artifacts are not ---------
build_fixture
PATH="$IDLE_PATH" "$RECLAIM" --root "$TARGET" --days 7 > "$TMP/prune.log"

[[ ! -d "$TARGET/debug/incremental" ]]
[[ ! -d "$TARGET/aarch64-apple-darwin/release-lto/incremental" ]]
[[ ! -d "$TARGET/release-worktree-cache/debug/incremental" ]]
[[ ! -d "$TARGET/release-worktree-cache/parallel-arch-cache/x86_64-apple-darwin/debug/incremental" ]]
[[ ! -d "$TARGET/release-worktree-cache/release-stage" ]]
[[ ! -d "$TARGET/release-resume/0123456789ab" ]]

# The warm cache is younger than the threshold and must survive.
[[ -d "$TARGET/warm/incremental" ]]

# Nothing expensive may be touched.
[[ -f "$TARGET/debug/deps/libwarp.rlib" ]]
[[ -f "$TARGET/release-worktree-cache/debug/deps/libwarp.rlib" ]]
[[ -f "$TARGET/aarch64-apple-darwin/release-lto/deps/libwarp.rlib" ]]
[[ -d "$TARGET/debug/build" ]]

grep -Fq "Reclaimed" "$TMP/prune.log"
grep -Fq "6 cache(s)" "$TMP/prune.log"

# --- --days 0 also takes the warm cache -------------------------------------
build_fixture
PATH="$IDLE_PATH" "$RECLAIM" --root "$TARGET" --days 0 > "$TMP/prune-all.log"
[[ ! -d "$TARGET/warm/incremental" ]]
[[ -f "$TARGET/debug/deps/libwarp.rlib" ]]

# --- --dry-run reports without deleting -------------------------------------
build_fixture
PATH="$IDLE_PATH" "$RECLAIM" --root "$TARGET" --days 7 --dry-run > "$TMP/dry.log"
[[ -d "$TARGET/debug/incremental" ]]
[[ -d "$TARGET/release-resume/0123456789ab" ]]
grep -Fq "Would reclaim" "$TMP/dry.log"
grep -Fq "would remove" "$TMP/dry.log"
! grep -q "^removing" "$TMP/dry.log"

# --- a running build blocks the prune ----------------------------------------
build_fixture
PATH="$BUSY_PATH" "$RECLAIM" --root "$TARGET" --days 7 > "$TMP/busy.log"
[[ -d "$TARGET/debug/incremental" ]]
grep -Fq "a build is running" "$TMP/busy.log"

# --force overrides the guard.
PATH="$BUSY_PATH" "$RECLAIM" --root "$TARGET" --days 7 --force > "$TMP/forced.log"
[[ ! -d "$TARGET/debug/incremental" ]]
grep -Fq "Reclaimed" "$TMP/forced.log"

# --- an empty target reports cleanly rather than failing ---------------------
mkdir -p "$TMP/empty-target"
PATH="$IDLE_PATH" "$RECLAIM" --root "$TMP/empty-target" --days 7 > "$TMP/empty.log"
grep -Fq "Nothing to reclaim" "$TMP/empty.log"

# --- --quiet stays silent when there is nothing to do ------------------------
PATH="$IDLE_PATH" "$RECLAIM" --root "$TMP/empty-target" --days 7 --quiet > "$TMP/quiet.log"
[[ ! -s "$TMP/quiet.log" ]]

# --- a target reached twice is only removed once -----------------------------
# The release worktree symlinks its `target` at release-worktree-cache, so the
# same physical directory can be discovered through two paths.
build_fixture
ln -s "$TARGET" "$TMP/target-link"
PATH="$IDLE_PATH" "$RECLAIM" --root "$TARGET" --root "$TMP/target-link" --days 7 > "$TMP/dedupe.log"
[[ "$(grep -c '^removing' "$TMP/dedupe.log")" == 6 ]]

# --- a missing root is not an error ------------------------------------------
PATH="$IDLE_PATH" "$RECLAIM" --root "$TMP/does-not-exist" --days 7 > "$TMP/missing.log"
grep -Fq "No target directories found" "$TMP/missing.log"

# --- argument validation -----------------------------------------------------
! "$RECLAIM" --days notanumber > /dev/null 2>&1
! "$RECLAIM" --nonsense > /dev/null 2>&1
"$RECLAIM" --help | grep -Fq "Usage: script/reclaim-build-space"

echo "PASS"
