# Clinch in-app updates — technical design

## Context

The implementation is based on commit `387288b418353f0e7f920bb3a2c44df5f3cb1149`.

- [`app/src/autoupdate/mod.rs`](https://github.com/elliot-ylambda/clinch-terminal/blob/387288b418353f0e7f920bb3a2c44df5f3cb1149/app/src/autoupdate/mod.rs) already owns update polling, UI state, download readiness, and relaunch coordination, but a successful check immediately downloads an update.
- [`app/src/autoupdate/mac.rs`](https://github.com/elliot-ylambda/clinch-terminal/blob/387288b418353f0e7f920bb3a2c44df5f3cb1149/app/src/autoupdate/mac.rs) expects Warp-named DMGs and Warp's Apple Team ID and swaps the bundle before termination.
- [`app/src/bin/stable.rs`](https://github.com/elliot-ylambda/clinch-terminal/blob/387288b418353f0e7f920bb3a2c44df5f3cb1149/app/src/bin/stable.rs) uses a backend-free channel configuration with updates disabled. Stable releases currently publish a DMG, ZIP, and checksum through the Makefile.

`PRODUCT.md` defines the user-visible behavior and recovery invariants.

## Proposed changes

1. Extend `AutoupdateConfig` with a backward-compatible provider discriminator. The Clinch stable
   binary selects a GitHub provider and exposes update menu items; private Warp configurations
   default to the existing Warp provider and behavior.

2. Add a Clinch update provider module that:
   - fetches the latest release record from the pinned GitHub repository;
   - resolves the manifest, signature, and archive from that same release;
   - verifies the raw manifest bytes with Ring's Ed25519 verifier and embedded/persisted trusted
     keys before using any manifest field;
   - validates schema, repository-owned HTTPS URLs, bundle ID, release tag, sequence, size, and
     hash; and
   - streams the ZIP to the update cache, verifies it while streaming, extracts with `ditto`, and
     validates the staged bundle with macOS system tools.

3. Add `AutoupdateStage::UpdateAvailable`. Clinch checks stop in this state; the existing Warp
   provider continues downloading immediately. The install action presents a native confirmation
   dialog and only then starts the download. A consented download transitions to `UpdateReady` and
   initiates the existing cancellable relaunch workflow.

4. Bundle a narrow unprivileged updater helper and universal atomic-swap utility; do not bundle or
   invoke an AppleScript authorization shim. Before termination, the helper runs the existing
   live-agent repair and recovery snapshot routines, validates all paths, executes from the
   installed signed bundle, copies the authenticated ZIP into a private same-filesystem transaction
   directory, verifies its signed size and SHA-256 identity, writes a readiness marker, and waits
   for the old PID. Installation is allowed only when the app and its parent are user-writable and
   uses a same-directory atomic bundle exchange, first-frame success marker, rollback on failure,
   and scrubbed relaunch environment. Non-writable installations use the authenticated manual
   installer. Cancellation uses a per-update marker checked by the waiting helper.

5. Generate `Clinch.update.json` and `Clinch.update.sig` during packaging. The canonical JSON
   contains schema/tag/sequence/minimum macOS/bundle identity/archive size and hash/release notes,
   plus optional rollback and next-key data. The release private key is supplied through
   `CLINCH_UPDATE_SIGNING_KEY`; only the public key is checked in. Candidate verification exercises
   the same signature and artifact invariants before `gh release create` publishes all assets.

6. Register a `CheckForUpdates` custom action in the Clinch application menu and retain Settings,
   overflow-menu, and command-palette entry points. User-facing update strings become app-neutral
   or Clinch-specific without changing unrelated Warp branding.

## Testing and validation

- Unit tests cover manifest/signature verification, schema and identity validation, trusted-key
  rotation, release ordering/rollback rules, asset selection, hash/size enforcement, and the new
  consent state transition (Behavior 1–7, 13).
- Shell tests exercise updater path validation, readiness/cancellation, exact-PID waiting, atomic
  install, rollback, marker cleanup, and environment scrubbing with command seams (Behavior 8–12).
- Menu/state layout tests confirm the updater surfaces can render and dispatch without a panic
  (Behavior 1, 3–5).
- Release-script tests generate and verify a fixture manifest/signature and assert packaging fails
  closed for absent keys or tampered artifacts (Behavior 13–15).
- Run focused tests, stable `cargo check`, shell syntax/tests, formatting, Clippy, and the repository
  launch/presubmit gate before opening the PR.

## Risks and mitigations

- The updater intentionally has no elevated path. It accepts only fixed subcommands, numeric IDs,
  authenticated archives in the current user's isolated Clinch app-state directory, and a
  destination matching the installed bundle identity. A same-directory atomic exchange ensures
  interruption never leaves the installed app path empty; the old bundle remains available until
  the new app acknowledges its first frame.
- Ad-hoc app signing does not authenticate the publisher. The signed manifest supplies publisher
  authentication now; Developer ID signing/notarization remains the preferred future hardening.
- Multiple windows observe one update model. State transitions are idempotent, so only the first
  observer can begin relaunch and later observers see `Updating`.

## Parallelization

The state machine, bundle staging, helper markers, and release manifest share types and fixtures and
are safest to implement sequentially in one checkout. No parallel agents are proposed.
