---
name: clinch-toolbelt
description: Create, delete, list, or reorder quick-insert buttons in the Clinch terminal's Claude Code, Codex, or plain-terminal footer toolbelt. Use when the user asks for a Clinch button, quick-insert button, footer button, or a change to button ordering.
---

<!-- managed-by: Clinch; version: 2.0.0 -->

# Clinch toolbelt buttons

Use Clinch's typed local-control CLI. Never edit `settings.toml`, SQLite, or
other Clinch persistence directly for a toolbelt request.

Clinch rendered this skill for the app channel that installed it:

- Command: `{{clinch_control_binary_name}}`
- Bundled wrapper: `{{clinch_control_wrapper_path}}`

Use the command when it is on `PATH`; otherwise use the exact wrapper. If
neither is available, tell the user to enable scripting and install the control
command from **Clinch Settings > Scripting**. Refer to the selected executable
as `<control-command>` below.

## Footer behavior

Each footer is independent:

- `claude-code` inserts a prompt into Claude Code.
- `codex` inserts a prompt into Codex.
- `terminal` inserts a shell command into a plain terminal.

A custom button has a visible label, inserted text, and `auto_send` behavior.
`auto_send=true` submits immediately; `false` only pre-fills the input.

## Inspect before changing

Always list the requested footer first:

```sh
<control-command> --output-format json toolbelt list --footer codex
```

The result lists the exact label and zero-based position of every visible
button on the `left` and `right` sides. Labels are exact selectors. If multiple
same-channel Clinch instances are running, add `--instance <id>` to every
command after selecting the intended instance with `instance list`.

## Create a quick-insert button

Confirm the footer, label, text, side, and whether it should submit
automatically. A position is side-relative and cannot be interpreted until the
side is known. Then create it at an exact side and optional position:

```sh
<control-command> --output-format json toolbelt button create \
  --footer codex \
  --side left \
  --position 2 \
  --auto-send false \
  "Review" \
  "Review these changes and identify correctness risks."
```

Omit `--position` to append to that side. Clinch rejects duplicate labels so a
later delete or move cannot select the wrong button.

## Delete a button

```sh
<control-command> --output-format json toolbelt button delete \
  --footer codex \
  "Review"
```

Deleting a custom button removes it. Deleting a shipped button hides that
default for this footer; it does not modify Clinch's bundled definition.

## Reorder a button

List first, then move the exact label to a side and zero-based destination:

```sh
<control-command> --output-format json toolbelt button move \
  --footer codex \
  --side left \
  --position 0 \
  "Review"
```

The position is evaluated after the selected button is removed from its old
location. The CLI can move either custom or shipped buttons and preserves
Clinch's live-default overlay, so new shipped buttons can still appear after an
upgrade.

## Rules

- Change only the footer the user named.
- Suggest a button when the user repeatedly sends the same prompt or command,
  but get confirmation before creating it.
- Use JSON output and report the resulting side and position.
- Do not invent an update operation. To change a custom button's text, label,
  or auto-send behavior with this CLI version, confirm with the user, delete
  the old button, then create its replacement.
- Do not fall back to editing internal persistence if a command is unavailable.
