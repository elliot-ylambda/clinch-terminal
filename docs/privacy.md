# Privacy and network behavior

**Clinch has no telemetry, no analytics, no crash reporting, and no account.** It does not
collect or transmit anything about you, your commands, your files, or how you use the app. No
usage data, no event stream, and no identifier ever leaves your machine. This is a property of
how the app is built, not a preference you have to find and switch off.

The stable channel is created by
[`ChannelConfig::clinch()`](../crates/warp_core/src/channel/config.rs), which ships
`telemetry_config: None`, `crash_reporting_config: None`, and no bundled MCP credentials. Every
inherited Warp backend URL—application server, real-time server, and Oz—is replaced with an
unroutable [RFC 5737](https://www.rfc-editor.org/rfc/rfc5737) black-hole address rather than
merely left unused, so no code path can deliver data to a Warp service even by accident. Account
login, onboarding, Warp Drive, session sharing, and Warp's own AI are gated behind
`ChannelState::has_backend()` and are inert. Every telemetry entry point—including the innermost
send—returns early when telemetry is unavailable, so the runtime schedules no telemetry task,
writes no queue to disk, and deletes stale Clinch telemetry queues without uploading them.

The shipped binary links no crash-reporting framework and contains no RudderStack destination,
Sentry DSN, or analytics SDK. The bundle requests no entitlements at all, and carries no
privileged update authorizer.

## What talks to the network

| Activity | Runs when | Destination |
| --- | --- | --- |
| Update check | Automatically, at most once an hour — **and you can turn it off** | `api.github.com`, this repository's releases |
| Update download | Only after you approve the update | GitHub release assets |
| Plan-limit gauges | **Off by default**; only if you enable them in Settings | `api.anthropic.com` for Claude. For Codex, your installed `codex app-server` asks its own service using your Codex login |
| [Remote Control](remote-control.md) | **Off by default**; only while you have it enabled | Your own tailnet, phone to Mac — no Clinch relay |
| Language servers, MCP servers, remote assets | Only when you start them | Wherever you point them |
| Claude Code, Codex, Devin, SSH, package tooling | Only when you run them | Their own services |

An update check uses ordinary HTTPS GETs for the release feed, manifest, and signature. Clinch
attaches no unique identifier, machine fingerprint, installed version, operating-system version,
or usage data to them, and it downloads no app archive until you approve the update. GitHub
necessarily sees ordinary transport metadata such as the requesting IP address and time. Clinch
records an automatic check before sending it, so successes, failures, cancellations, and app
restarts are all capped at one check per hour.

Software you launch inside Clinch is outside this statement. Provider CLIs, MCP servers, SSH, and
package tooling reach their own services under your own accounts, with their own privacy behavior.

## Turn off automatic update checks

**To make Clinch issue no automatic network requests at all**, turn off **Settings → Clinch →
Updates → Check for updates automatically**. Checking on demand from **Clinch → Check for
Updates…** keeps working, so turning this off does not strand you on an old build. The equivalent
without opening the GUI:

```bash
# In ~/.clinch/settings.toml
[clinch.updates]
automatic_check = false

# Or per-launch, which overrides the setting
CLINCH_NO_UPDATE_CHECK=1 open -a Clinch
```

## Verify it yourself

None of the above has to be taken on trust:

```bash
# No crash reporter linked, and no bundled frameworks at all
otool -L /Applications/Clinch.app/Contents/MacOS/stable | grep -i sentry
ls /Applications/Clinch.app/Contents/Frameworks

# No analytics destination or crash-reporting DSN compiled in
strings -a /Applications/Clinch.app/Contents/MacOS/stable \
  | grep -Ei 'rudderstack\.com|ingest\.sentry\.io|segment\.io|amplitude\.com'

# No entitlements requested (prints an empty <dict/>)
codesign -d --entitlements - /Applications/Clinch.app

# Every socket the running app holds open
lsof -nP -i -a -p "$(ps -Ao pid,comm | awk '/Clinch.app\/Contents\/MacOS\/stable/{print $1; exit}')"
```

Expected results: the two `grep` commands match nothing, `ls` reports that no `Frameworks`
directory exists, and `codesign` prints an empty `<dict/>`. Most of the time `lsof` shows only
`LISTEN` sockets on `127.0.0.1`, which accept connections from your own Mac and never reach the
network, plus GitHub connections around an hourly update check. Nothing should ever resolve to a
Warp service or an analytics host.

These invariants are also locked by tests in
[`config_tests.rs`](../crates/warp_core/src/channel/config_tests.rs): a change that reintroduced a
telemetry destination, a crash reporter, or a live backend URL fails the build.

## Local data and credentials

Session-capture data stays in local Clinch-owned files. Claude Code, Codex and Devin continue to
manage their own transcripts and credentials. Clinch does not delete provider transcripts or Keychain
credentials during uninstall.

Clinch launches the provider CLIs with their existing sign-ins. On macOS, Clinch's own Keychain
access never opens a password dialog. The optional Claude plan meter uses the existing Claude
Code login only when macOS permits a silent read; **Retry** follows the same rule. If access is
unavailable, plan limits remain unavailable while local transcript statistics and agent sessions
continue to work. The provider CLIs still control their own authentication flows.

Blocked Keychain access does not erase saved data or create replacement credentials. Local control
is managed in **Clinch Settings → Local control**, including the optional command installation for
other terminals. Clinch stores that non-secret choice in an owner-only local file, so app-signature
changes cannot make it unreadable. New installs default to enabled, and readable legacy Keychain
choices (including disabled) migrate automatically. If the old Keychain choice is unreadable or
invalid, the upgrade saves Clinch's enabled default without changing the legacy entry; no manual
toggle is needed. An existing local preference always takes precedence, and local file errors
leave control off rather than replacing a saved choice.

Remote Control still remains disabled when its saved authorization cannot be read. Stable app
signing is needed to preserve Keychain trust across updates; suppressing dialogs does not grant
access to an entry that trusts an older build.
