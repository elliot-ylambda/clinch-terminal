# Clinch agent notes

## Releases

- **Releases ship for Apple Silicon only.** `make release` and `make candidate` build only the
  native arm64 app by default. Do not add an Intel build on your own initiative: pass `INTEL=1`
  (separate native Intel app) or `UNIVERSAL=1` (one legacy fat app) only when the user explicitly
  asks for an Intel release in the current request. Older docs, scripts, tests, or past releases
  that mention Intel are not a reason to build it.
- After publishing a release that includes Intel, set `LAST_INTEL_RELEASE` in `install.sh` to that
  tag. Apple Silicon-only releases point Intel Macs there, both in the installer and in the release
  notes.
- Releases are signed by the committed self-signed **Clinch Code Signing** certificate so macOS
  privacy grants survive updates; `verify-clinch-release` refuses an ad-hoc build. See
  `SECURITY.md` for setting up the signing keychain on a release workstation.
- The publish step asks for a typed confirmation, so `make release` needs a real terminal on stdin.
  If the gate fails, rerunning for the same commit skips the stages that already passed.
