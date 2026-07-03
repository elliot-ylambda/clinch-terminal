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
- `make update` — publishes a release **and** updates + relaunches the prod
  `Clinch` app on this machine.

### Individual targets
- `make release` — builds a **self-signed** `Clinch.dmg` (`./script/bundle -c
  stable --selfsign`) and publishes a GitHub Release on
  `elliot-ylambda/clinch-terminal` with the DMG **and** `Clinch.app.zip`
  attached (`gh release create`). The zip must always be attached: the
  install one-liner (`curl -fsSL https://clinch.sh/install | sh`, which
  clinch.sh redirects to `install.sh` at this repo's root) downloads
  `releases/latest/download/Clinch.app.zip`. This is publish-only — it does not
  touch the app installed on this machine.
  - `make release VERSION=v0.2.0` — set the tag (default: `v0.<date>`).
  - `make release UNIVERSAL=1` — universal Intel+ARM DMG (slower; default is this
    machine's arch only).
  - The DMG is built from your **local checkout**, but `make release` first runs
    `script/require-latest-main`, which fetches `clinch/main` and
    **fast-forwards your local `main` to it** before building — so a stale
    checkout can't silently ship an old build (the "pushed ≠ shipped" gap). It
    **aborts before building** if you're not on `main`, the working tree is
    dirty, or `main` has diverged from `clinch/main`. Pass `make release
    SKIP_SYNC=1` to bypass the guard and build the current HEAD as-is
    (intentional feature-branch or dirty-tree test builds). `make update`
    inherits this because it runs `make release`.
- `make update` — runs `make release` (build + publish for everyone), then swaps
  the freshly built bundle into `/Applications/Clinch.app` and relaunches it via
  `script/update-installed-clinch`. This is the everyday "ship it and run it on
  my machine" command: you run the same prod `Clinch` app as everyone else
  (bundle id `sh.clinch.Clinch`), so your local sessions/history persist across
  updates — they live in `~/Library/Application Support/sh.clinch.Clinch`, which
  the swap never touches. The helper runs **detached**, so `make update` is safe
  to run from inside Clinch itself: it quits the running app (checkpointing its
  session), swaps the bundle, and relaunches.

> There is no separate personal dev app anymore (the old `make install-local` /
> `WarpLocal.app` flow was removed). For fast iterative development use
> `cargo run`; the `local` channel still exists as a compile-time target but is
> no longer packaged into an installable app.

### The released app is self-signed
It is **not** notarized, so macOS quarantines **browser-downloaded** copies
and blocks them on first launch (macOS 15+ removed the right-click → **Open**
bypass; the only escapes are System Settings → Privacy & Security → **Open
Anyway**, or `xattr -dr com.apple.quarantine /Applications/Clinch.app`). The
recommended install path is the `curl -fsSL https://clinch.sh/install | sh`
one-liner (`install.sh` at this repo's root): curl downloads never get the
quarantine flag, so Gatekeeper never runs. To ship browser downloads without
warnings you'd need an Apple Developer ID cert + notarization (paid) — note
`script/macos/bundle` already has the full codesign→notarize→staple pipeline
(`--read-passwords-from-env`); it just needs your own team ID and certs.

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
Clinch registers **`clinch://`** — the Stable channel's `url_scheme()`
in `crates/warp_core/src/channel/state.rs`, kept in sync with `WARP_SCHEME_NAME`
in `script/macos/bundle`. This isolates any deep links from an installed
`Warp.app` (previously both used `warp://` and collided).

> **No login in Clinch:** backendless builds (stable + local, via
> `ChannelConfig::no_backend()`) have login and onboarding fully gutted —
> `ChannelState::has_backend()` is `false`, so `RootView` launches straight
> into the terminal and `AuthManager::initialize_user_from_auth_payload`
> ignores any incoming auth callback. There is no OAuth flow to verify. The
> `?scheme=clinch` auth-server concern that used to live here no longer
> applies. (See `docs/superpowers/specs/2026-07-01-clinch-no-backend-gut-design.md`.)

### Other follow-ups (not done)
- **CLI command** for stable is still `oz`; renaming it is separate.
- The copyright string in the bundle metadata is still Warp's entity.
- `warp://cli-agent` (`app/src/terminal/cli_agent_sessions/event/mod.rs`) is an
  internal CLI↔app OSC sentinel, **not** an OS URL scheme — intentionally left
  as `warp://` (changing it needs a matching CLI change, no OAuth benefit).
