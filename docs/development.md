# Building, releasing, and maintenance

## Build and verify from source

```bash
./script/bootstrap
./script/install_cargo_test_deps
./script/launch-check
make candidate
```

`script/launch-check` covers formatting, installer/integration/update-format tests, the stable
build, component tests, Clippy, dependency license policy, bundled notices, and advisories. It
checks licenses and advisories first, because the advisory database is a live feed that can fail an
unchanged commit. `./script/launch-check --quick` skips the build, Rust tests, Clippy, and
advisories; it is the check every pull request runs on GitHub Actions. `make candidate` builds and
verifies the native Apple Silicon app, ZIP, and DMG, both manifest signatures, minimum OS,
entitlements, default-on/opt-out flow, and the ZIP/DMG app pair. It does not create a tag or
release.

## Releases

Releases are Apple Silicon only by default. Build Intel only when it is explicitly wanted:
`INTEL=1` adds a separate native Intel app, ZIP, and DMG, and `UNIVERSAL=1` ships one legacy fat
app instead. Both work with `make candidate` and `make release`. Builds and the gate run under
`nice` (`RELEASE_NICE`, default 10) so the Mac stays responsive.

Release builds use the persistent local Cargo cache instead of paid hosted macOS runners. Install
the normal build/test/release dependencies plus OpenSSL 3 and Syft (`brew install openssl@3 syft`)
before the first release; `make release` checks every required command, signing key, and at least
40 GiB of free space before starting the expensive gate.

Run `make release` from a clean, current `main` checkout. It selects the next version, records the
exact commit, and creates a detached worktree pinned to that commit at a stable per-repository path.
The worktree uses the dedicated `target/release-worktree-cache/` directory, and its stable source
path keeps incremental Cargo fingerprints reusable across releases without conflicting with normal
development builds. Edits made in the caller's checkout after the release starts cannot dirty or
change the release source. The command runs the full local gate, builds and verifies the native
Apple Silicon artifacts (and Intel ones with `INTEL=1`), generates a CycloneDX SBOM for each app, a
vendored offline-buildable Corresponding Source archive, and signed local provenance, then
assembles the exact signed asset set under `target/release-worktree-cache/release-stage/<version>/`.
The release-only Nextest profile permits up to 120 seconds for macOS UI tests whose first system-font
scan can consume most of the stricter 60-second local timeout; test assertions and selection are
unchanged.

After a version-and-commit-specific publication confirmation, it pushes the signed tag, creates or
refreshes a private draft release, downloads the uploaded assets into a fresh directory, verifies
them again, and publishes the draft. The command does not require or record a manual QA
attestation. A running installation discovers the release on its next at-most-hourly check; choose
**Clinch → Check for Updates…** to refresh immediately without resetting the automatic privacy
cadence. Advanced users can override `VERSION`.

Release publication runs no GitHub Actions job. Immediately before publication, the local command
rechecks the signed remote tag, current `main`, both manifest signatures, exact asset set and
digests, SBOM, validation record, local provenance, and monotonic update sequence. It also rejects
a draft whose metadata changes during or after download. A failure leaves the draft private and
safe to retry. Verified progress is stored by commit under
`target/release-worktree-cache/release-resume/`: a retry first revalidates existing candidate or
staged artifacts and skips completed build phases only when those checks still pass. Set
`CLINCH_RELEASE_RESUME=0` to force a fresh run.

An `INTEL=1` release builds Apple Silicon and Intel bundles separately and reuses one generated
settings schema across them. The verifier requires the large Clinch executable in each archive to
contain exactly its advertised native slice. The small updater swap tool, jq helper, and dock-tile
plugin remain universal so either architecture can safely complete an update. A `UNIVERSAL=1`
build compiles its two executable slices one after the other; `CLINCH_PARALLEL_ARCH_BUILDS=1`
(or `auto`, on hosts with at least 32 GiB of RAM and 8 logical CPUs) builds them in parallel at
roughly twice the peak memory, and `CLINCH_PARALLEL_ARCH_JOBS` sets the jobs per architecture.

If a release fails partway through the gate, rerunning `make release` for the same commit skips
the gate stages that already passed; the license and advisory checks always run again.

After these changes land on `main`, run `make configure-release-repository` once. It validates both
workstation key copies before deleting the obsolete GitHub signing secrets and `public-release`
environment, then reapplies branch, scanning, Actions-token, and immutable-release controls.

## Local disk and resource maintenance

From a source checkout, `make worktrees` previews registered worktrees, their disk use, and
what prevents cleanup. It deletes nothing. Once a worktree is finished, close its tabs and
processes, review its ignored files in the preview, then select its exact path:

```bash
./script/clean-worktrees --remove "$HOME/.clinch/worktrees/clinch-terminal/finished-branch"
./script/clean-worktrees --remove "$HOME/.clinch/worktrees/clinch-terminal/finished-branch" --apply
```

Removal requires a clean, unlocked, attached worktree under the managed worktree directory,
with its commits merged into the selected base. The default base is local `main`, `master`,
or `origin/HEAD`; use `--base` for another branch. This check does not fetch and conservatively
keeps squash-merged branches whose commits are not ancestors of the base. Primary/current
checkouts, symlink paths, indexes that hide tracked changes, and checkouts used as one of your
running processes' working directories are protected. Paths with backslashes, control characters, or
non-ASCII characters are kept because process inspection may escape them. If process inspection
fails, cleanup is blocked. Checks run again before removal;
Git performs the final removal without force, and the branch is preserved.

Removing a checkout **also removes ignored files**, including build outputs and local `.env`
files, and means Undo Close can no longer reopen that checkout. Move any local files you want
to keep first. Process checks are a snapshot: don't start new work in a checkout while removing
it. Tab closure alone never deletes a worktree. `--repo`, `--managed-root`, `--json`, and `--size`
are available for other repositories and scripted previews. To reclaim only regenerable Cargo
caches while retaining checkouts and compiled dependencies, use `make prune` (default: seven
days old), or preview with `./script/reclaim-build-space --dry-run`.

On macOS, `make resources` watches the Clinch process bound to the current terminal. It uses
OS counters every five seconds and displays CPU, resident memory, full physical footprint
(including compressed memory), and separate totals for child processes such as shells and
coding agents. CPU at 100% means one core. Child totals are approximate: shared memory may be
counted more than once, short-lived processes may exit between samples, and unreadable children
are counted separately. It reads no terminal text or provider transcripts and sends nothing
over the network. To record a bounded sample from another terminal:

```bash
./script/watch-clinch-resources --pid 12345 --samples 60 --csv /tmp/clinch-resources.csv
```

Replace `12345` with the Clinch PID from Activity Monitor. CSV output creates a new file and
refuses to overwrite an existing one. Ctrl-C stops an interactive watch; nothing is installed
to run in the background. Clinch also logs a local warning when its own footprint reaches
2 GB, with resident memory and estimated open/Undo Close terminal retention. That warning
re-arms only after the footprint falls below 1.5 GB.
