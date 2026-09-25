# Clinch session handoff

`clinch sessions` discovers saved Claude Code and Codex conversations across all
Clinch projects, resumes them in Orca, and transfers stopped conversations and Git
checkouts between macOS and Linux. This is a saved-state handoff: it does not move a
running process, terminal scrollback, unsent input, or background jobs.

Requires Python 3.9+, Git, and the destination agent CLI and Orca CLI. Transfers use
your existing SSH configuration and authentication. The SSH worker runs in memory;
the other host does not need a Clinch desktop installation. Install and sign in to
Claude/Codex on that host separately.

## Current-session shortcuts

```sh
clinch sessions to-devbox --dry-run --json
clinch sessions to-devbox
clinch sessions from-devbox
clinch sessions transferred
clinch sessions transferred --host devbox
```

`to-devbox` identifies this Clinch pane's exact Claude/Codex conversation and opens
a visible **To DevBox** handoff tab. Finish the turn, then exit the source agent
with `/exit` or Ctrl-D. The task waits for the agent to stop and the transcript to
stabilize, copies its saved state, and starts it in DevBox's Orca runtime. Keep the
task open and the Mac awake until it prints **Complete**. Other same-provider
agents sharing the checkout also block the transfer; no agent is killed. Waiting
times out after 30 minutes. `clinch sessions cancel JOB_ID` cancels a waiting task.

`from-devbox` retrieves the latest completed outbound handoff for this conversation,
including new DevBox messages and code changes. Exit the agent in Orca first. If
this conversation is also running on Mac, exit that copy too. The returned agent
opens in its original Clinch project, in a **Transferred** section. For sessions
not sent by this Mac, choose `claude:SESSION_ID` or `codex:SESSION_ID` from
`transferred --host devbox`; use `--remote-cwd` for ambiguous remote copies.

The original project tab must still exist. Tab creation uses its durable pane
identity even when another project is active. The installed app's section API
only addresses the selected project: switching projects during handoff can leave
the new tab ungrouped. Select the original project and run
`clinch sessions organize JOB_ID`. Exact tab IDs prevent grouping a session in the
wrong project. If the original tab was closed, the import remains with a failed
launch receipt; after checking that no agent started, use
`open-in clinch SESSION_ID --cwd DESTINATION` from the desired project to retry.

Each handoff creates `~/.clinch/transfers/PROJECT/HANDOFF_ID/PROJECT` on the destination.
Use `--dest` to override it and `--host` to override the `devbox` SSH alias.
`--include` selects required ignored files; selections carry into the return trip.
Dependencies and provider credentials/configuration need separate provisioning.

For GitHub repositories these shortcuts select a credential-free `clinch` or
`origin` remote and a fetched tracking ancestor. Published history downloads from
GitHub on the destination. The upload contains unpublished commits, working
changes, and conversation artifacts. Nothing is pushed to GitHub. The new checkout
retains a clean GitHub `origin` and a tracking base for return trips. Use
`--git-remote NAME` to choose another GitHub remote. The destination must already
have access through GitHub CLI or SSH. A missing published base stops the transfer.
Without a usable GitHub ancestor, the bounded full-history bundle is used.

## Find the session on your phone

Pair Orca Mobile **directly with DevBox's Orca server** over Tailscale. After
Complete, open **DevBox → PROJECT → [From Mac] CONVERSATION**. The checkout retains
the project name and Orca receives the conversation title. Open its terminal tab
even if Orca does not show an agent-status badge. The agent runs in the server's
Orca terminal, so the Mac can now sleep. Pairing only to a Mac that exposes an SSH
worktree does not provide this lifetime. See
[Orca's headless mobile pairing instructions](https://www.onorca.dev/docs/remote-servers#mobile-from-a-headless-server).

## Use the helper before the next app release

```sh
python3 tools/session-transfer/install-local
~/.local/bin/clinch-sessions to-devbox --dry-run --json
```

This installs a versioned helper under `~/.local/share/clinch/session-transfer/`
and a `clinch-sessions` command without changing the signed app. It accepts the
same arguments as `clinch sessions`, which is available with the updated app bundle.
Old versions stay available to queued jobs.

Custom Clinch toolbar buttons submit prompts to the current agent. Shared
Claude/Codex buttons **To DevBox**, **Back to Mac**, and **Transfers** can instruct
the agent to run these shortcuts. The first two prepare a handoff and tell the
user to exit the source; they must not kill an agent or start a detached transfer.
Jobs record status, destination, and recovery details under
`~/.clinch/session-transfer/jobs/`.

## Inspect before applying

```sh
clinch sessions list --json
clinch sessions open-in orca --all --dry-run --json
clinch sessions open-in orca claude:SESSION_ID --dry-run
```

`list` reads every saved project, not just the visible one. It combines saved pane
restore IDs with Clinch's current agent-resume registry. Provider IDs are different
from Clinch pane IDs. Non-agent panes and panes without a recorded resume ID are
reported separately. Prompt text is never printed. This inventory is the saved tab
set, not every conversation ever recorded in the provider's history.

When several installations exist, select the database explicitly:

```sh
clinch sessions list --database "$HOME/Library/Application Support/sh.clinch.Clinch/warp.sqlite"
```

Use `--registry` for a custom registry. `WARP_AGENT_RESUME_DIR` is also respected.
Stable Clinch defaults to `~/.warp/agent-resume`; Clinch Dev uses
`~/.clinch-local/agent-resume`. Data profiles require an explicit `--database`.

An exact provider ID can be used without a saved Clinch tab:

```sh
clinch sessions open-in orca codex:SESSION_ID --cwd /absolute/repository
```

`--agent-home` selects a nondefault `CODEX_HOME` or `CLAUDE_CONFIG_DIR`.
`--transcript` disambiguates duplicate transcripts inside that home. Original
commands are parsed for provider/ID only and are never executed verbatim.

## Open existing Mac sessions in Orca

Exit the source Claude/Codex process first; its saved conversation remains.
Then run:

```sh
clinch sessions open-in orca claude:SESSION_ID
clinch sessions open-in orca --all
```

Orca registers the existing checkout and creates a terminal with the agent's resume
command. It does not create a new worktree. Titles are retained; Clinch sections
become `[Section] Title` prefixes because Orca does not support the same sidebar
sections. The JSON preview retains section metadata, including empty sections.
Split layouts, other tab types, pins and colors are not recreated in Orca.

`--all` reports per-session failures and continues with the other selected sessions.
A running or unsupported session yields exit code 1 rather than being silently
skipped. Exit code 0 means every selected session was ready/opened. Plain shell
panes are listed separately and are not launched as guessed agent conversations.

The current working directories must remain available. In particular, do not delete
a Clinch-managed worktree after registering that same directory in Orca.

## Move between machines

The destination must be a **new absolute checkout root**. Existing directories are
never overwritten. `local` means the machine executing the command; other values
are SSH host aliases or `user@host`. Both directions can be driven from your Mac:

```sh
clinch sessions move codex:SESSION_ID --to devbox \
  --dest /home/elliot/projects/my-repo-handoff --dry-run --json

clinch sessions move codex:SESSION_ID --to devbox \
  --dest /home/elliot/projects/my-repo-handoff

clinch sessions move codex:SESSION_ID --from devbox --to local \
  --dest "$HOME/Projects/my-repo-returned" --dry-run
```

Remove `--dry-run` to apply the return transfer. Imported sessions are discoverable
from receipts on a headless host. If the remote session was not imported with this
tool, pass `--agent claude|codex`, `--cwd`, and optionally `--agent-home`.

The transfer contains the selected Git HEAD and its reachable history, branch name,
staged and unstaged binary patches, untracked files, conversation log, and supported
session artifacts. It constructs an independent checkout; it does not copy `.git`
worktree pointers or repository hooks/configuration. This lower-level `move`
command omits remote URLs because they can contain credentials; configure the
remote afterward. The shortcuts above restore a validated GitHub remote instead.
A session started in a repository subdirectory
resumes in the corresponding subdirectory of the new root.

Ignored files (including `.env`, build products, and dependencies) are not included
automatically. The plan reports whether ignored files exist. Explicitly select
required files or directories relative to the Git root:

```sh
clinch sessions move claude:SESSION_ID --to devbox \
  --dest /home/elliot/projects/my-repo-handoff --include .env --include local-data
```

`--orca-bin` selects Orca's executable on the host that will launch the terminal.
`--dest-agent-home` selects the destination provider store; it does not provision
credentials, plugins, hooks, skills, MCP servers, or dependencies.

## History and ownership

- Source agents must be stopped. Detection uses the registry and process table;
  other same-provider agents in the same working directory conservatively block a
  handoff too. No process is killed. Don't start the source again during transfer.
- A per-provider/session lock prevents concurrent Clinch handoffs on a host. A
  short launch lease covers the delay before Orca starts the agent process.
- Source project files and history remain available. A return transfer may extend
  an earlier transcript only when its hash appears in the handoff ancestry and
  the incoming JSONL is an append-only continuation. Earlier bytes are backed up.
  Divergent history is rejected. Shared hardlinked history requires a separate
  destination provider home.
- Claude's main JSONL, per-session subagent/tool-result directories, checkpoint
  backups, image cache, uploads, and referenced plans/paste-cache files are copied.
  Codex rollouts and descendants recorded by `collab_agent_spawn_end` are copied.
  Codex verifies/indexes the received parent through `app-server thread/read`;
  Clinch never edits Codex's SQLite database or sends a model turn for verification.
- Historical absolute paths are **not rewritten** in prose or tool results. A
  warning is emitted before transfer; receipts record source/destination mappings.
  References to old attachment paths may need reopening on the destination. This
  preserves the files but is not a guarantee of transparent attachment relocation.

## Failure and recovery

Checksums, size limits, path validation, stopped-source checks, and source-change
detection run before launch. No credentials or source files are deleted.

Receipts are saved with private permissions under
`~/.clinch/session-transfer/receipts/`. After checkout installation, a failed or
uncertain launch leaves the imported files and a receipt marked `launch_failed` or
`launching`. A repeat `move` cannot overwrite that checkout or launch it again.

Inspect the receipt and Orca's terminal list on that host. If the terminal was
created, use it. If no process/terminal was created, resolve the reported error,
remove the matching pending launch record under
`~/.clinch/session-transfer/launches/` **only after checking that the session is not
running**, then use `open-in orca` with the receipt's session ID and destination
`--cwd`. This explicitly retries only the launch; don't repeat the checkout move.
An already completed launch has a 30-second grace period before another handoff.
Transcript backups from return transfers live under the sibling `backups/` folder.

## Current limits and development

Transfers are bounded to 128 MiB of uncompressed payload and 10,000 artifacts.
Submodules, Git LFS, non-Git folder moves, symlinked artifacts/untracked files, and
Codex stores without portable `response_item` rollout history are rejected.
Shell-only tabs and nonpersisted/deleted conversations cannot be reconstructed.
This tool does not implement arbitrary named Orca chat sections or a skill wrapper.

Run directly from a checkout (without installing or restarting Clinch):

```sh
python3 tools/session-transfer/clinch-sessions --help
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tools/session-transfer/tests -v
script/macos/test_create_warpctrl_wrapper
```

Tests use temporary homes, repositories and fake Orca/Claude executables. If Codex
is installed, a real app-server indexes an isolated fixture without an API call.
The same suite runs on macOS and Linux. No test reads or transfers personal chats.

An optional SSH round-trip test creates temporary fixture homes on an existing
SSH host with Python, Git, and Claude installed. Its fake Orca executable records
launches without starting agents; the fixtures are removed afterward:

```sh
CLINCH_SESSION_TEST_HOST=devbox PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s tools/session-transfer/tests -p test_remote_ssh.py -v
```
