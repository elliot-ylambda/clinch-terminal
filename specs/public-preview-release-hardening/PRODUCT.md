# Clinch public-preview release hardening

## Summary

Clinch can be distributed publicly without an Apple Developer Program membership as a clearly
identified, unnotarized open-source preview. Installation, optional integrations, updates,
release provenance, privacy behavior, removal, and public documentation must fail safely and
must never imply that Apple has verified the app.

## Problem

The current preview relies on mutable release URLs, removes macOS quarantine, may modify Claude
Code and Codex configuration without a bounded payload, and can publish before the full validation
suite completes. That is too much implicit trust and surprise for a public download, especially
when the app cannot use Apple's Developer ID and notarization trust path.

## Goals

- Make the downloadable app and its release metadata independently verifiable with Clinch-owned
  release keys and immutable version identifiers.
- Make default-on session capture clearly disclosed, reversible, and persistently disableable, and
  make automatic notification-plugin provisioning local, pinned, versioned, and removable.
- Prevent a release unless the exact source revision and artifacts pass the release gate.
- Ensure the stable build does not send or persist Clinch/Warp analytics or crash telemetry.
- State the remaining macOS trust limitation plainly on every installation surface.

## Non-goals

- Apple Developer ID signing, notarization, stapling, or Mac App Store distribution.
- Claiming that a self-review is an independent security audit or a guarantee of security.
- Preventing network activity initiated by commands, plugins, MCP servers, language servers,
  remote sessions, or other tools the user intentionally runs inside the terminal.

## Behavior

1. Every public surface identifies the release as an **unnotarized public preview**. It does not
   describe the app as Apple-verified, notarized, Gatekeeper-approved, or guaranteed safe.

2. The primary installation path is a versioned DMG or ZIP from the Clinch GitHub release. Users
   are shown the release version, a release-key-signed checksum list, SHA-256 verification
   instructions, and the normal macOS **Privacy & Security → Open Anyway** flow before being asked
   to launch the app.

3. Clinch never removes, clears, or bypasses `com.apple.quarantine`, never tells a user to disable
   Gatekeeper globally, and never treats the absence of quarantine as proof of safety.

4. The convenience installer remains secondary to the manual download. Before changing
   `/Applications` or `~/Applications`, it authenticates signed release metadata with a public
   key embedded in the reviewed installer, resolves one exact release tag, and downloads every
   artifact from that exact tag. A checksum fetched from the same mutable location is not
   sufficient authentication by itself.

5. The installer fails closed when the release signature, repository identity, tag, archive URL,
   archive size, SHA-256 digest, bundle identifier, executable, supported architecture, minimum
   macOS version, or structural code signature does not match the authenticated metadata.

6. A user can explicitly request an older or newer valid Clinch version. The installer never
   silently changes that requested version and refuses unsafe tag syntax or a release whose
   signed metadata names another version.

7. Installing Clinch changes only the selected application directory by default. It does not
   edit Claude Code or Codex configuration, install plugins, create agent hooks, open the app,
   request elevated privileges, or write outside its temporary directory and destination.

8. If an existing Clinch bundle is replaced, the installer verifies the new bundle first and
   preserves the old installation until the replacement can complete. An interrupted or failed
   install must not leave a partially copied app at the destination.

9. Claude Code and Codex session capture is enabled on first app launch so session restore works by
   default. The README and settings page disclose which configuration files, managed hook blocks,
   executable files, and local data directories are created or changed. Release notes and installer
   output disclose the default, and the settings page lets the user turn capture off or back on.

10. Once session capture is enabled, Clinch may refresh only its clearly marked managed entries
    on launch. It preserves unrelated user configuration byte-for-byte where the underlying file
    format permits, creates restrictive file permissions, and fails open without preventing the
    terminal from launching when a third-party configuration is invalid.

11. With no prior capture preference, first launch creates a durable local enabled marker only
    after the managed hooks install successfully. Disabling capture writes a durable opt-out;
    future launches make no hook changes until the user explicitly re-enables it. A retained
    receipt from an older disabled installation is also honored as an opt-out during migration.

12. On first launch and whenever its bundled plugin version changes, Clinch best-effort installs
    its bundled Warp notification plugins into detected Claude Code and Codex user plugin stores.
    The authenticated app also carries the plugins' pinned universal macOS `jq` dependency on the
    pane path. A missing CLI or provider policy failure never blocks launch and is retried later.

13. Plugin code bundled and automatically installed by Clinch is pinned to an exact upstream
    revision and digest, travels inside the authenticated Clinch artifact, and updates only with
    a new Clinch release. User-chosen third-party or remote plugin sources remain outside this
    bundle.

14. Session-capture removal deletes only Clinch-managed Claude and Codex hook entries and
    Clinch-owned helper files. It preserves unrelated configuration and, by default, preserves
    captured conversation metadata so accidental uninstall does not destroy recovery data.

15. A separate, explicit purge option removes Clinch-owned session-capture data. It lists the
    directories to be removed and does not delete Claude Code or Codex transcripts owned by those
    applications.

16. App removal documentation distinguishes the application bundle, preferences/state, cached
    update files, session-capture helpers, captured metadata, and optional provider plugins so a
    user can choose what to retain.

17. The stable Clinch channel has no Warp account flow, no configured Warp backend, no configured
    RudderStack destination, and no linked Sentry crash reporter. Backend-dependent UI remains
    unavailable.

18. When a channel has no telemetry configuration, telemetry is treated as unavailable and off:
    Clinch does not schedule telemetry flush tasks, send active-usage events, persist queued
    telemetry at shutdown, or retry telemetry left by an older build. Obsolete Clinch telemetry
    queue files are deleted locally without being uploaded.

19. Stable Clinch configures no Warp account/backend request or telemetry/crash destination. Its
    only automatic project-owned network request is a quiet, at-most-daily check of authenticated
    stable release metadata on GitHub. Optional Claude plan-limit gauges contact Anthropic only
    after the user enables them; other network-capable integrations remain attributable to an
    explicit user action or a process the user launched.

20. The app's privacy UI and documentation do not offer telemetry or crash-reporting controls
    that have no effect in the stable channel. Stored upstream defaults cannot silently re-enable
    unavailable telemetry.

21. The public app is a universal Intel and Apple Silicon build. Its authenticated metadata and
    bundle declare the same minimum supported macOS version, and installation refuses an older
    system with an actionable error rather than relying on a launch failure.

22. Release entitlements are limited to capabilities Clinch intentionally supports and documents.
    Development-only entitlements are forbidden. Any privacy-sensitive entitlement retained for
    terminal child-process compatibility is recorded in the release security documentation.

23. In-app updating is available only for installations writable by the current user. It uses an
    unprivileged helper and same-directory atomic bundle exchange; the public preview never invokes
    the existing AppleScript administrator-authorization path. Non-writable installations use the
    same authenticated, exact-version manual installer used for first installation. Builds shipped
    before this bridge release require that manual installer once before they can discover updates.

24. The DMG and ZIP attached to one release contain the same Clinch app bytes and identity. Release
    verification mounts the DMG and compares its app with the verified ZIP instead of checking
    only the disk-image container.

25. Public release tags are immutable, annotated, and cryptographically signed by a dedicated
    Clinch release key. `make release` builds from one clean, current `main` revision and refuses
    to stage a tag whose commit differs from the revision that passed the automated release gate.

26. Before it creates a tag or draft release, `make release` runs the full formatting,
    script/integration, stable-build, component-test, lint, dependency-license, and advisory gates
    locally, then builds and verifies the universal candidate. A timeout, interruption, skipped
    required step, or test failure prevents staging and publication.

27. The local release command creates a signed tag and a private draft release containing only
    fully verified assets. It downloads that draft into a fresh directory, repeats the complete
    portable signature, digest, provenance, version, commit, and sequence checks, and only then
    makes the draft public. Public release publication does not run GitHub Actions.

28. Every release publishes the DMG, ZIP, authenticated metadata and signatures, a
    release-key-signed checksum list, machine-readable SBOM, signed validation record, and
    release-key-signed local build provenance. The provenance identifies the exact source commit
    and artifact digests without claiming that GitHub built a workstation-produced artifact.
    Publication remains compatible with GitHub immutable releases.

29. The release update sequence is strictly greater than every previously published sequence,
    even when a build machine's clock is wrong or two releases are initiated close together.

30. The `main` branch rejects force pushes and deletion, GitHub secret scanning remains enabled,
    and release keys remain only on the release workstation. Final publication is authorized by
    the version-and-commit-specific local confirmation and the operator's authenticated GitHub
    credentials; no release signing secret or publication workflow is stored or run on GitHub.

31. Security documentation includes the installer, release keys, GitHub release API, update helper,
    hooks, provider credentials, local transcripts, command construction, and plugin sources in
    its threat model. It distinguishes verified controls, residual risks, and validation that
    still requires independent reviewers or real-world beta users.

32. The website, README, FAQ, release notes, installer output, and in-app copy agree on platform
    support, network behavior, integration side effects, signing/notarization status, update
    behavior, licenses, and removal. Claims are tied to observable behavior rather than promises.

33. A release candidate is not promoted until the complete automated source gate and universal
    artifact verification pass on the exact source revision. The signed validation record names
    those automated gates, the artifact checks, and the local builder environment without claiming
    manual QA occurred. `make release` does not require or create a hands-on QA attestation. After
    the automated build and verification finish, it requires a version-and-commit-specific
    confirmation before creating remote staging state and publishing it after verification.
    Mechanical defaults never count as permission to publish.

34. If any authenticity or safety check cannot be completed, the installer or local release
    command stops with a specific error and leaves the existing installation and every published
    release unchanged. An interrupted staging attempt may leave a correctly signed tag or private
    draft for an idempotent retry, but it never exposes an unverified public release.
