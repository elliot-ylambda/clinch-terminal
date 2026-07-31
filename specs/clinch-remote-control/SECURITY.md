# Clinch Remote Control — Security and Privacy Contract

## Status

This contract describes the implemented Preview architecture. Automated negative-path validation
is part of this change. Real-device/network validation and an independent security review remain
release gates before the Preview label can be removed.

## Trust boundaries

- **Clinch process:** owns the loopback gateway, workspace adapter, Tailscale configuration, device
  registry, invitations, challenges, sessions, writer leases, and upload staging.
- **Tailscale:** supplies tailnet reachability and private HTTPS. Tailnet membership is necessary but
  is not Clinch authorization.
- **Phone browser/PWA:** is an untrusted network client until its locally generated P-256 key is
  approved on the Mac. Every message and target supplied by it remains untrusted after approval.
- **clinch.sh:** is documentation and marketing only and is never part of the control data path.
- **Provider processes and terminals:** continue to run on the Mac. Provider credentials, macOS
  Keychain contents, and shell environment values are never protocol fields.

## Security invariants

1. Remote Control is disabled by default and starts no listener or Tailscale operation until the
   user explicitly enables it.
2. The HTTP/WebSocket gateway binds only to an operating-system-selected port on `127.0.0.1`.
   Tailscale Serve publishes one random, path-scoped private route. Clinch never invokes Funnel,
   never binds the gateway to a LAN/public address, and never resets unrelated Serve routes.
3. Every API request requires the exact configured Host and HTTPS Origin. WebSocket upgrades also
   require a valid, short-lived, HttpOnly, Secure, SameSite=Strict cookie scoped to Clinch's random
   Serve path.
4. The pairing invitation is a 256-bit random, five-minute, single-use secret carried in the URL
   fragment. The gateway receives it only when the PWA deliberately claims the invitation. A bad
   claim consumes it. Cancellation, expiry, or use makes it invalid.
5. A claim is not authorization. The Mac shows the proposed device name and P-256 public-key
   fingerprint and requires explicit approval.
6. The Mac persists only the approved public key, device metadata, capabilities, timestamps,
   revocation state, and random route ID in local secure storage with an owner-only fallback.
   Invitation secrets, claim secrets, challenges, cookie tokens, and private keys are never
   persisted by the Mac.
7. The phone generates its signing key with WebCrypto and requests a non-exportable private key.
   It stores the key locally in IndexedDB; the private key is never transmitted. Clearing website
   data intentionally requires pairing again.
8. Every reconnect proves possession of the approved key by signing a fresh 32-byte challenge.
   Challenges are single-use and short-lived. A WebSocket cookie lasts 15 minutes, claims one live
   connection, and is removed on disconnect. Device authorization lasts until revocation or 90 days
   of inactivity.
9. Revoking one device invalidates its outstanding challenges and sessions. Revoke All invalidates
   every invitation, claim, challenge, and session. Revocation is checked again for each inbound
   message and at the periodic workspace tick, so an open socket cannot retain access.
10. Capabilities (`view`, `control`, `create_session`, and `upload`) are enforced on the Mac for each
    command. UI state is not an authorization boundary.
11. Every mutation names an exact app instance, project, tab, and pane and carries the expected
    workspace revision. A missing, moved, closed, or changed target fails closed; there is no
    fallback to another pane. Provider creation and resume use typed provider/session data and the
    existing strict durable-session grammar.
12. A pane has one remote writer lease. The desktop can preempt it immediately. Disconnect releases
    it, and input is disabled while the phone is reconnecting, resynchronizing, read-only, or no
    longer owns the target.
13. Reconnect starts from an authoritative workspace snapshot and a newly selected terminal
    snapshot/stream. V1 does not retain terminal data in a disconnected-client replay buffer and
    never queues input for an offline Mac.
14. Mobile usage is serialized only from the latest in-memory local snapshot. A phone request
    cannot enable provider gauges, read the Keychain, mutate usage settings, or receive provider
    tokens.
15. Quick inserts are opaque IDs plus configuration revisions. The Mac re-resolves an item at tap
    time; a stale, removed, or modified item cannot be invoked using old phone text.
16. Upload metadata, JSON, binary chunks, total size, filename, digest, ordering, and destination
    are bounded and validated. Uploads are staged collision-safely with owner-only permissions,
    checked with SHA-256, published without overwrite, and revalidate the target/CWD before a
    shell-escaped path is inserted without Enter. V1 rejects SSH upload destinations.
17. Static responses use a restrictive Content Security Policy, no-referrer policy, MIME sniffing
    protection, frame denial, and no-store for dynamic data. The service worker caches only the
    versioned shell/assets, never API responses, terminal output, prompts, usage, or credentials.
18. Request bodies, frames, command rate, pairing/auth attempts, concurrent connections, pending
    claims/challenges, device records, snapshots, and upload sizes are bounded. Invalid or oversized
    messages fail closed.
19. Remote Control adds no Clinch telemetry or account dependency. Logs do not intentionally include
    terminal contents, prompts, invitation/challenge/session secrets, provider credentials, or
    uploaded bytes. Recoverable operational errors may include bounded non-secret Tailscale output
    or local filesystem errors.

## Threat-to-control map

| Threat | Implemented control |
| --- | --- |
| Public exposure | Loopback bind, private Serve only, path-scoped route, no Funnel |
| Tailnet peer without approval | Device-key pairing and Mac approval |
| QR replay or leakage | Fragment secret, five-minute TTL, single-use consume, Cancel |
| Stolen/replayed cookie | Secure scoped cookie, 15-minute TTL, one WebSocket claim, per-message reauth |
| Revoked or inactive device | Registry checks on challenge, socket admission, every message, and tick |
| Cross-site WebSocket / DNS rebinding | Exact Origin and Host allowlists before API and upgrade |
| Confused target / race | Exact stable IDs, app-instance ID, workspace revision, fail-closed resolver |
| Concurrent input corruption | Per-pane writer lease and desktop preemption |
| Quick-insert replay | Opaque ID plus configuration revision and Mac-side re-resolution |
| Oversized or abusive client | HTTP/frame/message/upload limits, rate limits, bounded registries |
| Malicious upload | Filename/path validation, canonical local CWD, ordered chunks, digest, no overwrite |
| Offline command surprise | No command queue; input requires a live authorized WebSocket and target |
| Sensitive cache or website leak | Shell-only service worker; clinch.sh outside data path; no telemetry |

## Residual risks and release gates

- A compromised approved phone browser profile can act with that device's granted capabilities
  until it is revoked. The Mac device list and Revoke All are the recovery controls.
- A compromised Mac or terminal process is outside this feature's protection boundary.
- Tailscale availability, identity-provider security, tailnet policy, TLS, and relay behavior are
  governed by Tailscale and the user's configuration.
- Non-exportable WebCrypto key persistence, mobile background suspension, Home Screen behavior,
  keyboard/safe-area behavior, and clearing Safari website data require real iPhone/iPad testing.
- Same-Wi-Fi, 5G, network-switch, DERP, sleep/wake, restart, and multi-phone behavior require the
  documented manual network matrix.
- SSH uploads remain denied until a structured remote-filesystem API replaces command-string SFTP.
- Removing Preview requires an independent review of the protocol, gateway, device lifecycle,
  origin/Host handling, abuse controls, upload pipeline, logs, and release/source packaging.
