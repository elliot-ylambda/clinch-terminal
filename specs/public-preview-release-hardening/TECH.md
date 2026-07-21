# Clinch public-preview release hardening: technical design

## Context

This design applies to the Clinch fork at source revision
`bcf40defc05b0fb66f35214c75c2227696e1433d`. It deliberately does not change
upstream Warp behavior. The public artifact remains an unnotarized macOS preview because the
project will not enroll in the Apple Developer Program.

The current path has five material release risks:

- `install.sh` trusts mutable release locations and a sibling checksum, clears quarantine, then
  installs hooks and plugins as side effects.
- `app/src/lib.rs` repairs agent hooks on every stable launch without a persisted user-controlled
  enabled/disabled state.
- Clinch has no telemetry destination, but the collector can still schedule work and persist its
  queue because shared Warp privacy defaults are true.
- the self-signed bundle inherits unused privacy and development entitlements.
- local Make targets can publish a release before the complete CI suite finishes, while the former
  privileged in-app update path has root symlink and interrupted-swap failure modes.

## Design

### 1. Release identities and authenticated metadata

Keep two narrowly scoped Ed25519 keys outside the repository:

- the existing OpenSSL key signs `Clinch.update.json` for the dormant in-app updater format;
- a new OpenSSH Ed25519 release key signs Git tags and an SSHSIG envelope over that same manifest.

Commit only an `allowed_signers` public-key record. The installer verifies the SSHSIG with the
stock macOS `ssh-keygen -Y verify` command, identity `clinch-release`, and namespace
`clinch-install` before it parses any release-controlled field. This avoids requiring Homebrew or
OpenSSL 3 for bootstrap verification.

The manifest remains the single authenticated description of the ZIP. It must name the exact
repository, tag, version, bundle identifier, archive URL, archive filename, byte length, SHA-256,
minimum macOS version, architectures, update sequence, and signing-key identifier. Release assets
also include the SSHSIG, release-key-signed human-readable checksum list, DMG, installer,
uninstaller, SBOM, validation record, and release-key-signed local build provenance.

`install.sh` resolves `latest` once through the GitHub API, or accepts a strict `--version` value,
then fetches only exact-tag assets. It size-limits metadata, authenticates it before parsing, and
validates every relevant manifest field. The archive is streamed to a private temporary directory,
checked for exact size and digest, expanded without following a user-selected destination, and
checked for bundle identity, version, executable, universal architectures, minimum OS, and a valid
structural code signature. It never changes quarantine metadata.

Installation first stages and validates a complete sibling bundle. Replacement uses same-volume
renames with a backup and restores that backup on every handled failure. The installer changes no
integration, plugin, preference, or running-app state.

### 2. Default-on, reversible session capture

Refactor `tools/agent-resume/install.sh` into explicit `enable`, `disable`, `status`, and `purge`
commands. With no command it prints help and performs no mutation. Enabling explains and then:

- installs Clinch-owned runtime files with restrictive modes;
- structurally adds only the managed Claude hooks and Codex notify block;
- writes a versioned enabled-state marker and receipt after successful configuration.

The receipt records ownership plus pre/post hashes and modes without copying provider transcripts,
configuration contents, or secrets. Disable performs a structural removal of the exact managed
entries; the receipt is an audit record, not a backup. On first Clinch launch with no prior state,
`app/src/agent_resume.rs` runs the bundled enable command. Later launches repair managed entries
only while the enabled marker exists. A durable disabled marker prevents startup mutation, and a
retained receipt without an enabled marker migrates older explicit disables to the same opt-out
semantics. Invalid third-party JSON or malformed Codex managed markers are reported without
preventing Clinch launch or rewriting unrelated configuration.

Disabling atomically records the opt-out, removes the enabled marker, then removes only exact
managed entries and owned runtime files. It performs a structural removal and preserves every
unrelated key or line; captured metadata is retained by default. `purge` is a separate explicit
operation for Clinch-owned capture data and retains the opt-out.

Add setup, remove, and status controls to the Clinch settings page. Session capture remains
user-controlled and enabled by default, while `app/src/agent_plugins.rs` runs the bundled
`resources/bundled/agent-plugins/install.sh` before the first pane launches. The script uses exact
local Claude Code and Codex marketplace snapshots, skips already-current installs, and treats a
missing CLI or provider-policy failure as non-fatal. Dedicated Clinch marketplace IDs keep the
local bundle from replacing upstream Warp marketplaces or their Oz plugins. A pinned universal jq
binary is copied to `Contents/Resources/bin`, already part of every local pane's path, so the exact
upstream hooks run without a system package install. Existing footer auto-install paths remain
disabled for backend-free Clinch so they cannot replace the local bundle with floating Git
sources; manual source-identified instructions remain the fallback.

### 3. Privacy and startup behavior

Treat channel capability as authoritative in `app/src/settings/privacy.rs`: when the active
channel lacks telemetry or crash configuration, its effective values are false regardless of a
stored Warp default, enterprise override, or experiment. Do not persist a false value back into
shared Warp settings merely because Clinch lacks the capability.

The telemetry collector checks availability before it creates timers or subscriptions. In the
unavailable path it drains memory and deletes only Clinch's current and legacy telemetry queue
files. Every persistence and send entry point also checks availability, so shutdown cannot recreate
the queue and macros avoid constructing unused events.

Configure Clinch's `autoupdate_config` for the pinned signed GitHub release provider. Automatic
checks fetch metadata only and occur at most once per active calendar day; a persistent header
indicator and the macOS **Check for Updates…** action require explicit install consent. Bundle only
the unprivileged helper and atomic-swap utility. The AppleScript administrator-authorization path
remains excluded, and non-writable installations fall back to the authenticated manual installer.
Release verification requires the unprivileged components and rejects the authorization shim.

### 4. Bundle permissions and platform contract

Use a Clinch-only release entitlement plist selected by the Clinch bundle identifier, not by the
shared stable channel. The current release plist is empty because no shipped Clinch capability
needs a privacy entitlement. Keep hardened-runtime code signing but use deterministic ad-hoc
signing for the public preview unless an explicit compatible identity is provided. Do not request
app sandboxing or microphone, camera, contacts, calendar, location, Photos, app-group, Apple
Events, JIT, library-validation bypass, or debugger entitlements.

`script/update_plist` omits unrelated privacy usage descriptions and writes an explicit minimum
macOS version. Release verification rejects forbidden entitlements or dead usage keys, confirms
both `arm64` and `x86_64` app slices, and rejects the disabled messaging helper and its resource
bundles if any are present.

### 5. Local release gate and direct publication

Move every expensive or secret-bearing release operation to the release workstation. The local
`make release` command owns version selection, exact-source validation, the complete automated
gate, universal packaging, SBOM generation, signing, artifact verification, private draft staging,
independent verification of the uploaded draft, and the final draft-to-public transition. Release
publication runs no GitHub Actions job.

The local flow:

1. synchronizes a clean `main` with `clinch/main`, records its full commit, validates a strict
   monotonic version, and checks release tools, free disk space, and both private keys;
2. runs formatting, shell/JavaScript tests, stable compilation, component tests, Clippy,
   dependency-license policy, bundled-notice generation, and advisories before creating remote
   release state;
3. derives a sequence strictly newer than the latest authenticated public manifest, builds both
   architectures with the persistent local Cargo cache, packages them, and runs the complete
   app/ZIP/DMG verifier;
4. generates a CycloneDX SBOM, signed validation record, and an in-toto/SLSA-shaped local build
   provenance statement whose subjects are the staged release assets and whose source material is
   the exact Git commit and `Cargo.lock` digest;
5. signs the provenance and the complete checksum list with the dedicated OpenSSH release key,
   then independently verifies every staged digest and signature;
6. requires an interactive `PUBLISH <version> <short-commit>` confirmation, creates or
   verifies the signed annotated tag, pushes it to `clinch`, creates or refreshes a private draft
   release, uploads the exact verified asset set, downloads it into a fresh temporary directory,
   repeats the portable asset and monotonic-sequence verification, rechecks the remote tag and
   `main`, and changes only that verified draft to the public latest release; and
7. leaves a correct existing tag or draft reusable after interruption, but refuses a mismatched
   tag, a published release, a non-draft staging release, or an unsigned/extra/missing asset.

`target/release-stage/<version>/` is the local staging boundary. It contains a `dist` directory
with the exact public asset set and a sibling release-notes file. Generated state remains ignored
by Git and may be recreated only after the source gate and candidate verification succeed.

The local publication phase downloads the private draft assets rather than trusting the upload
operation. It requires the current remote `main` and signed remote tag to remain identical to the
verified local commit and tag, snapshots release/asset metadata before and after download, verifies
the exact asset allowlist, committed trust roots, both manifest signatures, checksum signature and
digests, provenance signature and subjects, release validation record, version, and monotonic sequence,
and snapshots the draft once more immediately before publication. A failure leaves the draft
private. The operator's existing `gh` authentication is the only publication credential.

GitHub provenance is intentionally absent because GitHub does not build the artifacts. The
dedicated release key authenticates a local provenance statement that accurately names the
workstation build type, source material, invocation, and artifact subjects. The checksum signature
also covers the provenance statement and signature.

The dependency license gate includes workspace and non-publishable git dependencies, keeps the
`cargo-deny` and `cargo-about` allowlists synchronized, and fails packaging if a complete bundled
third-party notice cannot be generated from the locked dependency graph.

The signed release validation asset records the automated gate, artifact checks, exact commit, and
local builder OS and architecture. It deliberately omits manual-QA results because the release
command neither requires nor performs hands-on QA.

`make release` is the interactive operator front end for the local build and direct publish. It
first synchronizes a clean local `main` with `clinch/main`, compares its timestamp-derived version
with the latest public release, and increments the latest version when necessary. It proceeds
directly into the automated gate and candidate verification without a manual-QA prompt or GitHub
QA issue. Remote staging and publication still require the interactive version-and-commit-specific
publish confirmation. The command stops before publication if `main`, the signed tag, or the
private draft changes between local verification and publication.

There is no release workflow. Release private keys are removed from the obsolete GitHub release
environment, and the environment itself is deleted after the local key copies are validated. The
repository configuration script enables immutable releases, protects `main` against force pushes
and deletion, restricts default Actions permissions, and leaves secret scanning/push protection
enabled.

The monotonic sequence helper reads the latest signed manifest, verifies it, and computes
`max(previous + 1, current UTC epoch seconds)`. Publication refuses a sequence provided by a local
clock or caller that is not strictly greater.

### 6. Removal and documentation

Ship a top-level `uninstall.sh` release asset. By default it removes the selected app bundle and
offers an explicit integration-disable operation; flags separately remove Clinch preferences/cache,
captured metadata, and known provider plugins. It never deletes `~/.warp`, provider transcripts,
or Keychain credentials.

`README.md`, `SECURITY.md`, release notes, installer messages, and the separate Clinch website use
the same terminology and limits. Manual DMG/ZIP download is primary; the authenticated shell
installer is a convenience. All surfaces explain the unnotarized Open Anyway flow, signed
at-most-daily metadata checks, explicit update consent, the one-time updater bootstrap and
non-writable-install fallback, network-capable optional features, exact side effects, and residual
review gaps.

## Testing

- Shell fixtures cover valid/tampered/wrong-key/wrong-namespace manifests, explicit integration
  no-op/enable/repair/disable/purge behavior, malformed provider configuration, preservation, and
  selective uninstallation. The release verifier enforces manifest identity, size/hash, universal
  bundles, and the absence of quarantine-changing code in the shipped installer.
- Agent integration tests cover no-argument no-op, default-on startup selection,
  enable/status/disable/re-enable/purge idempotence, invalid
  third-party config, Claude and Codex structural preservation, receipt modes, and launch repair
  gated by the persisted enabled state.
- Rust unit tests cover unavailable telemetry beating stored and forced values, signed-GitHub
  updater configuration and consent states, and agent-resume default/opt-out state selection;
  stable compilation exercises the guarded collector, settings action, and backend-free plugin
  paths.
- Release verification mounts the DMG read-only and recursively compares its app with the verified
  ZIP, validates the plist, entitlements, signatures, architectures, authenticated manifest,
  unprivileged universal updater components, excluded authorization shim, and default-on and
  opt-out/re-enable integration lifecycle. The signed validation record contains only automated
  results and local builder metadata. The universal artifact verifier requires both architecture
  slices.
- Release orchestration fixtures stub every `gh` and Git mutation and prove that automated failures
  cannot create a tag or draft, remote staging requires the exact interactive confirmation, retries
  accept only a matching signed tag/private draft, uploaded assets are re-downloaded and verified,
  draft/tag/source mutation blocks publication, and only the verified draft is made public.
  Portable staged-asset fixtures cover missing, extra, tampered, wrong-version, stale-sequence,
  wrong-commit, invalid-signature, and valid release sets.
- The website runs lint and a production build; its release claims are reviewed against this
  contract before deployment.

## Risks and rollout

- An ad-hoc signed, unnotarized app still produces macOS warnings and has no Apple-issued publisher
  identity. Documentation cannot remove this residual risk.
- A first-time installer can authenticate the embedded key but cannot distinguish the newest valid
  release from an older legitimately signed release without an independent freshness source.
- GitHub, the release signing workstation, and both private keys remain high-value dependencies.
  Private keys must never enter repository history or ordinary build artifacts.
- A local build is less hermetic than an ephemeral hosted runner. The signed provenance records
  that fact rather than overstating assurance; clean-source checks, exact tool requirements,
  complete artifact verification, re-verification of downloaded draft assets, and public source
  reproducibility are the compensating controls. Removing the protected publication job also
  removes independent environment approval: the release workstation and authenticated operator
  are the sole publication authority.
- Restricting the in-app updater to user-writable installations trades universal replacement for a
  smaller attack surface. The preview performs a quiet authenticated metadata check, requires
  explicit consent, and falls back to the manual installer rather than elevating.
- CI and self-authored tests are not an independent audit. Promotion remains a public preview until
  outside reviewers and beta users validate the documented release checks.

Roll out in this order: land code and tests, remove the obsolete GitHub release environment and
signing secrets with the repository configuration command, run a candidate build without staging,
perform the recorded Apple Silicon smoke test and optional Intel smoke test, then run the first
fully local release publication.

## Parallelization

The release implementation is intentionally kept in one checkout because the Make targets,
orchestrator, staged-asset contract, and shell fixtures evolve together. Separate agents
or worktrees would create more merge risk than time savings. At release time, cheap non-Cargo
preflight checks may run before compilation, but the heavyweight Cargo gate and two release builds
share one workstation and persistent target directory; uncontrolled concurrent builds would trade
determinism and memory pressure for little reliable wall-clock improvement.
