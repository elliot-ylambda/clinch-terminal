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
  universal architectures, archive URL, byte length, and SHA-256 digest;
- the convenience installer authenticates the manifest before parsing it and downloads only from
  the authenticated exact tag;
- the local release gate verifies the app, ZIP, and mounted DMG, including their identity,
  architectures, the single documented Messages Automation entitlement, nested-helper and app
  code-signature structure, and matching app files;
- the release workstation publishes a CycloneDX SBOM, signed validation record, signed checksum
  list, and release-key-signed local provenance for the exact gated commit;
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
- the optional Messages Automation sender, read-only Messages database watcher, phone reply
  routing, and locally queued reply bodies;
- optional SSH, MCP, remote assets, language servers, and other user-launched network clients; and
- any future privileged update helper.

Clinch is a terminal, so it intentionally is not App Sandbox constrained. It launches arbitrary
user commands and needs normal filesystem, PTY, process, and network access. The public bundle's
Clinch-specific entitlement plist contains only `com.apple.security.automation.apple-events` for
the optional, user-enabled Messages integration. Development, microphone, camera, contacts,
calendars, location, Photos, app-group, JIT, and library-validation bypass entitlements are
rejected by release verification.

## Privacy posture

The stable Clinch channel has no Warp backend, telemetry destination, crash-reporting destination,
or automatic updater. Unavailable telemetry overrides stored defaults and experiment or enterprise
flags. The collector drains memory, removes stale Clinch queue files without upload, and does not
start telemetry timers or write a shutdown queue.

Optional session capture is off until a user enables it. Its managed Claude Code and Codex hooks
write local pane mappings, a journal, and prompt mirrors. Those files can contain sensitive paths,
commands, identifiers, and prompt text and are created with restrictive permissions. Disabling the
integration preserves captured data by default; purge is separate.

Optional two-way iMessage is off until calibration succeeds. It uses Apple Events to ask Messages
to send iMessage-only text and Full Disk Access to read the calibrated conversation from the local
Messages database. The destination is stored in local secure storage. Route state, database
cursors, GUIDs, short-lived outgoing text hashes, ambiguous messages, and queued reply bodies are
stored in owner-only local files, expire where applicable, and are never included in telemetry or
ordinary logs. Disconnecting
deletes that Clinch-owned state but does not alter Messages history. Messages synchronization does
not provide a trustworthy source-device identity, so Clinch cannot prove that a synchronized reply
came from an iPhone rather than another device on the same Apple Account.

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
- Automatic updates are disabled. The old privileged helper is not bundled while its root-owned
  control files, atomic swap, interruption recovery, and symlink defenses await redesign and
  independent review.
- Restoring a pane intentionally executes a captured provider resume command. A compromised local
  account, provider config, transcript, hook, plugin, or Clinch state file can affect that command.
- Ad-hoc signing verifies internal bundle consistency, not publisher identity. Anyone can create a
  different ad-hoc signed app; users must also authenticate the Clinch checksum list and compare
  the artifact digest.
- Session recovery is best-effort after crashes, power loss, provider changes, or transcript
  retention. Security controls do not guarantee data recovery.
- The Messages database schema and Apple Events behavior are not public stability contracts. A
  macOS update may pause the optional bridge until compatibility is restored. Full Disk Access
  gives Clinch broad read access at the OS boundary even though the helper opens the database
  read-only and filters its watcher to the calibrated Messages conversation.

The current target is a public preview, not a claim that the app is risk-free or independently
certified.
