# Sessions: restore, continue, and sleep

## Session restore is enabled by default

Clinch can reopen a captured Claude Code, Codex or Devin conversation in the pane that owned it. This
requires provider hooks, so Clinch enables its local session-capture integration on first launch.
You can turn it off or back on at any time from **Clinch Settings → Agents → Claude Code, Codex and
Devin session capture**.

While session capture is enabled, Clinch:

- adds clearly marked managed hooks to `~/.claude/settings.json`, `~/.codex/config.toml` and
  `~/.config/devin/config.json`, leaving a Devin config with comments untouched;
- installs Clinch-owned helper files in `~/.warp/agent-resume-bin/`;
- stores local pane mappings, a journal, and prompt mirrors in `~/.warp/agent-resume/`, including
  Devin's final replies because its own history has no documented format; and
- records the enabled/disabled preference and a hash/mode receipt under
  `~/Library/Application Support/sh.clinch.Clinch/agent-integration/`.

Clinch repairs those managed entries on later launches only while the setting is enabled. Turning
it off persists the opt-out and deletes its hooks and helper files while preserving unrelated
provider configuration and captured metadata. Purging captured metadata is a separate action. The
command-line equivalents are:

```bash
bash tools/agent-resume/install.sh enable
bash tools/agent-resume/install.sh disable
bash tools/agent-resume/install.sh purge
```

Conversation recovery is best-effort. Clinch starts a new provider process with a resume command;
it does not preserve a live process through quit, crash, or reboot. Provider retention, invalid or
deleted transcripts, changed CLIs, and abrupt power loss can prevent an exact restore.

## Notification plugins

Notification plugins are separate from session capture. Clinch ships pinned snapshots of the
Warp notification plugins for Claude Code and Codex and best-effort installs them into the
provider-owned user plugin stores on launch when those CLIs are present. The payload is local to
the Clinch app and uses dedicated Clinch marketplace IDs, so installation neither clones a plugin
repository nor replaces other Warp/Oz marketplace plugins. The app also puts a pinned universal
`jq` on each local pane's `PATH`, satisfying the plugins' runtime dependency without Homebrew.
Missing CLIs and provider policy failures do not block launch and are retried later. Use
`./uninstall.sh --remove-plugins` when you want to remove the provider plugins as well as Clinch.

## After a restart

A session that was mid-turn when Clinch stopped, working or waiting on a permission prompt or a
question, comes back first and is told to continue where it left off and to check before repeating
anything with outside effects. Anything still running in that session's process, such as a command
or a background subagent, ended with it. Clinch skips the prompt if you typed in the pane first, and
ignores a record more than 24 hours old. Turn this off with **Settings → Clinch → Continue running
sessions after a restart**.

Every other session comes back asleep: its tab, title and conversation are there, and its agent
does not start until you press **Wake**, on its row in the tab list or on the card over the tab, or
something sends to it. An asleep tab shows a moon. This keeps a launch from starting dozens of
agents you may not use; an idle Claude Code session holds about 700 MB.

## Sleeping tabs

A Claude Code or Codex tab you are not using can sleep while Clinch runs, too. Clinch stops the
agent and its helper processes, and keeps the tab. Waking it runs the same resume command a
restart does, so the conversation picks up where it stopped.

- Right-click a tab and choose **Sleep**, or **Sleep idle sessions** for the project or for every
  project.
- Wake one with the **Wake** button on its row, which leaves you where you are, or on the card
  over the tab, which shows the agent and the tab's title. Going to an asleep tab doesn't wake it,
  and it takes no typing until it's awake.
- Tabs you have not used for a day (24 hours) sleep on their own. Change the time or turn this
  off with **Settings → Clinch → Put idle sessions to sleep after**.
- A session never sleeps while it is working, waiting on you, running a background shell, monitor
  or subagent, or holding a message you have not sent. The Coordinator and the tab on screen do
  not sleep on their own.

More implementation detail is in
[tools/agent-resume/README.md](../tools/agent-resume/README.md).
