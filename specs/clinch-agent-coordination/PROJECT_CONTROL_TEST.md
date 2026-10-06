# Project control validation

Date: 2026-09-28. Base: `d9cd5350932af41ae8863c6118faddaa3a3b2895`.
Implementation branch: `codex/clinch-project-control`.

A signed development build passed a live CLI smoke test in a fresh `WARP_DATA_PROFILE`
with separate discovery and capture directories. Startup plugin/skill provisioning was
omitted from this disposable bundle. Existing Clinch apps stayed running; no existing
session was moved or closed. The shared development settings file was unchanged.
The disposable app and its test process exited after the checks.

Verified through the actual CLI and running app:

- Created two project tabs with explicit local directories.
- Listed and inspected an inactive project without changing the active project.
- Rejected stale project IDs.
- Created a sidebar section and changed its color and collapsed state.
- Moved a running process into an exact section position in another project. Its process
  PID, tab identity, title, and explicit color survived; the destination section expanded.
- Rejected an invalid destination position without removing the source session.
- Exported and restored section colors, tab titles, and split panes into a separate project
  with fresh identities. Changing the original section's color left the restored copy intact.
- Rejected an unsupported layout version without creating a project.

Automated coverage includes native project drag handoff, last-session transfers, project
task preservation, destination filter clearing and mouse release; exact inactive-project
targeting and window mismatch; layout validation and round trips; fresh pane identities;
explicit agent resume; CLI parsing/file limits; and managed skills. The main focused suite
passed 354 tests. A further 11 app/CLI checks passed after the final inspection changes;
44 protocol/client tests and 191 CLI tests passed after adding the older-app selector guard.
Clippy with warnings denied and repository formatting checks passed. See the commands below
for reproducible checks.

```sh
export CARGO_INCREMENTAL=0 WARP_CHANNEL=local WARP_BIN_NAME=warp
export FRAMEWORK_OVERRIDE=dev MACOSX_DEPLOYMENT_TARGET=14.0
cargo nextest run --locked -p warp -p local_control -p warp_cli --lib \
  --features gui,warp_control_cli --profile clinch-release --no-fail-fast \
  -E 'test(local_control) | test(project_window) | test(agent_skills) | test(transferred_sidebar_tab) | package(local_control) | package(warp_cli)'
cargo clippy --locked -p warp -p local_control -p warp_cli --lib --bin warp --tests \
  --features gui,warp_control_cli -- -D warnings
./script/format --check
```

The live test used a terminal process, not a paid provider turn. Provider resume identity
mapping is covered by native tests; this extension did not repeat real Claude/Codex login
or resume flows. Existing real-provider coordination evidence remains in [LIVE_TEST.md](LIVE_TEST.md).
Portable layouts omit shell/profile settings, launch flags, terminal output, and arbitrary
running commands. Mouse behavior was verified through native UI regression tests, not
computer-use automation. This validation does not cover a packaged production release.
