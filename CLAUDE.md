# CLAUDE.md

Guidance for Claude Code when working in this repository.

See @WARP.md for architecture, build, test, lint, and feature-flag guidance.
This file covers only the **release & local-update flow**, which is not in
`WARP.md`.

## Releasing & updating locally

Clinch ships as a **GitHub Release with a downloadable DMG** — users download
and install it themselves. Everything is built **locally and free**: no CI, no
GitHub Actions secrets, no macOS runner minutes. A root `Makefile` wraps it; run
`make help` for the list.

> This fork is **not** wired to Warp's release/auto-update backend (no signing
> certs, GCS buckets, Sentry, or `channel-versions` dispatch). The upstream
> `Cut New Releases` GitHub Actions workflow is intentionally unused; do not
> rely on it.

### After landing changes on `main`
- `make ship` — rebuilds/installs your personal app **and** publishes a release.

### Individual targets
- `make release` — builds a **self-signed** `Clinch.dmg` (`./script/bundle -c
  stable --selfsign`) and publishes a GitHub Release on
  `elliot-ylambda/clinch-terminal` with the DMG **and** `Clinch.app.zip`
  attached (`gh release create`). The zip must always be attached: the
  clinch.sh site's Install button downloads
  `releases/download/<tag>/Clinch.app.zip`.
  - `make release VERSION=v0.2.0` — set the tag (default: `v0.<date>`).
  - `make release UNIVERSAL=1` — universal Intel+ARM DMG (slower; default is this
    machine's arch only).
  - The DMG is built from your **local checkout**, so build from `main` for a
    release that matches what you merged.
- `make install-local` — builds the `local` channel and installs it as
  `/Applications/WarpLocal.app`, a standalone app that won't clobber (or be
  clobbered by) anything else you have installed.
- `make ship` — `install-local` + `release` (two builds: your `WarpLocal` dev
  app and the distributable `Clinch` app).

### The released app is self-signed
It is **not** notarized, so on first launch macOS warns. The release notes tell
users to right-click → **Open**, or run
`xattr -dr com.apple.quarantine /Applications/Clinch.app`. To ship without
warnings you'd need an Apple Developer ID cert + notarization (paid).

### Prerequisites
- `gh` authenticated with access to `elliot-ylambda/clinch-terminal`. Note `gh`
  may default to upstream `warpdotdev/warp`; the `Makefile` always passes
  `--repo` explicitly.
- `create-dmg`: `brew install create-dmg` (`script/bundle` always builds a DMG).

## Clinch app identity (stable channel)

The distributed stable app is branded **Clinch** via two files that must stay in
sync: `script/macos/bundle` (`WARP_APP_NAME`, `BUNDLE_ID` in the `stable` branch)
and `app/Cargo.toml` (`[package.metadata.bundle.bin.stable]` `name`/`identifier`).
It uses bundle id **`sh.clinch.Clinch`** (distinct from Warp's
`dev.warp.Warp-Stable`) so it coexists with an installed `Warp.app` and gets its
own isolated storage/keychain.

The release channel is still `Channel::Stable`, chosen at **compile time** by
`--bin stable` — the bundle id is never used for channel detection, so this
rename is safe.

### URL scheme
Clinch registers and uses **`clinch://`** — the Stable channel's `url_scheme()`
in `crates/warp_core/src/channel/state.rs`, kept in sync with `WARP_SCHEME_NAME`
in `script/macos/bundle`. This isolates deep links and the OAuth login callback
from an installed `Warp.app` (previously both used `warp://` and collided).

> **Verify on first login:** the fork uses Warp's auth server, which receives
> `?scheme=clinch` (`app/src/auth/auth_manager.rs`). If that server rejects
> unknown schemes, sign-in may fail — test a real login after your first
> `make release`. To revert, set the scheme back to `warp` in both files above.

### Other follow-ups (not done)
- **Icon** is still Warp's (`app/channels/stable/icon`); a Clinch icon is a
  separate visual task.
- **CLI command** for stable is still `oz`; renaming it is separate.
- The copyright string in the bundle metadata is still Warp's entity.
- `warp://cli-agent` (`app/src/terminal/cli_agent_sessions/event/mod.rs`) is an
  internal CLI↔app OSC sentinel, **not** an OS URL scheme — intentionally left
  as `warp://` (changing it needs a matching CLI change, no OAuth benefit).
