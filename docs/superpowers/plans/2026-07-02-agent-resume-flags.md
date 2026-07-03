# Agent-Resume Flag Carry-Over Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restored/resumed Claude Code and Codex sessions come back with the permission mode and model they were actually running with, including modes toggled mid-session.

**Architecture:** All capture logic lives in `tools/agent-resume/` shell scripts (the Rust side replays an opaque command). Claude gets one capture script wired to three hook events (SessionStart = launch-argv capture; UserPromptSubmit/Stop = payload `permission_mode` is authoritative). Codex reads `permission_mode`/`model` straight from its SessionStart payload. The only Rust change fixes `derive_fork_command`, which still parses a dead pre-launcher command format. `install.sh` migrates the installed hooks and `make install-local` runs it so installs can never go stale again.

**Tech Stack:** bash + jq (hooks, tests), Rust (`app/src/agent_resume.rs`), Makefile.

**Spec:** `docs/superpowers/specs/2026-07-02-agent-resume-flags-design.md`

## Global Constraints

- Working directory: the worktree `/Users/ellioteckholm/projects/clinch-terminal/.claude/worktrees/agent-resume-flags` (branch `agent-resume-flags`, based on `clinch/main`). All paths below are relative to it.
- Mutating git commands may run with a stripped PATH — invoke as `/usr/bin/git`.
- Flag whitelist (exact): Claude `--dangerously-skip-permissions`, `--permission-mode <m>`, `--model <m>`; Codex `--dangerously-bypass-approvals-and-sandbox`, `--model <m>`. Nothing else is ever carried.
- Registry command format (exact): `warp_agent_resume_launch <agent> <sid>[ <flags…>]`.
- Shell tests are standalone scripts in `tools/agent-resume/tests/`; each prints `PASS` and exits 0.
- Rust: unit tests live in `agent_resume_tests.rs` included via `#[path]`; no `_`-prefixed params; inline format args (`format!("{id}")`); before the PR run `./script/format` and the presubmit clippy line.
- Do not modify files outside `tools/agent-resume/`, `app/src/agent_resume*.rs`, `Makefile`, `docs/superpowers/`.
- Every commit message ends with:
  `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>` and
  `Claude-Session: https://claude.ai/code/session_017t8QQQB9wxyExYT5Q7Dar4`

---

### Task 1: Rename `claude-session-start.sh` → `claude-capture.sh` (mechanical, zero behavior change)

**Files:**
- Rename: `tools/agent-resume/claude-session-start.sh` → `tools/agent-resume/claude-capture.sh`
- Modify: `tools/agent-resume/install.sh:17` (install list), `tools/agent-resume/claude.zsh:3` (comment), `tools/agent-resume/README.md:124` (table row)
- Modify (test refs): `tools/agent-resume/tests/test_claude_hook.sh:10,20,25,31,37,42`, `tools/agent-resume/tests/test_claude_flags.sh:10`, `tools/agent-resume/tests/test_claude_argv_walk.sh:7`

**Interfaces:**
- Produces: `tools/agent-resume/claude-capture.sh` — same functions (`_warp_agent_resume_extract_flags`, `_warp_agent_resume_claude_argv`, `_warp_agent_resume_capture_main`), same behavior. Every later task references this filename.
- Note: `install.sh`'s settings.json wiring (`HOOK_CMD` at line 64) is intentionally NOT touched here — Task 5 rewrites it wholesale.

- [ ] **Step 1: Rename with git and update every textual reference**

```bash
cd /Users/ellioteckholm/projects/clinch-terminal/.claude/worktrees/agent-resume-flags
/usr/bin/git mv tools/agent-resume/claude-session-start.sh tools/agent-resume/claude-capture.sh
```

Then apply these exact replacements (Edit tool, per file):

- `tools/agent-resume/install.sh` line 17: `"$SRC/claude-session-start.sh"` → `"$SRC/claude-capture.sh"`
- `tools/agent-resume/claude.zsh` line 3: `# Capture is done by Claude's SessionStart hook (claude-session-start.sh) and Codex's` → `# Capture is done by Claude's hooks (claude-capture.sh) and Codex's`
- `tools/agent-resume/README.md` line 124: `| \`claude-session-start.sh\` | Claude \`SessionStart\` hook — captures the live session per pane, plus its permission-mode / \`--model\` launch flags |` → `| \`claude-capture.sh\` | Claude \`SessionStart\` hook — captures the live session per pane, plus its permission-mode / \`--model\` launch flags |`
- In the three test files, replace every occurrence of `claude-session-start.sh` with `claude-capture.sh` (`test_claude_hook.sh` has it 6×: the `install` line and five `"$BIN/..."`/`"$0"` invocations; the other two have it once each in their `source` line).

- [ ] **Step 2: Verify nothing references the old name and tests still pass**

```bash
grep -rn "claude-session-start" tools/ Makefile app/ && echo "LEFTOVER REFS" || echo "clean"
cd tools/agent-resume/tests && for t in test_*.sh; do echo "== $t"; bash "$t" >/dev/null 2>&1 && echo PASS || { echo "FAIL($?)"; exit 1; }; done
```
Expected: `clean`, then 7× PASS. (The spec doc mentions the old name historically — `docs/` matches are fine; the grep above deliberately excludes `docs/`.)

- [ ] **Step 3: Commit**

```bash
/usr/bin/git add -A tools/agent-resume
/usr/bin/git commit -m "refactor(agent-resume): rename claude-session-start.sh to claude-capture.sh

Pure rename; the script is about to serve UserPromptSubmit and Stop in
addition to SessionStart."
```
(Append the Global Constraints trailer lines to this and every commit.)

---

### Task 2: Claude live-mode updater (UserPromptSubmit / Stop)

**Files:**
- Modify: `tools/agent-resume/claude-capture.sh`
- Test: `tools/agent-resume/tests/test_claude_flags.sh` (pure helpers), `tools/agent-resume/tests/test_claude_hook.sh` (end-to-end hook behavior)

**Interfaces:**
- Consumes: Task 1's renamed script.
- Produces: two new pure functions in `claude-capture.sh` —
  `_warp_agent_resume_mode_flags_from_payload <mode>` (prints ` --dangerously-skip-permissions` for `bypassPermissions`, ` --permission-mode <m>` for `plan`/`acceptEdits`, empty for `default`; **returns 1** for empty/unknown) and
  `_warp_agent_resume_extract_model <argv>` (prints only ` --model <m>` tokens).
  `_warp_agent_resume_capture_main` now dispatches on the payload's `hook_event_name` (missing → `SessionStart`, which keeps old fixtures/real SessionStart payloads working).

- [ ] **Step 1: Write the failing pure-function tests**

Append to `tools/agent-resume/tests/test_claude_flags.sh` (before the final `echo "PASS"`):

```bash
# 7. _warp_agent_resume_extract_model carries only --model, never the mode flags.
out="$(_warp_agent_resume_extract_model "node /x/cli.js --dangerously-skip-permissions --model opus")"
[[ "$out" == " --model opus" ]] || fail "extract_model carries only the model (got: '$out')"
out="$(_warp_agent_resume_extract_model "claude --model=sonnet")"
[[ "$out" == " --model sonnet" ]] || fail "extract_model equals form (got: '$out')"
out="$(_warp_agent_resume_extract_model "claude --permission-mode plan")"
[[ -z "$out" ]] || fail "extract_model must ignore mode flags (got: '$out')"

# 8. Payload permission_mode -> flags mapping. `default` maps to no flags (still rc 0);
# unknown/empty values return nonzero so the caller falls back to argv detection.
out="$(_warp_agent_resume_mode_flags_from_payload "bypassPermissions")" || fail "bypassPermissions must map"
[[ "$out" == " --dangerously-skip-permissions" ]] || fail "bypassPermissions mapping (got: '$out')"
out="$(_warp_agent_resume_mode_flags_from_payload "plan")" || fail "plan must map"
[[ "$out" == " --permission-mode plan" ]] || fail "plan mapping (got: '$out')"
out="$(_warp_agent_resume_mode_flags_from_payload "acceptEdits")" || fail "acceptEdits must map"
[[ "$out" == " --permission-mode acceptEdits" ]] || fail "acceptEdits mapping (got: '$out')"
out="$(_warp_agent_resume_mode_flags_from_payload "default")" || fail "default must map (to nothing)"
[[ -z "$out" ]] || fail "default maps to no flags (got: '$out')"
_warp_agent_resume_mode_flags_from_payload "somethingNew" >/dev/null && fail "unknown mode must return nonzero"
_warp_agent_resume_mode_flags_from_payload "" >/dev/null && fail "empty mode must return nonzero"
```

- [ ] **Step 2: Write the failing hook-behavior tests**

In `tools/agent-resume/tests/test_claude_hook.sh`, insert before the `# Outside a Warp pane: no-op.` block (note: `WARP_AGENT_RESUME_FAKE_ARGV=""` is still exported here, so argv contributes nothing unless a case overrides it):

```bash
# --- Live-mode updater (UserPromptSubmit / Stop) ---
# The payload's permission_mode is authoritative for the mode; --model still comes from
# the live argv. `default` strips the mode flag; unknown values fall back to argv.

# Toggled to bypass mid-session (entry owned by this sid): entry rewritten with the flag.
echo '{"session_id":"sess-ddd","cwd":"/tmp/repo"}' | "$BIN/claude-capture.sh"
echo '{"session_id":"sess-ddd","cwd":"/tmp/repo","hook_event_name":"UserPromptSubmit","permission_mode":"bypassPermissions"}' | "$BIN/claude-capture.sh"
grep -q '"command": "warp_agent_resume_launch claude sess-ddd --dangerously-skip-permissions"' "$f" || { echo "FAIL: updater did not add bypass flag"; exit 1; }

# Toggled back to default (via Stop): the mode flag must be stripped again.
echo '{"session_id":"sess-ddd","cwd":"/tmp/repo","hook_event_name":"Stop","permission_mode":"default"}' | "$BIN/claude-capture.sh"
grep -q '"command": "warp_agent_resume_launch claude sess-ddd"' "$f" || { echo "FAIL: default did not strip mode flag"; exit 1; }

# plan maps to --permission-mode plan.
echo '{"session_id":"sess-ddd","cwd":"/tmp/repo","hook_event_name":"UserPromptSubmit","permission_mode":"plan"}' | "$BIN/claude-capture.sh"
grep -q '"command": "warp_agent_resume_launch claude sess-ddd --permission-mode plan"' "$f" || { echo "FAIL: plan mode not carried"; exit 1; }

# Model from the live argv is kept alongside the payload mode.
WARP_AGENT_RESUME_FAKE_ARGV="node /x/cli.js --model opus" \
  bash -c 'echo "{\"session_id\":\"sess-ddd\",\"cwd\":\"/tmp/repo\",\"hook_event_name\":\"UserPromptSubmit\",\"permission_mode\":\"bypassPermissions\"}" | "$0"' "$BIN/claude-capture.sh"
grep -q '"command": "warp_agent_resume_launch claude sess-ddd --dangerously-skip-permissions --model opus"' "$f" || { echo "FAIL: model not kept with payload mode"; exit 1; }

# Unknown permission_mode falls back to argv-derived flags (mode + model).
WARP_AGENT_RESUME_FAKE_ARGV="node /x/cli.js --permission-mode acceptEdits" \
  bash -c 'echo "{\"session_id\":\"sess-ddd\",\"cwd\":\"/tmp/repo\",\"hook_event_name\":\"UserPromptSubmit\",\"permission_mode\":\"weird\"}" | "$0"' "$BIN/claude-capture.sh"
grep -q '"command": "warp_agent_resume_launch claude sess-ddd --permission-mode acceptEdits"' "$f" || { echo "FAIL: unknown mode did not fall back to argv"; exit 1; }

# Session-id guard: an updater event from a DIFFERENT session must not clobber the entry…
echo '{"session_id":"sess-intruder","cwd":"/tmp/other","hook_event_name":"UserPromptSubmit","permission_mode":"bypassPermissions"}' | "$BIN/claude-capture.sh"
grep -q 'sess-ddd' "$f" || { echo "FAIL: foreign session clobbered the pane entry"; exit 1; }

# …but a missing entry is (re)created — this heals pre-flag registry entries.
rm -f "$f"
echo '{"session_id":"sess-eee","cwd":"/tmp/repo","hook_event_name":"UserPromptSubmit","permission_mode":"bypassPermissions"}' | "$BIN/claude-capture.sh"
grep -q '"command": "warp_agent_resume_launch claude sess-eee --dangerously-skip-permissions"' "$f" || { echo "FAIL: missing entry not healed"; exit 1; }

# Unknown events are ignored.
echo '{"session_id":"sess-eee","cwd":"/tmp/repo","hook_event_name":"PreCompact","permission_mode":"default"}' | "$BIN/claude-capture.sh"
grep -q -- '--dangerously-skip-permissions' "$f" || { echo "FAIL: unknown event must not rewrite the entry"; exit 1; }
```

- [ ] **Step 3: Run both tests to verify they fail**

```bash
cd tools/agent-resume/tests && bash test_claude_flags.sh; bash test_claude_hook.sh
```
Expected: `test_claude_flags.sh` fails at case 7 (`_warp_agent_resume_extract_model: command not found` surfaces as an empty-capture FAIL); `test_claude_hook.sh` fails at "updater did not add bypass flag" (old script treats the UserPromptSubmit payload as a SessionStart and writes a flag-less entry).

- [ ] **Step 4: Implement in `claude-capture.sh`**

Insert the two pure functions after `_warp_agent_resume_extract_flags` (keep that function unchanged):

```bash
# Like _warp_agent_resume_extract_flags but carries only --model. Used when a hook
# payload already provides the authoritative permission mode.
_warp_agent_resume_extract_model() {
  local argv="${1:-}"
  local -a toks=()
  read -ra toks <<<"$argv"
  local out="" tok next
  local i=0 n=${#toks[@]}
  while ((i < n)); do
    tok="${toks[i]}"
    next=""
    ((i + 1 < n)) && next="${toks[i + 1]}"
    case "$tok" in
      --model)   [[ -n "$next" ]] && { out+=" --model $next"; i=$((i + 1)); } ;;
      --model=*) out+=" --model ${tok#*=}" ;;
    esac
    i=$((i + 1))
  done
  printf '%s' "$out"
}

# Maps a hook payload permission_mode to resume-command flags. Prints the flag tokens
# (leading space; empty for `default`, which must strip a previously-carried mode).
# Returns 1 for empty/unknown values so the caller can fall back to argv detection.
_warp_agent_resume_mode_flags_from_payload() {
  case "${1:-}" in
    bypassPermissions) printf ' --dangerously-skip-permissions' ;;
    plan|acceptEdits)  printf ' --permission-mode %s' "$1" ;;
    default)           ;;
    *) return 1 ;;
  esac
}
```

Replace `_warp_agent_resume_capture_main` with:

```bash
_warp_agent_resume_capture_main() {
  set -uo pipefail
  [[ -n "${WARP_TERMINAL_SESSION_UUID:-}" ]] || return 0   # only act inside a Warp pane
  local payload sid cwd event pmode extra mode_part entry_file BIN
  payload="$(cat)"
  sid="$(printf '%s' "$payload" | jq -r '.session_id // empty')"
  cwd="$(printf '%s' "$payload" | jq -r '.cwd // empty')"
  event="$(printf '%s' "$payload" | jq -r '.hook_event_name // "SessionStart"')"
  [[ -n "$sid" ]] || return 0
  case "$event" in
    SessionStart)
      # Fresh capture: a new session in this pane takes over the entry unconditionally.
      extra="$(_warp_agent_resume_extract_flags "$(_warp_agent_resume_claude_argv)")"
      ;;
    UserPromptSubmit|Stop)
      # Live-mode update. Guard: only touch an entry this session owns — a missing entry
      # is healed (pre-flag registries), but an entry recording a different session id
      # (e.g. a nested claude run from a tool in the same pane env) is left alone.
      entry_file="${WARP_AGENT_RESUME_DIR:-$HOME/.warp/agent-resume}/$WARP_TERMINAL_SESSION_UUID.json"
      if [[ -f "$entry_file" ]] && ! grep -qE "warp_agent_resume_launch claude $sid( |\")" "$entry_file"; then
        return 0
      fi
      pmode="$(printf '%s' "$payload" | jq -r '.permission_mode // empty')"
      if mode_part="$(_warp_agent_resume_mode_flags_from_payload "$pmode")"; then
        extra="${mode_part}$(_warp_agent_resume_extract_model "$(_warp_agent_resume_claude_argv)")"
      else
        extra="$(_warp_agent_resume_extract_flags "$(_warp_agent_resume_claude_argv)")"
      fi
      ;;
    *) return 0 ;;
  esac
  # Call the registry CLI by absolute path (sibling of this script) so the hook does not
  # depend on the agent inheriting the shell PATH.
  BIN="$(cd "$(dirname "$0")" && pwd)"
  "$BIN/warp-agent-resume" write "$WARP_TERMINAL_SESSION_UUID" \
    "warp_agent_resume_launch claude $sid$extra" "$cwd" >/dev/null 2>&1 || true
  return 0
}
```

Also update the file's header comment: replace the sentence claiming the permission mode is unavailable ("SessionStart's stdin payload does NOT include the permission mode, so we read it off the live `claude` process argv") with:

```
# SessionStart's stdin payload does not include the permission mode, so launch capture
# reads it off the live `claude` process argv (the alias expands before exec). Per-turn
# payloads (UserPromptSubmit, Stop) DO include permission_mode, so this script is also
# wired to those events and keeps the entry in sync with the session's live mode --
# including modes toggled mid-session (shift+tab) and entries that predate flag capture.
```

- [ ] **Step 5: Run all shell tests to verify they pass**

```bash
cd tools/agent-resume/tests && for t in test_*.sh; do echo "== $t"; bash "$t" >/dev/null 2>&1 && echo PASS || { echo "FAIL($?)"; bash "$t"; exit 1; }; done
```
Expected: 7× PASS.

- [ ] **Step 6: Commit**

```bash
/usr/bin/git add tools/agent-resume/claude-capture.sh tools/agent-resume/tests/test_claude_flags.sh tools/agent-resume/tests/test_claude_hook.sh
/usr/bin/git commit -m "feat(agent-resume): track Claude's live permission mode via UserPromptSubmit/Stop

The payload's permission_mode is authoritative: bypass toggled mid-session
sticks, toggling back to default strips the flag, and flag-less entries from
pre-flag installs heal on the session's next prompt. A session-id guard keeps
nested claudes from clobbering the pane entry."
```

---

### Task 3: Codex capture — payload flags + absolute-path fix

**Files:**
- Modify: `tools/agent-resume/codex-session-start.sh`, `tools/agent-resume/codex-session-end.sh`
- Test: `tools/agent-resume/tests/test_codex_hooks.sh`

**Interfaces:**
- Consumes: `warp-agent-resume write|remove` (sibling CLI).
- Produces: codex registry commands of the form `warp_agent_resume_launch codex <sid>[ --dangerously-bypass-approvals-and-sandbox][ --model <m>]`.

- [ ] **Step 1: Empirically verify the real codex payload (best-effort, fail-safe either way)**

```bash
cp ~/.warp/agent-resume-bin/codex-session-start.sh "$HOME/.warp/codex-hook-backup.sh"
printf '#!/usr/bin/env bash\ncat >> "$HOME/.warp/codex-payload-dump.jsonl"\n' > ~/.warp/agent-resume-bin/codex-session-start.sh
chmod 755 ~/.warp/agent-resume-bin/codex-session-start.sh
WARP_TERMINAL_SESSION_UUID=payloadprobe codex exec "reply with just: ok" || true
cat "$HOME/.warp/codex-payload-dump.jsonl" 2>/dev/null || echo "NO PAYLOAD CAPTURED"
codex resume --help 2>&1 | grep -E "dangerously|model" || echo "resume help: flags not listed"
# restore
mv "$HOME/.warp/codex-hook-backup.sh" ~/.warp/agent-resume-bin/codex-session-start.sh
chmod 755 ~/.warp/agent-resume-bin/codex-session-start.sh
rm -f "$HOME/.warp/codex-payload-dump.jsonl"
```
Decision rule: if the dump shows `permission_mode`/`model` under different names, use the real names in Step 2/4 and note it in the commit message. If `NO PAYLOAD CAPTURED` (e.g. `codex exec` doesn't fire the hook), proceed with the documented names — absent fields degrade to a plain resume command, same as today. If `codex resume --help` does not accept `--model`, drop the model mapping and its test case.

Also check whether the installed codex exposes a per-turn hook event that carries `permission_mode` (inspect `hook_event_name` values in the dump and the hooks section of `codex --help` / the local docs). If one exists, wire it in `install.sh`'s codex config block with a `[[hooks.<Event>]]` entry pointing at the same `codex-session-start.sh` (the script doesn't branch on event, so per-turn rewrites are safe — same pane, same sid). If none exists, mid-session codex mode toggles stay a documented limitation (Task 6 records it in the README).

- [ ] **Step 2: Write the failing tests**

In `tools/agent-resume/tests/test_codex_hooks.sh`: delete line 6 (`export PATH="$HERE:$PATH"`) — the scripts must now find `warp-agent-resume` as a sibling, not via PATH. Insert after the existing cwd assertion (line 12):

```bash
# Bypass + model from the payload are carried into the resume command.
echo '{"session_id":"sess-88","cwd":"/tmp/repo","permission_mode":"bypassPermissions","model":"gpt-5.3-codex"}' | bash "$HERE/codex-session-start.sh"
grep -q '"command": "warp_agent_resume_launch codex sess-88 --dangerously-bypass-approvals-and-sandbox --model gpt-5.3-codex"' "$f" || { echo "FAIL: codex bypass+model"; exit 1; }

# Non-bypass modes carry only the model (conservative mapping).
echo '{"session_id":"sess-99","cwd":"/tmp/repo","permission_mode":"acceptEdits","model":"gpt-5.3-codex"}' | bash "$HERE/codex-session-start.sh"
grep -q '"command": "warp_agent_resume_launch codex sess-99 --model gpt-5.3-codex"' "$f" || { echo "FAIL: codex non-bypass carries model only"; exit 1; }
```

- [ ] **Step 3: Run to verify failure**

```bash
cd tools/agent-resume/tests && bash test_codex_hooks.sh
```
Expected: the test aborts at the first `bash "$HERE/codex-session-start.sh"` invocation — with the PATH export removed, the old script's bare-name `warp-agent-resume` call is command-not-found, which under its `set -euo pipefail` makes the script (and the piping test, which also runs `set -euo pipefail`) exit nonzero before any assertion prints. This failure also proves the pre-existing PATH bug.

- [ ] **Step 4: Rewrite `codex-session-start.sh`**

```bash
#!/usr/bin/env bash
# Codex SessionStart hook: record the live session for this Warp pane so it can be
# resumed on restore. The payload also carries permission_mode and model, so the
# recorded resume command reopens the session the way it is currently running.
# Unknown or absent fields degrade to a plain resume (fail-safe).
set -euo pipefail
[[ -n "${WARP_TERMINAL_SESSION_UUID:-}" ]] || exit 0
payload="$(cat)"
sid="$(printf '%s' "$payload" | jq -r '.session_id // empty')"
cwd="$(printf '%s' "$payload" | jq -r '.cwd // empty')"
pmode="$(printf '%s' "$payload" | jq -r '.permission_mode // empty')"
model="$(printf '%s' "$payload" | jq -r '.model // empty')"
[[ -n "$sid" ]] || exit 0
extra=""
[[ "$pmode" == "bypassPermissions" ]] && extra+=" --dangerously-bypass-approvals-and-sandbox"
[[ -n "$model" ]] && extra+=" --model $model"
# Absolute sibling path: hooks do not reliably inherit the shell PATH.
BIN="$(cd "$(dirname "$0")" && pwd)"
"$BIN/warp-agent-resume" write "$WARP_TERMINAL_SESSION_UUID" "warp_agent_resume_launch codex $sid$extra" "$cwd"
```

And in `codex-session-end.sh`, replace the bare call with the sibling form:

```bash
#!/usr/bin/env bash
set -euo pipefail
[[ -n "${WARP_TERMINAL_SESSION_UUID:-}" ]] || exit 0
# Absolute sibling path: hooks do not reliably inherit the shell PATH.
BIN="$(cd "$(dirname "$0")" && pwd)"
"$BIN/warp-agent-resume" remove "$WARP_TERMINAL_SESSION_UUID"
```

Caveat for the `extra+=` lines under `set -e`: `[[ cond ]] && cmd` as the *last* command of a script fails the script when cond is false, but mid-script lines are fine — both lines here are mid-script; keep them mid-script if editing further.

- [ ] **Step 5: Run tests to verify pass**

```bash
cd tools/agent-resume/tests && bash test_codex_hooks.sh && for t in test_*.sh; do bash "$t" >/dev/null 2>&1 || { echo "REGRESSION: $t"; exit 1; }; done && echo ALL PASS
```
Expected: `PASS` then `ALL PASS`.

- [ ] **Step 6: Commit**

```bash
/usr/bin/git add tools/agent-resume/codex-session-start.sh tools/agent-resume/codex-session-end.sh tools/agent-resume/tests/test_codex_hooks.sh
/usr/bin/git commit -m "feat(agent-resume): carry Codex bypass + model flags from the hook payload

Also call warp-agent-resume by absolute sibling path in both codex hooks;
the bare-name call silently no-oped when the hook env lacked the
agent-resume-bin PATH entry."
```

---

### Task 4: Rust — fix `derive_fork_command` for the launcher command format

**Files:**
- Modify: `app/src/agent_resume.rs:37-54` (`derive_fork_command`)
- Test: `app/src/agent_resume_tests.rs:28-82`

**Interfaces:**
- Consumes: registry `command` strings in the launcher format (Global Constraints).
- Produces: `derive_fork_command(&str) -> Option<String>` returning `claude --resume <id>[ flags…] --fork-session` / `codex fork <id>[ flags…]`. `read_fork_launch` / `ForkLaunch` signatures unchanged.

- [ ] **Step 1: Rewrite the fork tests for the launcher format**

Replace the four tests `derives_claude_fork_command`, `derives_codex_fork_command`, `no_fork_command_for_unknown`, `no_fork_command_for_prefix_with_no_id` (`app/src/agent_resume_tests.rs:28-57`) with:

```rust
#[test]
fn derives_claude_fork_command() {
    assert_eq!(
        derive_fork_command("warp_agent_resume_launch claude abc-123").as_deref(),
        Some("claude --resume abc-123 --fork-session")
    );
}

#[test]
fn derives_codex_fork_command() {
    assert_eq!(
        derive_fork_command("warp_agent_resume_launch codex abc-123").as_deref(),
        Some("codex fork abc-123")
    );
}

#[test]
fn fork_command_carries_launch_flags() {
    assert_eq!(
        derive_fork_command(
            "warp_agent_resume_launch claude abc-123 --dangerously-skip-permissions --model opus"
        )
        .as_deref(),
        Some("claude --resume abc-123 --dangerously-skip-permissions --model opus --fork-session")
    );
    assert_eq!(
        derive_fork_command(
            "warp_agent_resume_launch codex abc-123 --dangerously-bypass-approvals-and-sandbox"
        )
        .as_deref(),
        Some("codex fork abc-123 --dangerously-bypass-approvals-and-sandbox")
    );
}

#[test]
fn no_fork_command_for_unknown() {
    assert_eq!(derive_fork_command("vim"), None);
    assert_eq!(derive_fork_command(""), None);
    // Pre-launcher registry formats are dead; nothing writes them anymore.
    assert_eq!(derive_fork_command("claude --resume abc-123"), None);
    assert_eq!(derive_fork_command("codex resume abc-123"), None);
    // Unknown agents and missing ids are not forkable.
    assert_eq!(
        derive_fork_command("warp_agent_resume_launch gemini abc-123"),
        None
    );
    assert_eq!(derive_fork_command("warp_agent_resume_launch claude"), None);
    assert_eq!(derive_fork_command("warp_agent_resume_launch claude "), None);
}
```

Also update the fixtures in `read_fork_launch_reads_derived_command_and_cwd` (lines 60-82): the `feedface.json` command becomes `"warp_agent_resume_launch codex xyz-9"` (assert unchanged: `codex fork xyz-9`), and the `cafe.json` command becomes `"warp_agent_resume_launch claude id-1 --dangerously-skip-permissions"` with the assert `claude --resume id-1 --dangerously-skip-permissions --fork-session`.

- [ ] **Step 2: Run tests to verify they fail** (first build in this worktree is cold — run in background, allow ~20+ min)

```bash
cargo nextest run -p warp agent_resume
```
Expected: the rewritten tests FAIL (current parser returns `None` for launcher-form strings).

- [ ] **Step 3: Rewrite `derive_fork_command` in `app/src/agent_resume.rs`**

```rust
/// Turns a stored resume command (`warp_agent_resume_launch <agent> <id> [flags…]`) into
/// a fork command, carrying the session's launch flags into the fork. Returns `None` for
/// commands we don't know how to fork (the only forkable agents today are Claude and
/// Codex).
fn derive_fork_command(command: &str) -> Option<String> {
    let rest = command.trim().strip_prefix("warp_agent_resume_launch ")?;
    let mut parts = rest.split_whitespace();
    let agent = parts.next()?;
    let id = parts.next()?;
    let flags = parts.collect::<Vec<_>>().join(" ");
    let flags = if flags.is_empty() {
        String::new()
    } else {
        format!(" {flags}")
    };
    match agent {
        "claude" => Some(format!("claude --resume {id}{flags} --fork-session")),
        "codex" => Some(format!("codex fork {id}{flags}")),
        _ => None,
    }
}
```

(The `_` arm is unavoidable for `&str` matching. `split_whitespace` never yields empty tokens, so `id` is guaranteed non-empty.)

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo nextest run -p warp agent_resume
```
Expected: all `agent_resume` tests PASS (7 tests).

- [ ] **Step 5: Commit**

```bash
/usr/bin/git add app/src/agent_resume.rs app/src/agent_resume_tests.rs
/usr/bin/git commit -m "fix(agent-resume): fork parser understands the launcher command format

derive_fork_command still parsed the pre-hook formats (claude --resume <id> /
codex resume <id>), so fork-from-registry silently returned None for every
entry written since the SessionStart-hook rework. Parse the launcher form and
carry the session's flags into the fork; drop the dead old-format parsing."
```

---

### Task 5: install.sh hook migration + Makefile wiring

**Files:**
- Create: `tools/agent-resume/wire-claude-hooks.sh`
- Modify: `tools/agent-resume/install.sh` (stale-file cleanup + replace lines 61-79 with a call to the new script)
- Modify: `Makefile` (new `agent-resume` target; `install-local` depends on it)
- Test: `tools/agent-resume/tests/test_wire_claude_hooks.sh` (new)

**Interfaces:**
- Produces: `wire-claude-hooks.sh <settings.json> <installed-bin-dir>` — idempotently wires `<bin>/claude-capture.sh` into `SessionStart`, `UserPromptSubmit`, `Stop`; removes any hook entry whose command is `<bin>/claude-session-start.sh`; never touches unrelated hooks.

- [ ] **Step 1: Write the failing test** — create `tools/agent-resume/tests/test_wire_claude_hooks.sh`:

```bash
#!/usr/bin/env bash
# Tests the settings.json hook wiring: adds claude-capture.sh to the three events,
# removes stale pre-rename entries, preserves unrelated hooks, and is idempotent.
set -euo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d)"
CFG="$TMP/settings.json"
BIN="/fake/bin"
NEW="$BIN/claude-capture.sh"
OLD="$BIN/claude-session-start.sh"

fail() { echo "FAIL: $1"; exit 1; }
count() { jq -r --arg c "$NEW" "[.hooks.$1[]?.hooks[]? | select(.command == \$c)] | length" "$CFG"; }

# 1. Missing file: created with all three events wired.
bash "$HERE/wire-claude-hooks.sh" "$CFG" "$BIN"
for ev in SessionStart UserPromptSubmit Stop; do
  [[ "$(count "$ev")" == 1 ]] || fail "$ev not wired on fresh file"
done

# 2. Idempotent: run again, still exactly one entry per event.
bash "$HERE/wire-claude-hooks.sh" "$CFG" "$BIN"
for ev in SessionStart UserPromptSubmit Stop; do
  [[ "$(count "$ev")" == 1 ]] || fail "$ev duplicated on re-run"
done

# 3. Migration: stale pre-rename entry removed; unrelated hooks preserved.
cat > "$CFG" <<EOF
{
  "model": "opus",
  "hooks": {
    "SessionStart": [ { "hooks": [ { "type": "command", "command": "$OLD" } ] } ],
    "PostToolUse": [ { "matcher": "Bash", "hooks": [ { "type": "command", "command": "/keep/me.sh" } ] } ]
  }
}
EOF
bash "$HERE/wire-claude-hooks.sh" "$CFG" "$BIN"
[[ "$(jq -r --arg c "$OLD" '[.hooks[][]?.hooks[]? | select(.command == $c)] | length' "$CFG")" == 0 ]] || fail "stale entry not removed"
[[ "$(jq -r '[.hooks.PostToolUse[]?.hooks[]? | select(.command == "/keep/me.sh")] | length' "$CFG")" == 1 ]] || fail "unrelated hook clobbered"
[[ "$(jq -r '.model' "$CFG")" == "opus" ]] || fail "unrelated settings clobbered"
for ev in SessionStart UserPromptSubmit Stop; do
  [[ "$(count "$ev")" == 1 ]] || fail "$ev not wired after migration"
done

echo "PASS"
```

- [ ] **Step 2: Run to verify it fails**

```bash
bash tools/agent-resume/tests/test_wire_claude_hooks.sh
```
Expected: FAIL — `wire-claude-hooks.sh: No such file or directory`.

- [ ] **Step 3: Create `tools/agent-resume/wire-claude-hooks.sh`** (mode 0755):

```bash
#!/usr/bin/env bash
# Wires the Claude capture hook (claude-capture.sh) into a Claude settings.json:
# SessionStart captures the live session; UserPromptSubmit and Stop keep the entry's
# permission-mode flags in sync with the session's live mode. Also removes entries left
# by the pre-rename install (claude-session-start.sh). jq-merge only -- never clobbers
# unrelated settings or hooks. Idempotent.
#
# Usage: wire-claude-hooks.sh <settings.json> <installed-bin-dir>
set -euo pipefail
CFG="$1"; BIN="$2"
command -v jq >/dev/null || { echo "error: jq is required to wire the Claude hooks" >&2; exit 1; }
mkdir -p "$(dirname "$CFG")"
[[ -f "$CFG" ]] || echo '{}' > "$CFG"
tmp="$(mktemp)"
jq --arg old "$BIN/claude-session-start.sh" --arg c "$BIN/claude-capture.sh" '
  .hooks = (.hooks // {})
  | .hooks |= with_entries(
      .value |= (map(.hooks = ((.hooks // []) | map(select(.command != $old))))
                 | map(select((.hooks | length) > 0)))
    )
  | reduce ("SessionStart", "UserPromptSubmit", "Stop") as $ev (.;
      if ([.hooks[$ev][]?.hooks[]? | select(.command == $c)] | length) > 0 then .
      else .hooks[$ev] = ((.hooks[$ev] // []) + [{ "hooks": [{ "type": "command", "command": $c }] }])
      end)
' "$CFG" > "$tmp"
mv "$tmp" "$CFG"
```

- [ ] **Step 4: Run the test to verify it passes**

```bash
bash tools/agent-resume/tests/test_wire_claude_hooks.sh
```
Expected: PASS.

- [ ] **Step 5: Update `install.sh`**

After the `install -m 0644 "$SRC/claude.zsh" "$BIN/claude.zsh"` line (line 20), add:

```bash
# Remove the pre-rename capture script so a stale settings.json entry can't run it.
rm -f "$BIN/claude-session-start.sh"
```

Replace the whole Claude-wiring block (the comment at lines 61-62 through the `fi` at line 79) with:

```bash
# Wire the Claude capture hooks into ~/.claude/settings.json (SessionStart +
# UserPromptSubmit + Stop; migrates entries from the pre-rename script).
"$SRC/wire-claude-hooks.sh" "$HOME/.claude/settings.json" "$BIN"
echo "Wired Claude capture hooks (SessionStart, UserPromptSubmit, Stop)"
```

- [ ] **Step 6: Update `Makefile`**

Change line 62 `.PHONY: help release install-local ship` → `.PHONY: help release install-local ship agent-resume`.

Change the `install-local` rule header to `install-local: _require-create-dmg agent-resume ## Build the local channel and install /Applications/WarpLocal.app`, and add after the `ship:` line:

```make
agent-resume: ## Install/refresh the agent-resume capture layer (hooks + ~/.warp/agent-resume-bin)
	bash tools/agent-resume/install.sh
```

- [ ] **Step 7: Verify** — full shell suite passes and the Makefile parses:

```bash
cd tools/agent-resume/tests && for t in test_*.sh; do bash "$t" >/dev/null 2>&1 || { echo "FAIL: $t"; exit 1; }; done && echo ALL PASS
cd ../../.. && make -n agent-resume && make -n install-local | head -3
```
Expected: `ALL PASS`; `make -n agent-resume` prints the install.sh line; `make -n install-local` shows the installer running as part of the target.

- [ ] **Step 8: Commit**

```bash
/usr/bin/git add tools/agent-resume/wire-claude-hooks.sh tools/agent-resume/install.sh tools/agent-resume/tests/test_wire_claude_hooks.sh Makefile
/usr/bin/git commit -m "feat(agent-resume): wire UserPromptSubmit/Stop hooks; refresh install via make

install.sh migrates settings.json off the pre-rename hook script and deletes
its stale installed copy. make install-local (and therefore make ship) now
runs the idempotent installer, so the installed capture layer can never drift
from the repo again -- that drift is how flag capture shipped in the repo but
never reached ~/.warp/agent-resume-bin."
```

---

### Task 6: Docs + repo-wide checks

**Files:**
- Modify: `tools/agent-resume/README.md` (component table + any capture-flow prose that says SessionStart-only)

**Interfaces:** none (docs + verification only).

- [ ] **Step 1: Update `tools/agent-resume/README.md`**

Read the file first; then: update the line-124 table row (already renamed in Task 1) to `| \`claude-capture.sh\` | Claude \`SessionStart\`/\`UserPromptSubmit\`/\`Stop\` hook — captures the live session per pane and keeps its permission-mode/\`--model\` flags in sync with the live session |`, update the codex row to mention that bypass/model come from the payload, add a `wire-claude-hooks.sh` row, and fix any prose that describes Claude capture as SessionStart-only or claims flags are launch-argv-only. If Task 3 Step 1 found no codex per-turn hook, add one sentence to the README's limitations section: codex mode changes made mid-session are only re-captured at the next session start.

- [ ] **Step 2: Format + lint gate (required before PR)**

```bash
./script/format
cargo clippy --workspace --all-targets --all-features --tests -- -D warnings
```
Run clippy in the background (cold worktree; long). Expected: format makes no changes to our files; clippy exits 0. Fix anything it flags in our touched files only.

- [ ] **Step 3: Full test sweep**

```bash
cd tools/agent-resume/tests && for t in test_*.sh; do bash "$t" >/dev/null 2>&1 || { echo "FAIL: $t"; exit 1; }; done && echo SHELL OK
cargo nextest run -p warp agent_resume
```
Expected: `SHELL OK`; all rust `agent_resume` tests pass.

- [ ] **Step 4: Commit**

```bash
/usr/bin/git add tools/agent-resume/README.md
/usr/bin/git commit -m "docs(agent-resume): document live-mode tracking and codex payload flags"
```

---

### Task 7: Install on this machine + live verification

**Files:** none in-repo (touches `~/.warp/agent-resume-bin/`, `~/.claude/settings.json`).

- [ ] **Step 1: Run the installer from the worktree**

```bash
bash tools/agent-resume/install.sh
```
Expected output includes `Wired Claude capture hooks (SessionStart, UserPromptSubmit, Stop)`.

- [ ] **Step 2: Verify the installed state**

```bash
diff tools/agent-resume/claude-capture.sh ~/.warp/agent-resume-bin/claude-capture.sh && echo "capture: in sync"
test ! -f ~/.warp/agent-resume-bin/claude-session-start.sh && echo "stale script gone"
jq '[.hooks.SessionStart, .hooks.UserPromptSubmit, .hooks.Stop | .[]?.hooks[]? | select(.command | test("claude-capture"))] | length' ~/.claude/settings.json
jq '[.hooks[][]?.hooks[]? | select(.command | test("claude-session-start"))] | length' ~/.claude/settings.json
```
Expected: `capture: in sync`, `stale script gone`, `3`, `0`.

- [ ] **Step 3: Simulate one live update against the installed scripts** (scratch registry — do not touch the real one):

```bash
TMP="$(mktemp -d)"
WARP_AGENT_RESUME_DIR="$TMP" WARP_TERMINAL_SESSION_UUID=smoketest WARP_AGENT_RESUME_FAKE_ARGV="" \
  bash -c 'echo "{\"session_id\":\"s1\",\"cwd\":\"/tmp\",\"hook_event_name\":\"UserPromptSubmit\",\"permission_mode\":\"bypassPermissions\"}" | ~/.warp/agent-resume-bin/claude-capture.sh'
cat "$TMP/smoketest.json"
```
Expected: `"command": "warp_agent_resume_launch claude s1 --dangerously-skip-permissions"`.

- [ ] **Step 4: Report the remaining manual/user steps** (cannot be automated from inside this session):
  - Real sessions heal on their next prompt (each running claude's UserPromptSubmit now fires the new hook).
  - Restart Clinch → bypass sessions must resume with bypass. Codex: start one codex session with `--dangerously-bypass-approvals-and-sandbox` inside a pane, restart, confirm.
  - The fork fix ships with the next `make ship` / `make install-local` (Rust binary rebuild).

---

### Task 8: Finish the branch

- [ ] **Step 1:** Use the superpowers:finishing-a-development-branch skill — push `agent-resume-flags` to the `clinch` remote (plain `git push`, never `gp`), open a PR against `main` on `elliot-ylambda/clinch-terminal` using `.github/pull_request_template.md`, with changelog line:
  `CHANGELOG-IMPROVEMENT: Restored agent sessions now come back with the permission mode and model they were running with (bypass permissions, plan/acceptEdits, --model), including modes toggled mid-session.`
