# Clinch in-app updates

## Summary

Clinch can discover, authenticate, install, and recover from stable application updates without
requiring users to rerun a terminal installer after every release. Update discovery is quiet,
installation always requires explicit consent, and an update must never put saved windows,
projects, tabs, or Claude/Codex conversations at risk.

## Figma

Figma: none provided. The feature reuses the existing macOS menu, Settings update status, and
platform-native confirmation dialog.

## Behavior

1. A released Clinch app checks stable update metadata at most once per active calendar day. A user
   can also choose **Clinch → Check for Updates…** or the equivalent Settings/command-palette action
   at any time.

2. An automatic check downloads metadata only. Clinch never downloads an application archive or
   changes the installed app until the user explicitly chooses **Download and Install**.

3. A manual check reports one of the following accessibly: Clinch is current, an update is
   available, the check failed, or the update metadata could not be authenticated. Automatic-check
   failures remain non-disruptive and are retried on a later eligible check.

4. When an update is available, existing update surfaces show the target version. Choosing the
   install action opens a macOS confirmation dialog containing the current and target versions,
   release notes, and **Download and Install** and **Later** actions. **Later**, Escape, or closing
   the dialog leaves the installation and downloaded state unchanged.

5. **Download and Install** downloads the exact archive named by authenticated release metadata.
   Clinch shows downloading/installing state and prevents concurrent checks or installations.

6. Clinch rejects an update before quitting if any authenticated metadata, archive hash/size,
   release ordering, bundle identity/version, executable architecture, archive layout, or complete
   code-signature check fails. The installed app remains untouched and the user can retry.

7. A normal update cannot downgrade Clinch. An older release is accepted only when its signed
   metadata explicitly marks it as a rollback and its monotonic release sequence is valid.

8. If the current installation is writable, the verified update installs without an authorization
   prompt. If it is not writable, macOS requests administrator authorization after download and
   before Clinch quits. Canceling or failing authorization leaves Clinch running and unchanged.

9. Once the installer helper is ready, Clinch refreshes agent ownership, snapshots recovery data,
   requests a normal cancellable quit, and saves the final physical-window, project-tab,
   terminal-tab, split-pane, and Claude/Codex restore state. Canceling that quit cancels the helper
   and leaves the verified update available for a later attempt.

10. The helper waits for the exact old Clinch process to exit before touching its bundle. It keeps
    the previous bundle as a rollback candidate, installs the staged bundle atomically, clears the
    quarantine flag, and relaunches Clinch with a clean Dock-like environment.

11. A successful first frame from the new app acknowledges the update. The helper then removes the
    rollback bundle and temporary update files. Clinch restores the saved windows, projects, tabs,
    splits, and agent sessions through the normal durable restore path.

12. If installation or initial launch fails, the helper restores and relaunches the previous
    bundle. Recovery databases, registry journals, prompt mirrors, and provider transcripts are
    never deleted by installation or rollback.

13. All update metadata is signed by a Clinch release key embedded in the app. A manifest signed by
    a currently trusted key may introduce the next trusted key. Missing, unknown, malformed, or
    invalid signatures fail closed.

14. Maintainers publish updater metadata and its signature with every stable GitHub Release.
    `make release` fails before publishing when the signing key is missing or any generated release
    asset cannot be verified. `make update` remains a maintainer-only local workflow.

15. Existing builds without the updater require one final manual installation. The curl installer
    remains a supported bootstrap and recovery path and refuses to replace a running Clinch app.
