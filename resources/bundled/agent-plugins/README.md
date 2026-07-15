# Bundled Warp agent plugins

Clinch installs these local marketplace snapshots into Codex and Claude Code on startup when
the corresponding CLI is present. `UPSTREAM_REVISIONS` records each exact upstream commit,
plugin version, and SHA-256 digest of `git archive <commit> plugins/warp`. Each marketplace
contains only the `warp` notification plugin and uses a Clinch-specific marketplace id, so it does
not replace a user's upstream marketplace or remove Oz plugins. Oz orchestration plugins remain
outside this bundle.

The provider CLIs copy installed plugins into their own user-level caches. The startup installer
re-runs only when the installed plugin is missing or older than the bundled version.

Both upstream plugins use `jq`. Clinch ships jq 1.8.2 as a universal macOS executable in its
pane `PATH`, so users do not need to install that runtime dependency separately.
