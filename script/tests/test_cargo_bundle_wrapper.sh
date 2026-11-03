#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TMP="$(mktemp -d -t clinch-cargo-bundle-test)"
trap 'rm -rf "$TMP"' EXIT
BIN="$TMP/bin"
mkdir -p "$BIN"

cat > "$BIN/cargo" <<'STUB'
#!/usr/bin/env bash
printf 'TERM=%s args=%s\n' "${TERM-<unset>}" "$*"
STUB
chmod +x "$BIN/cargo"

run_with_term() {
  PATH="$BIN:$PATH" TERM="$1" "$ROOT/script/macos/cargo-bundle" --bin stable
}

# cargo-bundle panics on colorless terminals, so the wrapper drops TERM there.
[[ "$(run_with_term dumb)" == "TERM=<unset> args=bundle --bin stable" ]]
[[ "$(run_with_term no-such-terminal)" == "TERM=<unset> args=bundle --bin stable" ]]
# Color terminals keep TERM so humans still get colored status lines.
[[ "$(run_with_term xterm-256color)" == "TERM=xterm-256color args=bundle --bin stable" ]]

echo "PASS"
