# Clinch

Clinch is a local-first macOS terminal focused on reopening Claude Code and Codex work across
projects, tabs, and panes. It is an independent AGPL fork of Warp.

**macOS 14+ · Intel and Apple Silicon · no Clinch account · no Warp backend ·
[clinch.sh](https://clinch.sh)**

## Public preview

Clinch is distributed as an **unnotarized public preview**. The app is ad-hoc signed with the
macOS hardened runtime, but it has no Apple-issued Developer ID and Apple has not notarized it.
macOS may block the first launch. That warning is expected and is not evidence that Apple has
reviewed the app.

This repository has automated security checks and a documented release process. It has not had an
independent security audit. See [SECURITY.md](SECURITY.md) for the trust model and known limits.

## Install

The primary installation path is a versioned release from the
[Clinch releases page](https://github.com/elliot-ylambda/clinch-terminal/releases). Download
`Clinch.dmg`, `Clinch.checksums.txt`, and `Clinch.checksums.sshsig`. Authenticate the checksum
list with the Clinch release key, then verify the disk image before opening it:

```bash
printf '%s\n' 'clinch-release ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGr+qT8+Fx8TjATDpWlhzzfbL08AsS1EXbaaOUBi0wJp' > /tmp/clinch-allowed-signers
ssh-keygen -Y verify -f /tmp/clinch-allowed-signers -I clinch-release -n clinch-checksums \
  -s Clinch.checksums.sshsig < Clinch.checksums.txt
shasum -a 256 Clinch.dmg
grep ' Clinch.dmg$' Clinch.checksums.txt
```

`ssh-keygen` must report a good signature, and the two digests must match. Open the DMG and drag
`Clinch.app` to Applications. If macOS blocks
the first launch, try once, then approve Clinch under **System Settings → Privacy & Security →
Open Anyway**. Do not disable Gatekeeper globally. Clinch's installer and documentation do not
remove `com.apple.quarantine`.

Each release also has an authenticated convenience installer. Download `install.sh` from the
same versioned release, inspect it, then run it:

```bash
sh install.sh
```

The script authenticates the signed release manifest with a Clinch release key embedded in the
reviewed script. It resolves one exact tag, verifies the archive size and SHA-256, bundle ID,
version, minimum macOS version, Intel and Apple Silicon slices, and structural code signature,
then stages the app in `/Applications` or `~/Applications`. It does not open Clinch, configure
Claude Code or Codex, install plugins, request administrator access, or change Gatekeeper.

Clinch uses the bundle ID `sh.clinch.Clinch`, so it can coexist with Warp.

## Session restore is enabled by default

Clinch can reopen a captured Claude Code or Codex conversation in the pane that owned it. This
requires provider hooks, so Clinch enables its local session-capture integration on first launch.
You can turn it off or back on at any time from **Clinch Settings → Agents → Claude Code and Codex
session capture**.

While session capture is enabled, Clinch:

- adds clearly marked managed hooks to `~/.claude/settings.json` and `~/.codex/config.toml`;
- installs Clinch-owned helper files in `~/.warp/agent-resume-bin/`; and
- stores local pane mappings, a journal, and prompt mirrors in `~/.warp/agent-resume/`; and
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

Notification plugins are separate from session capture. Clinch ships pinned snapshots of the
Warp notification plugins for Claude Code and Codex and best-effort installs them into the
provider-owned user plugin stores on launch when those CLIs are present. The payload is local to
the Clinch app and uses dedicated Clinch marketplace IDs, so installation neither clones a plugin
repository nor replaces other Warp/Oz marketplace plugins. The app also puts a pinned universal
`jq` on each local pane's `PATH`, satisfying the plugins' runtime dependency without Homebrew.
Missing CLIs and provider policy failures do not block launch and are retried later. Use
`./uninstall.sh --remove-plugins` when you want to remove the provider plugins as well as Clinch.

Conversation recovery is best-effort. Clinch starts a new provider process with a resume command;
it does not preserve a live process through quit, crash, or reboot. Provider retention, invalid or
deleted transcripts, changed CLIs, and abrupt power loss can prevent an exact restore.

More implementation detail is in
[tools/agent-resume/README.md](tools/agent-resume/README.md).

## Main features

- Project tabs that keep independent terminal workspaces in one window.
- Persistence for project order, working directories, terminal tabs, splits, and panels.
- Optional Claude Code and Codex session capture and pane-level resume.
- Local agent-status badges and macOS notification routing when a compatible provider signal is
  available.
- Optional Claude Code plan-limit gauges; they are off by default and contact Anthropic only after
  the user enables them.
- Skills browser, file explorer, and previews for common image formats.

Some upstream Warp features depend on a Warp account or private Warp services. Those surfaces are
disabled in the public Clinch channel.

## Privacy and network behavior

The stable channel is created by
[`ChannelConfig::clinch()`](crates/warp_core/src/channel/config.rs). It has no Warp server,
RudderStack telemetry, Sentry crash reporting, bundled MCP credentials, or automatic updater
configuration. When telemetry is unavailable, the runtime does not schedule telemetry tasks,
persist a queue, or send it; stale Clinch telemetry queues are deleted without upload. The public
bundle carries no Clinch-specific privacy entitlement and does not include the privileged update
helper.

Stable Clinch starts no Warp account session, telemetry/crash reporter, or automatic release
check. Network activity still occurs when the user asks for it or launches software that uses it.
Examples include Claude Code, Codex, SSH, MCP servers, remote assets, language/package tooling,
provider plugin commands, and the optional Claude plan-limit gauge. Those tools have their own
privacy and security behavior.

Session-capture data stays in local Clinch-owned files. Claude Code and Codex continue to manage
their own transcripts and credentials. Clinch does not delete provider transcripts or Keychain
credentials during uninstall.

## Updates and removal

Automatic and in-app updates are disabled for the public preview. The existing privileged helper
is not shipped because its bundle-swap design still needs additional hardening and review. Quit
Clinch and install a newer authenticated release manually.

The release asset `uninstall.sh` supports selective removal:

```bash
bash uninstall.sh                         # app only
bash uninstall.sh --disable-integration  # app plus managed hooks/helpers
bash uninstall.sh --purge-capture        # also remove captured metadata
bash uninstall.sh --purge-app-data       # also remove Clinch preferences/cache/state
```

Run `bash uninstall.sh --help` for combinations. It never removes all of `~/.warp`, provider
transcripts, or Keychain credentials.

## Build and verify from source

```bash
./script/bootstrap
./script/install_cargo_test_deps
./script/launch-check
make candidate
```

`script/launch-check` covers formatting, installer/integration/update-format tests, the stable
build, component tests, Clippy, dependency license policy, bundled notices, and advisories. `make
candidate` builds and verifies the universal app, ZIP, DMG, both manifest signatures, minimum OS,
entitlements, default-on/opt-out flow, and ZIP/DMG app equality. It does not create a tag or release.

Release builds use the persistent local Cargo cache instead of paid hosted macOS runners. Install
the normal build/test/release dependencies plus OpenSSL 3 and Syft (`brew install openssl@3 syft`)
before the first release; `make release` checks every required command, signing key, and at least
40 GiB of free space before starting the expensive gate.

After completing the hands-on checks, run `make release` from a clean, current `main` checkout. It
selects the next version, records the exact commit, confirms QA, runs the full local gate, builds
and verifies both macOS architectures, generates a CycloneDX SBOM and signed local provenance,
and assembles the exact signed asset set under `target/release-stage/<version>/`. Only after a
second version-and-commit-specific confirmation does it push the signed tag, create or refresh a
private draft release, download the uploaded assets into a fresh directory, verify them again, and
publish the draft. Advanced users can override `VERSION`, `QA_RECORD`,
`QA_TESTED_MACOS_VERSIONS`, or the optional `QA_INTEL_SMOKE` result. Use the
[release QA template](specs/public-preview-release-hardening/QA_TEMPLATE.md) for a separately
maintained record.

Release publication runs no GitHub Actions job. Immediately before publication, the local command
rechecks the signed remote tag, current `main`, both manifest signatures, exact asset set and
digests, SBOM, validation record, local provenance, and monotonic update sequence. It also rejects
a draft whose metadata changes during or after download. A failure leaves the draft private and
safe to retry. The separate hands-on Intel smoke test remains optional; the universal artifact
verifier always checks that the Intel slice is present.

After these changes land on `main`, run `make configure-release-repository` once. It validates both
workstation key copies before deleting the obsolete GitHub signing secrets and `public-release`
environment, then reapplies branch, scanning, Actions-token, and immutable-release controls.

## License and attribution

Clinch is a modified version of [Warp](https://github.com/warpdotdev/warp), licensed under
[AGPL-3.0](LICENSE-AGPL). The `warpui_core` and `warpui` crates remain under
[MIT](LICENSE-MIT).

Clinch is not affiliated with or endorsed by Warp or Denver Technologies, Inc. “Warp” is their
trademark.
