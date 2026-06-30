# Clinch

**A local-only fork of [Warp](https://github.com/warpdotdev/warp) that brings your CLI agents back when you reopen it.** macOS only.

Quit Clinch with Claude Code or Codex running in your tabs, reopen it, and each tab returns with its agent resumed (`claude --resume` / `codex resume`) — not a dead shell. No sign-in, no account, never phones home.

## Download

### [⬇ Download Clinch for macOS](https://github.com/elliot-ylambda/clinch-terminal/releases/latest/download/Clinch.app.zip)

1. Unzip and move **Clinch.app** to **/Applications**.
2. Clinch is signed with a development certificate (not notarized), so clear the download quarantine once:
   ```bash
   xattr -dr com.apple.quarantine /Applications/Clinch.app
   ```
   Then open it normally — Clinch boots straight to a terminal, no login.

It co-installs next to your real Warp without conflict (separate bundle id and data dir), so you can keep both.

### Enable agent-session resume

Resume needs a one-time set of capture hooks for your CLI agents:

```bash
git clone https://github.com/elliot-ylambda/clinch-terminal.git
cd clinch-terminal && ./tools/agent-resume/install.sh
# then restart your shell (or: source ~/.zshrc)
```

This wires `SessionStart` hooks for Claude Code and Codex (your existing settings are preserved) so Clinch knows which session each tab was running. Requires `jq` (`brew install jq`).

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
