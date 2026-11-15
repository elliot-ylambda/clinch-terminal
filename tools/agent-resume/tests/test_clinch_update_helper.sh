#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
TMP="$(mktemp -d -t clinch-update-helper-test)"
TMP="$(cd "$TMP" && pwd -P)"
trap 'rm -rf "$TMP"' EXIT

source "$ROOT/resources/update/clinch-update-helper"

HOME_DIR="$TMP/home"
SUPPORT="$HOME_DIR/Library/Application Support/sh.clinch.Clinch"
mkdir -p "$HOME_DIR" "$SUPPORT"
preflight "$HOME_DIR" "$SUPPORT"

if (require_absolute_path "relative/path") >/dev/null 2>&1; then
  echo "relative updater path unexpectedly accepted" >&2
  exit 1
fi
if (preflight "$HOME_DIR" "$TMP/wrong-support") >/dev/null 2>&1; then
  echo "unexpected support directory was accepted" >&2
  exit 1
fi
if (install_update /tmp/swap /tmp/archive /Applications/Clinch.app not-a-pid 501 "$HOME_DIR" \
    update v1 1 "$(printf '0%.0s' {1..64})" 1 /tmp/ready /tmp/cancel \
    /tmp/success) >/dev/null 2>&1; then
  echo "invalid updater process identity was accepted" >&2
  exit 1
fi

# The pre-update snapshot keeps the registry and each registered Claude transcript, found in one
# walk of the transcript tree, and prunes automatic snapshots beyond the newest 15.
SNAP_HOME="$TMP/snapshot-home"
SNAP_SUPPORT="$SNAP_HOME/Library/Application Support/sh.clinch.Clinch"
SNAP_REG="$SNAP_HOME/.warp/agent-resume"
PROJECTS="$SNAP_HOME/.claude/projects"
mkdir -p "$SNAP_REG" "$SNAP_SUPPORT" "$PROJECTS/-repo-a" "$PROJECTS/-repo-b/nested"
printf '{"command": "clinch_agent_resume_launch claude sess-a --model opus"}\n' > "$SNAP_REG/pane-1.json"
printf '{"command": "warp_agent_resume_launch claude sess-b"}\n' > "$SNAP_REG/pane-2.json"
printf '{"command": "clinch_agent_resume_launch claude sess-missing"}\n' > "$SNAP_REG/pane-3.json"
printf '{"command": "clinch_agent_resume_launch codex sess-codex"}\n' > "$SNAP_REG/pane-4.json"
printf 'transcript a\n' > "$PROJECTS/-repo-a/sess-a.jsonl"
printf 'transcript b\n' > "$PROJECTS/-repo-b/nested/sess-b.jsonl"
printf 'codex\n' > "$PROJECTS/-repo-a/sess-codex.jsonl"
printf 'unregistered\n' > "$PROJECTS/-repo-a/sess-other.jsonl"
for day in $(seq -w 1 15); do
  old="$SNAP_SUPPORT/session-recovery-202601$day-000000"
  mkdir -p "$old"
  : > "$old/.auto-snapshot"
  touch -t "202601${day}0000" "$old"
done
mkdir -p "$SNAP_SUPPORT/session-recovery-manual"
touch -t 202001010000 "$SNAP_SUPPORT/session-recovery-manual"

FIND_CALLS="$TMP/find-calls"
find() {
  echo call >> "$FIND_CALLS"
  command find "$@"
}
snapshot_session_state "$SNAP_HOME" "$SNAP_SUPPORT"
unset -f find

snapshot_failure() {
  echo "session snapshot: $1" >&2
  exit 1
}
# The real registry names hundreds of sessions; one walk per session made updates take minutes.
[[ "$(wc -l < "$FIND_CALLS" | tr -d ' ')" == 1 ]] \
  || snapshot_failure "walked the transcript tree more than once"
snap="$(ls -dt "$SNAP_SUPPORT"/session-recovery-*/ | head -1)"
snap="${snap%/}"
[[ -f "$snap/.auto-snapshot" && -f "$snap/agent-resume/pane-1.json" ]] \
  || snapshot_failure "registry was not copied"
[[ "$(cat "$snap/sess-a.jsonl")" == "transcript a" ]] || snapshot_failure "sess-a was not copied"
[[ "$(cat "$snap/sess-b.jsonl")" == "transcript b" ]] \
  || snapshot_failure "a transcript in a nested project folder was not copied"
for unexpected in sess-missing.jsonl sess-codex.jsonl sess-other.jsonl .claude-session-ids; do
  [[ ! -e "$snap/$unexpected" ]] || snapshot_failure "unexpected $unexpected in the snapshot"
done
printf 'appended later\n' >> "$PROJECTS/-repo-a/sess-a.jsonl"
[[ "$(cat "$snap/sess-a.jsonl")" == "transcript a" ]] \
  || snapshot_failure "the snapshot changed when the live transcript did"
auto_count=0
for dir in "$SNAP_SUPPORT"/session-recovery-*/; do
  if [[ -f "$dir/.auto-snapshot" ]]; then auto_count=$((auto_count + 1)); fi
done
[[ "$auto_count" == 15 ]] || snapshot_failure "kept $auto_count automatic snapshots instead of 15"
[[ ! -e "$SNAP_SUPPORT/session-recovery-20260101-000000" ]] \
  || snapshot_failure "the oldest automatic snapshot was not pruned"
[[ -d "$SNAP_SUPPORT/session-recovery-manual" ]] || snapshot_failure "a manual snapshot was pruned"

echo "clinch update helper validation tests passed"

SWAP_BIN="$TMP/clinch-update-swap"
/usr/bin/clang -std=c11 -Os -Wall -Wextra -Werror \
  "$ROOT/resources/update/clinch-update-swap.c" -o "$SWAP_BIN"

mkdir -p "$TMP/symlink-swap/real/Clinch.app" "$TMP/symlink-swap/.Clinch.app.update-test"
ln -s "$TMP/symlink-swap/real/Clinch.app" "$TMP/symlink-swap/Clinch.app"
if "$SWAP_BIN" "$TMP/symlink-swap/Clinch.app" \
    "$TMP/symlink-swap/.Clinch.app.update-test" >/dev/null 2>&1; then
  echo "atomic updater unexpectedly followed a symbolic-link bundle" >&2
  exit 1
fi

make_app() {
  local app="$1" version="$2" sequence="$3"
  mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources/update"
  cp /bin/sleep "$app/Contents/MacOS/stable"
  cp "$SWAP_BIN" "$app/Contents/Resources/update/clinch-update-swap"
  /usr/bin/plutil -create xml1 "$app/Contents/Info.plist"
  /usr/bin/plutil -insert CFBundleIdentifier -string sh.clinch.Clinch "$app/Contents/Info.plist"
  /usr/bin/plutil -insert CFBundleExecutable -string stable "$app/Contents/Info.plist"
  /usr/bin/plutil -insert WarpVersion -string "$version" "$app/Contents/Info.plist"
  /usr/bin/plutil -insert ClinchUpdateSequence -string "$sequence" "$app/Contents/Info.plist"
  /usr/bin/codesign --force --deep --sign - "$app" >/dev/null
}

OPEN_MOCK="$TMP/open-mock"
printf '%s\n' \
  '#!/usr/bin/env bash' \
  'set -euo pipefail' \
  'destination="$1"; marker="$2"; finish="$3"' \
  'printf "%s %s\\n" "$destination" "$finish" >> "${CLINCH_UPDATE_OPEN_LOG:?}"' \
  'if [[ "$finish" == "1" && "${CLINCH_UPDATE_MOCK_ACK:-0}" == "1" ]]; then : > "$marker"; fi' \
  > "$OPEN_MOCK"
chmod +x "$OPEN_MOCK"

run_transaction() {
  local update_id="$1" acknowledge="$2" expected_version="$3" tamper="${4:-0}"
  local transaction transaction_home transaction_support update_dir destination staged archive
  local sha256 size ready cancel success open_log old_pid helper_pid
  transaction="$TMP/$update_id"
  transaction_home="$transaction/home"
  transaction_support="$transaction_home/Library/Application Support/sh.clinch.Clinch"
  update_dir="$transaction_support/autoupdate/$update_id"
  destination="$transaction/Applications/Clinch.app"
  staged="$transaction/staging/Clinch.app"
  archive="$update_dir/Clinch.app.zip"
  ready="$update_dir/helper-ready"
  cancel="$update_dir/helper-cancelled"
  success="$transaction_support/update-success-$update_id"
  open_log="$transaction/open.log"
  mkdir -p "$(dirname "$destination")" "$update_dir" "$(dirname "$staged")"
  make_app "$destination" v0.2099.01.01.0001 100
  make_app "$staged" v0.2099.01.02.0002 200
  /usr/bin/ditto -c -k --keepParent "$staged" "$archive"
  sha256="$(/usr/bin/shasum -a 256 "$archive" | /usr/bin/awk '{print $1}')"
  size="$(/usr/bin/stat -f %z "$archive")"
  if [[ "$tamper" == "1" ]]; then
    printf 'tampered after authentication\n' >> "$archive"
  fi
  # Execute the system sleep binary while presenting the exact app executable path as argv[0].
  # GitHub's macOS runner kills copied system Mach-O binaries launched from ad-hoc-signed fixture
  # bundles, but the helper's PID ownership check intentionally operates on the process command.
  /bin/bash -c 'exec -a "$1" /bin/sleep 120' _ "$destination/Contents/MacOS/stable" &
  old_pid=$!
  mkdir -p "$transaction_home/.warp/agent-resume/shutdown-owners"
  printf '%s\n' "$old_pid" > "$transaction_home/.warp/agent-resume/.app-terminating"
  printf 'protected owner\n' > "$transaction_home/.warp/agent-resume/shutdown-owners/aa11.json"

  set +e
  CLINCH_UPDATE_HELPER_TEST=1 \
  CLINCH_UPDATE_HELPER_SLEEP=0.01 \
  CLINCH_UPDATE_SUCCESS_ATTEMPTS=2 \
  CLINCH_UPDATE_OPEN_BIN="$OPEN_MOCK" \
  CLINCH_UPDATE_OPEN_LOG="$open_log" \
  CLINCH_UPDATE_MOCK_ACK="$acknowledge" \
    "$ROOT/resources/update/clinch-update-helper" install \
      "$destination/Contents/Resources/update/clinch-update-swap" \
      "$archive" "$destination" "$old_pid" "$(id -u)" "$transaction_home" \
      "$update_id" v0.2099.01.02.0002 200 "$sha256" "$size" \
      "$ready" "$cancel" "$success" &
  helper_pid=$!
  set -e

  if [[ "$tamper" == "1" ]]; then
    for _ in $(seq 1 200); do
      kill -0 "$helper_pid" 2>/dev/null || break
      sleep 0.01
    done
    if kill -0 "$helper_pid" 2>/dev/null; then
      kill "$helper_pid" 2>/dev/null || true
      fail "tampered archive helper did not fail promptly"
    fi
    if wait "$helper_pid"; then
      fail "tampered authenticated archive unexpectedly installed"
    fi
    kill "$old_pid"
    wait "$old_pid" 2>/dev/null || true
    [[ ! -f "$ready" ]]
    [[ "$(bundle_value "$destination" WarpVersion)" == "$expected_version" ]]
    [[ ! -e "$(dirname "$destination")/.Clinch.app.transaction-$update_id" ]]
    return
  fi

  for _ in $(seq 1 200); do
    [[ -f "$ready" ]] && break
    kill -0 "$helper_pid" 2>/dev/null || break
    sleep 0.01
  done
  [[ -f "$ready" ]] || { kill "$old_pid" 2>/dev/null || true; fail "helper never became ready"; }
  kill "$old_pid"
  wait "$old_pid" 2>/dev/null || true
  if [[ "$acknowledge" == "1" ]]; then
    if ! wait "$helper_pid"; then
      cat "$transaction_support/update-$update_id.log" >&2 || true
      fail "acknowledged update helper failed"
    fi
  elif wait "$helper_pid"; then
    fail "unacknowledged update unexpectedly succeeded"
  fi
  [[ "$(bundle_value "$destination" WarpVersion)" == "$expected_version" ]]
  [[ -f "$transaction_home/.warp/agent-resume/.app-terminating" ]]
  [[ "$(cat "$transaction_home/.warp/agent-resume/shutdown-owners/aa11.json")" == 'protected owner' ]]
}

run_transaction success-update 1 v0.2099.01.02.0002
run_transaction rollback-update 0 v0.2099.01.01.0001
run_transaction tampered-update 0 v0.2099.01.01.0001 1

echo "clinch update helper transaction and rollback tests passed"
