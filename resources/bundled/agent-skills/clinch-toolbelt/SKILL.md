---
name: clinch-toolbelt
description: Add, edit, or remove quick-insert buttons in the Clinch terminal's footer toolbelts. Use when the user asks for a new Clinch button, quick-insert button, footer or toolbelt button, or wants to change what a Clinch footer button inserts — for the Claude Code/Codex agent footer or the plain terminal footer.
---

<!-- managed-by: Clinch; version: 1.5.0 -->

# Clinch toolbelt quick-insert buttons

Clinch shows rows of clickable quick-insert buttons. Each custom button has a
`label` (shown on the button) and `text` (submitted when clicked). There are two
independent rows:

- **CLI agent footer** — shown under Claude Code and Codex panes. Its text is
  sent to the running agent as a prompt.
- **Terminal footer** — shown under plain shell panes. Its text is run as a
  shell command.

Shipped buttons are live defaults owned by the installed Clinch build. Custom
buttons are an overlay and render after those defaults. Do not copy the shipped
default list into settings: future Clinch buttons must be able to appear without
overwriting the user's additions.

## 1. Locate the active Clinch settings file

- Stable Clinch: `~/.clinch/settings.toml`
- Preview Clinch: `~/.clinch-preview/settings.toml`
- Clinch Dev/local source build: `~/.clinch-local/settings.toml`

Use the path belonging to the running Clinch build. Never edit
`~/.warp/settings.toml` for Clinch; that file belongs to Warp.

## 2. Back up before editing

For stable Clinch:

```bash
cp ~/.clinch/settings.toml ~/.clinch/settings.toml.bak
```

Use the corresponding preview or local path when that is the active build.

## 3. Edit only the requested footer

Each setting is either `"default"` (or absent, with the same meaning) or a
`custom` table:

- CLI agent footer: `agents.third_party.cli_agent_toolbar_chip_selection_setting`
- Terminal footer: `terminal.footer_toolbar_chip_selection`

A live-default custom table has these fields:

```toml
[agents.third_party.cli_agent_toolbar_chip_selection_setting.custom]
inherit_defaults = true
left = [
  { custom_insert = { label = "Ship It", text = "Run the tests, then ship this." } },
]
right = []
```

This displays the current shipped CLI buttons first and `Ship It` afterward.
For a terminal command, use the terminal setting instead:

```toml
[terminal.footer_toolbar_chip_selection.custom]
inherit_defaults = true
left = [
  { custom_insert = { label = "Status", text = "git status --short --branch" } },
]
right = []
```

### Existing `custom` table

Append a requested button to the existing `left` or `right` array without
removing or reordering other entries. Add `inherit_defaults = true` if it is
absent. Older settings may contain a snapshot of shipped entries; leave those
entries intact. Clinch recognizes them as defaults, uses the definitions and
ordering from the current build, and appends only genuine custom entries.

### `"default"` or absent setting

Replace `"default"`, or create the missing setting, with a `custom` table like
the examples above. Include only the requested custom entries and
`inherit_defaults = true`; do not materialize shipped defaults.

## 4. Edit or remove buttons

- To edit a user-created button, change its matching `custom_insert` entry.
- To remove a user-created button, delete its matching entry.
- To remove a shipped button, add its serialized toolbar item to
  `hidden_defaults`. Clinch persists that explicit removal while still adding
  defaults introduced by later releases.
- To restore a shipped button, remove it from `hidden_defaults`.

For example, this hides the shipped `compact` button while retaining live
defaults and a custom button:

```toml
[agents.third_party.cli_agent_toolbar_chip_selection_setting.custom]
inherit_defaults = true
hidden_defaults = ["compact"]
left = [
  { custom_insert = { label = "Ship It", text = "Run the tests, then ship this." } },
]
right = []
```

Shipped quick inserts are represented by their full `custom_insert` value in
`hidden_defaults`; use the current value already present in settings or shown
by Clinch's toolbar editor. Their label is the stable identity, so prompt-text
updates do not re-enable a button the user hid.

## Rules

- Touch only the requested toolbar setting and leave every other key intact.
- Preserve all existing custom entries unless the user explicitly asks to
  change them.
- Keep the file valid TOML; a parse error prevents settings from applying.
- Clinch watches this file and hot-reloads valid changes. Tell the user to look
  at the footer; no restart is normally required.
- Put agent prompts in the CLI agent footer and shell commands in the terminal
  footer.
