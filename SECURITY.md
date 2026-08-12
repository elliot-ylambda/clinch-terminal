# Security policy

Clinch is an independent fork of [Warp](https://github.com/warpdotdev/warp). Do not report
Clinch-specific issues to Warp.

## Report a vulnerability

Do not open a public issue for a suspected vulnerability. Use one of these private channels:

- [GitHub private security advisory](https://github.com/elliot-ylambda/clinch-terminal/security/advisories/new)
- [contact@ylambda.com](mailto:contact@ylambda.com)

Include the affected Clinch version, macOS version and architecture, reproduction steps, expected
impact, and whether you have published or shared the finding. Reports are normally acknowledged
within a few days. If the issue also affects upstream Warp, report it under
[Warp's security policy](https://github.com/warpdotdev/warp/blob/main/SECURITY.md).

Only the newest public-preview release receives fixes. Older previews may be useful for diagnosis,
but they are not maintained security branches.

## Release trust model

Clinch does not use Apple's Developer ID and notarization distribution path. Public builds are
ad-hoc signed with the hardened runtime and are not notarized. Apple does not identify the
publisher or vouch for the artifact, and Gatekeeper may require **Privacy & Security → Open
Anyway**.

Clinch adds its own release controls:

- a dedicated OpenSSH Ed25519 key signs annotated Git tags and an SSHSIG envelope over the exact
  release manifest, plus the checksum list that covers the public DMG and other assets;
- a separate Ed25519 key signs the manifest format retained for independent verification;
- the manifest binds the repository, tag, version, sequence, bundle ID, minimum macOS version,
  supported architectures, and each architecture-specific archive URL, byte length, and SHA-256
  digest;
- the convenience installer authenticates the manifest before parsing it, detects the Mac's
  hardware architecture (including processes running under Rosetta), and downloads only that
  archive from the authenticated exact tag;
- the local release gate verifies the app, ZIP, and mounted DMG, including their identity,
  exact native architecture (or both slices for an explicit legacy universal release), absence of
  unused privacy entitlements and the privileged update authorizer, presence of the universal
  unprivileged update components, code-signature structure, and matching app files;
- the release workstation publishes an SBOM for each native app, a signed validation record,
  signed checksum list, and release-key-signed local provenance for the exact gated commit;
- the local release command downloads and independently verifies the private draft before making
  it public, without running GitHub Actions; and
- repository policy protects `main`, secret scanning, push protection, and GitHub immutable
  releases.

The public keys are in `resources/release/clinch-release-allowed-signers` and
`resources/update/clinch-update-public-key.json`. Private keys are kept out of the repository and
remain only on the release workstation; GitHub stores no release signing secrets.

These controls authenticate a Clinch release against the embedded project keys. They are not a
substitute for Apple notarization or an independent audit. On a first install, a user who receives
an old but legitimately signed release has no independent freshness oracle. GitHub, the local
release workstation, maintainer credentials, and both signing keys remain trusted dependencies.

## Security boundaries

The release review treats these as security-sensitive:

- installer URL resolution, archive extraction, destination replacement, and failure rollback;
- GitHub release API credentials, tags, drafts, assets, signing keys, SBOM, and provenance;
- terminal command construction, shell environment inheritance, PTYs, process inspection, and
  restored commands;
- Claude Code and Codex configuration hooks, local transcripts, prompt mirrors, and provider
  credentials;
- plugin sources and provider CLI commands;
- optional SSH, MCP, remote assets, language servers, and other user-launched network clients; and
- the unprivileged in-app updater and any future privileged update helper.

Clinch is a terminal, so it intentionally is not App Sandbox constrained. It launches arbitrary
user commands and needs normal filesystem, PTY, process, and network access. The current public
bundle has no Clinch-specific privacy entitlement. Development, microphone, camera, contacts,
calendars, location, Photos, app-group, JIT, library-validation bypass, and Apple Events
entitlements are rejected by release verification.

## Privacy posture

The stable Clinch channel has no Warp backend, telemetry destination, or crash-reporting
destination. Its updater makes a quiet, at-most-weekly GitHub request for signed release metadata;
automatic checks do not download an archive. That check is the only request Clinch issues on its
own, and it can be disabled entirely with `clinch.updates.automatic_check` or the
`CLINCH_NO_UPDATE_CHECK` environment variable, leaving on-demand checks from the Clinch menu as
the only path to the network. Unavailable telemetry overrides stored defaults and
experiment or enterprise flags. The collector drains memory, removes stale Clinch queue files
without upload, and does not start telemetry timers or write a shutdown queue.

Local session capture is enabled on first launch so Claude Code and Codex conversations can be
restored by default. Its managed hooks write local pane mappings, a journal, and prompt mirrors.
Those files can contain sensitive paths, commands, identifiers, and prompt text and are created
with restrictive permissions. Disabling the integration persists the opt-out and preserves
captured data by default; purge is separate.

Software launched inside Clinch is outside this no-telemetry statement. Provider CLIs, SSH, MCP
servers, plugins, package managers, remote assets, and other user commands may use the network or
handle credentials according to their own policies.

## Known residual risks

- The build is not Apple-notarized and has no Apple-issued publisher identity.
- This project has not completed an independent security audit or broad public beta.
- The dependency policy currently acknowledges several unmaintained transitive crates listed with
  reasons in `deny.toml`. None of those exceptions is a vulnerability advisory, but they remain
  maintenance debt and should be replaced as upstream dependency chains allow.
- The convenience installer is a bootstrap: users must obtain or review a script containing the
  correct project public key. Manual versioned DMG download remains the primary path.
- In-app replacement is limited to app bundles and parent directories writable by the current
  user. It uses an unprivileged helper and atomic same-directory exchange; non-writable installs
  fall back to the authenticated manual installer. No administrator authorization shim is bundled.
- The updater and release controls have not received an independent audit. A client released before
  the updater bridge in `v0.2026.07.20.1643` cannot discover it and needs one final authenticated
  manual installation. An Intel client that predates architecture-specific archive selection also
  needs one manual bridge install unless an intervening universal release carries the new updater.
- Restoring a pane intentionally executes a captured provider resume command. A compromised local
  account, provider config, transcript, hook, plugin, or Clinch state file can affect that command.
- Ad-hoc signing verifies internal bundle consistency, not publisher identity. Anyone can create a
  different ad-hoc signed app; users must also authenticate the Clinch checksum list and compare
  the artifact digest.
- Session recovery is best-effort after crashes, power loss, provider changes, or transcript
  retention. Security controls do not guarantee data recovery.
The current target is a public preview, not a claim that the app is risk-free or independently
certified.
