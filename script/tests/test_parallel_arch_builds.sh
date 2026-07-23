#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TMP="$(mktemp -d -t clinch-parallel-arch-test)"
trap 'rm -rf "$TMP"' EXIT
BIN="$TMP/bin"
mkdir -p "$BIN" "$TMP/workspace/app" "$TMP/starts"

cat > "$BIN/cargo" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
target=
args=("$@")
for ((index = 0; index < ${#args[@]}; index++)); do
  if [[ "${args[index]}" == --target ]]; then
    target="${args[index + 1]}"
    break
  fi
done
[[ -n "$target" ]]
printf '%s\n' "$*" >> "$CARGO_LOG"
touch "$START_DIR/$target"
for _ in {1..100}; do
  [[ "$(find "$START_DIR" -type f | wc -l | tr -d ' ')" == 2 ]] && exit 0
  sleep 0.02
done
echo "parallel build peer never started" >&2
exit 9
STUB
chmod +x "$BIN/cargo"

PATH="$BIN:$PATH" \
CARGO_LOG="$TMP/cargo.log" \
START_DIR="$TMP/starts" \
CLINCH_PARALLEL_ARCH_BUILDS=1 \
CLINCH_PARALLEL_ARCH_CPU_COUNT=12 \
  "$ROOT/script/macos/build-universal-targets-parallel" \
    release-lto stable aarch64-apple-darwin x86_64-apple-darwin \
    release_bundle,gui "$TMP/workspace" > "$TMP/parallel.out"

[[ "$(wc -l < "$TMP/cargo.log" | tr -d ' ')" == 2 ]]
grep -Fq -- '--bin stable --bin generate_settings_schema --target aarch64-apple-darwin' \
  "$TMP/cargo.log"
grep -Fq -- '--bin stable --target x86_64-apple-darwin' "$TMP/cargo.log"
[[ "$(grep -Fc -- '--jobs 6' "$TMP/cargo.log")" == 2 ]]

rm -f "$TMP/cargo.log"
if PATH="$BIN:$PATH" \
    CARGO_LOG="$TMP/cargo.log" \
    START_DIR="$TMP/starts" \
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

echo "PASS"
