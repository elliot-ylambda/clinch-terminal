# Clinch channel config without the private generator — design

**Date:** 2026-07-01
**Status:** Approved, implementing
**Branch:** `fix/channel-config-no-backend`

## Problem

`make ship` (and `make release` / `make install-local`) fail to compile:

```
error: couldn't read .../build/warp-.../out/local_config.json: No such file or directory
  → app/src/bin/local.rs: channel_config::load_config!("local")  (via include_str!)
```

Bundled builds enable the `release_bundle` feature, under which `load_config!("<channel>")`
expands to `include_str!(OUT_DIR/<channel>_config.json)`. Those JSON files are emitted by
`app/build.rs::generate_channel_config_if_needed`, which shells out to the **private**
`warp-channel-config` binary (installed from `warpdotdev/warp-channel-config` over SSH). When
that binary is absent, the build script silently returns without writing the files, so the
`include_str!` fails to compile.

This fork has **no access** to `warpdotdev/warp-channel-config` (confirmed: token 404, SSH
denied), so the ship flow can never produce the embedded configs. The flow has never actually
built a DMG.

## Constraints / decisions

- **Backend posture: fully local, no backend.** Clinch guarantees no telemetry / no backend
  (commit `40e5c15c9`). The config must never contact Warp servers.
- **Autoupdate: disabled.** Distribution is via GitHub Releases; there is no Warp-protocol
  update feed. `autoupdate_config: None`.
- Only the `local` (→ `WarpLocal.app`) and `stable` (→ `Clinch.app`) channels are shipped by
  the Makefile. `dev`/`preview` are not built by this fork (YAGNI — left untouched).

## Approach

Follow the existing in-repo pattern in `app/src/bin/oss.rs`, which constructs its
`ChannelConfig` **inline in Rust** (no `load_config!`, no generator). The proven-runnable
no-backend values come from `crates/integration/src/bin/integration.rs`, which black-holes all
server traffic with an unroutable address and which the integration tests actually launch the
app against.

**Why not empty-string URLs:** `http_client/lib.rs:782` does `Url::parse(server_root_url).unwrap()`
and `warp_server_client/.../auth/session.rs:200` does `.expect(...)`. An unparseable `""` would
panic. Instead we use `http://192.0.2.0:9` (TEST-NET-1, RFC 5737; discard port 9) — parseable
but unroutable — matching the integration binary.

### Changes

1. **`crates/warp_core/src/channel/config.rs`** — add, beside the existing `production()`:
   - `WarpServerConfig::offline()` → `server_root_url`/`rtc_server_url` = black-hole
     (`http://192.0.2.0:9`, `ws://192.0.2.0:9/graphql/v2`), `session_sharing_server_url: None`,
     `firebase_auth_api_key: ""`, `iap_config: None`.
   - `OzConfig::offline()` → `oz_root_url` black-hole, `workload_audience_url: None`.
   - `ChannelConfig::no_backend(app_id, logfile_name)` → `server_config: offline()`,
     `oz_config: offline()`, all of telemetry/autoupdate/crash/mcp = `None`.

2. **`app/src/bin/stable.rs`** — replace `channel_config::load_config!("stable")` with
   `ChannelConfig::no_backend(AppId::new("sh", "clinch", "Clinch"), "clinch.log")`. Remove the
   now-unused `#[path] mod channel_config;`. Import `ChannelConfig`, `AppId`.

3. **`app/src/bin/local.rs`** — replace `channel_config::load_config!("local")` with
   `ChannelConfig::no_backend(AppId::new("dev", "warp", "Warp-Local"), "warp-local.log")`,
   keeping the existing `with_additional_features(...)` chain and `WITH_SANDBOX_TELEMETRY`
   handling. Remove the now-unused `#[path] mod channel_config;`. Import `ChannelConfig`, `AppId`.

`app_id`s match the macOS bundle ids the bundle script assigns (`sh.clinch.Clinch`,
`dev.warp.Warp-Local`), so app storage/keychain namespaces line up with the bundle identity.

### Dead code

- `channel_config.rs` / `load_config!` / `load_config_from_embedded` stay — still used by
  `dev.rs` and `preview.rs`.
- `WarpServerConfig::production()` / `OzConfig::production()` stay — still used by `oss.rs`.
- `app/build.rs::generate_channel_config_if_needed` stays — it already returns gracefully when
  the generator is absent, and remains correct for internal builds that have it. No change.

## Testing

1. **Unit** (`crates/warp_core/src/channel/config_tests.rs`): `no_backend()` builds; serde
   round-trips; every URL parses via `url::Url::parse` (proves no startup panic); firebase key
   empty; all optional configs `None`; `app_id` renders as expected.
2. **Compile**: `cargo build --bin stable --bin warp --features release_bundle,extern_plist`
   succeeds (proves the generator dependency is gone).
3. **End-to-end**: `make ship` produces `/Applications/WarpLocal.app` and publishes a GitHub
   Release with `Clinch.dmg`.
4. **Runtime smoke**: launch the built app; confirm it opens a terminal and does not panic.

## Risks

- **Empty-URL panic** — mitigated by using integration.rs's black-hole URLs (parseable).
- **Feature that assumes a live backend panics at startup** — covered by the runtime smoke
  test; if found, gate that feature on a reachable server rather than reverting to a real URL.
