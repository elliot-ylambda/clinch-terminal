# Clinch launch feature inventory

This is the homepage-ready inventory for the first public Clinch launch. It summarizes the
user-visible work added across the Clinch fork and separates shipped behavior from optional or
external dependencies.

## Recommended launch announcement

**Clinch is ready for macOS.**

Clinch is a local-first terminal built around Claude Code and Codex. Close the app with agents
running, reopen it, and every project, tab, pane, working directory, and resumable agent session
comes back where you left it. There is no account, no onboarding, no Warp backend, and no
separate agent-resume setup.

Install it with one command:

```bash
curl -fsSL https://clinch.sh/install | sh
```

The installer verifies the release checksum and app signature, installs Clinch, configures its
Claude/Codex capture hooks without replacing unrelated settings, and opens the app. Agent resume
requires no repository clone, Homebrew package, `jq`, shell-rc edit, or shell restart.

## Homepage hero

**Eyebrow:** `LOCAL-FIRST AGENT TERMINAL FOR macOS`

**Headline:** Close the window. Keep the work.

**Subhead:** Clinch restores your projects, panes, and Claude Code or Codex sessions exactly where
you left them—then shows which agent needs you, what it is running, and how much context it has
used.

**Primary CTA:** Install Clinch

**Secondary CTA:** View source

**Proof line:** Open source · no account · no Warp backend · local session history

## Homepage feature cards

### Resume every agent session

Clinch records the real Claude Code or Codex session running in each pane. Reopen the app—or undo
a closed tab—and Clinch resumes that exact conversation with its working directory, model, and
permission mode intact.

### Keep multiple projects in one window

Project tabs hold complete, independent workspaces: inner tabs, split panes, panels, repositories,
and live agents. Switch instantly, drag to reorder or move a project between windows, and restore
the same arrangement after a restart.

### Know when an agent needs you

Unfocused tabs show an awaiting-you indicator when an agent finishes or asks a question. Project
tabs roll that unread state up to the project level, and macOS notifications take you back to the
originating pane.

### See models and usage at a glance

Running-agent tabs show a friendly model label. The tab header summarizes locally scanned Claude
Code and Codex tokens and rate-limit windows, with a detailed breakdown on click.

### Act without breaking flow

Use Continue and LGTM replies, add persistent custom quick-insert buttons from discovered slash
commands, compact a long Claude conversation, or fork a Claude/Codex session into a new tab.

### Browse the skills available to your agents

The Skills panel groups available skills by scope and provider, filters them for Claude or Codex,
refreshes as repository context changes, and opens the source definition for inspection or edits.

### Preview images without leaving the terminal

Open SVG, PNG, JPEG, GIF, or WebP files in an in-app preview pane from file links and supported
workspace surfaces.

### Local-first by construction

Clinch ships without sign-in, onboarding, telemetry, crash reporting, or a Warp backend. Its app
identity and data directory are separate from Warp, so both applications can be installed at the
same time.

## Complete change and feature list

### Agent continuity and recovery

- Per-pane Claude Code and Codex session capture keyed by Clinch's stable pane UUID.
- Restore-time replay after the shell is fully bootstrapped.
- Resume on app relaunch and when reopening an accidentally closed tab.
- Correct capture for fresh starts, explicit resume, interactive session pickers, and Claude
  `--continue`.
- Preservation of Claude permission modes and model overrides, including mode changes recorded on
  later prompt/stop events.
- Preservation of Codex bypass-approval mode and model selection from its session payload.
- Claude cloud-bridge recovery through `--teleport`, with fast-failure fallback to a local
  transcript and a recoverable cloud URL.
- Safe fallback for missing/stub Claude sessions: adopt the newest unclaimed conversation for the
  same working directory before starting fresh.
- Blank-session protection so an automatic fresh fallback cannot overwrite the only recoverable
  mapping to an existing local or bridged conversation.
- Restore-time compatibility for registry entries written before the Clinch launcher rename.
- A Fork control that opens a forked Claude or Codex session in a new tab with its launch flags and
  working directory preserved.
- A Compact control for reducing long Claude session context from the agent footer.
- Append-only local registry journal for historical pane/session/working-directory/bridge mappings.
- Per-session local prompt mirror, protected with user-only filesystem permissions and capped to
  prevent unbounded growth.
- Pre-update recovery snapshots of registry data and locally available referenced transcripts.
- `clinch-agent-resume list [--cwd <dir>]` conversation discovery across journal and prompt data.
- Dynamic removal of inherited `CLAUDE_CODE_*`, `CLAUDECODE`, and `AI_AGENT` identity at the new-pane
  boundary so a Clinch launch from inside an agent cannot poison later transcripts.

### Agent attention and controls

- Claude Code and Codex detection in fresh and restored panes.
- Awaiting-you badges for completed turns and questions in unfocused tabs.
- Project-level unread dots for activity inside inactive project workspaces.
- macOS desktop notifications and navigation back to the originating project, tab, and pane after
  notification permission and the provider notification plugin are enabled.
- Friendly running-model chips in agent tabs.
- Responsive header usage status with a detailed panel.
- Incremental local scanning for Claude Code and Codex input, output, cached, and reasoning tokens.
- Per-model token and session/provider breakdowns.
- Codex local rate-limit windows.
- Optional Claude five-hour and weekly plan gauges. This setting is off by default because enabling
  it asks macOS for Keychain access and queries Anthropic's usage endpoint.
- Larger agent input controls and a cleaner default footer.
- Built-in Continue and LGTM quick replies.
- Persistent user-defined quick-insert buttons.
- Slash-command discovery for creating quick inserts.
- Insert-only and insert-and-send actions.

### Projects, tabs, and window organization

- Fixed vertical inner tabs as the default Clinch layout.
- A separate horizontal project-tab strip for complete workspaces within one physical window.
- Independent live inner tabs, panes, panels, focus, scroll state, inputs, repositories, and agent
  processes per project.
- Instant switching without recreating inactive workspaces.
- `Command+N` for New Project plus a separate explicit New Window action.
- Wrapping previous/next project shortcuts with customizable bindings.
- Project-tab reordering, live detachment to a new window, and attachment to another compatible
  Clinch window.
- Project close behavior that reuses running-session confirmation and closes the physical window
  only when its final project closes.
- Repository-derived project labels with a `New Project` fallback.
- Persistence of physical windows, project order, active project, and every contained workspace;
  legacy single-workspace windows migrate to one-project windows.
- Restore of every project represented in launch configuration state.
- Repository name above the vertical tab list.
- Repository-derived window-header tint that follows the active project.
- Directory-based tab and window colors.
- File Explorer toggle in the header.
- Inner-tab drag between windows and Move Tab to New Window action.
- Agent-aware undo-close that resumes the recovered session.
- Guarded project and window close behavior after the final inner tab exits.

### Skills, files, and media

- Skills panel in the left sidebar, enabled in the stable default build.
- All/Claude/Codex subtabs with scope grouping and reachable-provider filtering.
- Live skill-list refresh as working-directory/repository context changes.
- Open-skill-source action for inspecting or editing a skill definition.
- Bundled skills and skill arguments in the stable build.
- In-app image preview pane enabled in the stable default build.
- SVG, PNG, JPEG, GIF, and WebP routing to the preview pane.
- Existing editor, source tree, and file-link flows continue to handle code and text files.

### Local-first product and branding

- Dedicated `sh.clinch.Clinch` app identity, data domain, URL scheme, app icon, menus, About view,
  permission strings, help links, feedback links, issue templates, and security/support contacts.
- No-backend stable channel that can build without Warp's private channel-config generator.
- No account creation, login, onboarding, signup re-entry, or backend-only settings surfaces.
- Warp AI, Drive, teams, and sharing surfaces hidden or disabled when they require the removed
  backend.
- No telemetry configuration, analytics destination, Sentry crash reporting, or autoupdate backend
  in the shipped stable binary.
- Dark appearance and vertical tabs as fresh-profile defaults.
- Co-installation with Warp without sharing application state.
- AGPL attribution and independent-fork disclosure throughout the release surface.

### Install, release, and supply-chain hardening

- One-command HTTPS installer with a safe `main` boundary for truncated-pipe protection.
- Mandatory download of the release checksum sidecar and fail-closed SHA-256 verification.
- Bundle-identifier and complete code-signature verification before installation.
- One universal macOS artifact for Apple Silicon and Intel Macs.
- Bundled agent-resume runtime copied into every release app.
- Automatic, idempotent Claude/Codex hook setup before the first GUI pane opens.
- Native macOS JSON handling through JXA; no runtime `jq`, Python, or Homebrew dependency.
- Structural Claude settings merge that preserves unrelated settings and hooks.
- Managed Codex configuration block replacement that preserves unrelated TOML and does not
  duplicate on repeated launches.
- Standalone replay executables on Clinch's injected PATH; no `.zshrc` edit or restart.
- Best-effort notification plugin installation from the curl installer and one-click management in
  the app when provider CLIs are available.
- Stable release version stamping, zip + checksum assets, headless DMG creation, and protection
  against releasing a stale branch or changing the version mid-build.
- Clinch-only GitHub CI on public macOS runners; private Warp runners, GCP jobs, repository sync,
  and scheduled upstream release jobs removed.
- Release source gate for formatting, shell runtime tests, stable build checks, shipped-component
  tests, Clippy, and dependency advisories.
- Pre-publish artifact gate for app identity/version, code signature, entitlements, bundled runtime,
  checksum, clean-account install, zip round-trip, and DMG integrity.
- `git2` and `plist` dependency upgrades that remove four 2026 advisories from both shipped macOS
  dependency graphs plus the unmaintained `safemem` dependency.

## FAQ copy

### Does Clinch need an account?

No. Clinch starts directly in a local terminal. Login, account creation, onboarding, and Warp's
backend-dependent surfaces are disabled in the stable build.

### Do I have to install agent hooks separately?

No. The app carries the capture runtime and configures Claude Code and Codex idempotently on launch.
The curl installer also does this before opening Clinch. Existing unrelated agent settings are
preserved.

### Does Clinch send telemetry?

The stable build has no telemetry destination, crash-reporting configuration, or Warp backend.
Claude Code, Codex, MCP servers, and any commands you run still contact their own configured
services. The optional Claude plan-limit gauges contact Anthropic only after you enable them.

### Is my conversation content uploaded by Clinch?

No. Clinch reads local agent metadata and transcripts to restore sessions and calculate local usage.
Its recovery journal, prompt mirror, and update snapshots stay on your Mac. Provider agents retain
their own normal network behavior; a Claude session you explicitly bridge is restored from its
Claude cloud copy.

### How do updates work?

Clinch does not run a background updater. Re-run the install command to fetch, verify, replace, and
open the latest release. Local recovery data and agent configuration remain in place.

### Is the macOS build notarized?

Not yet. The current public flow produces a code-signed, checksum-verified build, but it does not
carry an Apple notarization ticket. The curl installer avoids browser quarantine; manual DMG/zip
downloads may require approval in System Settings or removal of the quarantine attribute. Do not
claim “notarized” or “no Gatekeeper prompt” until the Developer ID/notary gate is enabled and passes.

### What happens after a crash or power loss?

Agent conversations remain in their provider's local transcript store, but exact window/pane
restoration is best-effort between persistence points. Graceful quit, reopen, update, and undo-close
are the launch guarantees; do not market Clinch as a live process checkpoint or crash-proof VM.

## Claims to avoid on the homepage

- Do not say “notarized” until `REQUIRE_NOTARIZATION=1` passes for the published artifacts.
- Do not say “automatic updates”; updates are an explicit reinstall today.
- Do not say “zero network traffic.” Clinch has no Warp backend/telemetry, but hosted agent tools,
  MCP servers, optional Claude plan gauges, plugin installation, and some user-selected assets can
  use the network.
- Do not imply agent processes survive a reboot. Clinch restores resumable conversations in new
  processes.
- Do not promise desktop notifications before macOS permission and the provider notification plugin
  are available.
- Do not advertise Linux or Windows. This release is macOS-only.

## Internal launch verification

Candidate `v0.2026.07.11.2319` was built as a universal Apple Silicon + Intel app and verified
without publishing it:

- 5,550 shipped-component Rust tests passed; 7 additional tests were skipped by their own gates.
- All 11 agent capture, install, durability, replay, and notification-plugin shell tests passed.
- Stable configuration checks and shipped-target Clippy passed with warnings denied.
- Both macOS dependency graphs passed the advisory gate. Two transitive font-stack crates carry
  documented unmaintained-only exceptions because their advisories provide no safe upgrade; no
  known vulnerability is being waived.
- Both the arm64 and x86_64 executable slices loaded and returned identical CLI smoke-test output.
- App/zip bundle identity, version, signature, entitlements, universal architectures, bundled
  first-run runtime, clean-account setup, SHA-256, zip round-trip, and DMG integrity passed.
- Final zip SHA-256: `698f1e3e0a4f9c40f87173f425fcad759a1ae2350aa9df3f7acdf706364e3efb`.

Publishing the GitHub release and updating the live website remain separate external actions. The
candidate is code-signed but cannot be called notarized until a Developer ID Application identity
and Apple notary credentials are available and the `REQUIRE_NOTARIZATION=1` gate passes.
