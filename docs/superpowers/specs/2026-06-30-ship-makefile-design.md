# Ship Makefile + Clinch Release Flow — Design

**Date:** 2026-06-30 (revised 2026-07-01)
**Status:** Implemented

## Goal

Give a one-command way to do the two things we do after landing changes on
`master`:

1. **Release to everyone** — publish a new downloadable version.
2. **Update our own machine** — rebuild and install the app locally.

## How releases actually work for this fork (important context)

The original plan was to trigger the upstream `Cut New Releases` GitHub Actions
workflow. Investigation showed that is the **wrong model for this fork**:

- The fork has **zero Actions secrets** and is **not wired to Warp's release /
  auto-update backend** (no Apple signing certs, GCS buckets, Sentry, or
  `channel-versions` dispatch — `release_configurations.json` still points at
  Warp's infra).
- `Cut New Releases` had **never succeeded** in the fork; its one run failed at
  "Update branch and tag" with `git exit 128`.
- The `dev_00` tags in git are **mirrored from upstream** (they live on
  `warpdotdev/warp`, not on the fork). The fork has no release tags/branches of
  its own.

**Chosen model:** build a **self-signed DMG locally** and publish it as a
**GitHub Release** on the fork; users download and install it. This is free
(no CI, no runner minutes, no secrets) and fully under our control.

## The `Makefile` (root)

Self-documenting via `make help`. Overridable vars with `?=`.

| Target | Command | Purpose |
|---|---|---|
| `help` (default) | grep docstrings | List targets |
| `release` | `./script/bundle -c stable --selfsign [--nouniversal]` → `gh release create $(VERSION) $(RELEASE_DMG) --repo $(CLINCH_REPO) …` | Build `Clinch.dmg`, publish a GitHub Release |
| `install-local` | `./script/bundle -c local --selfsign --nouniversal` → copy to `/Applications/WarpLocal.app` | Update this machine (personal dev app) |
| `ship` | `install-local` then `release` | Full post-merge flow |

Key vars: `CLINCH_REPO ?= elliot-ylambda/clinch-terminal`, `STABLE_APP ?= Clinch`,
`VERSION ?= v0.<date>`, `UNIVERSAL` (unset → `--nouniversal` arm64-only; set → universal).
Release notes (a `define`d, `export`ed var) document the self-signed "right-click →
Open / `xattr -dr com.apple.quarantine`" step. `install-local` guards on
`create-dmg` (`brew install create-dmg`).

## Clinch rebrand (stable channel)

Renamed the distributed app Warp → **Clinch** in the two files that drive it:

- `script/macos/bundle` (stable branch): `WARP_APP_NAME="Clinch"`,
  `BUNDLE_ID="sh.clinch.Clinch"`. Also `--volname "$WARP_APP_NAME"` so each
  channel's DMG volume is named per-app (was hardcoded `Warp`).
- `app/Cargo.toml` `[package.metadata.bundle.bin.stable]`: `name = "Clinch"`,
  `identifier = "sh.clinch.Clinch"`.

**Why a new bundle id is safe:** the release channel is decided at compile time
(`app/src/bin/stable.rs` hardcodes `Channel::Stable`); the bundle id is never
compared for channel detection. It is only read (`channel/state.rs
app_id_from_bundle`) to namespace storage/keychain. A distinct id gives Clinch
isolated data and lets it coexist with an installed `Warp.app` (same-id would
collide on LaunchServices single-instance + shared data).

**URL scheme (fixed):** `Channel::Stable` now returns `clinch://`
(`crates/warp_core/src/channel/state.rs`), synced with `WARP_SCHEME_NAME` in
`script/macos/bundle`, and the one hardcoded `warp://` deep link
(`cloud_agent_capacity_modal`) was made scheme-dynamic. This isolates Clinch's
deep links / OAuth callback from an installed Warp. Residual risk: the fork uses
Warp's auth server (`auth_manager.rs` sends `?scheme=clinch`); if that server
rejects unknown schemes, login could fail — verify on first real sign-in.
`warp_core` compiles clean; full app compiles on first `make release`.

**Remaining follow-ups (not done):** icon still Warp's; CLI command still `oz`;
bundle copyright still Warp's; `warp://cli-agent` OSC sentinel left as-is
(internal CLI↔app marker, not an OS scheme).

## Separate infra fix applied

Root-caused the `git exit 128` failure: the fork's
`default_workflow_permissions` was `read`, so `github-actions[bot]` was denied
(403) pushing the release branch/tag. Set it to `write` (reversible repo
setting; not a YAML edit, since it's genuinely a repo setting). This is now
moot for our chosen local-release model but left in place as harmless/correct.

## Dead/outdated code check

- The CI-trigger Makefile targets (`release-dev`, `release-weekly`,
  `watch-release`, the remote-master guard) from the first draft were **removed**
  — they targeted Warp's backend and can't complete. Replaced, not stacked.
- `app/DockTilePlugin/Makefile` is unrelated and untouched.
- `WARP.md` remains the dev-command source of truth; `CLAUDE.md` references it
  via `@WARP.md` rather than duplicating.

## Verification

- `make help` and `make -n release|install-local|ship` expand to the intended
  commands (verified).
- Rebrand: `grep` confirms `Clinch` / `sh.clinch.Clinch` in both files with no
  lingering stable-Warp id/name; `bash -n script/macos/bundle` passes.
- **Not run:** a full stable LTO bundle (slow) — first `make release` will
  confirm `Clinch.dmg` is produced. The token fix was not exercised via a live
  CI run (chosen: don't spend CI, and the model no longer uses it).
