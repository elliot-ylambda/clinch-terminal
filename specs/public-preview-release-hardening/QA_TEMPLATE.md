# Clinch public-preview release QA

`make release` creates a release issue from this checklist after explicit operator confirmation.
For a separately maintained record, copy this checklist into a release issue or another durable
public record. Do not start remote staging until every required item passes. Pass the record URL
or identifier as `QA_RECORD`; it is embedded in the signed local validation record. The local
command re-downloads and verifies the private draft before publishing it without GitHub Actions.

## Candidate

- Version:
- Commit:
- Update sequence:
- Tester:
- Date:
- Mac model and CPU:
- macOS version:
- Untested macOS versions:

## Required checks

- [ ] The candidate update manifest and signatures authenticate with the committed Clinch keys.
- [ ] Clean manual install from the candidate DMG succeeds.
- [ ] First launch succeeds using the documented Privacy & Security approval when required.
- [ ] Authenticated manual upgrade preserves the existing app until replacement succeeds.
- [ ] Session capture is on after a clean install and first launch, with the affected paths
      disclosed, and adds only Clinch-managed entries.
- [ ] Session capture opt-out persists across relaunch; re-enable restores the managed entries.
- [ ] Session capture removal preserves unrelated Claude Code and Codex configuration.
- [ ] Capture purge lists and removes only the Clinch capture directory.
- [ ] No iMessage control appears in the header, Clinch Settings, or Claude Code/Codex footer.
- [ ] The app contains no Messages helper, Apple Events entitlement, or Apple Events usage
      description.
- [ ] App-only uninstall removes Clinch without removing capture data, provider transcripts,
      Keychain credentials, or unrelated `~/.warp` data.
- [ ] Startup while offline succeeds without a Warp account, telemetry, crash-reporting, or update
      request.
- [ ] Native Apple Silicon smoke test covers launch, a local shell, tabs/panes, settings, quit, and
      relaunch.

## Optional Intel check

- [ ] Native Intel smoke test covers launch and a local shell.
- Not run / reason:

## Result

- [ ] PASS — eligible for local signing, private staging, verification, and publication.
- Notes:
