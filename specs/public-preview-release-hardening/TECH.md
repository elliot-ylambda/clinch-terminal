# Clinch public-preview release hardening: technical design

## Context

This design applies to the Clinch fork at source revision
`e423139fbb5b31ebced183e8db09c5e9ce384435`. It deliberately does not change
upstream Warp behavior. The public artifact remains an unnotarized macOS preview because the
project will not enroll in the Apple Developer Program.

The current path has five material release risks:

- `install.sh` trusts mutable release locations and a sibling checksum, clears quarantine, then
  installs hooks and plugins as side effects.
- `app/src/lib.rs` repairs agent hooks on every stable launch without a consent record.
- Clinch has no telemetry destination, but the collector can still schedule work and persist its
  queue because shared Warp privacy defaults are true.
- the self-signed bundle inherits unused privacy and development entitlements.
- local Make targets can publish a release before the complete CI suite finishes, while the
  privileged in-app update helper has root symlink and interrupted-swap failure modes.

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
uninstaller, SBOM, and GitHub provenance attestation.

`install.sh` resolves `latest` once through the GitHub API, or accepts a strict `--version` value,
then fetches only exact-tag assets. It size-limits metadata, authenticates it before parsing, and
validates every relevant manifest field. The archive is streamed to a private temporary directory,
checked for exact size and digest, expanded without following a user-selected destination, and
checked for bundle identity, version, executable, universal architectures, minimum OS, and a valid
structural code signature. It never changes quarantine metadata.

Installation first stages and validates a complete sibling bundle. Replacement uses same-volume
renames with a backup and restores that backup on every handled failure. The installer changes no
integration, plugin, preference, or running-app state.

### 2. Explicit, reversible session capture

Refactor `tools/agent-resume/install.sh` into explicit `enable`, `disable`, `status`, and `purge`
commands. With no command it prints help and performs no mutation. Enabling explains and then:

- installs Clinch-owned runtime files with restrictive modes;
- structurally adds only the managed Claude hooks and Codex notify block;
- writes a versioned consent/receipt file after successful configuration.

The receipt records ownership plus pre/post hashes and modes without copying provider transcripts,
configuration contents, or secrets. Disable performs a structural removal of the exact managed
entries; the receipt is an audit record, not a backup. On stable startup,
`app/src/agent_resume.rs` may refresh managed entries only when this consent record exists. Invalid
third-party JSON or malformed Codex managed markers are reported without preventing Clinch launch
or rewriting unrelated configuration.

Disabling removes the consent marker first, then removes only exact managed entries and owned
runtime files. It performs a structural removal and preserves every unrelated key or line;
captured metadata is retained by default. `purge` is a separate explicit operation for
Clinch-owned capture data.

Add setup, remove, and status controls to the Clinch settings page. The app installer and first
launch do not install notification plugins. Existing harness auto-install paths are disabled for
backend-free Clinch; the UI may show manual, source-identified plugin instructions after a direct
user action.

### 3. Privacy and startup behavior

Treat channel capability as authoritative in `app/src/settings/privacy.rs`: when the active
channel lacks telemetry or crash configuration, its effective values are false regardless of a
stored Warp default, enterprise override, or experiment. Do not persist a false value back into
shared Warp settings merely because Clinch lacks the capability.

The telemetry collector checks availability before it creates timers or subscriptions. In the
unavailable path it drains memory and deletes only Clinch's current and legacy telemetry queue
files. Every persistence and send entry point also checks availability, so shutdown cannot recreate
the queue and macros avoid constructing unused events.

Set Clinch's `autoupdate_config` to `None`. That removes the automatic release-check request and
keeps the known privileged helper outside the preview's reachable surface. The UI and documentation
point to authenticated manual updates. The helper is not re-enabled until it uses
root-owned control state, an atomic bundle exchange or externally recoverable journal, fault
injection tests, and independent review.

### 4. Bundle permissions and platform contract

Add a Clinch-only release entitlement plist containing an empty dictionary. Select it by the
Clinch bundle identifier, not by the shared stable channel. Keep hardened-runtime code signing but
use deterministic ad-hoc signing for the public preview unless an explicit compatible identity is
provided. Do not request app sandboxing or unused Apple Events, microphone, camera, contacts,
calendar, location, Photos, app-group, JIT, library-validation bypass, or debugger entitlements.

`script/update_plist` omits corresponding privacy usage descriptions for Clinch and writes an
explicit minimum macOS version. Release verification rejects forbidden entitlements or dead usage
keys and confirms both `arm64` and `x86_64` slices.

### 5. Release gate and publication

Replace local publication with a manually dispatched GitHub release workflow on `main`. The local
Make target may build a candidate or dispatch the workflow, but cannot call `gh release create`.
The workflow:

1. validates a strict semantic version and ensures it is greater than the last release;
2. runs formatting, shell/JavaScript tests, stable compilation, component tests, Clippy, and
   dependency license policy, bundled-notice generation, and advisories as independently visible
   required jobs;
3. builds the exact gated commit on an Intel runner and verifies the universal artifact;
4. derives a monotonic sequence greater than the latest authenticated manifest;
5. packages and verifies ZIP and DMG contents, signs metadata and the annotated tag, and produces
  a release-key-signed checksum list plus a CycloneDX SBOM;
6. creates the release from the preverified tag and uploads GitHub provenance attestations only
  after every gate succeeds.

The dependency license gate includes workspace and non-publishable git dependencies, keeps the
`cargo-deny` and `cargo-about` allowlists synchronized, and fails packaging if a complete bundled
third-party notice cannot be generated from the locked dependency graph.

The manual dispatch requires confirmation of clean install, authenticated manual upgrade, session
integration opt-in/removal, selective uninstall, offline startup, and native Apple Silicon smoke
results. It also requires the tested macOS versions and a QA record identifier. Those fields, plus
the optional hands-on Intel result, are written into the signed release validation asset. The
checked-in `QA_TEMPLATE.md` defines the record expected by the workflow.

`make release` is the interactive operator front end for that dispatch. It first synchronizes a
clean local `main` with `clinch/main`, compares its timestamp-derived version with the latest public
release and increments the latest version when necessary, and detects the local macOS version. It
then displays the required hands-on checklist and requires the operator to type `RELEASE`; it does
not infer a pass from default Make variables. Unless the operator supplies an existing QA record,
the dispatcher creates a public GitHub issue containing the exact version, commit, machine, OS,
checked results, and optional Intel result, then uses that URL for the workflow input. Explicit
variables remain available for tested automation, but noninteractive dispatch requires
`QA_CONFIRMED=true`. The dispatch also carries that exact commit as a required input; the workflow
stops before building if `main` changed between local QA confirmation and GitHub dispatch.

Actions are pinned to immutable commit SHAs and receive least-privilege job permissions. A
repository configuration script enables immutable releases, protects `main` against force pushes
and deletion, requires review and the named gate checks, and leaves secret scanning/push protection
enabled. It is safe to run only after the new workflow exists on GitHub.

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
installer is a convenience. All surfaces explain the unnotarized Open Anyway flow, manual updates,
network-capable optional features, exact side effects, and residual review gaps.

## Testing

- Shell fixtures cover valid/tampered/wrong-key/wrong-namespace manifests, explicit integration
  no-op/enable/repair/disable/purge behavior, malformed provider configuration, preservation, and
  selective uninstallation. The release verifier enforces manifest identity, size/hash, universal
  bundles, and the absence of quarantine-changing code in the shipped installer.
- Agent integration tests cover no-argument no-op, enable/status/disable/purge idempotence, invalid
  third-party config, Claude and Codex structural preservation, receipt modes, and launch repair
  gated by consent.
- Rust unit tests cover unavailable telemetry beating stored and forced values, disabled
  auto-update, and agent-resume runtime consent; stable compilation exercises the guarded
  collector, settings action, and backend-free plugin paths.
- Release verification mounts the DMG read-only and recursively compares its app with the verified
  ZIP, validates the plist, entitlements, signatures, architectures, authenticated manifest, and
  explicit integration lifecycle. Manual release QA records first install, upgrade, uninstall,
  offline launch, and a native Apple Silicon smoke check. The universal build and Intel-hosted CI
  remain required; a separate hands-on Intel smoke check is recorded when practical but is not a
  preview release blocker.
- The website runs lint and a production build; its release claims are reviewed against this
  contract before deployment.

## Risks and rollout

- An ad-hoc signed, unnotarized app still produces macOS warnings and has no Apple-issued publisher
  identity. Documentation cannot remove this residual risk.
- A first-time installer can authenticate the embedded key but cannot distinguish the newest valid
  release from an older legitimately signed release without an independent freshness source.
- GitHub, the release signing workstation, and both private keys remain high-value dependencies.
  Private keys must never enter repository history or ordinary build artifacts.
- Disabling the in-app updater trades convenience for a smaller privileged attack surface. The
  preview must make manual update availability visible without performing a background check.
- CI and self-authored tests are not an independent audit. Promotion remains a public preview until
  outside reviewers and beta users validate the documented release checks.

Roll out in this order: land code and tests, merge the workflow, configure branch and immutable
release protection, run a candidate build without publishing, perform the recorded Apple Silicon
smoke test and optional Intel smoke test, then dispatch the first gated public-preview release.
