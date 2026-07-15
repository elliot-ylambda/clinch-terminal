# Clinch public-preview release QA

`make release` creates a release issue from this checklist after explicit operator confirmation.
For a separately maintained record, copy this checklist into a release issue or another durable
public record. Do not dispatch the release workflow until every required item passes. Put the
record URL or identifier in the `qa_record` workflow input.

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
- [ ] Session capture is off on a clean install and first launch.
- [ ] Session capture opt-in shows the affected paths and adds only Clinch-managed entries.
- [ ] Session capture removal preserves unrelated Claude Code and Codex configuration.
- [ ] Capture purge lists and removes only the Clinch capture directory.
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

- [ ] PASS — eligible for the protected public-preview workflow.
- Notes:
