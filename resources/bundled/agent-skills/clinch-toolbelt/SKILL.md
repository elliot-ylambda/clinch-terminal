---
name: clinch-toolbelt
description: Creates, deletes, lists, or reorders Clinch quick-insert buttons and proactively suggests locally learned reusable prompts at the beginning of a new Claude Code or Codex conversation. Use for every new agent conversation in Clinch, when the user asks for a button, footer, or toolbelt change, or when reusable text emerges in the visible conversation.
---

<!-- managed-by: Clinch; version: 3.0.0 -->

# Clinch toolbelt buttons

Use Clinch's typed local-control CLI. Never edit `settings.toml`, SQLite, or
other Clinch persistence directly for a toolbelt request.

Current Clinch versions inject `CLINCH_CONTROL_COMMAND`, the exact
`CLINCH_CONTROL_WRAPPER` path, and `CLINCH_CONTROL_PID` into every local host
terminal shell. When the wrapper is executable and the PID is present, use
the wrapper as `<control-command>` and add `--pid "$CLINCH_CONTROL_PID"` to
every toolbelt command. The wrapper takes precedence over `PATH`; the PID
prevents one local worktree build from controlling another. If either binding
is incomplete, stop and ask the user to relaunch or rebuild that Clinch app.
Never fall back to another channel or instance. In the examples,
`<control-command>` means the quoted `"$CLINCH_CONTROL_WRAPPER"` value in a
bound terminal.

Before the first toolbelt action, run `<control-command> --output-format json
app ping --pid "$CLINCH_CONTROL_PID"`. New Clinch installs enable local control
by default. If a bound terminal returns `no_instance`, retry once, then report
that local control is unavailable or may have been explicitly disabled. Point
to **Settings > Local control** only as recovery, and do not re-enable a setting
the user deliberately disabled. No global command installation is required.

If `/.dockerenv` exists and the binding is incomplete, the isolated container
cannot control the host app. Tell the user to make the request from a host
Clinch terminal instead of diagnosing the container as an outdated install.

If the wrapper is absent and `WARP_FOCUS_URL` begins with `clinch://` or
`clinchdev://`, the session came from an older version that predates
current-app binding. Tell the user to update Clinch and start a new
terminal/agent session. `TERM_PROGRAM=WarpTerminal` alone does not identify
Clinch. Outside Clinch, check `warpctrl` and `warpctrl-local` on `PATH`, run
`instance list`, and proceed only when exactly one live channel is found.
Never use an absolute bundle path persisted in this user-scope skill or edit a
`/usr/local/bin` symlink without explicit approval.

## Footer behavior

Each footer is independent:

- `claude-code` inserts a prompt into Claude Code.
- `codex` inserts a prompt into Codex.
- `terminal` inserts a shell command into a plain terminal.

A custom button has a visible label, inserted text, and `auto_send` behavior.
`auto_send=true` submits immediately; `false` only pre-fills the input.

## Check local cross-conversation suggestions

At the first appropriate turn of every new Claude Code or Codex conversation
inside Clinch, perform the normal ping and then one quiet check for the active
provider footer:

```sh
<control-command> --output-format json toolbelt suggestion list \
  --pid "$CLINCH_CONTROL_PID" --footer codex
```

Use `claude-code` when running in Claude Code. If `suggestions` is empty,
continue without mentioning the check. If there is a candidate, offer only one
concrete button proposal: a short generated label, the returned exact `text`,
the active footer, `side: left`, and `auto_send: false`. Say that Clinch noticed
the text recurring locally, but do not identify, reconstruct, or claim access
to the conversations that produced it.

One affirmative answer authorizes that exact proposal. List the target footer,
resolve any label conflict, create the button, and only after creation succeeds
record the candidate as accepted:

```sh
<control-command> --output-format json toolbelt suggestion resolve \
  --pid "$CLINCH_CONTROL_PID" \
  --outcome accepted \
  "<suggestion-id>"
```

If the user declines, record `--outcome declined` so Clinch does not offer the
candidate in later conversations. If the user ignores the proposal or asks to
change it, do not resolve it until they clearly accept or decline the final
proposal. Never resolve accepted before the typed button-create action succeeds.

The suggestion command exposes eligible aggregate candidates, not transcript
history. Learning is local and user-scoped, requires the same normalized text
in at least two distinct captured conversations, and stops when Claude Code and
Codex session capture is disabled. Never inspect prompt-mirror files directly.

## Learn from the visible conversation

Proactively notice a repeated or near-identical user prompt or command, or a
statement that the user uses some text often. Do not wait for an explicit
button request. Suggest one concrete quick-insert configuration: footer, label,
exact inserted text, side, and `auto_send` behavior. Prefer the active Claude
Code or Codex footer; use `terminal` only for a reusable shell command.

Ask whether to add the proposed button. One affirmative answer authorizes that
exact proposal: list the footer, resolve any label or position conflict, and
create it without asking the user to repeat already-confirmed fields. If the
user changes a field, use the correction. If they decline, do not suggest the
same pattern again in the current conversation.

Do not suggest secrets, credentials, personal data, destructive commands, or
text whose paths, IDs, or other values make it useful only once. This in-context
path uses only conversation content already visible to the coding agent. The
CLI executes the confirmed typed mutation; it does not provide general access
to hidden or past conversations.

## Inspect before changing

Always list the requested footer first:

```sh
<control-command> --output-format json toolbelt list \
  --pid "$CLINCH_CONTROL_PID" --footer codex
```

The result lists the exact label and zero-based position of every visible
button on the `left` and `right` sides. Labels are exact selectors. Outside a
bound Clinch terminal, if multiple same-channel Clinch instances are running,
add `--instance <id>` to every command after selecting the intended instance
with `instance list`.

## Create a quick-insert button

Confirm the footer, label, text, side, and whether it should submit
automatically. A position is side-relative and cannot be interpreted until the
side is known. Then create it at an exact side and optional position:

```sh
<control-command> --output-format json toolbelt button create \
  --pid "$CLINCH_CONTROL_PID" \
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
  --pid "$CLINCH_CONTROL_PID" \
  --footer codex \
  "Review"
```

Deleting a custom button removes it. Deleting a shipped button hides that
default for this footer; it does not modify Clinch's bundled definition.

## Reorder a button

List first, then move the exact label to a side and zero-based destination:

```sh
<control-command> --output-format json toolbelt button move \
  --pid "$CLINCH_CONTROL_PID" \
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
- Follow the conversation-learning flow even when the user did not explicitly
  ask about toolbelt buttons.
- Check learned suggestions only once per conversation and never nag after a
  decline.
- Use JSON output and report the resulting side and position.
- Do not invent an update operation. To change a custom button's text, label,
  or auto-send behavior with this CLI version, confirm with the user, delete
  the old button, then create its replacement.
- Do not fall back to editing internal persistence if a command is unavailable.
