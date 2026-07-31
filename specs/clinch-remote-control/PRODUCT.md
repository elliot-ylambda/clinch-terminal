# Clinch Remote Control

## Summary

Clinch Remote Control lets a person securely view and control the Clinch instance on their Mac
from a phone or tablet. The first release uses the person's own Tailscale network, requires no
Clinch account or Clinch-hosted relay, and presents the most important Clinch project, tab, agent,
usage, and quick-insert controls in a touch-first installable web app.

## Problem

Long-running Claude Code, Codex, and terminal work often needs attention while the person is away
from their Mac. General-purpose SSH apps can start a shell, but cannot navigate the person's
existing Clinch projects and tabs, show agent state and usage, or safely target Clinch's existing
sessions.

## Goals

- Make Remote Control discoverable from the normal Clinch window and from clinch.sh.
- Make first-time setup understandable without requiring terminal commands.
- Work when the phone and Mac are on the same Wi-Fi or on different networks, including phone 5G,
  by using the person's Tailscale tailnet.
- Preserve Clinch's no-Clinch-account, zero-telemetry, local-first product promises.
- Provide a mobile interface designed around projects, tabs, agent attention, usage, composing
  input, and Clinch quick inserts.
- Treat remote terminal control as privileged access, with explicit pairing, visible targets, and
  desktop revocation.

## Non-goals

- Operating a public Clinch relay or requiring a Clinch account in the Tailscale release.
- Exposing a Mac directly to the public internet or using Tailscale Funnel.
- Reproducing every desktop setting and panel in the first mobile release.
- Running anything while the Mac is powered off, asleep, offline, or unable to run Clinch.
- Shipping photo/file upload in the first control milestone. The design reserves a composer
  attachment entry point and adds the shared upload pipeline in a later milestone.

## Figma

Figma: none provided. The first design artifact is an interactive narrow-screen web prototype,
tested on iPhone and iPad sizes before the live terminal transport is finalized.

## Behavior

### Desktop discovery and setup

1. Every backend-free Clinch window displays a phone-shaped **Remote Control** affordance in the
   right side of its header. It remains visible whether the person uses horizontal or vertical
   tabs and does not depend on a signed-in Warp account.

2. Hovering or focusing the affordance identifies it as “Remote Control” and explains that it
   connects a phone to this Mac. The affordance has an accessible name and a keyboard-reachable
   click target.

3. Activating the affordance opens **Clinch Settings** with **Remote Control** as the first visible
   section. Opening the same page from the normal settings menu produces the same section and
   setup state.

4. Before live pairing ships, the header affordance and settings section are labeled Preview and
   clearly say that pairing is not yet available in that build. No disabled or unfinished control
   is described as connected or ready.

5. The setup section explains, before the person opts in, that:
   - Tailscale must be installed and signed in on both the Mac and phone.
   - Both devices must belong to the same tailnet.
   - A personal Tailscale account is normally free, while organizational use follows the
     organization’s plan.
   - Clinch itself still requires no account and sends no analytics.
   - The Mac must be awake, online, and running Clinch.

6. Setup is presented as an ordered checklist, with one primary action per step:
   1. Install and connect Tailscale on the Mac.
   2. Install and connect Tailscale on the phone using the same tailnet.
   3. Enable Remote Control in Clinch.
   4. Scan the one-time pairing QR code and optionally add the web app to the phone’s Home Screen.

7. The Mac and phone installation actions open official Tailscale destinations. Clinch never
   silently installs a VPN, accepts an operating-system permission, creates a Tailscale account,
   or signs into an identity provider for the person.

8. When live setup is available, the section reports one of these concrete states rather than a
   generic failure:
   - Tailscale not installed.
   - Tailscale installed but stopped.
   - Tailscale signed out or not connected.
   - Tailscale connected, but private HTTPS is not enabled.
   - Ready to enable Remote Control.
   - Starting the local companion service.
   - Ready to pair.
   - Pairing QR active, including its remaining validity.
   - One or more paired devices connected or offline.
   - Setup needs attention, with a recoverable explanation and retry action.

9. A person can rerun setup checks without restarting Clinch. A failure never erases an existing
   paired-device authorization or changes unrelated Tailscale configuration.

10. Enabling Remote Control is explicit. Before activation, Clinch does not open a listener,
    configure a private Tailscale service, or make a Remote Control network request.

11. Disabling Remote Control stops new mobile connections. The person chooses whether to retain
    paired-device authorizations for the next enable or revoke them immediately.

### Pairing and device trust

12. Starting pairing shows a QR code, a copyable link for the same invitation, an expiration time,
    and a Cancel action. The invitation is single-use and expires after five minutes.

13. The QR invitation contains no terminal history, provider credential, Tailscale credential, or
    reusable Clinch password. Canceling, expiration, or successful use invalidates it.

14. The phone identifies itself with a locally generated device key. The person confirms the
    device name on the Mac before the first control session becomes authorized.

15. Once approved, the phone reconnects without scanning another QR code until its authorization
    is revoked, it exceeds the configured inactivity limit, its browser storage is cleared, or it
    leaves the tailnet.

16. Clinch Settings lists paired devices with name, platform, last-seen time, connection state,
    and granted capabilities. The desktop user can revoke any device immediately; a revoked phone
    cannot reconnect with an already-open page or replayed invitation.

17. Network membership and Clinch authorization are separate gates. Being on the same tailnet is
    never sufficient by itself to read or control Clinch.

18. The Tailscale release never enables Funnel, binds the companion service to a public interface,
    or displays a publicly routable Remote Control URL.

### Mobile application shell

19. The pairing link opens a responsive web app over private HTTPS. The app can be installed from
    Safari as a Home Screen app; installing it is optional and the same experience works in a
    normal browser tab.

20. The primary mobile layout has four stable regions:
    - A top horizontal project tab strip.
    - A compact header row with a tab-drawer button, current target/connection state, and a
      trailing overflow button.
    - The selected terminal or agent session as the focus area.
    - A bottom composer with quick inserts, explicit Send, and an attachment affordance.

21. The project strip mirrors the Mac’s project order and active project. It scrolls horizontally,
    keeps the selected project visible, and displays the same meaningful working, done, waiting,
    unread, and running-command indicators that are available on desktop. A compact plus button
    creates and activates a real shared Clinch project in the same native window and opens its first
    Terminal tab; it never creates mobile-only project state.

22. The leading header button opens a left drawer. On phones the drawer overlays the focus area;
    on wider tablets it may be pinned. Closing it restores focus mode without changing the active
    project or tab.

23. The drawer shows only tabs and sessions belonging to the currently selected project. It shows
    each tab’s provider/type, title, agent or command status, unread state, and remote-host marker
    where relevant. Selecting a tab activates that exact desktop target and closes the overlay
    drawer on phones; the top strip remains the only project switcher.

24. The trailing overflow button opens a bottom sheet containing:
    - Claude Code and Codex usage summaries and their last-updated time.
    - Connection path and health, paired device name, and Mac name.
    - Mobile preferences and a link to connection help.
    - Disconnect for this phone.

25. Usage shown on the phone is a read-only copy of Clinch’s latest local snapshot. Opening the
    mobile sheet never reads the macOS Keychain, enables live Claude plan gauges, or sends a
    provider credential to the phone. Missing or stale values are labeled as such.

26. The focus area shows the selected session’s current scrollback and live output. It maintains
    terminal text selection, links, ANSI color, resize, and follow-output behavior appropriate for
    a touch screen. Reconnecting does not duplicate or reorder output.

27. When the selected project has no controllable session, mobile creates one Terminal tab in that
    real project exactly once and follows it on both Mac and mobile. The focus area may show a brief
    purposeful opening state with explicit Terminal, Claude Code, and Codex alternatives while the
    Mac completes creation.

28. The mobile app honors the iPhone/iPad safe area, remains usable when the software keyboard is
    visible, supports portrait and landscape, and never places Send under a browser or Home Screen
    gesture area.

### Navigation, session creation, and input

29. The header New action creates a Terminal tab immediately in the selected project, using the
    current pane directory when available and Clinch’s normal new-tab directory otherwise. The
    drawer’s advanced New session action can create Terminal, Claude Code, or Codex tabs, optionally
    override the working directory, include an initial agent prompt, or select a recoverable recent
    conversation to resume.

30. Newly created tabs appear in both desktop and mobile navigation. If creation fails, the mobile
    app leaves the previous target selected and reports the exact failure without creating a
    phantom tab.

31. Mobile input always targets the project, tab, and pane displayed immediately above the
    composer. If that target closes or changes before submission, Clinch rejects the input and asks
    the person to select a live target; it never falls back to the newest pane or another provider.

32. Text entry uses an explicit Send action. Terminal input, Claude Code prompts, and Codex prompts
    preserve their normal desktop semantics, including multiline text where supported.

33. The composer includes touch controls for Escape, Tab, arrow keys, Control-C, and an expandable
    set of less-common terminal keys. Destructive interrupt actions are visually distinct from
    normal text insertion.

34. A pane has at most one remote writer lease. The desktop remains authoritative and can preempt
    the phone. When the phone is read-only or loses the lease, output continues but input controls
    clearly indicate why they are unavailable.

### Quick inserts and attachments

35. The row above the composer mirrors the effective Clinch quick-insert/toolbelt configuration
    for the selected pane, including built-in actions and custom labels. Changing the configuration
    on the Mac updates the phone without a reload.

36. By default, tapping a quick insert puts its current text into the composer for review; the
    person then presses Send. A per-device preference may enable one-tap submission, but Clinch
    never enables it silently.

37. Quick inserts are identified and validated by the Mac at activation time. A stale phone cannot
    invoke a removed or changed action merely by replaying its old label or text.

38. The attachment affordance is visible from the first mobile design but may explain that uploads
    are not yet available in the first milestone. When uploads ship, the same control accepts
    photos, camera capture, and files for local terminal directories, reports progress/cancel/retry,
    and inserts the resulting safely quoted path without automatically pressing Enter. SSH-target
    uploads remain unavailable until Clinch has a structured remote-filesystem API that does not
    construct an interpolated transfer command.

### Connectivity and recovery

39. The same paired phone works when both devices share Wi-Fi and when the phone is on another
    network such as 5G, provided Tailscale can connect both devices.

40. Connection state is always visible as Connected, Reconnecting, Mac offline, Tailscale needed,
    Authorization revoked, or Version incompatible. The app does not show stale output as live.

41. Brief network interruptions reconnect automatically. The phone receives either missing state
    or a fresh authoritative snapshot, and input is not re-enabled until that resynchronization is
    complete. The first Preview transport always takes the fresh-snapshot path rather than retaining
    terminal contents in a reconnect buffer after a WebSocket closes.

42. Backgrounding the mobile app may suspend its connection. Returning to it revalidates the
    device and target before accepting input.

43. If Clinch exits or the Mac sleeps, the phone shows that the Mac is unavailable. It cannot queue
    terminal commands for later execution unless a separate, explicit queued-command feature is
    designed and enabled in the future.

### Privacy and website communication

44. Remote Control requires no Clinch registration, email address, password, analytics consent, or
    cloud workspace. Tailscale account and network behavior remain subject to Tailscale’s own terms
    and privacy policy.

45. Project names, tab titles, terminal contents, prompts, usage details, device authorizations,
    and provider credentials are not sent to clinch.sh or a Clinch relay in the Tailscale release.
    The public website may provide documentation, but it is not in the live control data path.

46. clinch.sh presents Remote Control as a main product feature while it is in Preview, with a
    prominent feature section, a dedicated setup/security page, and links to the source. Copy must
    state the Tailscale prerequisite, supported Apple platforms, Mac-awake requirement, and current
    read/write capabilities.

47. The website’s global “no sign in needed” claim is qualified adjacent to Remote Control: Clinch
    requires no sign-in, while the optional Tailscale transport requires a Tailscale account. The
    site never describes the feature as account-free without that distinction.

48. Website and in-app setup copy advances with implementation. It must not advertise pairing,
    command execution, uploads, background wake, or perpetual authorization before those behaviors
    are available and validated in the public build.

49. All interactive mobile and desktop setup controls expose accessible labels, visible keyboard
    focus, sufficient contrast, and reduced-motion behavior. Status is communicated by text and
    iconography rather than color alone.
