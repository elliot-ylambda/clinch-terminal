# Contributing to Clinch

Clinch is a small, solo-maintained fork of
[warpdotdev/warp](https://github.com/warpdotdev/warp) focused on one thing:
resuming your CLI agent sessions (Claude Code, Codex) when the terminal
restarts. It is **not affiliated with Warp** — please don't take Clinch
questions to Warp's community channels or issue tracker.

## Bugs & feature requests

Open a [GitHub issue](https://github.com/elliot-ylambda/clinch-terminal/issues).
Include your Clinch version (**Settings → About**, or the release tag you
installed) and your macOS version.

For security vulnerabilities, see [SECURITY.md](SECURITY.md) — please don't
open a public issue.

## Pull requests

PRs are welcome and reviewed on a best-effort basis. Before opening one:

1. `./script/format`
2. `cargo clippy --workspace --all-targets --all-features --tests -- -D warnings`
3. `cargo nextest run --no-fail-fast --workspace --exclude command-signatures-v2`

Build-from-source and verification commands are documented in the
[README](README.md#build-and-verify-from-source).

## Code of conduct

The [Contributor Covenant](CODE_OF_CONDUCT.md) applies in all project spaces.
