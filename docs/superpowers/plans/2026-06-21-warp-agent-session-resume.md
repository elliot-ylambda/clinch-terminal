# Auto-resume Agent Sessions on Pane Restore — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When Warp restores panes on relaunch, each pane that had a live Claude Code / Codex session auto-runs the exact resume command for *that* pane's session after its shell boots.

**Architecture:** Warp stays agent-agnostic — Rust persists one opaque `on_restore_command` string per pane (keyed by the stable pane UUID) and replays it after the `Bootstrapped` event. All agent-specific capture lives in user-space scripts (a `claude` wrapper, Codex hooks) that write a pane-UUID-keyed JSON registry at `~/.warp/agent-resume/`.

**Tech Stack:** Rust (gpui app + Diesel/SQLite persistence), zsh/bash scripts, `serde_json`, `hex`. Tests via `cargo nextest run` and self-contained bash test scripts.

**Design doc:** `docs/superpowers/specs/2026-06-20-warp-agent-session-resume-design.md`

## Global Constraints

- **Platform:** macOS only. Personal local build. No feature flag.
- **Agents:** Claude Code (`claude`) + Codex CLI (`codex`).
- **Registry path:** `~/.warp/agent-resume/<pane_uuid_hex>.json`, dir mode `0700`, files mode `0600`, atomic writes (temp + `rename`).
- **Registry key:** lowercase hex of the pane UUID bytes — MUST byte-for-byte match `$WARP_TERMINAL_SESSION_UUID` as Warp sets it (`app/src/terminal/focus_env.rs`). Verify casing equality before relying on it.
- **Registry value (verbatim):** `{ "command": "<resume command>", "cwd": "<absolute path>" }`.
- **Resume commands:** Claude → `claude --resume <id>`; Codex → `codex resume <id>`.
- **Snapshot struct:** `TerminalPaneSnapshot` (`app/src/app_state.rs:202`). Diesel models in `crates/persistence/src/{schema.rs,model.rs}`, migrations in `crates/persistence/migrations/`.
- **Run a single test:** `cargo nextest run -p <crate> <test_name>`. Scripts: `bash tools/agent-resume/tests/<name>.sh`.

---

## File Structure

| File | Responsibility |
|---|---|
| `tools/agent-resume/warp-agent-resume` (create) | Shared CLI: `write <uuid> <command> <cwd>` / `remove <uuid>`; owns registry path, perms, atomic writes |
| `tools/agent-resume/claude.zsh` (create) | `claude()` wrapper function: capture on fresh start / explicit `--resume <id>`, passthrough otherwise |
| `tools/agent-resume/codex-session-start.sh` (create) | Codex `SessionStart` hook: stdin JSON + `$WARP_TERMINAL_SESSION_UUID` → registry write |
| `tools/agent-resume/codex-session-end.sh` (create) | Codex `SessionEnd` hook: registry remove |
| `tools/agent-resume/config.toml.snippet` (create) | Codex hook registration to paste into `~/.codex/config.toml` |
| `tools/agent-resume/install.sh` (create) | Idempotent installer for wrapper + hooks |
| `tools/agent-resume/tests/*.sh` (create) | Self-contained bash tests (no bats dependency) |
| `app/src/agent_resume.rs` (create) | Rust registry reader: `read_on_restore_command(uuid)` (+ pure testable inner fn) |
| `app/src/app_state.rs` (modify ~202) | Add `on_restore_command: Option<String>` to `TerminalPaneSnapshot` |
| `crates/persistence/migrations/<ts>_add_on_restore_command_to_terminal_panes/{up,down}.sql` (create) | DB column |
| `crates/persistence/src/schema.rs` (modify ~406) | Add column to `terminal_panes!` table |
| `crates/persistence/src/model.rs` (modify ~589) | Add field to `NewTerminalPane` + `TerminalPane` |
| `app/src/persistence/sqlite.rs` (modify ~1175, ~2162) | Write/read the column |
| `app/src/pane_group/pane/terminal_pane.rs` (modify ~581) | Populate field from registry in `snapshot()` |
| `app/src/pane_group/mod.rs` (modify ~1655) | Replay: subscribe to `Bootstrapped`, `write_command` once |
| `crates/integration/src/test/session_restoration.rs` (modify) | Round-trip + replay integration test |

---

## Task 1: Shared registry CLI (`warp-agent-resume`)

**Files:**
- Create: `tools/agent-resume/warp-agent-resume`
- Test: `tools/agent-resume/tests/test_registry_cli.sh`

**Interfaces:**
- Produces: a script honoring `WARP_AGENT_RESUME_DIR` (default `$HOME/.warp/agent-resume`) with subcommands:
  - `warp-agent-resume write <uuid_hex> <command> <cwd>` → writes `<uuid_hex>.json` atomically
  - `warp-agent-resume remove <uuid_hex>` → deletes `<uuid_hex>.json` (no error if absent)

- [ ] **Step 1: Write the failing test**

```bash
# tools/agent-resume/tests/test_registry_cli.sh
#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
export WARP_AGENT_RESUME_DIR="$(mktemp -d)/agent-resume"
CLI="$HERE/warp-agent-resume"

"$CLI" write deadbeef "claude --resume abc-123" "/tmp/proj"
f="$WARP_AGENT_RESUME_DIR/deadbeef.json"
[[ -f "$f" ]] || { echo "FAIL: file not created"; exit 1; }
grep -q '"command": "claude --resume abc-123"' "$f" || { echo "FAIL: command missing"; exit 1; }
grep -q '"cwd": "/tmp/proj"' "$f" || { echo "FAIL: cwd missing"; exit 1; }
perms="$(stat -f '%Lp' "$f")"; [[ "$perms" == "600" ]] || { echo "FAIL: file perms $perms"; exit 1; }
dperms="$(stat -f '%Lp' "$WARP_AGENT_RESUME_DIR")"; [[ "$dperms" == "700" ]] || { echo "FAIL: dir perms $dperms"; exit 1; }

"$CLI" remove deadbeef
[[ ! -f "$f" ]] || { echo "FAIL: file not removed"; exit 1; }
"$CLI" remove deadbeef   # must be idempotent / no error
echo "PASS"
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bash tools/agent-resume/tests/test_registry_cli.sh`
Expected: FAIL (script `warp-agent-resume` does not exist → non-zero exit).

- [ ] **Step 3: Write minimal implementation**

```bash
# tools/agent-resume/warp-agent-resume
#!/usr/bin/env bash
set -euo pipefail

DIR="${WARP_AGENT_RESUME_DIR:-$HOME/.warp/agent-resume}"

json_escape() { # escape backslash and double-quote for JSON string body
  local s="$1"; s="${s//\\/\\\\}"; s="${s//\"/\\\"}"; printf '%s' "$s"
}

cmd="${1:-}"; shift || true
case "$cmd" in
  write)
    uuid="$1"; command_str="$2"; cwd="$3"
    [[ -n "$uuid" ]] || { echo "write: empty uuid" >&2; exit 2; }
    mkdir -p "$DIR"; chmod 700 "$DIR"
    tmp="$DIR/$uuid.json.tmp.$$"
    printf '{ "command": "%s", "cwd": "%s" }\n' \
      "$(json_escape "$command_str")" "$(json_escape "$cwd")" > "$tmp"
    chmod 600 "$tmp"
    mv -f "$tmp" "$DIR/$uuid.json"
    ;;
  remove)
    uuid="$1"
    rm -f "$DIR/$uuid.json"
    ;;
  *)
    echo "usage: warp-agent-resume {write <uuid> <command> <cwd>|remove <uuid>}" >&2
    exit 2
    ;;
esac
```

Then: `chmod +x tools/agent-resume/warp-agent-resume`

- [ ] **Step 4: Run test to verify it passes**

Run: `bash tools/agent-resume/tests/test_registry_cli.sh`
Expected: `PASS`

- [ ] **Step 5: Commit**

```bash
git add tools/agent-resume/warp-agent-resume tools/agent-resume/tests/test_registry_cli.sh
git commit -m "feat(agent-resume): shared registry CLI with atomic writes"
```

---

## Task 2: Claude wrapper function

**Files:**
- Create: `tools/agent-resume/claude.zsh`
- Test: `tools/agent-resume/tests/test_claude_wrapper.sh`

**Interfaces:**
- Consumes: `warp-agent-resume` (Task 1) on `PATH`.
- Produces: a `claude()` shell function. Behavior:
  - No session-controlling flag → generate UUID, `warp-agent-resume write $WARP_TERMINAL_SESSION_UUID "claude --resume <uuid>" "$PWD"`, exec real binary with `--session-id <uuid>` + original args; `remove` on exit.
  - `--resume <id>` / `-r <id>` present → `write` with `"claude --resume <id>"`, exec unchanged; `remove` on exit.
  - `--continue`/`-c`, `--print`/`-p`, or no `$WARP_TERMINAL_SESSION_UUID` → exec unchanged, no registry writes.

- [ ] **Step 1: Write the failing test**

```bash
# tools/agent-resume/tests/test_claude_wrapper.sh
#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d)"
export WARP_AGENT_RESUME_DIR="$TMP/reg"
export PATH="$TMP/bin:$HERE:$PATH"
mkdir -p "$TMP/bin"
# Fake `claude` binary that records the args it was invoked with, then exits 0.
cat > "$TMP/bin/claude" <<'EOF'
#!/usr/bin/env bash
echo "$@" > "$WARP_AGENT_RESUME_DIR/../last_args"
exit 0
EOF
chmod +x "$TMP/bin/claude"

# Load wrapper; CLAUDE_REAL_BIN points at the fake to avoid `command -v` picking the function.
export CLAUDE_REAL_BIN="$TMP/bin/claude"
source "$HERE/claude.zsh" 2>/dev/null || . "$HERE/claude.zsh"

# Case A: fresh start in a Warp pane → registry written, --session-id injected
export WARP_TERMINAL_SESSION_UUID="aa11"
( cd "$TMP" && claude )
[[ -f "$WARP_AGENT_RESUME_DIR/aa11.json" ]] || { echo "FAIL A: no registry file"; exit 1; }
grep -q '"command": "claude --resume ' "$WARP_AGENT_RESUME_DIR/aa11.json" || { echo "FAIL A: bad command"; exit 1; }
grep -q -- '--session-id' "$TMP/last_args" || { echo "FAIL A: no --session-id passed"; exit 1; }

# Case B: explicit --resume <id> → registry records that id, no --session-id added
rm -f "$WARP_AGENT_RESUME_DIR/aa11.json"
claude --resume zzz-999
grep -q '"command": "claude --resume zzz-999"' "$WARP_AGENT_RESUME_DIR/aa11.json" || { echo "FAIL B"; exit 1; }

# Case C: --continue → no registry file
rm -f "$WARP_AGENT_RESUME_DIR/aa11.json"
claude --continue
[[ ! -f "$WARP_AGENT_RESUME_DIR/aa11.json" ]] || { echo "FAIL C: continue should not record"; exit 1; }

# Case D: outside a Warp pane → no registry file
unset WARP_TERMINAL_SESSION_UUID
claude
[[ -z "$(ls -A "$WARP_AGENT_RESUME_DIR" 2>/dev/null)" ]] || { echo "FAIL D: recorded outside pane"; exit 1; }
echo "PASS"
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bash tools/agent-resume/tests/test_claude_wrapper.sh`
Expected: FAIL (`claude.zsh` missing).

- [ ] **Step 3: Write minimal implementation**

```bash
# tools/agent-resume/claude.zsh
# Wrapper that records a resumable Claude session keyed by the Warp pane UUID.
claude() {
  local real="${CLAUDE_REAL_BIN:-}"
  if [[ -z "$real" ]]; then
    # Resolve the real binary, skipping this function.
    real="$(whence -p claude 2>/dev/null || command -v -p claude 2>/dev/null || true)"
  fi
  [[ -z "$real" ]] && { echo "claude: real binary not found" >&2; return 127; }

  # Only act inside a Warp pane.
  if [[ -z "${WARP_TERMINAL_SESSION_UUID:-}" ]]; then
    "$real" "$@"; return $?
  fi

  local uuid="$WARP_TERMINAL_SESSION_UUID"
  local args=("$@") resume_id="" mode="fresh"
  local i
  for ((i=1; i<=${#args[@]}; i++)); do
    case "${args[i]}" in
      --resume|-r) resume_id="${args[i+1]:-}"; mode="resume" ;;
      --resume=*)  resume_id="${args[i]#*=}"; mode="resume" ;;
      --continue|-c|--print|-p) mode="passthrough" ;;
      --session-id|--session-id=*) mode="passthrough" ;;
    esac
  done

  local cleanup() { warp-agent-resume remove "$uuid" 2>/dev/null || true; }

  case "$mode" in
    passthrough)
      "$real" "$@"; return $?
      ;;
    resume)
      [[ -n "$resume_id" ]] && warp-agent-resume write "$uuid" "claude --resume $resume_id" "$PWD"
      trap cleanup EXIT INT TERM
      "$real" "$@"; local rc=$?
      cleanup; trap - EXIT INT TERM; return $rc
      ;;
    fresh)
      local sid
      sid="$(uuidgen | tr 'A-Z' 'a-z')"
      warp-agent-resume write "$uuid" "claude --resume $sid" "$PWD"
      trap cleanup EXIT INT TERM
      "$real" --session-id "$sid" "$@"; local rc=$?
      cleanup; trap - EXIT INT TERM; return $rc
      ;;
  esac
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `bash tools/agent-resume/tests/test_claude_wrapper.sh`
Expected: `PASS`

> Note: the test sources the file under bash; the function uses zsh `whence`/array syntax guarded by `CLAUDE_REAL_BIN` in tests. If the harness runs strictly under bash and a syntax error appears, run the test with `zsh tools/agent-resume/tests/test_claude_wrapper.sh` (zsh is present on macOS) and keep the wrapper zsh-native, since it ships into `~/.zshrc`.

- [ ] **Step 5: Commit**

```bash
git add tools/agent-resume/claude.zsh tools/agent-resume/tests/test_claude_wrapper.sh
git commit -m "feat(agent-resume): claude wrapper captures resumable session per pane"
```

---

## Task 3: Codex hooks

**Files:**
- Create: `tools/agent-resume/codex-session-start.sh`, `tools/agent-resume/codex-session-end.sh`, `tools/agent-resume/config.toml.snippet`
- Test: `tools/agent-resume/tests/test_codex_hooks.sh`

**Interfaces:**
- Consumes: `warp-agent-resume` on `PATH`; stdin JSON `{session_id, cwd, source, ...}`; env `$WARP_TERMINAL_SESSION_UUID`.
- Produces: SessionStart writes `"codex resume <session_id>"`; SessionEnd removes. No-op if env var absent. Requires `jq`.

- [ ] **Step 1: Write the failing test**

```bash
# tools/agent-resume/tests/test_codex_hooks.sh
#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d)"
export WARP_AGENT_RESUME_DIR="$TMP/reg"
export PATH="$HERE:$PATH"
export WARP_TERMINAL_SESSION_UUID="bb22"

echo '{"session_id":"sess-77","cwd":"/tmp/repo","source":"startup"}' | bash "$HERE/codex-session-start.sh"
f="$WARP_AGENT_RESUME_DIR/bb22.json"
grep -q '"command": "codex resume sess-77"' "$f" || { echo "FAIL: start"; exit 1; }
grep -q '"cwd": "/tmp/repo"' "$f" || { echo "FAIL: cwd"; exit 1; }

echo '{"session_id":"sess-77","cwd":"/tmp/repo"}' | bash "$HERE/codex-session-end.sh"
[[ ! -f "$f" ]] || { echo "FAIL: end did not remove"; exit 1; }

# No-op outside a Warp pane.
unset WARP_TERMINAL_SESSION_UUID
echo '{"session_id":"x","cwd":"/tmp"}' | bash "$HERE/codex-session-start.sh"
[[ -z "$(ls -A "$WARP_AGENT_RESUME_DIR" 2>/dev/null)" ]] || { echo "FAIL: wrote outside pane"; exit 1; }
echo "PASS"
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bash tools/agent-resume/tests/test_codex_hooks.sh`
Expected: FAIL (hook scripts missing).

- [ ] **Step 3: Write minimal implementation**

```bash
# tools/agent-resume/codex-session-start.sh
#!/usr/bin/env bash
set -euo pipefail
[[ -n "${WARP_TERMINAL_SESSION_UUID:-}" ]] || exit 0
payload="$(cat)"
sid="$(printf '%s' "$payload" | jq -r '.session_id // empty')"
cwd="$(printf '%s' "$payload" | jq -r '.cwd // empty')"
[[ -n "$sid" ]] || exit 0
warp-agent-resume write "$WARP_TERMINAL_SESSION_UUID" "codex resume $sid" "$cwd"
```

```bash
# tools/agent-resume/codex-session-end.sh
#!/usr/bin/env bash
set -euo pipefail
[[ -n "${WARP_TERMINAL_SESSION_UUID:-}" ]] || exit 0
warp-agent-resume remove "$WARP_TERMINAL_SESSION_UUID"
```

```toml
# tools/agent-resume/config.toml.snippet — paste into ~/.codex/config.toml
[[hooks.SessionStart]]
matcher = "startup|resume"
[[hooks.SessionStart.hooks]]
type = "command"
command = "~/.warp/agent-resume-bin/codex-session-start.sh"

[[hooks.SessionEnd]]
[[hooks.SessionEnd.hooks]]
type = "command"
command = "~/.warp/agent-resume-bin/codex-session-end.sh"
```

Then: `chmod +x tools/agent-resume/codex-session-start.sh tools/agent-resume/codex-session-end.sh`

- [ ] **Step 4: Run test to verify it passes**

Run: `bash tools/agent-resume/tests/test_codex_hooks.sh`
Expected: `PASS`. (Install `jq` first if missing: `brew install jq`.)

- [ ] **Step 5: Commit**

```bash
git add tools/agent-resume/codex-session-start.sh tools/agent-resume/codex-session-end.sh tools/agent-resume/config.toml.snippet tools/agent-resume/tests/test_codex_hooks.sh
git commit -m "feat(agent-resume): codex SessionStart/End hooks capture resumable session"
```

---

## Task 4: Rust registry reader module

**Files:**
- Create: `app/src/agent_resume.rs`
- Modify: `app/src/lib.rs` (or the crate root that declares `mod`s) — add `pub mod agent_resume;`
- Test: inline `#[cfg(test)]` in `app/src/agent_resume.rs`

**Interfaces:**
- Produces:
  - `pub fn read_on_restore_command(uuid: &[u8]) -> Option<String>` — resolves `$HOME/.warp/agent-resume/<hex>.json`, returns its `command`.
  - `fn read_command_in(dir: &std::path::Path, uuid_hex: &str) -> Option<String>` — pure, testable.

- [ ] **Step 1: Write the failing test**

```rust
// in app/src/agent_resume.rs
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn reads_command_from_registry_file() {
        let dir = std::env::temp_dir().join(format!("agent_resume_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut f = std::fs::File::create(dir.join("deadbeef.json")).unwrap();
        write!(f, r#"{{ "command": "claude --resume abc-123", "cwd": "/tmp" }}"#).unwrap();

        assert_eq!(
            read_command_in(&dir, "deadbeef"),
            Some("claude --resume abc-123".to_string())
        );
        assert_eq!(read_command_in(&dir, "missing"), None);
    }

    #[test]
    fn uuid_hex_is_lowercase() {
        // Must match $WARP_TERMINAL_SESSION_UUID casing.
        assert_eq!(hex::encode([0xAB, 0xCD]), "abcd");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p app agent_resume`
Expected: FAIL (module/functions not defined).

- [ ] **Step 3: Write minimal implementation**

```rust
// app/src/agent_resume.rs
//! Reads the per-pane agent-resume registry written by the claude wrapper / codex hooks.
//! See docs/superpowers/specs/2026-06-20-warp-agent-session-resume-design.md.

use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Deserialize)]
struct RegistryEntry {
    command: String,
}

fn registry_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(Path::new(&home).join(".warp").join("agent-resume"))
}

fn read_command_in(dir: &Path, uuid_hex: &str) -> Option<String> {
    let path = dir.join(format!("{uuid_hex}.json"));
    let contents = std::fs::read_to_string(path).ok()?;
    let entry: RegistryEntry = serde_json::from_str(&contents).ok()?;
    Some(entry.command)
}

/// Returns the resume command stored for `uuid`, if any. `uuid` is the raw pane UUID bytes;
/// it is hex-encoded (lowercase) to match `$WARP_TERMINAL_SESSION_UUID`.
pub fn read_on_restore_command(uuid: &[u8]) -> Option<String> {
    let dir = registry_dir()?;
    read_command_in(&dir, &hex::encode(uuid))
}
```

Confirm `serde`, `serde_json`, and `hex` are already deps of the `app` crate (they are used widely). If `hex` is not in `app/Cargo.toml`, add it: `hex = "0.4"`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo nextest run -p app agent_resume`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add app/src/agent_resume.rs app/src/lib.rs app/Cargo.toml
git commit -m "feat(agent-resume): rust registry reader"
```

---

## Task 5: Persist `on_restore_command` through the snapshot + DB

**Files:**
- Modify: `app/src/app_state.rs:202` (struct field)
- Create: `crates/persistence/migrations/2026-06-21-000000_add_on_restore_command_to_terminal_panes/{up,down}.sql`
- Modify: `crates/persistence/src/schema.rs:406` (table column)
- Modify: `crates/persistence/src/model.rs:589` (`NewTerminalPane` + `TerminalPane`)
- Modify: `app/src/persistence/sqlite.rs:~1175` (write), `:~2162` (read)
- Test: round-trip test in `crates/persistence/src/model.rs` `#[cfg(test)]` (or existing persistence test module)

**Interfaces:**
- Produces: `TerminalPaneSnapshot.on_restore_command: Option<String>` that survives save→read.
- Consumes: nothing new.

- [ ] **Step 1: Write the failing test**

Add a round-trip assertion to the persistence tests. Locate the existing test that saves & reads a pane tree (search: `rg "fn .*restore" crates/persistence app/src/persistence/sqlite.rs`). If a pane round-trip test exists, extend it; otherwise add:

```rust
// crates/persistence/src/model.rs (in #[cfg(test)] mod)
#[test]
fn terminal_pane_has_on_restore_command_field() {
    // Compile-time guarantee the column/field exist and accept Some/None.
    let p = NewTerminalPane {
        id: 0,
        uuid: vec![1, 2, 3],
        cwd: None,
        is_active: false,
        shell_launch_data: None,
        input_config: None,
        llm_model_override: None,
        active_profile_id: None,
        conversation_ids: None,
        active_conversation_id: None,
        on_restore_command: Some("claude --resume x".into()),
    };
    assert_eq!(p.on_restore_command.as_deref(), Some("claude --resume x"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p persistence terminal_pane_has_on_restore_command_field`
Expected: FAIL (field `on_restore_command` not present → compile error).

- [ ] **Step 3: Write minimal implementation**

3a. Migration:

```sql
-- crates/persistence/migrations/2026-06-21-000000_add_on_restore_command_to_terminal_panes/up.sql
ALTER TABLE terminal_panes ADD COLUMN on_restore_command TEXT;
```
```sql
-- .../down.sql
ALTER TABLE terminal_panes DROP COLUMN on_restore_command;
```

3b. `crates/persistence/src/schema.rs` — add inside `terminal_panes (id) { ... }`, after `active_conversation_id`:
```rust
        active_conversation_id -> Nullable<Text>,
        on_restore_command -> Nullable<Text>,
```

3c. `crates/persistence/src/model.rs` — add field to **both** `NewTerminalPane` (Insertable) and `TerminalPane` (Queryable/Selectable), after `active_conversation_id`:
```rust
    /// Verbatim command to run after this pane's shell boots on restore (agent resume).
    pub on_restore_command: Option<String>,
```

3d. `app/src/app_state.rs` `TerminalPaneSnapshot` — add after `active_conversation_id`:
```rust
    /// Command to auto-run after the restored shell boots (e.g. `claude --resume <id>`).
    pub on_restore_command: Option<String>,
```

3e. `app/src/persistence/sqlite.rs` write site (~1175, where `NewTerminalPane { ... }` is built from the snapshot) — add:
```rust
        on_restore_command: terminal_snapshot.on_restore_command.clone(),
```

3f. `app/src/persistence/sqlite.rs` read site (`read_node`, ~2162, where `TerminalPaneSnapshot { ... }` is built) — add:
```rust
        on_restore_command: terminal_pane.on_restore_command,
```

3g. Fix every other `TerminalPaneSnapshot { ... }` constructor the compiler flags (e.g. the viewer/shared branch in `terminal_pane.rs`) by adding `on_restore_command: None,`. Find them: `rg "TerminalPaneSnapshot \{" app/`

- [ ] **Step 4: Run test + build to verify pass**

Run: `cargo nextest run -p persistence terminal_pane_has_on_restore_command_field && cargo build -p app`
Expected: PASS + clean build (all `TerminalPaneSnapshot` constructors updated).

- [ ] **Step 5: Commit**

```bash
git add crates/persistence app/src/app_state.rs app/src/persistence/sqlite.rs app/src/pane_group/pane/terminal_pane.rs
git commit -m "feat(agent-resume): persist on_restore_command on terminal panes"
```

---

## Task 6: Populate `on_restore_command` in `snapshot()`

**Files:**
- Modify: `app/src/pane_group/pane/terminal_pane.rs:~581` (the local-session `snapshot()` branch)
- Test: extend `crates/integration/src/test/session_restoration.rs`

**Interfaces:**
- Consumes: `agent_resume::read_on_restore_command(&self.uuid)` (Task 4); `TerminalPaneSnapshot.on_restore_command` (Task 5).

- [ ] **Step 1: Write the failing test**

Add to `crates/integration/src/test/session_restoration.rs` a test that: creates a pane, writes a registry file at `$HOME/.warp/agent-resume/<hex(uuid)>.json` (set `HOME` to a temp dir for the test), snapshots, and asserts `on_restore_command == Some("claude --resume test-id")`. Follow the file's existing Builder/TestStep patterns (search the file for an existing snapshot assertion to copy the harness setup). Pseudostructure:

```rust
#[test]
fn snapshot_captures_agent_resume_command() {
    // ... set up temp HOME, build app with one terminal pane (capture its uuid) ...
    // write registry file:
    //   <HOME>/.warp/agent-resume/<hex(uuid)>.json  ->  { "command": "claude --resume test-id", "cwd": "/tmp" }
    // take a snapshot of the pane group
    // assert the restored TerminalPaneSnapshot.on_restore_command == Some("claude --resume test-id")
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p integration snapshot_captures_agent_resume_command`
Expected: FAIL (snapshot returns `None`).

- [ ] **Step 3: Write minimal implementation**

In `terminal_pane.rs` `snapshot()`, in the local-session `LeafContents::Terminal(TerminalPaneSnapshot { ... })` constructor (the branch with `cwd: view.pwd_if_local(app)`), add:
```rust
            on_restore_command: crate::agent_resume::read_on_restore_command(&self.uuid),
```
Leave the viewer/shared branch as `on_restore_command: None` (a viewer pane has no local agent).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo nextest run -p integration snapshot_captures_agent_resume_command`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add app/src/pane_group/pane/terminal_pane.rs crates/integration/src/test/session_restoration.rs
git commit -m "feat(agent-resume): capture resume command into pane snapshot"
```

---

## Task 7: Replay the command after `Bootstrapped` on restore

**Files:**
- Modify: `app/src/pane_group/mod.rs:~1655` (restore path, right after `TerminalPane::new`)
- Test: extend `crates/integration/src/test/session_restoration.rs`

**Interfaces:**
- Consumes: `terminal_snapshot.on_restore_command`; the `Bootstrapped` event (`AnsiHandlerEvent::Bootstrapped`, `app/src/terminal/model_events.rs:449`); `PtyController::write_command(&str, ShellType, CommandExecutionSource::User, ctx)` (`app/src/terminal/writeable_pty/pty_controller.rs:511`).

> **Implementer note (resolve before coding):** The one detail not fully pinned is how to reach the pane's `PtyController` model handle from the restore path to call `write_command`, and which model dispatches `Bootstrapped`. Find the canonical pattern by locating an existing programmatic command-send: `rg "\.write_command\(" app/src` (the Warp AI agent already runs commands in a pane this way). Copy that access path (TerminalManager → writeable pty/PtyController model handle) and its `ShellType` source. Subscribe with the documented pattern `ctx.subscribe_to_model(&model_event_dispatcher, |me, event, ctx| match event { ModelEvent::Handler(AnsiHandlerEvent::Bootstrapped { .. }) => { /* fire once */ } _ => {} })` (see `pty_controller.rs:111-122`).

- [ ] **Step 1: Write the failing test**

Add to `session_restoration.rs`: restore a pane whose snapshot has `on_restore_command = Some("echo RESUMED")`, drive the shell to `Bootstrapped`, and assert the PTY received `echo RESUMED` (assert via the terminal manager's written-input/command history, following the harness's existing way of asserting executed commands — search the file for an existing "command was executed"/input assertion to copy).

```rust
#[test]
fn restored_pane_runs_on_restore_command_after_bootstrap() {
    // ... build a snapshot with on_restore_command = Some("echo RESUMED") ...
    // ... restore it; advance until Bootstrapped fires ...
    // assert the command "echo RESUMED" was written/executed exactly once in the pane
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p integration restored_pane_runs_on_restore_command_after_bootstrap`
Expected: FAIL (command never sent).

- [ ] **Step 3: Write minimal implementation**

In `mod.rs`, after the restored `TerminalPane::new(...)` (~1655), when `terminal_snapshot.on_restore_command` is `Some(cmd)`, register a one-shot subscription that, on the first `Bootstrapped`, calls `write_command(&cmd, shell_type, CommandExecutionSource::User, ctx)` on the pane's PtyController, then drops itself. Concrete shape (adapt the handle access per the implementer note):

```rust
if let Some(cmd) = terminal_snapshot.on_restore_command.clone() {
    // `pty_controller` + `dispatcher` obtained via the same path used by existing
    // `.write_command(` call sites (see implementer note).
    let mut fired = false;
    ctx.subscribe_to_model(&dispatcher, move |_view, event, ctx| {
        if fired { return; }
        if let ModelEvent::Handler(AnsiHandlerEvent::Bootstrapped { is_subshell, .. }) = event {
            if *is_subshell { return; } // only the top-level shell
            fired = true;
            pty_controller.update(ctx, |pty, ctx| {
                pty.write_command(&cmd, shell_type, CommandExecutionSource::User, ctx);
            });
        }
    });
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo nextest run -p integration restored_pane_runs_on_restore_command_after_bootstrap`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add app/src/pane_group/mod.rs crates/integration/src/test/session_restoration.rs
git commit -m "feat(agent-resume): auto-run resume command after restored shell boots"
```

---

## Task 8: Installer + end-to-end manual verification

**Files:**
- Create: `tools/agent-resume/install.sh`, `tools/agent-resume/README.md`

**Interfaces:**
- Consumes: all prior tasks. Installs scripts to `~/.warp/agent-resume-bin/`, sources the wrapper from `~/.zshrc`, prints the codex `config.toml` snippet.

- [ ] **Step 1: Write the installer**

```bash
# tools/agent-resume/install.sh
#!/usr/bin/env bash
set -euo pipefail
SRC="$(cd "$(dirname "$0")" && pwd)"
BIN="$HOME/.warp/agent-resume-bin"
mkdir -p "$BIN" "$HOME/.warp/agent-resume"; chmod 700 "$HOME/.warp/agent-resume"
install -m 0755 "$SRC/warp-agent-resume" "$SRC/codex-session-start.sh" "$SRC/codex-session-end.sh" "$BIN/"
install -m 0644 "$SRC/claude.zsh" "$BIN/claude.zsh"

marker="# >>> warp agent-resume >>>"
if ! grep -qF "$marker" "$HOME/.zshrc" 2>/dev/null; then
  {
    echo "$marker"
    echo "export PATH=\"\$HOME/.warp/agent-resume-bin:\$PATH\""
    echo "source \"\$HOME/.warp/agent-resume-bin/claude.zsh\""
    echo "# <<< warp agent-resume <<<"
  } >> "$HOME/.zshrc"
fi
echo "Installed. Now paste tools/agent-resume/config.toml.snippet into ~/.codex/config.toml,"
echo "fixing the hook paths to: $BIN/codex-session-start.sh and $BIN/codex-session-end.sh"
echo "Then restart your shell. Requires: jq, uuidgen (preinstalled on macOS)."
```

- [ ] **Step 2: Verify scripts wired**

Run: `bash tools/agent-resume/install.sh && zsh -ic 'type claude'`
Expected: `claude is a shell function`.

- [ ] **Step 3: Build Warp**

Run: `cargo build -p app` (or the project's release target). Launch the built Warp.

- [ ] **Step 4: Manual E2E — the target scenario**

In the built Warp, in ONE repo directory open 3 tabs: tab1 `claude`, tab2 `claude`, tab3 `codex`. Give each a distinct first message so the sessions differ. Then `cmd-Q`; relaunch Warp.
Expected: each of the 3 tabs auto-runs its own `claude --resume <id>` / `codex resume <id>` after the shell boots, resuming the correct distinct session.

- [ ] **Step 5: Verify the snapshot-cadence (crash) caveat**

Confirm the spec's open question: determine whether Warp persists pane snapshots periodically or only on shutdown (search: `rg -n "save_pane_state|snapshot" app/src/persistence app/src/app_state.rs` and trace callers). Document the finding in `tools/agent-resume/README.md`: `cmd-Q`/update-restart are fully covered; the crash case is covered only if persistence is periodic. No code change if shutdown-only — it simply degrades to today's behavior for crashes.

- [ ] **Step 6: Commit**

```bash
git add tools/agent-resume/install.sh tools/agent-resume/README.md
git commit -m "feat(agent-resume): installer + e2e docs"
```

---

## Self-Review

**Spec coverage:**
- Registry contract → Task 1. Claude capture → Task 2. Codex capture → Task 3. Rust reader → Task 4. Persist field/DB → Task 5. Snapshot population → Task 6. Replay on Bootstrapped → Task 7. Install + E2E + cadence verification → Task 8. Edge cases (continue/picker passthrough, outside-pane no-op, idempotent remove, recursion guard) → Tasks 1–3. ✅ All spec sections map to a task.

**Placeholder scan:** No "TBD/TODO". Two spots delegate to a `rg`-driven "copy the existing pattern" step (Task 6 harness setup, Task 7 PtyController handle): these are explicit, actionable instructions with the exact search command and the real APIs to call — required because the gpui model-plumbing handle path must be taken from live code rather than fabricated. Flagged as the only adaptation points.

**Type consistency:** `on_restore_command: Option<String>` is used identically in `TerminalPaneSnapshot`, `NewTerminalPane`, `TerminalPane`, schema column, both SQLite call sites, and the snapshot constructor. Registry JSON shape `{ "command", "cwd" }` matches between the shell `write` (Task 1), the Rust `RegistryEntry` (Task 4, reads only `command`), and the codex hook (Task 3). Resume command strings (`claude --resume <id>`, `codex resume <id>`) consistent across Tasks 2/3 and the design. ✅
