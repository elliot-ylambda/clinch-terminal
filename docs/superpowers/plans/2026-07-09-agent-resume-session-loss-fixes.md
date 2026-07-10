# Agent-Resume Session-Loss Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the agent-resume machinery survive a `make update` relaunch without losing or cross-planting sessions, and make any future incident diagnosable and reversible.

**Architecture:** All fixes live in the shell layer (`tools/agent-resume/*` + `script/update-installed-clinch`) — no Rust changes. Root causes from the 2026-07-09 incident, in causal order: (1) `open` forwards the caller's environment into the relaunched app, so the bridged pane that ran `make update` leaked its `CLAUDE_CODE_BRIDGE_SESSION_ID` into *every* restored pane; (2) `claude-capture.sh` trusts that env var blindly, so every pane's registry entry got stamped with the same bridge id; (3) at the next restore, panes teleported/attempted the *wrong* cloud session (cross-repo interactive pickers, dirty-tree failures, entry takeovers by teleported copies); (4) simultaneous restores can adopt the same fallback session twice; (5) entry overwrites are unlogged and unrecoverable. Fix order matches: trust boundary in the hook, env scrub at relaunch, an append-only journal, a cleanup subcommand, an atomic adoption claim.

**Tech Stack:** bash (hooks, registry CLI, update script), zsh (replay functions), existing self-contained test scripts under `tools/agent-resume/tests/`.

## Global Constraints

- **Work in an isolated git worktree** based on `clinch/main` (NOT `origin/master` — origin is upstream warpdotdev and is 100+ commits behind). The main checkout at `~/projects/clinch-terminal` is actively used by concurrent agent sessions; a live Codex agent currently owns its dirty tree. Use the `superpowers:using-git-worktrees` skill.
- Branch name: `agent-resume-incident-fixes`.
- Shell code only; match the comment-heavy style of the existing scripts (every non-obvious guard carries a *why* comment referencing the incident that motivated it).
- Tests are self-contained executables: `tools/agent-resume/tests/test_*.sh`, each printing `PASS` and exiting 0. Run any test directly by path; run all with `for t in tools/agent-resume/tests/test_*.sh; do echo "== $t"; "$t" || exit 1; done`.
- Registry entry format (single line, written by `warp-agent-resume write`) is `{ "command": "...", "cwd": "..." }` or `{ "command": "...", "cwd": "...", "bridge": "session_..." }` — several greps/seds depend on this exact shape; do not change it.
- No new runtime dependencies (no jq in the zsh/CLI layer — `claude-capture.sh` already uses jq, that's fine).

---

### Task 1: Capture hook must not trust an inherited bridge id

The core poisoning fix. `claude.zsh` is sourced by every interactive pane shell; have it record the `CLAUDE_CODE_BRIDGE_SESSION_ID` value the shell *inherited*. The capture hook then records a bridge id only when the live value **differs** from the inherited one — i.e. only when the owning `claude` process set it itself by actually bridging.

**Files:**
- Modify: `tools/agent-resume/claude.zsh` (top of file, after the header comment block, before `warp_agent_resume_resumable`)
- Modify: `tools/agent-resume/claude-capture.sh:184-187` (the `warp-agent-resume write` call site inside `_warp_agent_resume_capture_main`)
- Test: `tools/agent-resume/tests/test_claude_hook.sh`

**Interfaces:**
- Produces: env var `WARP_AGENT_RESUME_ENV_BRIDGE` (exported by every shell sourcing `claude.zsh`; consumed by `claude-capture.sh`, scrubbed by Task 2's relaunch scrub).

- [ ] **Step 1: Write the failing tests**

In `tools/agent-resume/tests/test_claude_hook.sh`, first extend the env pinning at line 20 (`unset CLAUDE_CODE_BRIDGE_SESSION_ID`) to also pin the new marker:

```bash
# Likewise pin the bridge id off: this test may itself run inside a bridged claude session,
# which would leak CLAUDE_CODE_BRIDGE_SESSION_ID into every capture below.
unset CLAUDE_CODE_BRIDGE_SESSION_ID WARP_AGENT_RESUME_ENV_BRIDGE
```

Then append before the final `echo "PASS"`:

```bash
# --- Bridge-id provenance (2026-07-09 env-leak incident) ---
# A bridge id identical to the shell-inherited one is ambient environment (e.g. leaked
# through the app relaunch), NOT evidence this session is bridged: entry written, bridge omitted.
rm -f "$f"
CLAUDE_CODE_BRIDGE_SESSION_ID="session_01LEAK" WARP_AGENT_RESUME_ENV_BRIDGE="session_01LEAK" \
  bash -c 'echo "{\"session_id\":\"sess-fff\",\"cwd\":\"/tmp/repo\"}" | "$0"' "$BIN/claude-capture.sh"
grep -q '"bridge"' "$f" && { echo "FAIL: inherited bridge id must not be recorded"; exit 1; }
grep -q 'sess-fff' "$f" || { echo "FAIL: entry itself must still be written"; exit 1; }

# A bridge id that DIFFERS from the inherited one was set by this claude process: recorded.
CLAUDE_CODE_BRIDGE_SESSION_ID="session_01MINE" WARP_AGENT_RESUME_ENV_BRIDGE="session_01LEAK" \
  bash -c 'echo "{\"session_id\":\"sess-fff\",\"cwd\":\"/tmp/repo\"}" | "$0"' "$BIN/claude-capture.sh"
grep -q '"bridge": "session_01MINE"' "$f" || { echo "FAIL: self-set bridge id must be recorded"; exit 1; }

# Inherited marker empty (the normal pane case: shell started clean, claude bridged later): recorded.
CLAUDE_CODE_BRIDGE_SESSION_ID="session_01FRESHB" WARP_AGENT_RESUME_ENV_BRIDGE="" \
  bash -c 'echo "{\"session_id\":\"sess-fff\",\"cwd\":\"/tmp/repo\"}" | "$0"' "$BIN/claude-capture.sh"
grep -q '"bridge": "session_01FRESHB"' "$f" || { echo "FAIL: bridge over empty inherited marker must be recorded"; exit 1; }

# No inherited marker at all (hook running outside a pane shell): value trusted as before.
CLAUDE_CODE_BRIDGE_SESSION_ID="session_01SOLO" \
  bash -c 'unset WARP_AGENT_RESUME_ENV_BRIDGE; echo "{\"session_id\":\"sess-fff\",\"cwd\":\"/tmp/repo\"}" | "$0"' "$BIN/claude-capture.sh"
grep -q '"bridge": "session_01SOLO"' "$f" || { echo "FAIL: bridge without inherited marker must be recorded"; exit 1; }
```

- [ ] **Step 2: Run the test to verify the new cases fail**

Run: `tools/agent-resume/tests/test_claude_hook.sh`
Expected: `FAIL: inherited bridge id must not be recorded` (exit 1) — the current hook records any non-empty `CLAUDE_CODE_BRIDGE_SESSION_ID`.

- [ ] **Step 3: Record the inherited value in claude.zsh**

Insert at the top of `tools/agent-resume/claude.zsh`, immediately after the header comment block (before `warp_agent_resume_resumable`):

```zsh
# Record the bridge id this shell INHERITED, so the capture hook can tell a bridge id the
# owning claude process set itself (real: that session is bridged) from one that was merely
# ambient in the pane environment. `open` forwards the caller's env into a relaunched app,
# so a `make update` run from a bridged pane stamped that pane's CLAUDE_CODE_BRIDGE_SESSION_ID
# into every restored pane's registry entry (2026-07-09 session-loss incident).
export WARP_AGENT_RESUME_ENV_BRIDGE="${CLAUDE_CODE_BRIDGE_SESSION_ID:-}"
```

- [ ] **Step 4: Gate the bridge id in claude-capture.sh**

In `_warp_agent_resume_capture_main`, replace the write call (lines 184–187):

```bash
  BIN="$(cd "$(dirname "$0")" && pwd)"
  "$BIN/warp-agent-resume" write "$WARP_TERMINAL_SESSION_UUID" \
    "warp_agent_resume_launch claude $sid$extra" "$cwd" \
    "${CLAUDE_CODE_BRIDGE_SESSION_ID:-}" >/dev/null 2>&1 || true
```

with:

```bash
  # Trust the bridge id only when the owning claude process set it itself: every pane shell
  # exports the value it inherited at startup (WARP_AGENT_RESUME_ENV_BRIDGE, claude.zsh), and
  # a live value identical to that is ambient environment -- e.g. leaked through `make
  # update`'s relaunch from a bridged pane (2026-07-09 incident) -- not evidence that THIS
  # session is bridged. When the marker is absent entirely (hook outside a pane shell) the
  # value is trusted as before. Known edge: while a leak is ACTIVE, the one pane whose real
  # bridge equals the leaked id (the pane that caused the leak) has its bridge suppressed
  # too -- acceptable, because the relaunch scrub in update-installed-clinch prevents the
  # leak from existing, and a bridged conversation always stays recoverable at claude.ai.
  local bridge="${CLAUDE_CODE_BRIDGE_SESSION_ID:-}"
  if [[ -n "$bridge" && -n "${WARP_AGENT_RESUME_ENV_BRIDGE+x}" \
     && "$bridge" == "${WARP_AGENT_RESUME_ENV_BRIDGE}" ]]; then
    bridge=""
  fi
  BIN="$(cd "$(dirname "$0")" && pwd)"
  "$BIN/warp-agent-resume" write "$WARP_TERMINAL_SESSION_UUID" \
    "warp_agent_resume_launch claude $sid$extra" "$cwd" \
    "$bridge" >/dev/null 2>&1 || true
```

(`bridge` is declared by the new `local` line; the existing `local` declaration list in `_warp_agent_resume_capture_main` stays unchanged.)

- [ ] **Step 5: Run the tests to verify they pass**

Run: `tools/agent-resume/tests/test_claude_hook.sh && tools/agent-resume/tests/test_claude_launch.sh`
Expected: both print `PASS` (the launch test guards against regressions in `claude.zsh` sourcing).

- [ ] **Step 6: Commit**

```bash
git add tools/agent-resume/claude.zsh tools/agent-resume/claude-capture.sh tools/agent-resume/tests/test_claude_hook.sh
git commit -m "fix(agent-resume): ignore inherited CLAUDE_CODE_BRIDGE_SESSION_ID in capture hook

A bridge id that is merely ambient in the pane environment (leaked through
an app relaunch) must not be recorded as the pane's bridge. Only a value
the owning claude process set itself -- one that differs from what the
shell inherited -- is evidence the session is bridged. Root cause #1 of
the 2026-07-09 session-loss incident."
```

---

### Task 2: Scrub session env vars when relaunching the app

Belt-and-suspenders for the same leak: `script/update-installed-clinch` relaunches the app with `open`, which forwards the caller's environment. Scrub every per-pane/per-session var before relaunching so the new app instance starts clean, whatever shell `make update` was typed in.

**Files:**
- Modify: `script/update-installed-clinch` (restructure into functions + main guard; swap `open "$dest"` for the scrubbed variant)
- Test: `tools/agent-resume/tests/test_update_env_scrub.sh` (new)

> **Merge note:** the unmerged branch `codex/launch-readiness-fixes` adds an `env -u` for
> Make process vars (`MAKEFLAGS`, `MFLAGS`, `GNUMAKEFLAGS`, `MAKELEVEL`, `MAKEOVERRIDES`,
> `MAKE_TERMOUT`, `MAKE_TERMERR`, `SKIP_SYNC`) at this same call site. This task's
> `CLINCH_SCRUB_VARS` includes that entire list, so this restructured file **supersedes**
> that change — whoever merges second should resolve the conflict by taking this file's
> version wholesale.

**Interfaces:**
- Consumes: `WARP_AGENT_RESUME_ENV_BRIDGE` from Task 1 (added to the scrub list).
- Produces: function `clinch_scrubbed_open <bundle-path>` and env override `CLINCH_OPEN_BIN` (test seam); the script becomes sourceable (functions only) like `claude-capture.sh`.

- [ ] **Step 1: Write the failing test**

Create `tools/agent-resume/tests/test_update_env_scrub.sh` (mode 0755):

```bash
#!/usr/bin/env bash
# Tests that the app-relaunch step of update-installed-clinch scrubs the per-session env
# vars `open` would otherwise forward into the new app instance (2026-07-09 env-leak
# incident: a bridged pane's CLAUDE_CODE_BRIDGE_SESSION_ID reached every restored pane).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
TMP="$(mktemp -d)"

# Sourcing must only define functions (main is guarded), like claude-capture.sh.
source "$ROOT/script/update-installed-clinch"

# Stub `open` that dumps the environment it received.
cat > "$TMP/open" <<EOF
#!/usr/bin/env bash
env > "$TMP/captured_env"
EOF
chmod +x "$TMP/open"
export CLINCH_OPEN_BIN="$TMP/open"

export CLAUDE_CODE_BRIDGE_SESSION_ID="session_01LEAK"
export CLAUDE_CODE_SESSION_ID="deadbeef"
export CLAUDECODE="1"
export CLAUDE_CODE_ENTRYPOINT="cli"
export WARP_TERMINAL_SESSION_UUID="pane-1"
export WARP_AGENT_RESUME_STARTED_FRESH="1"
export WARP_AGENT_RESUME_ENV_BRIDGE="session_01LEAK"
export MAKEFLAGS="n"
export SKIP_SYNC="1"
export UNRELATED_KEEP="yes"

clinch_scrubbed_open "/Applications/Fake.app"

for v in CLAUDE_CODE_BRIDGE_SESSION_ID CLAUDE_CODE_SESSION_ID CLAUDECODE CLAUDE_CODE_ENTRYPOINT \
         WARP_TERMINAL_SESSION_UUID WARP_AGENT_RESUME_STARTED_FRESH WARP_AGENT_RESUME_ENV_BRIDGE \
         MAKEFLAGS SKIP_SYNC; do
  grep -q "^$v=" "$TMP/captured_env" && { echo "FAIL: $v leaked through relaunch"; exit 1; }
done
grep -q '^UNRELATED_KEEP=yes' "$TMP/captured_env" || { echo "FAIL: unrelated env vars must survive"; exit 1; }
echo "PASS"
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `tools/agent-resume/tests/test_update_env_scrub.sh`
Expected: FAIL — sourcing the current script *executes* it (`usage:` error, exit 1), and `clinch_scrubbed_open` is not defined.

- [ ] **Step 3: Restructure update-installed-clinch**

Rewrite `script/update-installed-clinch` as functions plus a main guard. Keep the existing header comment and behavior byte-for-byte except where shown:

```bash
#!/usr/bin/env bash
# Replace the installed Clinch app with a freshly built bundle: gracefully quit
# the running app (so it checkpoints its session/SQLite), swap the bundle in
# /Applications, then relaunch it.
#
# Written to be safe to run detached — `make update` invokes it via nohup so the
# swap survives even when the terminal issuing the update IS Clinch itself
# (quitting Clinch would otherwise kill its child processes).
#
# Usage: script/update-installed-clinch <AppName> <source-bundle>
set -euo pipefail

# Env vars that must NOT reach the relaunched app: `open` forwards the caller's
# environment, the app hands it to every restored pane shell.
#
# Per-pane/per-session identity vars: the capture hooks would mistake the calling pane's
# identity for their own -- a bridged pane's CLAUDE_CODE_BRIDGE_SESSION_ID poisoned every
# pane's registry entry this way on 2026-07-09.
#
# Make process-control vars: this script normally runs under `make update`, and e.g.
# MAKEFLAGS=n leaking in would silently turn all later Make invocations in every pane into
# dry runs. SKIP_SYNC is a release-only override and must not escape either.
CLINCH_SCRUB_VARS=(
  CLAUDE_CODE_BRIDGE_SESSION_ID
  CLAUDE_CODE_SESSION_ID
  CLAUDECODE
  CLAUDE_CODE_ENTRYPOINT
  WARP_TERMINAL_SESSION_UUID
  WARP_AGENT_RESUME_STARTED_FRESH
  WARP_AGENT_RESUME_ENV_BRIDGE
  MAKEFLAGS
  MFLAGS
  GNUMAKEFLAGS
  MAKELEVEL
  MAKEOVERRIDES
  MAKE_TERMOUT
  MAKE_TERMERR
  SKIP_SYNC
)

# Relaunch the app with the session vars scrubbed. CLINCH_OPEN_BIN overrides `open` in tests.
clinch_scrubbed_open() {
  local bundle="$1" v
  local -a scrub=()
  for v in "${CLINCH_SCRUB_VARS[@]}"; do scrub+=(-u "$v"); done
  env "${scrub[@]}" "${CLINCH_OPEN_BIN:-open}" "$bundle"
}

clinch_update_main() {
  local app_name="${1:?usage: update-installed-clinch <AppName> <source-bundle>}"
  local src_bundle="${2:?usage: update-installed-clinch <AppName> <source-bundle>}"
  local dest="/Applications/${app_name}.app"

  if [ ! -d "$src_bundle" ]; then
    echo "✗ built bundle not found: $src_bundle" >&2
    exit 1
  fi

  # Match the running process by its executable path so we don't depend on the
  # binary's name (the stable channel's executable is "stable", not "Clinch").
  running() { pgrep -f "${dest}/Contents/MacOS/" >/dev/null 2>&1; }

  if running; then
    echo "Quitting running ${app_name}…"
    osascript -e "tell application \"${app_name}\" to quit" || true
    # Wait up to ~20s for it to flush state and exit before swapping the bundle.
    for _ in $(seq 1 40); do
      running || break
      sleep 0.5
    done
    if running; then
      echo "⚠ ${app_name} did not quit; forcing…" >&2
      pkill -f "${dest}/Contents/MacOS/" || true
      sleep 1
    fi
  fi

  echo "Installing ${src_bundle} → ${dest}…"
  rm -rf "$dest"
  cp -R "$src_bundle" "$dest"

  echo "Relaunching ${app_name}…"
  clinch_scrubbed_open "$dest"
  echo "✓ ${app_name} updated and relaunched."
}

# Run only when executed directly; sourcing (tests) just loads the functions.
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  clinch_update_main "$@"
fi
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `tools/agent-resume/tests/test_update_env_scrub.sh`
Expected: `PASS`

- [ ] **Step 5: Smoke the executed path still works**

Run: `script/update-installed-clinch; echo "exit=$?"`
Expected: the `usage: update-installed-clinch <AppName> <source-bundle>` error and `exit=1` (the parameter check still fires when executed, not sourced). Then run `bash -n script/update-installed-clinch` — expected: no output (syntax OK).

- [ ] **Step 6: Commit**

```bash
git add script/update-installed-clinch tools/agent-resume/tests/test_update_env_scrub.sh
git commit -m "fix(update): scrub per-session env vars before relaunching Clinch

open(1) forwards the caller's environment into the launched app, so a
make update run from a bridged Claude pane leaked that pane's
CLAUDE_CODE_BRIDGE_SESSION_ID (and pane uuid etc.) into every pane of the
relaunched app. Relaunch through env -u for each session-identity var."
```

---

### Task 3: Append-only registry journal (backup + diagnosability)

Every destructive registry operation (overwrite with different content, remove) first appends the *old* entry line to a monthly journal file. The 2026-07-09 recovery needed scrollback archaeology because overwritten entries were simply gone; with the journal, recovery is `grep <pane-uuid> journal-*.log`.

**Files:**
- Modify: `tools/agent-resume/warp-agent-resume`
- Test: `tools/agent-resume/tests/test_registry_cli.sh` (new)

**Interfaces:**
- Produces: journal files `$DIR/journal-YYYYMM.log`, lines of `<ISO8601-UTC>\t<pane-uuid>\t<old-entry-json>`; internal helper `journal_old_entry <uuid> <file>` (also used by Task 4's `scrub-bridge`).

- [ ] **Step 1: Write the failing test**

Create `tools/agent-resume/tests/test_registry_cli.sh` (mode 0755):

```bash
#!/usr/bin/env bash
# Tests the registry CLI: write/remove journal the entry they destroy (append-only,
# monthly file) so an overwritten entry is always recoverable -- the 2026-07-09 incident
# required scrollback archaeology because it wasn't.
set -euo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d)"
export WARP_AGENT_RESUME_DIR="$TMP/reg"
CLI="$HERE/warp-agent-resume"

# write creates the entry.
"$CLI" write pane1 'warp_agent_resume_launch claude aaa' /tmp/repo
grep -q '"command": "warp_agent_resume_launch claude aaa"' "$TMP/reg/pane1.json" || { echo "FAIL: write"; exit 1; }

# First write: nothing destroyed, no journal.
ls "$TMP/reg"/journal-*.log >/dev/null 2>&1 && { echo "FAIL: journal written on first write"; exit 1; }

# Overwrite with different content: the OLD entry is journaled (ts<TAB>uuid<TAB>old-json).
# BSD grep (macOS) has no -P; use -E with a literal tab via $'…' interpolation.
"$CLI" write pane1 'warp_agent_resume_launch claude bbb' /tmp/repo
j=("$TMP/reg"/journal-*.log)
[[ -f "${j[0]}" ]] || { echo "FAIL: overwrite did not journal"; exit 1; }
grep -Eq $'^[0-9]{4}-[0-9]{2}-[0-9]{2}T[^\t]+\tpane1\t\\{ "command": "warp_agent_resume_launch claude aaa"' "${j[0]}" \
  || { echo "FAIL: journal line malformed or missing old entry"; exit 1; }

# Rewrite with identical content: no new journal line.
lines_before=$(wc -l < "${j[0]}")
"$CLI" write pane1 'warp_agent_resume_launch claude bbb' /tmp/repo
(( $(wc -l < "${j[0]}") == lines_before )) || { echo "FAIL: identical rewrite must not journal"; exit 1; }

# remove journals the removed entry, then deletes it.
"$CLI" remove pane1
grep -q 'claude bbb' "${j[0]}" || { echo "FAIL: remove did not journal"; exit 1; }
[[ ! -f "$TMP/reg/pane1.json" ]] || { echo "FAIL: remove did not delete"; exit 1; }

# remove of a missing entry is still a silent no-op.
"$CLI" remove pane-never-existed
echo "PASS"
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `tools/agent-resume/tests/test_registry_cli.sh`
Expected: `FAIL: overwrite did not journal` (exit 1).

- [ ] **Step 3: Implement the journal in warp-agent-resume**

Add the helper after `json_escape` and wire it into `write` and `remove`:

```bash
# Append the current content of entry <file> to this month's journal before it is
# overwritten or removed. Append-only and per-month so the registry is always recoverable
# (the 2026-07-09 incident lost entries to same-day overwrites with no trace). A line is
# <ISO8601-UTC>\t<pane-uuid>\t<old-entry-json>.
journal_old_entry() { # journal_old_entry <uuid> <entry-file>
  [[ -f "$2" ]] || return 0
  local jf="$DIR/journal-$(date -u +%Y%m).log"
  printf '%s\t%s\t%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$1" "$(cat "$2")" >> "$jf"
  chmod 600 "$jf" 2>/dev/null || true
}
```

In the `write` branch, journal only when content actually changes — after building `$tmp`, before `mv`:

```bash
    chmod 600 "$tmp"
    if [[ -f "$DIR/$uuid.json" ]] && ! cmp -s "$tmp" "$DIR/$uuid.json"; then
      journal_old_entry "$uuid" "$DIR/$uuid.json"
    fi
    mv -f "$tmp" "$DIR/$uuid.json"
```

In the `remove` branch:

```bash
  remove)
    uuid="$1"
    journal_old_entry "$uuid" "$DIR/$uuid.json"
    rm -f "$DIR/$uuid.json"
    ;;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `tools/agent-resume/tests/test_registry_cli.sh && tools/agent-resume/tests/test_claude_hook.sh`
Expected: both `PASS` (the hook test exercises `write` heavily — it must not break on journaling).

- [ ] **Step 5: Commit**

```bash
git add tools/agent-resume/warp-agent-resume tools/agent-resume/tests/test_registry_cli.sh
git commit -m "feat(agent-resume): journal every destructive registry op

write (on content change) and remove append the old entry line to
~/.warp/agent-resume/journal-YYYYMM.log before destroying it, so any
future clobber is diagnosable and reversible with grep."
```

---

### Task 4: `scrub-bridge` subcommand to clean poisoned entries

Cleanup tool for the incident's residue: strip a given (leaked) bridge id from every entry that records it, journaling each change. This is what the operator runs for `session_011oGhGVPien5AbkKPi7NPMv` and `session_01QWSd4ysdZr5ZeJkpL6bbV4` after merging.

**Files:**
- Modify: `tools/agent-resume/warp-agent-resume` (new case branch + usage string)
- Test: `tools/agent-resume/tests/test_registry_cli.sh` (extend)

**Interfaces:**
- Consumes: `journal_old_entry` from Task 3.
- Produces: CLI `warp-agent-resume scrub-bridge <bridge-id>`; prints `scrubbed bridge from N entries`.

- [ ] **Step 1: Write the failing test**

Append to `tools/agent-resume/tests/test_registry_cli.sh` before `echo "PASS"`:

```bash
# scrub-bridge strips only entries recording that exact bridge id, journaling each.
"$CLI" write paneA 'warp_agent_resume_launch claude ccc' /tmp/a session_01LEAK
"$CLI" write paneB 'warp_agent_resume_launch claude ddd' /tmp/b session_01REAL
"$CLI" write paneC 'warp_agent_resume_launch claude eee' /tmp/c
out="$("$CLI" scrub-bridge session_01LEAK)"
[[ "$out" == "scrubbed bridge from 1 entries" ]] || { echo "FAIL: scrub-bridge count ($out)"; exit 1; }
grep -q '"bridge"' "$TMP/reg/paneA.json" && { echo "FAIL: scrub left the poisoned bridge"; exit 1; }
grep -q '"command": "warp_agent_resume_launch claude ccc"' "$TMP/reg/paneA.json" || { echo "FAIL: scrub damaged the entry"; exit 1; }
grep -q '"bridge": "session_01REAL"' "$TMP/reg/paneB.json" || { echo "FAIL: scrub hit a different bridge"; exit 1; }
grep -q 'session_01LEAK' "${j[0]}" || { echo "FAIL: scrub not journaled"; exit 1; }

# Empty argument is an error.
"$CLI" scrub-bridge "" 2>/dev/null && { echo "FAIL: empty bridge id must error"; exit 1; }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `tools/agent-resume/tests/test_registry_cli.sh`
Expected: usage error from the unknown `scrub-bridge` command → `FAIL: scrub-bridge count` (exit 1).

- [ ] **Step 3: Implement scrub-bridge**

Add a case branch before `*)` in `tools/agent-resume/warp-agent-resume`:

```bash
  scrub-bridge)
    # Strip a (leaked) bridge id from every entry recording it. Entries are one line in the
    # exact shape this script writes, so a tail-anchored sed is safe. Each change is
    # journaled first. Cleanup tool for the 2026-07-09 env-leak incident.
    bridge="${1:-}"
    [[ -n "$bridge" ]] || { echo "scrub-bridge: empty bridge id" >&2; exit 2; }
    shopt -s nullglob
    n=0
    for f in "$DIR"/*.json; do
      grep -qF "\"bridge\": \"$bridge\"" "$f" || continue
      uuid="$(basename "${f%.json}")"
      journal_old_entry "$uuid" "$f"
      tmp="$f.tmp.$$"
      sed 's/, "bridge": "[^"]*" }$/ }/' "$f" > "$tmp"
      chmod 600 "$tmp"
      mv -f "$tmp" "$f"
      n=$((n+1))
    done
    echo "scrubbed bridge from $n entries"
    ;;
```

Update the usage line in the `*)` branch to:

```bash
    echo "usage: warp-agent-resume {write <uuid> <command> <cwd> [bridge]|remove <uuid>|scrub-bridge <bridge-id>}" >&2
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `tools/agent-resume/tests/test_registry_cli.sh`
Expected: `PASS`

- [ ] **Step 5: Commit**

```bash
git add tools/agent-resume/warp-agent-resume tools/agent-resume/tests/test_registry_cli.sh
git commit -m "feat(agent-resume): scrub-bridge subcommand to strip a leaked bridge id

Removes the given bridge id from every registry entry recording it
(journaled). Cleanup for entries poisoned by the 2026-07-09 env leak."
```

---

### Task 5: Atomic adoption claims (no double-adoption at restore)

At restore, all panes replay simultaneously; two panes with dead ids in the same cwd can both grep the registry *before* either adopted session's SessionStart re-captures, and adopt the **same** session. Claim the adopted id atomically with a claim file (`noclobber`), with a staleness window so crashed claims don't block adoption forever.

**Files:**
- Modify: `tools/agent-resume/claude.zsh` (`warp_agent_resume_fallback_id`)
- Test: `tools/agent-resume/tests/test_claude_launch.sh`

**Interfaces:**
- Produces: claim files `$reg/.adopt-claim-<session-id>` (empty; ignored by the unclaimed-grep since they contain no launch command; stale after 120s, override with `WARP_AGENT_RESUME_CLAIM_TTL` seconds for tests).

- [ ] **Step 1: Write the failing test**

Append to `tools/agent-resume/tests/test_claude_launch.sh` before the final `echo "PASS"`:

```zsh
# --- Adoption claims: simultaneous restores must not adopt the same session twice ---
# The fake claude never runs the SessionStart hook, so (as in the real race window) nothing
# re-captures the adopted id into the registry; only the claim file can prevent a double adopt.
export WARP_AGENT_RESUME_DIR="$TMP/reg3"
mkdir -p "$WARP_AGENT_RESUME_DIR"
rm -f "$TMP/last_args"
( cd "$WORK" && HOME="$EHOME" warp_agent_resume_launch claude dead-A )
grep -q -- '--resume lost-2' "$TMP/last_args" || { echo "FAIL: first pane should adopt newest"; exit 1; }
rm -f "$TMP/last_args"
( cd "$WORK" && HOME="$EHOME" warp_agent_resume_launch claude dead-B )
grep -q -- '--resume lost-1' "$TMP/last_args" || { echo "FAIL: claimed session adopted twice"; exit 1; }

# A stale claim (older than the TTL) is reclaimable -- a crashed claimer must not block forever.
rm -f "$TMP/last_args" "$WARP_AGENT_RESUME_DIR/.adopt-claim-lost-1"
touch -t 202601010000 "$WARP_AGENT_RESUME_DIR/.adopt-claim-lost-2"
( cd "$WORK" && HOME="$EHOME" warp_agent_resume_launch claude dead-C )
grep -q -- '--resume lost-2' "$TMP/last_args" || { echo "FAIL: stale claim should be reclaimable"; exit 1; }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `tools/agent-resume/tests/test_claude_launch.sh`
Expected: `FAIL: claimed session adopted twice` (exit 1) — both launches adopt `lost-2` today.

- [ ] **Step 3: Implement the claim in warp_agent_resume_fallback_id**

In `tools/agent-resume/claude.zsh`, change the `setopt` line of `warp_agent_resume_fallback_id` to include `noclobber`, and add the claim check after the unclaimed-grep `continue` (line 66):

```zsh
warp_agent_resume_fallback_id() {
  setopt localoptions extendedglob nullglob noclobber
  local agent="$1" cwd="${2:-$PWD}"
  [[ "$agent" == claude ]] || return 1
  local reg="${WARP_AGENT_RESUME_DIR:-$HOME/.warp/agent-resume}"
  local match="\"cwd\":\"$cwd\"" f id claim
  local -a fresh_claim
  for f in "$HOME"/.claude/projects/**/*.jsonl(N.om); do
    [[ "$f" == */subagents/* ]] && continue   # sidechain transcripts are not resumable sessions
    head -c 131072 "$f" 2>/dev/null | grep -qF -- "$match" || continue
    id="${${f:t}%.jsonl}"
    warp_agent_resume_resumable "$agent" "$id" || continue
    grep -Eqsr "warp_agent_resume_launch claude $id( |\")" "$reg" && continue
    # Atomic claim: at restore every pane replays at once, and the adopted session's own
    # SessionStart hook (which records the claim in the registry proper) races this scan --
    # two dead-id panes could otherwise adopt the same session. noclobber makes creation
    # atomic (subshell form: unambiguous zsh parsing); a claim older than the TTL
    # (WARP_AGENT_RESUME_CLAIM_TTL seconds, default 120) is a crashed claimer, taken over.
    mkdir -p "$reg" 2>/dev/null
    claim="$reg/.adopt-claim-$id"
    if ! ( : > "$claim" ) 2>/dev/null; then
      fresh_claim=("$claim"(#qNms-${WARP_AGENT_RESUME_CLAIM_TTL:-120}))
      (( ${#fresh_claim} )) && continue
      rm -f "$claim"
      ( : > "$claim" ) 2>/dev/null || continue
    fi
    printf '%s' "$id"
    return 0
  done
  return 1
}
```

- [ ] **Step 4: Run the full launch test to verify it passes**

Run: `tools/agent-resume/tests/test_claude_launch.sh`
Expected: `PASS` — including the three pre-existing adoption cases (the new claim files live in `reg2`/`reg3` and must not break the "claimed session must not be stolen" case, which relies on the registry-entry grep, not claims).

- [ ] **Step 5: Commit**

```bash
git add tools/agent-resume/claude.zsh tools/agent-resume/tests/test_claude_launch.sh
git commit -m "fix(agent-resume): claim adopted sessions atomically during restore

Simultaneous pane restores could adopt the same fallback session twice
(the adopted session's SessionStart re-capture races the registry scan).
An .adopt-claim-<id> file created under noclobber closes the window; a
claim older than 120s is stale and taken over."
```

---

### Task 6: Docs, reinstall, and operator cleanup runbook

**Files:**
- Modify: `tools/agent-resume/README.md` (document the trust boundary, journal, scrub-bridge, claim files)
- No code changes.

**Interfaces:**
- Consumes: everything above.

- [ ] **Step 1: Document the new behavior**

Add a section to `tools/agent-resume/README.md` (match its existing tone; place after the section describing the registry entry format):

```markdown
## Incident hardening (2026-07-09)

- **Bridge-id trust boundary.** Every pane shell exports `WARP_AGENT_RESUME_ENV_BRIDGE`
  (the `CLAUDE_CODE_BRIDGE_SESSION_ID` it inherited). The capture hook records a bridge id
  only when the live value differs — i.e. the owning `claude` set it by actually bridging.
  `open` forwards the caller's env into a relaunched app, so without this a `make update`
  run from a bridged pane stamps that pane's bridge id into every entry (2026-07-09
  session-loss incident). `script/update-installed-clinch` additionally relaunches the app
  through `env -u` for every session-identity var.
- **Journal.** `warp-agent-resume` appends the old entry line to
  `~/.warp/agent-resume/journal-YYYYMM.log` before any overwrite (content change) or
  remove: `<ISO8601-UTC> TAB <pane-uuid> TAB <old-entry-json>`. To see what a pane pointed
  at before an incident: `grep <pane-uuid> ~/.warp/agent-resume/journal-*.log`.
- **`scrub-bridge <bridge-id>`** strips a leaked bridge id from every entry recording it
  (journaled). Use after an env-leak incident.
- **Adoption claims.** `.adopt-claim-<session-id>` files in the registry dir make the
  dead-id adoption fallback atomic across simultaneously restoring panes; they are empty,
  ignored by all other readers, and stale after 120s.
```

- [ ] **Step 2: Run the whole test suite**

Run: `for t in tools/agent-resume/tests/test_*.sh; do echo "== $t"; "$t" || exit 1; done`
Expected: every file prints `PASS`.

- [ ] **Step 3: Commit**

```bash
git add tools/agent-resume/README.md
git commit -m "docs(agent-resume): document bridge trust boundary, journal, scrub-bridge, claims"
```

- [ ] **Step 4: Operator runbook (post-merge, on the user's machine — not part of the branch)**

After merge to `clinch/main`, in this order:
1. `tools/agent-resume/install.sh` — refreshes `~/.warp/agent-resume-bin/` with the fixed scripts (idempotent). Verify each changed file synced (no output expected):
   `for f in claude-capture.sh claude.zsh warp-agent-resume; do diff ~/.warp/agent-resume-bin/$f tools/agent-resume/$f; done`
2. `~/.warp/agent-resume-bin/warp-agent-resume scrub-bridge session_011oGhGVPien5AbkKPi7NPMv`
3. `~/.warp/agent-resume-bin/warp-agent-resume scrub-bridge session_01QWSd4ysdZr5ZeJkpL6bbV4`
4. Open new pane shells (or restart Clinch) so `WARP_AGENT_RESUME_ENV_BRIDGE` is exported everywhere before the next `make update`.

---

## Self-Review Notes

- **Spec coverage:** issue (1) env leak → Tasks 1+2; (2) teleport-failure cascade → root-caused as a *consequence* of poisoning (a teleported copy legitimately takes over the pane entry; with poisoning fixed, teleport only ever targets the pane's true bridge), no separate launcher change needed — the existing fast-fail fallback and `STARTED_FRESH` guard are already correct and tested; (3) hook trusts inherited env → Task 1; (4) cross-window adoption → race closed by Task 5; the cwd-only matching limitation is inherent (the registry has no window identity) and documented; (5) no backups → Task 3 journal (superior to periodic snapshots: captures every transition, no cron).
- **Dead code:** Task 2's restructure replaces the whole script body — no orphaned copies of the old inline flow remain. No flags, settings, or code paths become vestigial; the entry format and all existing greps are unchanged.
- **Type/name consistency check:** `WARP_AGENT_RESUME_ENV_BRIDGE` (Tasks 1, 2, 6), `journal_old_entry` (Tasks 3, 4), `CLINCH_OPEN_BIN`/`clinch_scrubbed_open` (Task 2), claim TTL env `WARP_AGENT_RESUME_CLAIM_TTL` (Task 5) — names match across tasks.
