#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TMP="$(mktemp -d -t clinch-parallel-arch-test)"
trap 'rm -rf "$TMP"' EXIT
BIN="$TMP/bin"
BASE_TARGET_DIR="$TMP/workspace/target"
mkdir -p "$BIN" "$TMP/workspace/app" "$TMP/starts" "$BASE_TARGET_DIR"
BASE_TARGET_DIR="$(cd "$BASE_TARGET_DIR" && pwd -P)"
mkdir -p "$BASE_TARGET_DIR/release-lto" \
  "$BASE_TARGET_DIR/x86_64-apple-darwin/release-lto"
printf 'host cache\n' > "$BASE_TARGET_DIR/release-lto/seed-marker"
printf 'target cache\n' \
  > "$BASE_TARGET_DIR/x86_64-apple-darwin/release-lto/seed-marker"

cat > "$BIN/cargo" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
target=
profile=
args=("$@")
for ((index = 0; index < ${#args[@]}; index++)); do
  case "${args[index]}" in
    --target) target="${args[index + 1]}" ;;
    --profile) profile="${args[index + 1]}" ;;
  esac
done
[[ -n "$target" && -n "$profile" && -n "${CARGO_TARGET_DIR:-}" ]]
profile_dir="$profile"
[[ "$profile" == dev ]] && profile_dir=debug

mkdir -p "$CARGO_TARGET_DIR"
lock_dir="$CARGO_TARGET_DIR/.test-exclusive-lock"
if ! mkdir "$lock_dir" 2>/dev/null; then
  echo "builds shared a Cargo target-directory lock" >&2
  exit 8
fi
cleanup() { rmdir "$lock_dir"; }
trap cleanup EXIT

printf '%s|%s|%s\n' "$target" "$CARGO_TARGET_DIR" "$*" >> "$CARGO_LOG"
touch "$START_DIR/$target"
for _ in {1..100}; do
  if [[ "$(find "$START_DIR" -type f | wc -l | tr -d ' ')" == 2 ]]; then
    output_dir="$CARGO_TARGET_DIR/$target/$profile_dir"
    hashed_dsym="$output_dir/deps/stable-test.dSYM"
    mkdir -p "$hashed_dsym/Contents/Resources/DWARF"
    printf '%s\n' "$target" > "$output_dir/stable"
    printf 'symbols\n' > "$hashed_dsym/Contents/Resources/DWARF/stable-test"
    ln -s deps/stable-test.dSYM "$output_dir/stable.dSYM"
    exit 0
  fi
  sleep 0.02
done
echo "parallel build peer never started" >&2
exit 9
STUB
chmod +x "$BIN/cargo"

PATH="$BIN:$PATH" \
CARGO_LOG="$TMP/cargo.log" \
START_DIR="$TMP/starts" \
CARGO_TARGET_DIR="$BASE_TARGET_DIR" \
CLINCH_PARALLEL_ARCH_BUILDS=1 \
CLINCH_PARALLEL_ARCH_CPU_COUNT=12 \
  "$ROOT/script/macos/build-universal-targets-parallel" \
    release-lto stable aarch64-apple-darwin x86_64-apple-darwin \
    release_bundle,gui "$TMP/workspace" > "$TMP/parallel.out"

[[ "$(wc -l < "$TMP/cargo.log" | tr -d ' ')" == 2 ]]
grep -Fq -- '|build --profile release-lto --bin stable --bin generate_settings_schema --target aarch64-apple-darwin' \
  "$TMP/cargo.log"
grep -Fq -- '|build --profile release-lto --bin stable --target x86_64-apple-darwin' "$TMP/cargo.log"
[[ "$(grep -Fc -- '--jobs 6' "$TMP/cargo.log")" == 2 ]]
[[ "$(cut -d '|' -f 2 "$TMP/cargo.log" | sort -u | wc -l | tr -d ' ')" == 2 ]]
grep -Fq -- "aarch64-apple-darwin|$BASE_TARGET_DIR|" "$TMP/cargo.log"
grep -Fq -- "x86_64-apple-darwin|$BASE_TARGET_DIR/parallel-arch-cache/x86_64-apple-darwin|" \
  "$TMP/cargo.log"
grep -Fq 'host cache' \
  "$BASE_TARGET_DIR/parallel-arch-cache/x86_64-apple-darwin/release-lto/seed-marker"
grep -Fq 'target cache' \
  "$BASE_TARGET_DIR/parallel-arch-cache/x86_64-apple-darwin/x86_64-apple-darwin/release-lto/seed-marker"
[[ -f "$BASE_TARGET_DIR/x86_64-apple-darwin/release-lto/stable" ]]
[[ -L "$BASE_TARGET_DIR/x86_64-apple-darwin/release-lto/stable.dSYM" ]]
grep -Fq 'Architecture build timings:' "$TMP/parallel.out"

rm -f "$TMP/cargo.log"
if PATH="$BIN:$PATH" \
    CARGO_LOG="$TMP/cargo.log" \
    START_DIR="$TMP/starts" \
    CARGO_TARGET_DIR="$BASE_TARGET_DIR" \
    CLINCH_PARALLEL_ARCH_BUILDS=0 \
    "$ROOT/script/macos/build-universal-targets-parallel" \
      release-lto stable aarch64-apple-darwin x86_64-apple-darwin \
      release_bundle,gui "$TMP/workspace"; then
  echo "FAIL: disabled parallel build unexpectedly ran" >&2
  exit 1
else
  status=$?
  [[ "$status" == 2 ]]
fi
[[ ! -e "$TMP/cargo.log" ]]

if PATH="$BIN:$PATH" \
    CARGO_LOG="$TMP/cargo.log" \
    START_DIR="$TMP/starts" \
    CARGO_TARGET_DIR="$BASE_TARGET_DIR" \
    CLINCH_PARALLEL_ARCH_TARGET_DIR="$BASE_TARGET_DIR" \
    CLINCH_PARALLEL_ARCH_BUILDS=1 \
    "$ROOT/script/macos/build-universal-targets-parallel" \
      release-lto stable aarch64-apple-darwin x86_64-apple-darwin \
      release_bundle,gui "$TMP/workspace"; then
  echo "FAIL: shared Cargo target directories were accepted" >&2
  exit 1
else
  status=$?
  [[ "$status" == 64 ]]
fi

echo "PASS"
