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
- [ ] Session capture is off on a clean install and first launch.
- [ ] Session capture opt-in shows the affected paths and adds only Clinch-managed entries.
- [ ] Session capture removal preserves unrelated Claude Code and Codex configuration.
- [ ] Capture purge lists and removes only the Clinch capture directory.
- [ ] Two-way iMessage is off before setup and accurately identifies missing Messages sign-in,
      Automation, and Full Disk Access states.
- [ ] iPhone calibration succeeds without an iPhone app; denying, granting, revoking, and
      regranting both macOS permissions pauses and resumes without duplicate delivery.
- [ ] Simultaneous Codex and Claude Code sessions receive distinct route codes; reply-to-message,
      explicit-code, ambiguous-selection, busy FIFO, blocked-prompt, and restart routing all reach
      only the intended durable session.
- [ ] Per-session opt-out and Disconnect stop delivery; Disconnect clears Clinch routing state
      without deleting Messages history.
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
