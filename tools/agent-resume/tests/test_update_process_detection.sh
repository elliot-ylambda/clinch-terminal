#!/usr/bin/env bash
# Regression coverage for the updater replacing Clinch while LaunchServices still owns it.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
source "$ROOT/script/update-installed-clinch"

cat > "$TMP/lsappinfo" <<EOF
#!/usr/bin/env bash
case "\$1" in
  find) [[ -f "$TMP/running" ]] && echo 'ASN:0x0-0x1-"Clinch":' ;;
  info) echo '"pid"=4242' ;;
esac
EOF
cat > "$TMP/ps" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
cat > "$TMP/osascript" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" > "$TMP/osascript-args"
rm -f "$TMP/running"
EOF
cat > "$TMP/kill" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$TMP/kill-args"
EOF
chmod +x "$TMP/lsappinfo" "$TMP/ps" "$TMP/osascript" "$TMP/kill"

export CLINCH_LSAPPINFO_BIN="$TMP/lsappinfo"
export CLINCH_PS_BIN="$TMP/ps"
export CLINCH_OSASCRIPT_BIN="$TMP/osascript"
export CLINCH_KILL_BIN="$TMP/kill"
export CLINCH_SLEEP_BIN=/usr/bin/true
export CLINCH_QUIT_WAIT_ATTEMPTS=1
export CLINCH_FORCE_WAIT_ATTEMPTS=1

# LaunchServices detection must quit by bundle id even when path/pgrep-style detection sees
# nothing (the exact state of the July 9 process).
: > "$TMP/running"
clinch_quit_running_app Clinch sh.clinch.Clinch /Applications/Clinch.app
grep -q 'application id "sh.clinch.Clinch"' "$TMP/osascript-args" \
  || { echo "FAIL: updater did not quit the exact bundle id"; exit 1; }
[[ ! -f "$TMP/running" ]] || { echo "FAIL: simulated app stayed running"; exit 1; }

# If graceful and exact-PID termination both fail, the updater must abort instead of
# deleting a bundle out from under a live process.
cat > "$TMP/osascript" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
chmod +x "$TMP/osascript"
: > "$TMP/running"
if clinch_quit_running_app Clinch sh.clinch.Clinch /Applications/Clinch.app 2>/dev/null; then
  echo "FAIL: updater accepted a still-running app"
  exit 1
fi
grep -q -- '-TERM 4242' "$TMP/kill-args" \
  || { echo "FAIL: updater did not target the LaunchServices PID"; exit 1; }
grep -q -- '-KILL 4242' "$TMP/kill-args" \
  || { echo "FAIL: updater did not escalate the exact PID"; exit 1; }

# The public curl installer uses the same LaunchServices guard. It must refuse before any
# download or replacement even when a path-only process scan sees nothing.
# With no terminal to ask on. This test can itself run on a terminal, where the installer would
# otherwise wait for an answer.
if installer_out="$(CLINCH_INSTALL_TTY="$TMP/no-terminal" CLINCH_LSAPPINFO_BIN="$TMP/lsappinfo" \
    CLINCH_PS_BIN="$TMP/ps" sh "$ROOT/install.sh" 2>&1)"; then
  echo "FAIL: public installer accepted a LaunchServices-owned app"
  exit 1
fi
printf '%s\n' "$installer_out" | grep -q 'Clinch is currently running' \
  || { echo "FAIL: public installer did not report the running app"; exit 1; }

# Asked on a terminal, it offers to quit the app, and says what to do if the app won't.
cat > "$TMP/osascript" <<'SH'
#!/bin/sh
printf '%s\n' "$*" >> "$CLINCH_TEST_OSASCRIPT_LOG"
[ -z "${CLINCH_TEST_QUIT_FLAG:-}" ] || touch "$CLINCH_TEST_QUIT_FLAG"
SH
chmod +x "$TMP/osascript"
export CLINCH_TEST_OSASCRIPT_LOG="$TMP/osascript.log"
run_installer() {
  CLINCH_CURL_BIN=/usr/bin/false \
    CLINCH_INSTALL_TTY="$1" CLINCH_OSASCRIPT_BIN="$TMP/osascript" CLINCH_QUIT_WAIT_ATTEMPTS=1 \
    CLINCH_LSAPPINFO_BIN="$2" CLINCH_PS_BIN="$TMP/ps" sh "$ROOT/install.sh" 2>&1 || true
}

printf 'n\n' > "$TMP/answer-no"
: > "$CLINCH_TEST_OSASCRIPT_LOG"
installer_out="$(run_installer "$TMP/answer-no" "$TMP/lsappinfo")"
printf '%s\n' "$installer_out" | grep -q 'Quit it now to install' \
  || { echo "FAIL: installer did not offer to quit: $installer_out"; exit 1; }
printf '%s\n' "$installer_out" | grep -q 'Clinch is currently running' \
  || { echo "FAIL: declining did not stop the install: $installer_out"; exit 1; }
[ ! -s "$CLINCH_TEST_OSASCRIPT_LOG" ] || { echo "FAIL: installer quit the app after a no"; exit 1; }

printf 'y\n' > "$TMP/answer-yes"
installer_out="$(run_installer "$TMP/answer-yes" "$TMP/lsappinfo")"
grep -q 'sh.clinch.Clinch' "$CLINCH_TEST_OSASCRIPT_LOG" \
  || { echo "FAIL: installer did not ask the app to quit"; exit 1; }
printf '%s\n' "$installer_out" | grep -q 'A dialog may be open' \
  || { echo "FAIL: a refused quit did not mention a dialog: $installer_out"; exit 1; }

# Once the app has quit, the install carries on past the guard.
cat > "$TMP/lsappinfo-quits" <<SH
#!/bin/sh
[ -e "$TMP/quit-flag" ] || "$TMP/lsappinfo" "\$@"
SH
chmod +x "$TMP/lsappinfo-quits"
installer_out="$(CLINCH_TEST_QUIT_FLAG="$TMP/quit-flag" run_installer "$TMP/answer-yes" "$TMP/lsappinfo-quits")"
printf '%s\n' "$installer_out" | grep -q 'A dialog may be open\|currently running' \
  && { echo "FAIL: installer stopped after the app quit: $installer_out"; exit 1; }

echo "PASS"
