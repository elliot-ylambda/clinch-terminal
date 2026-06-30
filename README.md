# Clinch

**A local-only fork of [Warp](https://github.com/warpdotdev/warp) that brings your CLI agents back when you reopen it.** macOS only.

Quit Clinch with Claude Code or Codex running in your tabs, reopen it, and each tab returns with its agent resumed (`claude --resume` / `codex resume`) — not a dead shell. No sign-in, no account, never phones home.

## Download

### [⬇ Download Clinch for macOS](https://github.com/elliot-ylambda/clinch-terminal/releases/latest/download/Clinch.app.zip)

1. **(Recommended) Verify the download.** Each release ships a `Clinch.app.zip.sha256` — see [Is this safe?](#is-this-safe) below:
   ```bash
   shasum -a 256 -c Clinch.app.zip.sha256
   ```
2. Unzip and move **Clinch.app** to **/Applications**.
3. Clinch is open source but **not notarized** by Apple, so macOS quarantines downloaded copies. Clear the flag once, then open it:
   ```bash
   xattr -dr com.apple.quarantine /Applications/Clinch.app
   ```
   Clinch boots straight to a terminal — no login. It co-installs next to your real Warp without conflict (separate bundle id and data dir), so you can keep both.

### Enable agent-session resume

Resume needs a one-time set of capture hooks for your CLI agents:

```bash
git clone https://github.com/elliot-ylambda/clinch-terminal.git
cd clinch-terminal && ./tools/agent-resume/install.sh
# then restart your shell (or: source ~/.zshrc)
```

This wires `SessionStart` hooks for Claude Code and Codex (your existing settings are preserved) so Clinch knows which session each tab was running. Requires `jq` (`brew install jq`).

## Is this safe?

Fair question — you should be skeptical of any app that asks you to clear macOS quarantine. The honest picture:

- **It's open source.** Every line is in this repo under [AGPL-3.0](LICENSE-AGPL). The most trustworthy way to run Clinch is to **[build it yourself](#build-from-source)** — then you aren't trusting a binary from anyone.
- **Verify what you downloaded.** Each release publishes a SHA-256 and a `Clinch.app.zip.sha256` file; `shasum -a 256 -c Clinch.app.zip.sha256` confirms the bytes are exactly what's published here.
- **Why the `xattr` step?** Apple's notarization (the "we scanned this" stamp) requires a paid Developer account this project doesn't have. The app *is* code-signed — just not notarized — so Gatekeeper quarantines the download; the command clears that flag. It's the same reason many independent open-source Mac apps need it.
- **No telemetry, no account, no phone-home.** Clinch never signs in, and authenticated calls to Warp's servers hard-fail by design — see [Privacy & telemetry](#privacy--telemetry) for the specifics and how to verify it yourself.
- **`install.sh` is auditable.** The optional agent-resume installer only adds `SessionStart` hooks to `~/.claude/settings.json` (a non-destructive `jq` merge) and `~/.codex/config.toml`, and sources its replay functions from `~/.zshrc`. Read [`tools/agent-resume/install.sh`](tools/agent-resume/install.sh) before running it.

## Privacy & telemetry

**Clinch sends no telemetry and makes zero calls to Warp's backend.** This isn't a pinky-promise — it's how the build is compiled, and every claim below is verifiable:

- **No telemetry or analytics.** The build sets `telemetry_config`, `crash_reporting_config`, and `autoupdate_config` to `None` ([`app/src/bin/oss.rs`](app/src/bin/oss.rs)). No analytics write-keys or DSNs are baked in, and crash reporting (Sentry) isn't compiled into the binary at all. The telemetry code that exists upstream has no destination to send to and is gated off.
- **No backend, no sign-in.** Built with the `skip_login` feature: there's no login screen, and *every* authenticated request to Warp's servers hard-fails by design ([`crates/warp_server_client/src/auth/session.rs`](crates/warp_server_client/src/auth/session.rs)). It cannot phone home even if something tried.
- **Verified at runtime.** While running, the `warp-oss` process holds **zero** outbound network connections. See for yourself:
  ```bash
  lsof -nP -i -a -p "$(pgrep -x warp-oss | paste -sd, -)" | grep ESTABLISHED
  # no output = no connections
  ```
  Or just block it: add a firewall / Little Snitch rule denying `*.warp.dev`, and Clinch keeps working — because it needs nothing from them.

**What this does _not_ cover (honestly):**

- **Your CLI agents talk to their own providers.** Claude Code reaches Anthropic, Codex reaches OpenAI, MCP servers reach wherever you point them. That traffic is *theirs*, not Clinch's — the terminal only hosts them. So if you watch the wire you'll see your agents' connections; you won't see Warp's.
- **One image-only exception.** A code path exists for fetching some static assets (e.g. certain theme background images) from Warp's asset server, with bundled fallbacks. It's a *download*, never a *send*, and runtime monitoring shows it inactive — but it's the one place we won't claim "literally never contacts any Warp host."

Bottom line: **Clinch itself collects nothing, reports nothing, and phones home to no one.** It's open source — audit it or watch the wire; don't take our word for it.

## How Clinch differs from Warp

|  | Clinch | Warp |
|---|---|---|
| **Agent-session resume** | ✅ reopens each tab **and** re-launches the Claude Code / Codex agent it was running | restores the shell; the agent is gone |
| **Sign-in** | none — fully local, never contacts Warp's servers | account required |
| **Warp AI, Drive, teams, session sharing** | removed (can't run without Warp's backend) | included |
| **Platform** | macOS only | macOS / Linux / Windows |
| **Bring your own CLI agent** (Claude Code, Codex) | ✅ | ✅ |

The only functional addition is **agent-session resume** — see [`tools/agent-resume/`](tools/agent-resume/) for how it works. Everything else is Warp with the login and cloud surfaces stripped out.

## Build from source

```bash
./script/bootstrap                   # platform setup (Xcode + Rust)
./tools/agent-resume/build-app.sh    # build + install a self-contained Clinch.app to /Applications
./tools/agent-resume/install.sh      # install the agent-resume hooks, then restart your shell
```

`build-app.sh` makes a release build, so the installed app is fully self-contained — you can move, rename, or delete this repo afterward and Clinch still opens. `CLINCH_NAME="…"` renames the app; `CLINCH_DEBUG=1` does a faster, source-tethered dev build.

## License & attribution

Clinch is a modified version of [warpdotdev/warp](https://github.com/warpdotdev/warp), licensed under [AGPL-3.0](LICENSE-AGPL) (the `warpui_core` and `warpui` crates remain [MIT](LICENSE-MIT)). The functional changes versus upstream are the agent-session-resume feature and the local-only (no-login) build.

**Not affiliated with Warp or Denver Technologies, Inc.** "Warp" is their trademark; "Clinch" is an independent, unofficial fork and is not endorsed by them.
