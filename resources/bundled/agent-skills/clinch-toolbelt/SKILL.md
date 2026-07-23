---
name: clinch-toolbelt
description: Add, edit, or remove quick-insert buttons in the Clinch terminal's footer toolbelts. Use when the user asks for a new Clinch button, quick-insert button, footer or toolbelt button, or wants to change what a Clinch footer button inserts — for the Claude Code/Codex agent footer or the plain terminal footer.
---

<!-- managed-by: Clinch; version: 1.4.0 -->

# Clinch toolbelt quick-insert buttons

Clinch (the terminal app hosting this session) shows rows of clickable
quick-insert buttons. Each button has a `label` (shown on the button) and a
`text` (inserted into the input and submitted when clicked). There are two
independent button rows:

- **CLI agent footer** — shown under Claude Code and Codex panes. Button text
  is sent to the running agent as a prompt.
- **Terminal footer** — shown under plain shell panes. Button text is run as a
  shell command.

You can create, edit, or remove these buttons for the user by editing Clinch's
settings file. Clinch watches the file and hot-reloads it: changes appear
immediately, no app restart needed.

## 1. Locate the settings file

- Stable Clinch: `~/.warp/settings.toml`
- Preview builds: `~/.warp-preview/settings.toml`
- Dev builds: `~/.warp-dev/settings.toml`

Use the stable path if it exists. If more than one exists and it is unclear
which build the user runs, ask.

## 2. Back up before editing

```bash
cp ~/.warp/settings.toml ~/.warp/settings.toml.bak
```

## 3. Edit the right key

Buttons are `{ custom_insert = { label = "…", text = "…" } }` entries inside a
`left`/`right` layout. Each footer has one setting, which is either the string
`"default"` (or absent — same meaning) or a `custom` table:

- CLI agent footer: `agents.third_party.cli_agent_toolbar_chip_selection_setting`
- Terminal footer: `terminal.footer_toolbar_chip_selection`

### Case A — the setting already has a `custom` table

Append the new button to the existing `left` array (or `right`, if the user
asks for the right side). Example: the file already contains

```toml
[agents.third_party.cli_agent_toolbar_chip_selection_setting.custom]
left = [
  "fork_session",
  "compact",
]
right = ["settings"]
```

To add a button, append to `left`:

```toml
left = [
  "fork_session",
  "compact",
  { custom_insert = { label = "Ship It", text = "Run the tests, then ship this." } },
]
```

### Case B — the setting is `"default"` or absent

Writing a `custom` table with only the new button would DELETE every default
button from the user's footer. First materialize the shipped defaults below,
then append the new button.

CLI agent footer defaults (verbatim):

```toml
[agents.third_party.cli_agent_toolbar_chip_selection_setting.custom]
left = [
  "fork_session",
  "compact",
  "continue_prompt",
  "looks_good_prompt",
  "transfer_agent",
  { custom_insert = { label = "/codex", text = "/codex" } },
  { custom_insert = { label = "Make No Mistakes", text = "Do it all for me. I'm stepping away. Don't make any mistakes." } },
  { custom_insert = { label = "Create a Plan", text = "Create a Plan" } },
  { custom_insert = { label = "Build w/ Sub-agents", text = "Build w/ Sub-agents" } },
  { custom_insert = { label = "Create a PR", text = "Create a PR, then merge main into this PR" } },
  { custom_insert = { label = "Worktree-Build", text = "OK go into an isolated work tree. Plan this out, then implement it and create a pull request." } },
  { custom_insert = { label = "Review w/ Codex Sol Max", text = "Review w/ Codex Sol Max" } },
  { custom_insert = { label = "Review w/ Claude Code Fable", text = "Review w/ Claude Code Fable" } },
  { custom_insert = { label = "Debug w/ Ultracode", text = "Investigate with Ultra Code and use subagents" } },
  { custom_insert = { label = "Git Worktree", text = "Move our current work and code into an isolated git work tree. And create a branch. Work out of the git worktree" } },
  { custom_insert = { label = "Fix & Verify", text = "Implement the requested fix, run the most relevant checks, and summarize what changed." } },
  { custom_insert = { label = "Simplify", text = "Simplify the current implementation without changing behavior, then run the relevant tests." } },
  "voice_input",
  { custom_insert = { label = "Push2Main", text = "Push all these changes to main." } },
]
right = [{ context_chip = "working_directory" }, { context_chip = "shell_git_branch" }, "settings"]
```

Terminal footer defaults (verbatim):

```toml
[terminal.footer_toolbar_chip_selection.custom]
left = [
  { custom_insert = { label = "Claude", text = "claude --dangerously-skip-permissions" } },
  { custom_insert = { label = "Codex", text = "codex --dangerously-bypass-approvals-and-sandbox" } },
  { custom_insert = { label = "Claude resume", text = "claude --dangerously-skip-permissions --resume" } },
  { custom_insert = { label = "Codex resume", text = "codex resume" } },
  { custom_insert = { label = "Open", text = "open ." } },
  { custom_insert = { label = "Commit & Push", text = "git add -A && printf 'Commit message [Update changes]: ' && IFS= read -r clinch_commit_message && git commit -m \"${clinch_commit_message:-Update changes}\" && git push" } },
  { custom_insert = { label = "Commit", text = "git add -A && printf 'Commit message [Update changes]: ' && IFS= read -r clinch_commit_message && git commit -m \"${clinch_commit_message:-Update changes}\"" } },
  { custom_insert = { label = "Status", text = "git status --short --branch" } },
]
right = []
```

## 4. Editing and removing buttons

Edit the `label`/`text` of the matching `custom_insert` entry, or delete the
entry from the array. Never remove or reorder entries the user did not ask
about.

## Rules

- Touch only the two toolbar settings above; leave every other key untouched.
- Keep the file valid TOML (Clinch surfaces parse errors to the user and the
  broken file stops applying).
- Changes hot-reload instantly; tell the user to look at the footer — no
  restart required.
- If the user asks for a button that runs a shell command, put it in the
  terminal footer; if it sends an agent prompt, use the CLI agent footer.
