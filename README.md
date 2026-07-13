# Clinch

> A local-first macOS terminal that restores the Claude Code and Codex sessions running in
> every project, tab, and pane.

**macOS · open source · no account · no Warp backend · [clinch.sh](https://clinch.sh)**

Close Clinch with agents running. Reopen it, and the same project layout, working directories,
panes, and resumable conversations return where you left them.

## Install

```bash
curl -fsSL https://clinch.sh/install | sh
```

The installer:

1. downloads the latest `Clinch.app.zip` and its published checksum;
2. refuses a SHA-256 mismatch or invalid app signature;
3. installs Clinch to `/Applications` or `~/Applications`;
4. configures Claude Code and Codex session capture while preserving unrelated settings; and
5. opens Clinch.

Agent resume is built into the app. It requires no repository clone, Homebrew package, `jq`,
`.zshrc` edit, or shell restart.

For a manual install, download
[`Clinch.app.zip`](https://github.com/elliot-ylambda/clinch-terminal/releases/latest/download/Clinch.app.zip)
and
[`Clinch.app.zip.sha256`](https://github.com/elliot-ylambda/clinch-terminal/releases/latest/download/Clinch.app.zip.sha256),
then verify them:

```bash
shasum -a 256 -c Clinch.app.zip.sha256
```

Move `Clinch.app` to `/Applications`. The current public build is code-signed but not Apple
notarized, so a browser-downloaded copy may need approval under **System Settings → Privacy &
Security → Open Anyway**, or:

```bash
xattr -dr com.apple.quarantine /Applications/Clinch.app
```

Clinch uses its own bundle ID and data domain, so it can be installed alongside Warp.

## What Clinch adds

### Agent continuity

- Restores the exact Claude Code or Codex session captured in each pane.
- Handles fresh sessions, explicit resume, Claude's picker and `--continue`, and reopened closed
  tabs.
- Preserves working directory, model, and permission/bypass mode.
- Teleports a bridged Claude cloud conversation, with local and fresh-session fallbacks.
- Protects recoverable mappings from blank-session overwrite and adopts a safe, unclaimed local
  Claude conversation when a registry ID goes stale.
- Forks a Claude/Codex session into a new tab and compacts long Claude context from the footer.
- Keeps a local append-only session journal, prompt mirror, update recovery snapshots, and a
  conversation discovery command.

### Agent status and controls

- Shows awaiting-you badges when an unfocused agent finishes or asks a question.
- Routes enabled macOS notifications back to the originating project, tab, and pane.
- Shows the running model in agent tabs.
- Summarizes locally scanned token usage and rate-limit windows in the tab header.
- Keeps Claude plan-limit network/Keychain access off until the user explicitly enables it.
- Includes Continue and LGTM replies plus persistent custom quick inserts built from discovered
  slash commands.

### Project workspaces

- Groups complete, independent workspaces into project tabs within one physical window.
- Keeps inactive projects live, including terminals and agents.
- Persists project order, active project, inner tabs, split panes, panels, and window grouping.
- Supports New Project, keyboard cycling, close guards, drag reorder, detach, and attachment to
  another window.
- Rolls unread agent activity up to a project dot and follows the active project's repository title
  and header tint.
- Keeps inner tabs vertical, shows the active repository above them, supports tab tear-off, and
  resumes an agent when undoing a closed tab.

### Skills, files, and media

- Skills panel with All, Claude, and Codex filters, scope grouping, live repository-context refresh,
  and source-file opening.
- File Explorer toggle in the window header.
- In-app previews for SVG, PNG, JPEG, GIF, and WebP files.

The complete homepage-ready inventory and launch FAQ are in
[`docs/LAUNCH_FEATURES.md`](docs/LAUNCH_FEATURES.md).

## Privacy and network behavior

Clinch's shipped `stable` binary uses
[`ChannelConfig::no_backend()`](crates/warp_core/src/channel/config.rs). It has no login flow,
Warp backend, telemetry destination, crash-reporting configuration, or autoupdate backend. Sentry
is not compiled into the release binary.

Clinch stores session recovery data locally:

- `~/.warp/agent-resume/` — current pane mappings, append-only journal, and prompt mirrors;
- `~/Library/Application Support/sh.clinch.Clinch/` — Clinch app state and bounded pre-update
  recovery snapshots; and
- the normal Claude Code and Codex transcript locations managed by those tools.

Clinch does not upload that content. The software you run inside the terminal still has its own
network behavior: Claude Code contacts Anthropic, Codex contacts OpenAI, MCP servers contact their
configured services, and a bridged Claude session uses its Claude cloud copy. If enabled, the
optional Claude plan gauges read the Claude Code credential from macOS Keychain with system
permission and query Anthropic's usage endpoint.

Clinch does not claim literally zero possible network requests: plugin installation, provider tools,
MCP servers, optional plan gauges, and user-selected remote assets can use the network. The stable
app has no Warp telemetry or authenticated Warp backend path.

## Agent-resume implementation

Capture hooks read the actual provider session ID after the provider has chosen it and write a
per-pane registry entry. Only the outermost Claude/Codex process may own the pane; nested agents
retain prompt history without replacing that entry. Clinch publishes the active pane set and freezes
the newest registry state into a final full window/project/tab snapshot on shutdown. On restore it
reconciles the live registry and explicit exit tombstones with SQLite before executing the bundled
replay launcher after the shell bootstraps.

The public app runs its bundled installer idempotently before the first GUI pane can open. The JSON
merge/parser uses macOS's built-in JavaScript for Automation runtime, and the replay executable is
available through Clinch's bundled `Resources/bin` path. See
[`tools/agent-resume/README.md`](tools/agent-resume/README.md) for durability details and known
limitations.

## Update

Updater-enabled releases check authenticated GitHub release metadata once per active day. No app
archive is downloaded until the user chooses **Clinch → Check for Updates…** (or the equivalent
Settings action), reviews the release, and approves **Download and Install**. Clinch then verifies
the signed manifest, archive hash, bundle identity/version/architecture, and complete code
signature before saving its final recovery snapshot and relaunching through the rollback-capable
external helper.

Builds installed before the in-app updater require one final manual update. Quit Clinch and run:

```bash
curl -fsSL https://clinch.sh/install | sh
```

The installer replaces the app bundle without deleting local application or recovery data. It
checks LaunchServices plus the exact executable path and refuses replacement while Clinch is still
running, so it cannot swap a bundle out from under a live app. This remains the bootstrap and
manual-recovery path after in-app updates are available.

## Build and verify from source

```bash
./script/bootstrap
./script/install_cargo_test_deps
./script/launch-check
make candidate SKIP_SYNC=1
```

`script/launch-check` runs formatting, agent/update-runtime tests, the stable build check,
shipped-component tests, Clippy, and the dependency advisory gate. `make candidate` builds the
app/zip/DMG and signed update manifest without publishing, then checks bundle identity/version,
release sequence, signatures, entitlements, bundled setup, checksum, zip round-trip, and DMG
integrity.

To install a developer build directly:

```bash
./tools/agent-resume/build-app.sh
```

## Platform and recovery limits

- This release is macOS-only and ships as one universal Apple Silicon + Intel build.
- Agent conversations resume in new processes; live processes do not survive app exit or reboot.
- Graceful quit, update, undo-close, and normal relaunch are the intended recovery paths. Exact UI
  state after a crash or power loss is best-effort between persistence points.
- Desktop notifications require macOS permission and a compatible provider notification plugin.
- The current public flow is not Apple notarized and does not provide background automatic updates.

## License and attribution

Clinch is a modified version of [Warp](https://github.com/warpdotdev/warp), licensed under
[AGPL-3.0](LICENSE-AGPL). The `warpui_core` and `warpui` crates remain under
[MIT](LICENSE-MIT).

Clinch is an independent, unofficial fork. It is not affiliated with or endorsed by Warp or Denver
Technologies, Inc. “Warp” is their trademark.
