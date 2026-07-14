# Clinch public-preview release hardening

## Summary

Clinch can be distributed publicly without an Apple Developer Program membership as a clearly
identified, unnotarized open-source preview. Installation, optional integrations, updates,
release provenance, privacy behavior, removal, and public documentation must fail safely and
must never imply that Apple has verified the app.

## Problem

The current preview relies on mutable release URLs, removes macOS quarantine, modifies Claude
Code and Codex configuration during installation and first launch, and can publish before the
full validation suite completes. That is too much implicit trust and surprise for a public
download, especially when the app cannot use Apple's Developer ID and notarization trust path.

## Goals

- Make the downloadable app and its release metadata independently verifiable with Clinch-owned
  release keys and immutable version identifiers.
- Make every persistent integration an informed, reversible opt-in.
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

9. Claude Code and Codex session capture is a separate, explicit opt-in. Before enabling it, the
   user is told which configuration files, managed hook blocks, executable files, and local data
   directories will be created or changed.

10. Once session capture is enabled, Clinch may refresh only its clearly marked managed entries
    on launch. It preserves unrelated user configuration byte-for-byte where the underlying file
    format permits, creates restrictive file permissions, and fails open without preventing the
    terminal from launching when a third-party configuration is invalid.

11. Clinch does not infer consent from merely launching the app. A durable local consent marker
    is required before automatic repair of session-capture hooks. Removing that marker stops all
    future automatic hook changes.

12. Notification plugins are never installed by the app installer or first launch. Any in-app
    installation requires a direct user action and identifies the plugin, publisher, source, and
    files or configuration it will affect before execution.

13. Plugin code bundled or automatically installed by Clinch is pinned to an exact upstream
    revision and digest, travels inside the authenticated Clinch artifact, and updates only with
    a new authenticated Clinch release. User-chosen third-party or remote plugin sources are
    clearly outside this guarantee.

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

19. Stable Clinch configures no Warp account/backend request, telemetry/crash destination, or
    automatic release check at startup. Optional Claude plan-limit gauges contact Anthropic only
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

23. In-app updating is unavailable in the public preview. The existing privileged updater remains
    disabled until its root-owned control files and atomic bundle swap have been independently
    reviewed and tested against interruption and symlink attacks. Users update with the same
    authenticated, exact-version manual or convenience-install path used for first installation.

24. The DMG and ZIP attached to one release contain the same Clinch app bytes and identity. Release
    verification mounts the DMG and compares its app with the verified ZIP instead of checking
    only the disk-image container.

25. Public release tags are immutable, annotated, and cryptographically signed by a dedicated
    Clinch release key. A release is built from the exact tagged source revision, and the published
    release refuses a tag that was created or moved outside the gated release process.

26. The public release workflow runs the full formatting, script/integration, stable-build,
    component-test, lint, and dependency-advisory gates before packaging. A timeout, cancellation,
    skipped required step, or test failure prevents publication.

27. Release publication is performed only by the protected GitHub workflow. Local developer
    commands may build and verify candidates, but no bypass flag or local background job can
    publish a public release.

28. Every release publishes the DMG, ZIP, authenticated metadata and signatures, a
    release-key-signed checksum list, machine-readable SBOM, and GitHub artifact provenance.
    Publication uses an existing verified tag and is compatible with GitHub immutable releases.

29. The release update sequence is strictly greater than every previously published sequence,
    even when a build machine's clock is wrong or two releases are initiated close together.

30. The `main` branch requires review and the named release-gate checks before update. Force
    pushes and branch deletion are disabled, GitHub secret scanning remains enabled, and the
    release workflow receives only the minimal write permissions needed for tags, releases, and
    attestations.

31. Security documentation includes the installer, release keys, GitHub Actions, update helper,
    hooks, provider credentials, local transcripts, command construction, and plugin sources in
    its threat model. It distinguishes verified controls, residual risks, and validation that
    still requires independent reviewers or real-world beta users.

32. The website, README, FAQ, release notes, installer output, and in-app copy agree on platform
    support, network behavior, integration side effects, signing/notarization status, update
    behavior, licenses, and removal. Claims are tied to observable behavior rather than promises.

33. A release candidate is not promoted until it has passed first install, authenticated manual
    upgrade, integration opt-in, integration removal, application uninstall, offline-startup, and
    a native Apple Silicon smoke check. Intel remains covered by the universal artifact verifier
    and native Intel CI; a separate hands-on Intel smoke check is recorded when practical but does
    not block this preview. Results and any untested OS versions are recorded with the release.

34. If any authenticity or safety check cannot be completed, the installer or release workflow
    stops with a specific error and leaves the existing installation or public release unchanged.
