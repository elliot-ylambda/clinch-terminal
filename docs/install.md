# Install, update, and remove Clinch

## Install

```bash
curl -fsSL https://clinch.sh/install | sh
```

The URL redirects to the authenticated `install.sh` asset on the latest versioned release. The
script authenticates the signed release manifest with a Clinch release key embedded in the
reviewed script. It resolves one exact tag, verifies the archive size and SHA-256, bundle ID,
version, minimum macOS version, the Apple Silicon architecture, and structural code signature,
then stages the app in `/Applications` or `~/Applications` and opens it (pass `--no-open` to
skip launching). It does not configure Claude Code or Codex, install plugins, request
administrator access, or change Gatekeeper. Command-line downloads carry no browser quarantine
flag, so Gatekeeper does not block the first launch and no System Settings approval is needed.
To read the script before running it, download `install.sh` from a versioned release, inspect
it, then run `sh install.sh`.

Clinch uses the bundle ID `sh.clinch.Clinch` and keeps its UI configuration in `~/.clinch`, so it
can coexist with Warp without inheriting Warp's theme, zoom, keybindings, or other app settings.

## Manual install

Every current versioned release on the
[Clinch releases page](https://github.com/elliot-ylambda/clinch-terminal/releases) also ships
the artifacts to verify and install by hand. Download `Clinch.dmg`, plus `Clinch.source.tar.gz`,
`Clinch.checksums.txt`, and `Clinch.checksums.sshsig` from the same release. The source archive is
the version-matched Complete Corresponding Source, including every locked Cargo dependency.
Authenticate the checksum list with the Clinch release key, then verify the disk image and source
archive before opening or extracting them:

```bash
printf '%s\n' 'clinch-release ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGr+qT8+Fx8TjATDpWlhzzfbL08AsS1EXbaaOUBi0wJp' > /tmp/clinch-allowed-signers
ssh-keygen -Y verify -f /tmp/clinch-allowed-signers -I clinch-release -n clinch-checksums \
  -s Clinch.checksums.sshsig < Clinch.checksums.txt
shasum -a 256 Clinch.dmg
grep ' Clinch.dmg$' Clinch.checksums.txt
shasum -a 256 Clinch.source.tar.gz
grep ' Clinch.source.tar.gz$' Clinch.checksums.txt
```

`ssh-keygen` must report a good signature, and each printed digest must match its authenticated
entry. Open the DMG and drag `Clinch.app` to Applications. A browser download is quarantined and
the preview is not notarized, so macOS will block the first launch: try once, then approve Clinch
under **System Settings → Privacy & Security → Open Anyway**. Do not disable Gatekeeper globally.
Clinch's installer and documentation do not remove `com.apple.quarantine`.

## Intel Macs

Releases ship for Apple Silicon only. The installer refuses an Apple Silicon-only release on an
Intel Mac before downloading anything and names the last Intel release, which it can still install
with `--version`.

Intel installations stay on the last Intel release: an Apple Silicon-only manifest lists only
`arm64`, so the installer refuses it on Intel, and current updaters do not offer it there. Updaters
older than that check still offer it, but its download then fails the architecture check before
anything is installed.

## Public preview and signing

Clinch is distributed as an **unnotarized public preview**. The app is signed with Clinch's own
self-signed certificate and the macOS hardened runtime, but it has no Apple-issued Developer ID and
Apple has not notarized it. macOS may block the first launch. That warning is expected and is not
evidence that Apple has reviewed the app. Releases use the same signing identity so privacy
permissions you grant Clinch, such as Screen Recording, can survive updates. Upgrading from an
older signing identity may require the
[one-time repair](#permissions-are-enabled-but-the-agent-still-reports-access-denied) below.

This repository has automated security checks and a documented release process. It has not had an
independent security audit. See [SECURITY.md](../SECURITY.md) for the trust model and known limits.

## Updates

Updater-enabled builds check automatically and show **Update** in the header. You can also choose
**Clinch → Check for Updates…**. Either way, Clinch authenticates the release and a small pop-up
asks before anything is installed. **Update** then shows a progress bar while it downloads and
verifies the archive, saves restorable state, quits, atomically replaces the app, reopens, and
rolls back if the new app does not start. Sessions that were running reopen and continue; see
[Sessions](sessions.md). The pop-up also says which other running commands will stop.

The updater never requests administrator privileges. The public installer places Clinch in a
user-writable `/Applications` or `~/Applications` location; unusual non-writable installations
open the authenticated release for manual installation instead. Builds released before the updater
bridge in `v0.2026.07.20.1643` need one final bootstrap install:

```bash
curl -fsSL https://clinch.sh/install | sh
```

To turn automatic checks off, see [Privacy](privacy.md#turn-off-automatic-update-checks).

## Uninstall

The release asset `uninstall.sh` supports selective removal:

```bash
bash uninstall.sh                         # app only
bash uninstall.sh --disable-integration  # app plus managed hooks/helpers
bash uninstall.sh --purge-capture        # also remove captured metadata
bash uninstall.sh --purge-app-data       # also remove Clinch preferences/cache/state
bash uninstall.sh --remove-plugins       # also remove the bundled provider plugins
```

Run `bash uninstall.sh --help` for combinations. It never removes all of `~/.warp`, provider
transcripts, or Keychain credentials.

## Computer use permissions on macOS

Ordinary terminal use does not require Screen Recording or Accessibility. Grant them when an
agent such as Claude Code asks to see your screen or control other apps. In-app instructions and
links to the macOS permission panes are under **Clinch Settings → Computer use → Setup and
troubleshooting**. Opening the guide does not request or change permissions.

For a new installation:

1. Open **System Settings → Privacy & Security → Accessibility** and enable **Clinch**. This
   allows the agent to click, type, and scroll in other apps.
2. Open **Screen & System Audio Recording** (called **Screen Recording** on older macOS versions)
   and enable **Clinch** in the screen-recording list, not the audio-only list. This allows the
   agent to see your screen. If Clinch is missing from either pane, use **+** to add the copy you
   run, normally `/Applications/Clinch.app`.
3. Quit and reopen Clinch, resume your agent, and retry computer use. In Claude Code, choose
   **Try again** if its permission prompt is still open.

### Permissions are enabled, but the agent still reports access denied

Older Clinch builds used ad-hoc or Apple Development signing. When upgrading to the current
**Clinch Code Signing** identity, macOS can retain permission entries for the old identity.
Both switches may appear enabled even though access is denied. Toggling them or restarting alone
may not replace the stale entries.

1. Save your work and quit Clinch.
2. In **each** permission pane above, select **Clinch** and click **−** to remove its entry.
3. Click **+**, select the current `/Applications/Clinch.app` (or the actual copy you run), and
   enable it. Repeat for both Accessibility and Screen Recording.
4. Reopen Clinch, resume your agent, and retry computer use.

Only remove Clinch's entries. **Clinch Dev** is a separate app with separate permissions; grant
access to the app you actually run. Once granted to the current release identity, permissions
should carry across releases that retain that identity. This repair does not grant an agent
access to individual apps within its own computer-use approval flow.
