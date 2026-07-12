# Clinch

> **A local-only fork of [Warp](https://github.com/warpdotdev/warp) that brings your CLI agents back when you reopen it.** macOS only.

**macOS** · open source · no sign-in · no account · never phones home · **[clinch.sh »](https://clinch.sh)**

Quit Clinch with Claude Code or Codex running in your tabs, reopen it, and each tab returns with its agent resumed (`claude --resume` / `codex resume`) — not a dead shell.

**Jump to:** [Download](#download) · [Is this safe?](#is-this-safe) · [Privacy & telemetry](#privacy--telemetry) · [Clinch vs Warp](#how-clinch-differs-from-warp) · [Build from source](#build-from-source)

## Download

### Install with one command (recommended)

```bash
curl -fsSL https://clinch.sh/install | sh
```

The [install script](install.sh) downloads the latest `Clinch.app.zip` from this repo's GitHub Releases, prints its SHA-256 so you can compare it against the digest on the release page, moves **Clinch.app** into /Applications, and opens it. Because curl (not a browser) does the download, macOS never sets the quarantine flag — no Gatekeeper warning, no `xattr` step.

### Or download manually

[⬇ Download Clinch for macOS](https://github.com/elliot-ylambda/clinch-terminal/releases/latest/download/Clinch.app.zip)

1. **(Recommended) Verify the download.** Each release attaches a `Clinch.app.zip.sha256` (the same digest GitHub shows next to the asset on the release page):
   ```bash
   shasum -a 256 -c Clinch.app.zip.sha256
   ```
2. Unzip and move **Clinch.app** to **/Applications**.
3. Clinch is open source but **not notarized** by Apple, so macOS quarantines **browser-downloaded** copies. Clear the flag once, then open it:
   ```bash
   xattr -dr com.apple.quarantine /Applications/Clinch.app
   ```

Either way, Clinch boots straight to a terminal — no login. It co-installs next to your real Warp without conflict (separate bundle id and data dir), so you can keep both.

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
- **Verify what you downloaded.** Each release attaches a `Clinch.app.zip.sha256` file, and GitHub displays the same SHA-256 digest on the release page; `shasum -a 256 -c Clinch.app.zip.sha256` confirms the bytes are exactly what's published here.
- **Why the `xattr` step (manual downloads only)?** Apple's notarization (the "we scanned this" stamp) requires a paid Developer account this project doesn't have. The app *is* code-signed — just not notarized — so Gatekeeper quarantines browser downloads; the command clears that flag. It's the same reason many independent open-source Mac apps need it. The `curl` installer never triggers it, because curl downloads don't get the quarantine flag.
- **No telemetry, no account, no phone-home.** Clinch never signs in, and authenticated calls to Warp's servers hard-fail by design — see [Privacy & telemetry](#privacy--telemetry) for the specifics and how to verify it yourself.
- **`install.sh` is auditable.** The optional agent-resume installer only adds `SessionStart` hooks to `~/.claude/settings.json` (a non-destructive `jq` merge) and `~/.codex/config.toml`, and sources its replay functions from `~/.zshrc`. Read [`tools/agent-resume/install.sh`](tools/agent-resume/install.sh) before running it.

## Privacy & telemetry

**Clinch sends no telemetry and makes zero calls to Warp's backend.** This isn't a pinky-promise — it's how the build is compiled, and every claim below is verifiable:

- **No telemetry or analytics.** The released app is the `stable` binary, built from [`app/src/bin/stable.rs`](app/src/bin/stable.rs) with [`ChannelConfig::no_backend()`](crates/warp_core/src/channel/config.rs), which sets `telemetry_config`, `crash_reporting_config`, and `autoupdate_config` to `None`. No analytics write-keys or DSNs are baked in, and crash reporting (Sentry) isn't compiled into the binary at all. The telemetry code that exists upstream has no destination to send to and is gated off.
- **No backend, no sign-in.** `no_backend()` reports `has_backend() == false` — the login and cloud surfaces never initialize — and points every server URL at `http://192.0.2.0:9`, a reserved, unroutable test address. Clinch cannot reach Warp's servers even if something tried.
- **Verified at runtime.** The installed app's process is named `stable` (inside `Clinch.app`). While running, it holds zero outbound connections of its own:
  ```bash
  lsof -nP -i -a -p "$(pgrep -f 'Clinch.app/Contents/MacOS/stable' | paste -sd, -)" | grep ESTABLISHED
  # no output = no connections. (If you enable the optional plan-limit gauges,
  # you may see one connection to api.anthropic.com — see below.)
  ```
  Or just block it: add a firewall / Little Snitch rule denying `*.warp.dev`, and Clinch keeps working — because it needs nothing from them.

**What this does _not_ cover (honestly):**

- **Your CLI agents talk to their own providers.** Claude Code reaches Anthropic, Codex reaches OpenAI, MCP servers reach wherever you point them. That traffic is *theirs*, not Clinch's — the terminal only hosts them. So if you watch the wire you'll see your agents' connections; you won't see Warp's.
- **The optional plan-limit gauges query Anthropic — off by default.** If you turn on **Settings → Features → Show Claude Code live plan limits** (or run "Enable Claude Code live plan limits" from the Command Palette), Clinch reads Claude Code's OAuth token from your macOS Keychain (macOS asks for your permission first) and calls `https://api.anthropic.com/api/oauth/usage` to show your own rate-limit usage in the tab bar. The token goes only to Anthropic — the same host Claude Code itself sends it to — and nowhere else. Leave the setting off and Clinch never touches your Keychain or Anthropic; the local cost/token stats still work by scanning local `~/.claude` and `~/.codex` files (the Codex gauges are always local-only — no keychain, no network).
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
./script/bootstrap                    # platform setup (Xcode + Rust)
./script/bundle -c stable --selfsign  # build + self-sign Clinch.app (and a DMG)
./tools/agent-resume/install.sh       # install the agent-resume hooks, then restart your shell
```

This builds the same `stable` binary the releases ship — the no-backend build described in [Privacy & telemetry](#privacy--telemetry). The bundled app lands under `target/<arch>/release/bundle/osx/Clinch.app`; move it into /Applications. Bundling requires `create-dmg` (`brew install create-dmg`). For quick iteration, plain `cargo run` compiles and launches a dev build without bundling.

## License & attribution

Clinch is a modified version of [warpdotdev/warp](https://github.com/warpdotdev/warp), licensed under [AGPL-3.0](LICENSE-AGPL) (the `warpui_core` and `warpui` crates remain [MIT](LICENSE-MIT)). The functional changes versus upstream are the agent-session-resume feature and the local-only (no-login) build.

**Not affiliated with Warp or Denver Technologies, Inc.** "Warp" is their trademark; "Clinch" is an independent, unofficial fork and is not endorsed by them.
