---
name: clinch-toolbelt
description: Add, edit, or remove quick-insert buttons in the Clinch terminal's footer toolbelts. Use when the user asks for a new Clinch button, quick-insert button, footer or toolbelt button, or wants to change what a Clinch footer button inserts — for the Claude Code, Codex, or plain terminal footer.
---

<!-- managed-by: Clinch; version: 1.9.0 -->

# Clinch toolbelt quick-insert buttons

Clinch shows rows of clickable quick-insert buttons. Each custom button has a
`label` (shown on the button), `text`, and optional `auto_send` behavior. Existing
buttons default to `auto_send = true`; set it to `false` when clicking should only
pre-fill the active input. The footer popup has three independently persisted tabs:

- **Claude Code footer** — shown under Claude Code panes. Its text is sent to
  Claude Code as a prompt.
- **Codex footer** — shown under Codex panes. Its text is sent to Codex as a
  prompt.
- **Terminal footer** — shown under plain shell panes. Its text is run as a
  shell command.

Shipped buttons are live defaults owned by the installed Clinch build. Custom
buttons are an overlay and render after those defaults. Do not copy the shipped
default list into settings: future Clinch buttons must be able to appear without
overwriting the user's additions.

The UI presents each tab as one ordered **Footer buttons** list plus
**Available buttons**. `+` shows a button, `×` hides it, and prompt-style buttons
have an **Auto-send** toggle. The persisted `left` and `right` arrays remain for
backward compatibility; the single-list editor saves its ordered buttons in
`left` and leaves `right` empty.

The Claude Code and Codex tabs list only Clinch session actions, shipped prompt
recipes, and user-created prompt buttons. Generic directory, user, host, Git,
time, environment, and input-control chips belong elsewhere in the UI and are
not offered by this quick-insert editor.

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

Each provider-specific setting is optional. When the Claude Code or Codex value
is absent, Clinch reads the legacy shared CLI-agent value so existing users keep
their layout. Once a provider tab is changed, Clinch writes an independent
`"default"` value or `custom` table for that provider:

- Claude Code footer: `agents.third_party.claude_code_toolbar_chip_selection_setting`
- Codex footer: `agents.third_party.codex_toolbar_chip_selection_setting`
- Terminal footer: `terminal.footer_toolbar_chip_selection`
- Legacy fallback and other CLI agents:
  `agents.third_party.cli_agent_toolbar_chip_selection_setting`

Use the provider-specific path when the request names Claude Code or Codex. Do
not edit the legacy fallback unless the request explicitly targets all/other CLI
agents.

A live-default custom table has these fields:

```toml
[agents.third_party.claude_code_toolbar_chip_selection_setting.custom]
inherit_defaults = true
left = [
  { custom_insert = { label = "Ship It", text = "Run the tests, then ship this." } },
  { custom_insert = { label = "Draft", text = "Help me write this", auto_send = false } },
]
right = []
```

This displays the current shipped Claude Code footer buttons first and `Ship It`
afterward. For Codex, use the same shape under
`agents.third_party.codex_toolbar_chip_selection_setting.custom`. For a terminal
command, use the terminal setting instead:

```toml
[terminal.footer_toolbar_chip_selection.custom]
inherit_defaults = true
left = [
  { custom_insert = { label = "Status", text = "git status --short --branch" } },
]
right = []
```

To save a user-created button without showing it in the footer, put it in
`hidden_custom_inserts` instead of `left` or `right`:

```toml
[agents.third_party.claude_code_toolbar_chip_selection_setting.custom]
inherit_defaults = true
left = []
right = []
hidden_custom_inserts = [
  { custom_insert = { label = "Later", text = "Review this later", auto_send = false } },
]
```

The toolbar editor exposes the same choice as **Show in footer**. Hidden custom
buttons remain in **Available buttons**, where the user can re-enable or edit
them after restarting Clinch.

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
- Set `auto_send = false` to pre-fill without submitting. Omit it or set it to
  `true` to submit immediately, preserving the historical behavior.
- To hide a user-created button while keeping it available, move its matching
  entry from `left` or `right` to `hidden_custom_inserts`.
- To show a hidden user-created button, move it from `hidden_custom_inserts` to
  `left` or `right`.
- To delete a user-created button, remove it from all three lists.
- To remove a shipped button, add its serialized toolbar item to
  `hidden_defaults`. Clinch persists that explicit removal while still adding
  defaults introduced by later releases.
- To restore a shipped button, remove it from `hidden_defaults`.

For example, this hides the shipped `compact` button while retaining live
defaults and a custom button:

```toml
[agents.third_party.claude_code_toolbar_chip_selection_setting.custom]
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
- Put agent prompts in the requested Claude Code or Codex footer and shell
  commands in the terminal footer.
