#!/usr/bin/env bash
# Tests the Claude SessionStart capture hook: it records the live session id (fresh,
# --resume, picker, or --continue all deliver session_id in the payload) keyed by pane uuid.
set -euo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d)"
export WARP_AGENT_RESUME_DIR="$TMP/reg"
# The hook calls clinch-agent-resume as a sibling; put both in one bin and run from there.
BIN="$TMP/bin"; mkdir -p "$BIN"
install -m 0755 "$HERE/agent-json" "$HERE/clinch-agent-resume" \
  "$HERE/claude-capture.sh" "$BIN/"
install -m 0644 "$HERE/agent-json.js" "$BIN/"

export WARP_TERMINAL_SESSION_UUID="cc33"
export WARP_AGENT_RESUME_FAKE_ANCESTRY="claude"
f="$WARP_AGENT_RESUME_DIR/cc33.json"

# Pin the launch-flag detection off for the plain cases so they are deterministic regardless
# of how this test was launched (a real claude ancestor must not leak its flags in).
export WARP_AGENT_RESUME_FAKE_ARGV=""
# Likewise pin the bridge id off: this test may itself run inside a bridged claude session,
# which would leak CLAUDE_CODE_BRIDGE_SESSION_ID into every capture below.
unset CLAUDE_CODE_BRIDGE_SESSION_ID

# Fresh/startup: session_id recorded via the launcher form.
echo '{"session_id":"sess-aaa","cwd":"/tmp/repo","source":"startup"}' | "$BIN/claude-capture.sh"
grep -q '"command": "clinch_agent_resume_launch claude sess-aaa"' "$f" || { echo "FAIL: startup not recorded"; exit 1; }
grep -q '"cwd": "/tmp/repo"' "$f" || { echo "FAIL: cwd"; exit 1; }

# Resume/picker: the resumed id must OVERWRITE the pane entry (this is the bug being fixed).
echo '{"session_id":"sess-bbb","cwd":"/tmp/repo","source":"resume"}' | "$BIN/claude-capture.sh"
grep -q '"command": "clinch_agent_resume_launch claude sess-bbb"' "$f" || { echo "FAIL: resume did not overwrite stale entry"; exit 1; }

# Launched in bypass mode with a model override (e.g. the `CA` alias): the recorded resume
# command carries those flags through so restore reopens the session the same way.
WARP_AGENT_RESUME_FAKE_ARGV="node /x/claude-code/cli.js --dangerously-skip-permissions --model opus" \
  bash -c 'echo "{\"session_id\":\"sess-ccc\",\"cwd\":\"/tmp/repo\"}" | "$0"' "$BIN/claude-capture.sh"
grep -q '"command": "clinch_agent_resume_launch claude sess-ccc --dangerously-skip-permissions --model opus"' "$f" \
  || { echo "FAIL: launch flags not carried into resume command"; exit 1; }

# Missing session_id: no-op (don't write garbage).
rm -f "$f"
echo '{"cwd":"/tmp/repo","source":"startup"}' | "$BIN/claude-capture.sh"
[[ ! -f "$f" ]] || { echo "FAIL: wrote with no session_id"; exit 1; }

# --- Live-mode updater (UserPromptSubmit / Stop) ---
# The payload's permission_mode is authoritative for the mode; --model still comes from
# the live argv. `default` strips the mode flag; unknown values fall back to argv.

# Toggled to bypass mid-session (entry owned by this sid): entry rewritten with the flag.
echo '{"session_id":"sess-ddd","cwd":"/tmp/repo"}' | "$BIN/claude-capture.sh"
echo '{"session_id":"sess-ddd","cwd":"/tmp/repo","hook_event_name":"UserPromptSubmit","permission_mode":"bypassPermissions"}' | "$BIN/claude-capture.sh"
grep -q '"command": "clinch_agent_resume_launch claude sess-ddd --dangerously-skip-permissions"' "$f" || { echo "FAIL: updater did not add bypass flag"; exit 1; }

# Toggled back to default (via Stop): the mode flag must be stripped again.
echo '{"session_id":"sess-ddd","cwd":"/tmp/repo","hook_event_name":"Stop","permission_mode":"default"}' | "$BIN/claude-capture.sh"
grep -q '"command": "clinch_agent_resume_launch claude sess-ddd"' "$f" || { echo "FAIL: default did not strip mode flag"; exit 1; }

# plan maps to --permission-mode plan.
echo '{"session_id":"sess-ddd","cwd":"/tmp/repo","hook_event_name":"UserPromptSubmit","permission_mode":"plan"}' | "$BIN/claude-capture.sh"
grep -q '"command": "clinch_agent_resume_launch claude sess-ddd --permission-mode plan"' "$f" || { echo "FAIL: plan mode not carried"; exit 1; }

# Model from the live argv is kept alongside the payload mode.
WARP_AGENT_RESUME_FAKE_ARGV="node /x/cli.js --model opus" \
  bash -c 'echo "{\"session_id\":\"sess-ddd\",\"cwd\":\"/tmp/repo\",\"hook_event_name\":\"UserPromptSubmit\",\"permission_mode\":\"bypassPermissions\"}" | "$0"' "$BIN/claude-capture.sh"
grep -q '"command": "clinch_agent_resume_launch claude sess-ddd --dangerously-skip-permissions --model opus"' "$f" || { echo "FAIL: model not kept with payload mode"; exit 1; }

# Unknown permission_mode falls back to argv-derived flags (mode + model).
WARP_AGENT_RESUME_FAKE_ARGV="node /x/cli.js --permission-mode acceptEdits" \
  bash -c 'echo "{\"session_id\":\"sess-ddd\",\"cwd\":\"/tmp/repo\",\"hook_event_name\":\"UserPromptSubmit\",\"permission_mode\":\"weird\"}" | "$0"' "$BIN/claude-capture.sh"
grep -q '"command": "clinch_agent_resume_launch claude sess-ddd --permission-mode acceptEdits"' "$f" || { echo "FAIL: unknown mode did not fall back to argv"; exit 1; }

# A nested session's updater event must not clobber the outer entry…
echo '{"session_id":"sess-intruder","cwd":"/tmp/other","hook_event_name":"UserPromptSubmit","permission_mode":"bypassPermissions"}' \
  | WARP_AGENT_RESUME_FAKE_ANCESTRY="claude,zsh,claude" "$BIN/claude-capture.sh"
grep -q 'sess-ddd' "$f" || { echo "FAIL: foreign session clobbered the pane entry"; exit 1; }

# …and nested SessionStart is equally non-owning (the production incident happened here).
echo '{"session_id":"sess-nested-start","cwd":"/private/tmp","hook_event_name":"SessionStart"}' \
  | WARP_AGENT_RESUME_FAKE_ANCESTRY="claude,zsh,claude" "$BIN/claude-capture.sh"
grep -q 'sess-ddd' "$f" || { echo "FAIL: nested SessionStart clobbered the pane entry"; exit 1; }

# A detached nested tool may be reparented and lose the outer agent from its ancestry.
# The live owner PID/tty still prevents a different process from taking over the pane.
echo '{"session_id":"sess-detached-root","cwd":"/tmp/repo","hook_event_name":"SessionStart"}' \
  | WARP_AGENT_RESUME_FAKE_OWNER_PID=1111 "$BIN/claude-capture.sh"
grep -q '"owner_pid": "1111"' "$f" || { echo "FAIL: root owner metadata not recorded"; exit 1; }
echo '{"session_id":"sess-detached-child","cwd":"/tmp/child","hook_event_name":"SessionStart"}' \
  | WARP_AGENT_RESUME_FAKE_OWNER_PID=2222 WARP_AGENT_RESUME_FAKE_RECORDED_OWNER_ACTIVE=1 \
    "$BIN/claude-capture.sh"
grep -q 'sess-detached-root' "$f" || { echo "FAIL: detached nested agent replaced live owner"; exit 1; }

# …but a missing entry is (re)created — this heals pre-flag registry entries.
rm -f "$f"
echo '{"session_id":"sess-eee","cwd":"/tmp/repo","hook_event_name":"UserPromptSubmit","permission_mode":"bypassPermissions"}' | "$BIN/claude-capture.sh"
grep -q '"command": "clinch_agent_resume_launch claude sess-eee --dangerously-skip-permissions"' "$f" || { echo "FAIL: missing entry not healed"; exit 1; }

# Unknown events are ignored.
echo '{"session_id":"sess-eee","cwd":"/tmp/repo","hook_event_name":"PreCompact","permission_mode":"default"}' | "$BIN/claude-capture.sh"
grep -q -- '--dangerously-skip-permissions' "$f" || { echo "FAIL: unknown event must not rewrite the entry"; exit 1; }

# --- Bridge id capture ---
# A bridged session (claude.ai repl bridge) exports CLAUDE_CODE_BRIDGE_SESSION_ID to its
# hooks; the entry records it so restore can surface the cloud-copy URL.
CLAUDE_CODE_BRIDGE_SESSION_ID="session_01TESTBRIDGE" \
  bash -c 'echo "{\"session_id\":\"sess-fff\",\"cwd\":\"/tmp/repo\"}" | "$0"' "$BIN/claude-capture.sh"
grep -q '"bridge": "session_01TESTBRIDGE"' "$f" || { echo "FAIL: bridge id not recorded"; exit 1; }

# Per-turn events refresh it (bridge can attach after SessionStart)…
CLAUDE_CODE_BRIDGE_SESSION_ID="session_01LATER" \
  bash -c 'echo "{\"session_id\":\"sess-fff\",\"cwd\":\"/tmp/repo\",\"hook_event_name\":\"Stop\",\"permission_mode\":\"default\"}" | "$0"' "$BIN/claude-capture.sh"
grep -q '"bridge": "session_01LATER"' "$f" || { echo "FAIL: updater did not refresh bridge id"; exit 1; }

# …and an un-bridged capture writes no bridge field.
echo '{"session_id":"sess-ggg","cwd":"/tmp/repo"}' | "$BIN/claude-capture.sh"
grep -q '"bridge"' "$f" && { echo "FAIL: bridge field written without env"; exit 1; }
rm -f "$f"   # clear residue so the empty-dir check below only sees new writes

# --- Machinery-spawned fresh sessions must not clobber protected entries ---
# clinch_agent_resume_launch tags its fresh fallback with WARP_AGENT_RESUME_STARTED_FRESH;
# until the user engages, such a blank must leave an entry alone while it still points
# somewhere recoverable (2026-07-08 incident: blank restarts destroyed live mappings).
export WARP_AGENT_RESUME_CLAUDE_PROJECTS="$TMP/projects"
mkdir -p "$TMP/projects/-tmp-repo"
printf '{"type":"user","message":{}}\n' > "$TMP/projects/-tmp-repo/sess-old.jsonl"

# Entry points at a session with a real conversation: the blank's SessionStart skips it…
echo '{"session_id":"sess-old","cwd":"/tmp/repo"}' | "$BIN/claude-capture.sh"
echo '{"session_id":"sess-blank","cwd":"/tmp/repo"}' | WARP_AGENT_RESUME_STARTED_FRESH=1 "$BIN/claude-capture.sh"
grep -q 'sess-old' "$f" || { echo "FAIL: machinery blank clobbered a live entry"; exit 1; }

# …but its first real prompt takes the pane over (marker + UserPromptSubmit).
echo '{"session_id":"sess-blank","cwd":"/tmp/repo","hook_event_name":"UserPromptSubmit","permission_mode":"default"}' \
  | WARP_AGENT_RESUME_STARTED_FRESH=1 "$BIN/claude-capture.sh"
grep -q 'sess-blank' "$f" || { echo "FAIL: engaged fresh session not captured"; exit 1; }

# A bridge entry is protected even when its local sid has no conversation: the cloud copy
# is authoritative and the bridge field is the only durable link to it.
printf '{ "command": "clinch_agent_resume_launch claude sess-gone", "cwd": "/tmp/repo", "bridge": "session_01KEEP" }\n' > "$f"
echo '{"session_id":"sess-blank2","cwd":"/tmp/repo"}' | WARP_AGENT_RESUME_STARTED_FRESH=1 "$BIN/claude-capture.sh"
grep -q 'session_01KEEP' "$f" || { echo "FAIL: machinery blank destroyed a bridge entry"; exit 1; }

# An unprotected entry (dead sid, no bridge) is still taken over by the blank.
printf '{ "command": "clinch_agent_resume_launch claude sess-gone", "cwd": "/tmp/repo" }\n' > "$f"
echo '{"session_id":"sess-blank3","cwd":"/tmp/repo"}' | WARP_AGENT_RESUME_STARTED_FRESH=1 "$BIN/claude-capture.sh"
grep -q 'sess-blank3' "$f" || { echo "FAIL: blank should take over a dead entry"; exit 1; }

# Without the marker (user-started session), SessionStart takeover stays unconditional.
echo '{"session_id":"sess-old","cwd":"/tmp/repo"}' | "$BIN/claude-capture.sh"
echo '{"session_id":"sess-new","cwd":"/tmp/repo"}' | "$BIN/claude-capture.sh"
grep -q 'sess-new' "$f" || { echo "FAIL: user-started session must still take over"; exit 1; }

# SessionEnd removes only its own root entry, so exited agents do not resurrect. A stale
# nested/mismatched end and app-shutdown teardown both preserve the outer mapping.
echo '{"session_id":"sess-other","cwd":"/tmp/repo","hook_event_name":"SessionEnd"}' | "$BIN/claude-capture.sh"
grep -q 'sess-new' "$f" || { echo "FAIL: mismatched SessionEnd removed outer entry"; exit 1; }
mkdir -p "$WARP_AGENT_RESUME_DIR"; printf '%s\n' "$$" > "$WARP_AGENT_RESUME_DIR/.app-terminating"
echo '{"session_id":"sess-new","cwd":"/tmp/repo","hook_event_name":"SessionEnd"}' | "$BIN/claude-capture.sh"
grep -q 'sess-new' "$f" || { echo "FAIL: app shutdown removed restorable entry"; exit 1; }
rm -f "$WARP_AGENT_RESUME_DIR/.app-terminating"
echo '{"session_id":"sess-new","cwd":"/tmp/repo","hook_event_name":"SessionEnd"}' | "$BIN/claude-capture.sh"
[[ ! -f "$f" ]] || { echo "FAIL: user SessionEnd did not remove owned entry"; exit 1; }
rm -f "$f"

# --- Prompt mirror ---
# Every UserPromptSubmit appends the prompt to prompts/<sid>.jsonl so prompt text
# survives locally even for poisoned/child sessions that never write a transcript.
P="$WARP_AGENT_RESUME_DIR/prompts"

# The prompt text round-trips exactly through JSON escaping (quotes, backslash, newline).
prompt_in='say "hi" \ and
a second line'
jq -cn --arg p "$prompt_in" \
  '{session_id:"sess-mm", cwd:"/tmp/repo", hook_event_name:"UserPromptSubmit", permission_mode:"default", prompt:$p}' \
  | "$BIN/claude-capture.sh"
mf="$P/sess-mm.jsonl"
[[ -f "$mf" ]] || { echo "FAIL: prompt not mirrored"; exit 1; }
[[ "$(jq -r '.prompt' "$mf")" == "$prompt_in" ]] || { echo "FAIL: mirrored prompt text mangled"; exit 1; }
[[ "$(jq -r '.cwd' "$mf")" == "/tmp/repo" ]] || { echo "FAIL: mirror cwd wrong"; exit 1; }
jq -e '.ts | test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T")' "$mf" >/dev/null || { echo "FAIL: mirror ts not ISO8601"; exit 1; }
perms="$(stat -f '%Lp' "$mf")"; [[ "$perms" == "600" ]] || { echo "FAIL: mirror perms $perms"; exit 1; }
dperms="$(stat -f '%Lp' "$P")"; [[ "$dperms" == "700" ]] || { echo "FAIL: prompts dir perms $dperms"; exit 1; }

# Stop events and empty prompts mirror nothing.
echo '{"session_id":"sess-mm","cwd":"/tmp/repo","hook_event_name":"Stop","permission_mode":"default"}' | "$BIN/claude-capture.sh"
echo '{"session_id":"sess-mm","cwd":"/tmp/repo","hook_event_name":"UserPromptSubmit","permission_mode":"default","prompt":""}' | "$BIN/claude-capture.sh"
[[ "$(wc -l < "$mf")" -eq 1 ]] || { echo "FAIL: Stop/empty prompt must not mirror"; exit 1; }

# The mirror runs BEFORE the pane-ownership guard: a nested session's prompt is mirrored
# under its own sid even though the pane registry entry is left alone.
echo '{"session_id":"sess-own","cwd":"/tmp/repo"}' | "$BIN/claude-capture.sh"
echo '{"session_id":"sess-nested","cwd":"/tmp/repo","hook_event_name":"UserPromptSubmit","permission_mode":"default","prompt":"nested prompt"}' \
  | WARP_AGENT_RESUME_FAKE_ANCESTRY="claude,zsh,claude" "$BIN/claude-capture.sh"
grep -q 'sess-own' "$f" || { echo "FAIL: nested session clobbered the pane entry"; exit 1; }
grep -q 'nested prompt' "$P/sess-nested.jsonl" || { echo "FAIL: nested session prompt not mirrored"; exit 1; }

# A bridged session records its bridge id in each mirror line.
CLAUDE_CODE_BRIDGE_SESSION_ID="session_01MIRROR" \
  bash -c 'echo "{\"session_id\":\"sess-mm\",\"cwd\":\"/tmp/repo\",\"hook_event_name\":\"UserPromptSubmit\",\"permission_mode\":\"default\",\"prompt\":\"second\"}" | "$0"' "$BIN/claude-capture.sh"
[[ "$(tail -n 1 "$mf" | jq -r '.bridge')" == "session_01MIRROR" ]] || { echo "FAIL: bridge id not in mirror line"; exit 1; }

# Size cap: past ~5 MB the file gets ONE truncation marker and nothing further.
# (awk, not `yes | head`: a producer killed by SIGPIPE fails the test under pipefail.)
big="$P/sess-big.jsonl"
awk 'BEGIN { for (i = 0; i < 60000; i++) printf "%0100d\n", i }' > "$big"   # ~6 MB of full lines
echo '{"session_id":"sess-big","cwd":"/tmp/repo","hook_event_name":"UserPromptSubmit","permission_mode":"default","prompt":"over cap"}' | "$BIN/claude-capture.sh"
tail -n 1 "$big" | grep -q '"truncated":true' || { echo "FAIL: cap did not append truncation marker"; exit 1; }
size1="$(stat -f %z "$big")"
echo '{"session_id":"sess-big","cwd":"/tmp/repo","hook_event_name":"UserPromptSubmit","permission_mode":"default","prompt":"still over"}' | "$BIN/claude-capture.sh"
[[ "$(stat -f %z "$big")" -eq "$size1" ]] || { echo "FAIL: capped file kept growing"; exit 1; }
rm -f "$f"

# Outside a Clinch pane: no-op -- no pane entry, no journal growth, no prompt mirror.
unset WARP_TERMINAL_SESSION_UUID
J="$WARP_AGENT_RESUME_DIR/journal.jsonl"
before="$(find "$WARP_AGENT_RESUME_DIR" | sort); $(wc -l < "$J")"
echo '{"session_id":"sess-outside","cwd":"/tmp","hook_event_name":"UserPromptSubmit","prompt":"outside"}' | "$BIN/claude-capture.sh"
after="$(find "$WARP_AGENT_RESUME_DIR" | sort); $(wc -l < "$J")"
[[ "$before" == "$after" ]] || { echo "FAIL: wrote outside pane"; exit 1; }

echo "PASS"
