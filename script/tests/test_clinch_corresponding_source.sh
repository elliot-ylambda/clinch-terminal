#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TMP="$(mktemp -d -t clinch-source-test)"
trap 'rm -rf "$TMP"' EXIT
FIXTURE="$TMP/repo"
BIN="$TMP/bin"
REGISTRY_INDEX="$TMP/registry/src/index.fixture"
REGISTRY_PACKAGE="$REGISTRY_INDEX/fixture-1.0.0"
REGISTRY_CRATE="$TMP/registry/cache/index.fixture/fixture-1.0.0.crate"
GIT_PACKAGE="$TMP/git/fixture-1.0.0"
mkdir -p \
  "$FIXTURE/script" "$FIXTURE/.cargo" "$FIXTURE/src" "$BIN" "$TMP/out" \
  "$REGISTRY_PACKAGE/src" "$GIT_PACKAGE/src" "$(dirname "$REGISTRY_CRATE")"

cp \
  "$ROOT/script/assemble-clinch-corresponding-source" \
  "$ROOT/script/vendor-clinch-cargo-sources" \
  "$ROOT/script/verify-clinch-source-archive" \
  "$FIXTURE/script/"
cp "$ROOT/LICENSE-AGPL" "$ROOT/LICENSE-MIT" "$ROOT/NOTICE" "$FIXTURE/"
printf '[env]\nSOURCE_FIXTURE = "1"\n' > "$FIXTURE/.cargo/config.toml"
cat > "$FIXTURE/Cargo.toml" <<'EOF'
[package]
name = "clinch-source-fixture"
version = "0.1.0"
edition = "2021"
EOF
printf 'version = 3\n' > "$FIXTURE/Cargo.lock"
printf 'fn main() {}\n' > "$FIXTURE/src/main.rs"
for package in "$REGISTRY_PACKAGE" "$GIT_PACKAGE"; do
  cat > "$package/Cargo.toml" <<'EOF'
[package]
name = "fixture"
version = "1.0.0"
edition = "2021"
EOF
  printf 'pub fn fixture() {}\n' > "$package/src/lib.rs"
done
COPYFILE_DISABLE=1 tar -czf "$REGISTRY_CRATE" \
  -C "$REGISTRY_INDEX" fixture-1.0.0
REGISTRY_CHECKSUM="$(shasum -a 256 "$REGISTRY_CRATE" | awk '{print $1}')"
printf 'cache marker\n' > "$REGISTRY_PACKAGE/.cargo-ok"

cat > "$BIN/cargo" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
case "${1:-}" in
  vendor)
    destination="${@: -1}/fixture-1.0.0"
    mkdir -p "$destination/src"
    cp "$FIXTURE_GIT_MANIFEST" "$destination/Cargo.toml"
    cp "$(dirname "$FIXTURE_GIT_MANIFEST")/src/lib.rs" "$destination/src/lib.rs"
    ;;
  metadata)
    [[ " $* " == *' --all-features '* ]]
    if grep -Fq 'clinch-vendor-' .cargo/config.toml; then
      [[ "$(find vendor -name .cargo-checksum.json | wc -l | tr -d ' ')" == 2 ]]
      printf '{"packages":[]}\n'
    else
      cat <<EOF
{"packages":[
  {"name":"fixture","version":"1.0.0","source":"registry+https://github.com/rust-lang/crates.io-index","checksum":"$FIXTURE_REGISTRY_CHECKSUM","manifest_path":"$FIXTURE_REGISTRY_MANIFEST"},
  {"name":"fixture","version":"1.0.0","source":"git+https://example.com/fixture?rev=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","checksum":null,"manifest_path":"$FIXTURE_GIT_MANIFEST"}
]}
EOF
    fi
    ;;
  *)
    echo "unexpected cargo command: $*" >&2
    exit 2
    ;;
esac
STUB
chmod +x "$BIN/cargo" "$FIXTURE/script/"*

(
  cd "$FIXTURE"
  git init -q
  git add .
  git -c user.name='Source fixture' -c user.email='fixture@example.com' commit -qm fixture
)
COMMIT="$(git -C "$FIXTURE" rev-parse HEAD)"
VERSION=v0.2099.01.02.0304
ARCHIVE="$TMP/out/Clinch.source.tar.gz"

(
  cd "$FIXTURE"
  PATH="$BIN:$PATH" \
    FIXTURE_REGISTRY_MANIFEST="$REGISTRY_PACKAGE/Cargo.toml" \
    FIXTURE_REGISTRY_CHECKSUM="$REGISTRY_CHECKSUM" \
    FIXTURE_GIT_MANIFEST="$GIT_PACKAGE/Cargo.toml" \
    ./script/assemble-clinch-corresponding-source \
    "$VERSION" "$COMMIT" "$ARCHIVE" >/dev/null
)

[[ -f "$ARCHIVE" && -f "$ARCHIVE.sha256" ]]
(cd "$TMP/out" && shasum -a 256 -c Clinch.source.tar.gz.sha256 >/dev/null)
"$FIXTURE/script/verify-clinch-source-archive" \
  "$ARCHIVE" "$VERSION" "$COMMIT" "$FIXTURE" >/dev/null
[[ "$(tar -tzf "$ARCHIVE" | grep -Ec \
  "Clinch-${VERSION#v}-source/vendor/[0-9a-f]{16}/fixture-1.0.0/.cargo-checksum.json")" == 2 ]]
if tar -tzf "$ARCHIVE" | grep -Fq '/.git/'; then
  echo "FAIL: source archive contains Git administration data" >&2
  exit 1
fi

echo PASS
