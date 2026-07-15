# Bundled jq for macOS

Clinch places this universal `jq` binary in `Contents/Resources/bin`, which is appended to the
`PATH` of every local pane. The pinned Warp notification plugins use it to parse provider hook
payloads, so a fresh macOS install does not require Homebrew or another system package manager.

`UPSTREAM_REVISION` records the official jq release asset digests and the deterministic universal
binary construction. The final app signing step replaces the source binary's ad-hoc signature.
