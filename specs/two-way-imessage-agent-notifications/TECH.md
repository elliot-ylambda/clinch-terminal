# Two-way iMessage agent notifications: technical design

## Context

The product behavior is defined in [PRODUCT.md](./PRODUCT.md). Clinch already has most of the session-side primitives needed by the bridge:

- [`CLIAgentSessionKey` and structured session context](https://github.com/warpdotdev/warp/blob/bcf40defc30c67f031b31cd39f1f3853e0562561/app/src/terminal/cli_agent_sessions/mod.rs#L34-L98) provide a provider/session-ID identity and trustworthy final response.
- [`CLIAgentSessionsModelEvent::StatusChanged`](https://github.com/warpdotdev/warp/blob/bcf40defc30c67f031b31cd39f1f3853e0562561/app/src/terminal/cli_agent_sessions/mod.rs#L477-L503) carries completion state, terminal view ID, provider, and response context.
- [`TerminalView::handle_cli_agent_sessions_event`](https://github.com/warpdotdev/warp/blob/bcf40defc30c67f031b31cd39f1f3853e0562561/app/src/terminal/view.rs#L13371-L13530) is the existing per-pane completion subscriber.
- [`submit_text_to_cli_agent_pty`](https://github.com/warpdotdev/warp/blob/bcf40defc30c67f031b31cd39f1f3853e0562561/app/src/terminal/view/use_agent_footer/mod.rs#L776-L804) and the agent-specific submission path are the safe way to start a follow-up turn.
- [`ClinchSettingsPageView`](https://github.com/warpdotdev/warp/blob/bcf40defc30c67f031b31cd39f1f3853e0562561/app/src/settings_view/clinch_page.rs) supplies the existing local-only configuration surface.
- [`script/macos/bundle`](https://github.com/warpdotdev/warp/blob/bcf40defc30c67f031b31cd39f1f3853e0562561/script/macos/bundle#L340-L358) and [`script/verify-clinch-release`](https://github.com/warpdotdev/warp/blob/bcf40defc30c67f031b31cd39f1f3853e0562561/script/verify-clinch-release#L28-L88) currently set macOS 13 and intentionally reject Apple Events permission metadata. Both policies must change together.

Messages can send through its Apple Events scripting interface but exposes no supported incoming-message scripting API. Incoming delivery therefore requires read-only access to the user's local Messages SQLite database with explicit Full Disk Access. The bundled helper pins `openclaw/imsg`'s MIT-licensed `IMsgCore` package at commit `b5b7464bc748af482bfc3059b28d5dab0395da9e`, whose watcher handles WAL/SHM replacement, fallback polling, delayed chat joins, and attributed message bodies.

## Proposed changes

### Domain model and persistence

- Add an `imessage` module with a singleton `IMessageCoordinator`. It owns configuration health, the helper process, live route registry, outbound GUID map, database cursor, processed GUIDs, pending ambiguous selections, and per-route FIFO queues.
- Define `MobileRouteId`, `MobileSessionRoute`, `QueuedMobileReply`, `PendingRouteSelection`, `IMessageConnectionStatus`, and versioned persisted-state types. A route contains `CLIAgentSessionKey`; terminal view IDs are ephemeral lookup hints only.
- Generate four-character route IDs from an unambiguous alphabet with at least one letter and one digit, keep one ID for the durable session lifetime, and quarantine retired IDs for 30 days.
- Store the destination and calibrated chat identity through local secure storage. Store operational state beneath Clinch's Application Support directory in an atomic owner-only JSON file. Persist after each cursor, queue, route, or deduplication change. A generation-guarded local timer expires pending selections after ten minutes and queued bodies after 24 hours, emitting body-free notices while messaging is enabled.
- Treat a configured chat without a positive persisted database cursor as unsafe recovery. Clear its conversation correlation and require a new calibration; never reconstruct progress by watching the calibrated chat from ROWID zero.
- Add pure modules for Unicode-safe 3,000-character response splitting, route parsing, incoming classification, FIFO drain decisions, sanitization, and persisted-version validation so routing behavior is unit-testable without Messages or a PTY.

### Native bridge

- Add `tools/imessage-bridge`, a Swift Package producing `clinch-imessage-bridge` and depending on the exact `IMsgCore` revision above. Check in `Package.resolved` and the upstream MIT license.
- Use a line-delimited JSON protocol on stdin/stdout. Requests have `version`, `id`, and one of `health`, `configure`, `send`, `start_watch`, `stop_watch`, or `shutdown`; responses echo `id`; asynchronous events are `incoming`, `permission_required`, `delivery_failed`, and `watch_failed`.
- Send iMessage-only text with `MessageSender` and `allowSMSFallback = false`. Before each serialized send, record the current row cursor, then locate the newly created outgoing row in the calibrated chat and return its exact GUID. Buffer watcher events during this correlation window, suppress the exact correlated outgoing GUID, and then release genuine inbound events in order so a fast database write cannot race the Rust GUID map. Associate every multipart GUID with the same route.
- Open `chat.db` read-only, never immutable; watch `chat.db`, `chat.db-wal`, `chat.db-shm`, and their directory with a 250 ms debounce and five-second fallback poll. Filter at the SQL query to the calibrated chat while retaining ROWID progress.
- Emit normalized text, message GUID, ROWID, chat ID/GUID, reply/associated GUID metadata, timestamp, attachment state, and conservative reaction/edit fields. Decode plain `text` first and attributed bodies through `IMsgCore` when needed; unsupported edits, reactions, and attachments are never treated as replies.
- Treat only recorded outbound GUIDs as Clinch-authored suppression. Do not use `is_from_me` as an inbound filter because self-chat replies may synchronize with that value.
- Before invoking the helper for an outbound part, atomically persist a 24-hour send intent containing a random ID, SHA-256 text fingerprint, route, and current processed cursor. Resolve it when the send response supplies a GUID. After an indeterminate crash, use `is_from_me` only together with that exact fingerprint and a later ROWID to recover and record the outgoing GUID; unmatched self-chat messages continue through normal routing.
- Advance the persisted watcher cursor only after Rust classifies an incoming watcher event. A send response's outgoing ROWID never advances that cursor because an unseen inbound row may precede it; replayed outgoing rows are harmlessly removed by the persisted exact-GUID map.
- Supervise one helper per Clinch process. Use bounded restart backoff, protocol/version validation, bounded line sizes, and stderr redaction. A helper crash changes health to reconnecting/error and does not advance the cursor past unprocessed input; permission failures use a distinct paused state.

### Session integration

- Register and retire routes from `Started`, identity-changing `SessionUpdated`, and `Ended` events. Only Claude/Codex sessions with non-empty durable IDs are eligible.
- Handle successful `StatusChanged` centrally in `IMessageCoordinator`, rather than duplicating outbound logic in each `TerminalView`. Build multipart completion text from structured context and enqueue sends serially.
- Route incoming content by parent GUID, then explicit leading route token, then a sole eligible live route. For ambiguity, persist the original content, send a route menu, and consume a code-only selection against the pending item.
- Resolve the current terminal view from the route at delivery time and revalidate the exact `CLIAgentSessionKey`. Add a narrow `TerminalView` entry point that accepts already validated external text and delegates to the existing agent-aware submission pipeline.
- Submit immediately only when the exact session is `Success` and has no older queue, pending completion drain, or locally submitted reply awaiting a status transition. While `InProgress`, append to its FIFO. While `Blocked`, retain the queue without interacting with the TUI. On each later `Success`, submit only the oldest item; an explicit local in-flight gate closes the interval before the agent reports `InProgress`.
- On session removal, identity mismatch, opt-out, or expiry, cancel affected content and enqueue a failure iMessage. Never fall back to a view ID, provider alone, or the most recent completion.

### Settings and UI

- Extend local Clinch settings with global enabled/setup state and default-on behavior. Keep sensitive destination/chat values private and never cloud synced. Persist per-session explicit opt-outs in coordinator state.
- Add an iMessage category to Clinch Settings containing destination input, setup/start-over, Automation and Full Disk Access health, test/calibration status, global enablement, and troubleshooting actions that open the relevant System Settings panes.
- Add a compact header status item with disabled, setup-required, connected, paused, and error states. Clicking it opens the iMessage Settings category.
- Add a **Message me** CLI-agent footer item for durable Codex/Claude sessions. It reflects inherited global-on state and toggles an explicit session override.
- All UI observes coordinator and settings model changes; no view owns bridge lifecycle or persisted routing state.

### Packaging and release policy

- Raise Clinch's deployment target, Info.plist minimum, release manifest default, scripts, documentation, and verification expectations from macOS 13 to 14.
- Build the helper for arm64 and x86_64, combine it into a universal executable under `Contents/Helpers`, and self-test it before copying. Release signing performs an initial deep app sign, explicitly re-signs the helper with its Apple Events entitlement, and finally re-seals the outer app. Developer runs build the host architecture only.
- Add `NSAppleEventsUsageDescription` for Clinch and `com.apple.security.automation.apple-events` to Clinch entitlements; continue rejecting unrelated privacy entitlements. Verify the helper architecture, protocol smoke test, usage description, entitlements, and bundled licenses in release checks.
- A stable signing identity is the production expectation for durable TCC grants. Ad-hoc development builds remain supported but surface that an update or rebuild may require permission regranting.

## Testing and validation

- Rust domain tests cover multipart Unicode boundaries, stable/quarantined routes, restart deactivation/reactivation, parent/explicit/sole precedence, route stripping, ambiguity and queue expiry, GUID deduplication, FIFO drain, opt-out cancellation, unsupported-message filtering, terminal-control sanitization, and owner-only atomic persistence.
- Rust bridge tests cover request correlation, event separation, and recovery after an oversized protocol line. Swift tests cover protocol decoding, invalid requests, and a real PhoneNumberKit resource lookup. The pinned `IMsgCore` dependency owns its lower-level SQLite/WAL, delayed-chat-join, and attributed-body fixture coverage.
- Settings rendering tests instantiate the iMessage category alongside its required models. Compile-time session integration and exact `CLIAgentSessionKey` revalidation cover the external PTY entry point; real Messages/TCC behavior remains part of the release matrix below.
- Release tests assert macOS 14 across the binary/plist/manifest, both helper architectures, nested signing and helper entitlement, Apple Events metadata, absence of unrelated privacy entitlements, protocol/resource self-tests, and third-party license attribution.
- Before release, manually validate on Intel and Apple Silicon: first-time setup; permission denial, revocation, and regrant; Messages signed out; calibration from an iPhone; two simultaneous Codex/Claude sessions; reply-to-part and explicit-code routing; ambiguous selection; busy FIFO draining; blocked prompts; restart; and an in-place app update. This requires a signed app, a real Messages account, and an iPhone and is intentionally not simulated by unit tests.

## Risks and mitigations

- Messages database schema and Apple Events behavior are not public compatibility contracts. Isolate both in the versioned helper, pin the dependency, fail closed on protocol/schema errors, and keep routing logic independent of database details.
- A synced Messages account does not expose trustworthy source-device identity. The setup disclosure and exact-chat restriction make this limitation explicit; routing never claims cryptographic iPhone provenance.
- TCC attributes permissions to code identity and may behave differently for ad-hoc nested helpers. Release validation includes clean-install and update tests, and production uses a stable signing identity.
- Full responses may produce many messages. Sends are serialized, parts are numbered, delivery failures stop remaining parts for that completion, and the route remains available for a later retry.
