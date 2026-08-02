# Clinch Remote Control — Project Log

## Objective

Implement the approved Remote Control product and technical specs through the local companion,
Tailscale activation, device pairing, bundled mobile PWA, live control, session creation, usage,
quick inserts, attachments, and security hardening.

## Current checkpoint

- [x] Milestone 0: desktop discovery, truthful setup preview, website feature, interactive concept.
- [x] Milestone 1: protocol, local gateway, Tailscale detection/Serve lifecycle, live settings state.
- [x] Milestone 2: pairing, authorization, read-only snapshots/events, bundled installable PWA.
- [x] Milestone 3: exact-target control, writer leases, session creation, usage, quick inserts.
- [ ] Milestone 4: attachment pipeline, abuse controls, threat-model validation (implementation is
  complete; final real-device/network/security release gates remain).

## Fixed decisions

- The public clinch.sh site is documentation and marketing only; it is never in the control path.
- The gateway binds to loopback only. Tailscale Serve provides private HTTPS; Funnel is forbidden.
- Tailnet membership and a Clinch device authorization are separate gates.
- The existing local-control endpoint and inherited account-backed session-sharing transport are
  not exposed remotely.
- All mutations include exact target identifiers and workspace revisions; there is no implicit
  latest-pane fallback.
- The production phone client is a standalone React/TypeScript PWA bundled into the Mac app.
- Live terminal data, prompts, usage, and device secrets are never cached by the service worker.
- Desktop authority can revoke devices and preempt remote writer leases.

## Validation record

- 2026-07-30: Milestone 0 Rust formatting, focused tests, stable/dev checks, website lint/build pass.
- 2026-07-30: Shared protocol generation/check, native library check, and production PWA typecheck,
  unit tests, and build pass during Milestones 1–4 implementation. Added exact-origin gateway and
  collision-safe upload tests; full release-gate and real-device validation remain pending.
- 2026-07-30: Final local pass: `cargo check -p warp --lib`; 25 focused native tests; eight shared
  protocol tests; ten PWA unit tests; four Playwright flows across iPhone and iPad projects; bundled
  production PWA build; offline Yarn-source reconstruction; release-script syntax checks; and
  clinch.sh lint/production build. The focused native suite includes invitation cancellation/expiry,
  wrong-key challenge use, session/inactivity expiry, connection limits, revocation, origin/Host,
  message-rate, upload collision/digest/permissions, and path-scoped Tailscale lifecycle checks.
- 2026-07-30: Rendered iPhone inspection covered the focus shell, grouped drawer, full usage sheet,
  and resume-session sheet; a mobile preference-label spacing issue found in that pass was fixed.

## Implementation findings

- The production phone shell is a separate React/Vite PWA with xterm.js. It is built into
  `Contents/Resources/remote-control-web`; the public website remains outside the data path.
- An authoritative main-thread adapter now resolves process-local project IDs plus exact tab/pane
  IDs. It never falls back to the desktop's latest terminal when a target disappears.
- Initial scrollback uses Clinch's secret-obfuscating grid serialization. Future PTY reads use the
  existing ordered broadcast channel; receiver overflow closes that stream and the phone reselects
  for an authoritative snapshot.
- Composer submissions reuse Clinch's provider-aware CLI-agent prompt pipeline. New/resumed agent
  commands use typed providers, shell-quoted prompts, and the strict existing durable-session ID
  grammar.
- Effective terminal/CLI-agent footer settings are re-resolved on every quick-insert action.
  Mobile submits the opaque action exactly once through the Mac's terminal/agent-aware path.
- The touch keyboard accessory includes compact Esc, Tab, and directional controls, and hides
  partial Claude/Codex alternate-screen repaint frames during mobile resize handoffs.
- Uploads are local-terminal-only in V1. They stage with owner-only permissions, enforce ordered
  chunks and declared size, verify SHA-256, publish without overwrite, revalidate the exact target
  and CWD, and then insert a shell-aware path without pressing Enter. SSH upload is explicitly
  rejected until a structured remote filesystem API exists.
- A paired device remains durable without a Clinch account. WebSocket authentication sessions are
  deliberately short-lived and transparently renewed; inactive device authorizations expire under
  the protocol's 90-day policy and can be revoked immediately on the Mac.
- Reauthentication deliberately sends a fresh authoritative workspace snapshot and starts a new
  terminal stream. V1 does not retain terminal contents after disconnect for replay; the versioned
  replay fields remain reserved for a separately reviewed bounded implementation.

## Open implementation notes

- Record concrete integration findings and spec changes here as later milestones uncover them.
- Real-device Safari, Tailscale network-matrix, and independent security review remain release
  gates even after automated tests pass. A formal keyboard/screen-reader/contrast audit and the
  tag-based end-to-end Corresponding Source archive gate also run before removing Preview.
