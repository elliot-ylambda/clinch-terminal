#!/usr/bin/env bash
# `make release` runs under a terminal for its publish confirmation. `security unlock-keychain`
# prompts on a controlling terminal and reads stdin only without one, so a piped password used to
# hang the release. The helper must unlock from stdin with or without a controlling terminal.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TMP="$(mktemp -d -t clinch-unlock-keychain-test)"
trap 'rm -rf "$TMP"' EXIT

mkdir -p "$TMP/bin"
# Mirrors `security unlock-keychain`: with a controlling terminal it would wait for the password
# there (reported here as exit 99 instead of hanging); without one it reads stdin.
cat > "$TMP/bin/security" <<'EOF'
#!/usr/bin/env bash
[[ "$1" == unlock-keychain ]] || exit 2
if (: < /dev/tty) 2>/dev/null; then
  exit 99
fi
IFS= read -r password || true
[[ "$password" == correct-password ]] || exit 51
EOF
chmod +x "$TMP/bin/security"
printf 'correct-password\n' > "$TMP/password"
printf 'wrong-password\n' > "$TMP/wrong-password"

# Runs a command with a pseudo-terminal as its controlling terminal and prints its exit status.
under_tty() {
  /usr/bin/python3 - "$@" <<'PY'
import fcntl, os, subprocess, sys, termios
master, slave = os.openpty()
def attach():
    os.setsid()
    fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
process = subprocess.Popen(sys.argv[1:], stdin=slave, stdout=slave, stderr=slave,
                           preexec_fn=attach)
os.close(slave)
try:
    print(process.wait(timeout=20))
except subprocess.TimeoutExpired:
    process.kill()
    print("timeout")
PY
}

export PATH="$TMP/bin:$PATH"
helper="$ROOT/script/unlock-clinch-codesign-keychain"

# The stub must detect a controlling terminal, or this test proves nothing.
status="$(under_tty /bin/sh -c "security unlock-keychain kc < '$TMP/password'")"
[[ "$status" == 99 ]] || { echo "FAIL: stub did not detect the terminal ($status)" >&2; exit 1; }

status="$(under_tty "$helper" kc "$TMP/password")"
[[ "$status" == 0 ]] || { echo "FAIL: helper did not unlock under a terminal ($status)" >&2; exit 1; }

"$helper" kc "$TMP/password" < /dev/null \
  || { echo "FAIL: helper did not unlock without a terminal" >&2; exit 1; }

if "$helper" kc "$TMP/wrong-password" < /dev/null; then
  echo "FAIL: helper accepted a wrong password" >&2
  exit 1
fi

echo "PASS"
