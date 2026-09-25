//! Singleton that keeps the latest CLI-agent (Claude Code + Codex) usage snapshot
//! fresh for the footer. All blocking work (file IO + the Claude usage HTTP call)
//! runs on ONE dedicated `std::thread` — never the gpui background executor, which
//! is Tokio-backed and would make `reqwest::blocking` panic.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use cli_agent_usage::http::{FetchUsage, ReqwestUsage};
use cli_agent_usage::keychain::{
    acquire_claude_token, should_read_keychain, ClaudeToken, MacKeychain, TokenAcquisition,
};
use cli_agent_usage::{
    claude_plan_cache, fetch_plan_for_token_outcome, scan_local, snapshot_cache, Caches, Paths,
    PlanFetchOutcome, PlanLimits, UsageSnapshot,
};
use warpui::r#async::block_on;
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::settings::CliAgentUsageSettings;

/// How often the producer thread re-scans local files.
const FILE_POLL: Duration = Duration::from_secs(5);
/// While we lack a fresh, valid Keychain token, attempt to re-acquire one at
/// most this often. All reads fail without interaction if access is unavailable.
const REREAD_BACKOFF_MS: i64 = 5 * 60 * 1000;

pub enum CliAgentUsageModelEvent {
    Updated,
}

struct ProducerUpdate {
    snapshot: UsageSnapshot,
    refresh_attempt_finished: bool,
}

pub struct CliAgentUsageModel {
    latest: UsageSnapshot,
    last_updated_at: Option<DateTime<Utc>>,
    /// One-shot flag for a Turn on / Retry click. Requests a silent refresh.
    refresh: Arc<AtomicBool>,
    /// Wakes the producer out of its idle wait so an explicit gesture does not
    /// sit behind the normal file-poll interval.
    producer_thread: Option<std::thread::Thread>,
    /// UI-only acknowledgement shown from the click until the producer has
    /// completed that specific Keychain attempt and any immediate fetch.
    refresh_pending: bool,
}

impl Entity for CliAgentUsageModel {
    type Event = CliAgentUsageModelEvent;
}

impl SingletonEntity for CliAgentUsageModel {}

impl CliAgentUsageModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let (tx, rx) = async_channel::unbounded::<ProducerUpdate>();

        // Bridge the main-thread-only `show_plan_limits` setting to the
        // off-thread producer with a lock-free atomic (same pattern as
        // FeatureFlag). Seeded from the current value; kept live by the
        // subscription below. When false, the producer never reads the Keychain,
        // so disabling plan limits also disables credential access.
        let enabled = Arc::new(AtomicBool::new(
            *CliAgentUsageSettings::as_ref(ctx).show_plan_limits,
        ));
        let refresh = Arc::new(AtomicBool::new(false));

        let producer_thread = if let Some(paths) = Paths::detect() {
            // Dedicated OS thread => guaranteed no Tokio runtime context.
            let enabled = enabled.clone();
            let refresh = refresh.clone();
            std::thread::Builder::new()
                .name("cli-agent-usage".to_string())
                .spawn(move || producer_loop(paths, tx, enabled, refresh))
                .ok()
                .map(|handle| handle.thread().clone())
        } else {
            None
        };

        // Track setting changes (Settings UI or Command Palette toggle). The
        // producer observes the new value on its next tick. Turning the
        // gauges ON requests an immediate, non-interactive refresh.
        ctx.subscribe_to_model(&CliAgentUsageSettings::handle(ctx), {
            let enabled = enabled.clone();
            let refresh = refresh.clone();
            let producer_thread = producer_thread.clone();
            let mut was_enabled = *CliAgentUsageSettings::as_ref(ctx).show_plan_limits;
            move |_model, _handle, _event, ctx| {
                let is_enabled = *CliAgentUsageSettings::as_ref(ctx).show_plan_limits;
                let turned_on = is_enabled && !was_enabled;
                if turned_on {
                    refresh.store(true, Ordering::Relaxed);
                }
                was_enabled = is_enabled;
                enabled.store(is_enabled, Ordering::Relaxed);
                if turned_on {
                    if let Some(thread) = &producer_thread {
                        thread.unpark();
                    }
                }
            }
        });

        // Deliver each snapshot on the main thread; store it and notify observers.
        ctx.spawn_stream_local(rx, Self::on_update, |_, _| {});
        Self {
            latest: UsageSnapshot::default(),
            last_updated_at: None,
            refresh,
            producer_thread,
            refresh_pending: false,
        }
    }

    /// Retry the existing provider login silently after a Turn on / Retry click.
    pub fn request_refresh(&mut self, ctx: &mut ModelContext<Self>) {
        self.refresh.store(true, Ordering::Relaxed);
        if let Some(thread) = &self.producer_thread {
            thread.unpark();
        }
        if !self.refresh_pending {
            self.refresh_pending = true;
            ctx.emit(CliAgentUsageModelEvent::Updated);
            ctx.notify();
        }
    }

    pub fn refresh_pending(&self) -> bool {
        self.refresh_pending
    }

    /// Test-only constructor: skips the producer thread (which reads the macOS
    /// keychain and makes a blocking HTTP call) so workspace tests that build
    /// the footer can register and subscribe to this singleton without touching
    /// the network or keychain. Holds a default snapshot forever.
    #[cfg(test)]
    pub fn new_for_test() -> Self {
        Self {
            latest: UsageSnapshot::default(),
            last_updated_at: None,
            refresh: Arc::new(AtomicBool::new(false)),
            producer_thread: None,
            refresh_pending: false,
        }
    }

    /// Delivers a snapshot through the real notification path without
    /// starting the filesystem/HTTP producer thread.
    #[cfg(test)]
    pub(crate) fn update_snapshot_for_test(
        &mut self,
        snapshot: UsageSnapshot,
        ctx: &mut ModelContext<Self>,
    ) {
        self.on_update(
            ProducerUpdate {
                snapshot,
                refresh_attempt_finished: false,
            },
            ctx,
        );
    }

    pub fn latest(&self) -> &UsageSnapshot {
        &self.latest
    }

    pub fn last_updated_at(&self) -> Option<DateTime<Utc>> {
        self.last_updated_at
    }

    fn on_update(&mut self, update: ProducerUpdate, ctx: &mut ModelContext<Self>) {
        let pending_changed = update.refresh_attempt_finished && self.refresh_pending;
        if pending_changed {
            self.refresh_pending = false;
        }
        let snap = update.snapshot;
        // Emit only on real change — the producer sends every ~5s forever, and an
        // unconditional notify would wake the footer each poll even when nothing
        // changed (and even when the chip is hidden), defeating idle-frame suppression.
        //
        // Exception: while the widget is enabled and a plan window is exhausted,
        // the chip shows a live "resets …" countdown computed from `Utc::now()`
        // at render time. An identical snapshot would freeze that label, so keep
        // emitting on every producer tick until the exhaustion clears. Gated on
        // the setting so a disabled widget stays fully idle.
        let countdown_is_live = *CliAgentUsageSettings::as_ref(ctx).show_plan_limits
            && [snap.claude.plan, snap.codex.plan]
                .into_iter()
                .flatten()
                .any(|plan| plan.exhausted_until().is_some());
        if snap == self.latest && !countdown_is_live && !pending_changed {
            return;
        }
        self.latest = snap;
        self.last_updated_at = Some(Utc::now());
        ctx.emit(CliAgentUsageModelEvent::Updated);
        ctx.notify();
    }
}

/// Runs on the dedicated thread. Local scans run every `FILE_POLL`; the shared
/// Claude plan cache independently throttles endpoint requests by wall-clock
/// time, retaining the last good `PlanLimits` across transient failures and
/// coordinating Stable/Dev processes. Exits when the receiver is dropped.
///
/// The Claude OAuth token is cached in `cached_token`: the Keychain is read
/// only when we lack a usable token, not on every endpoint tick. When
/// `enabled` is false the Keychain is never read at all and the plan gauges
/// clear.
///
/// Every credential read is non-interactive and bounded. Retry clicks bypass
/// the normal cadence but cannot display a system credential dialog.
fn producer_loop(
    paths: Paths,
    tx: async_channel::Sender<ProducerUpdate>,
    enabled: Arc<AtomicBool>,
    refresh: Arc<AtomicBool>,
) {
    let mut caches = Caches::new();
    let keychain = MacKeychain;
    let fetch = ReqwestUsage;
    let mut cached_token: Option<ClaudeToken> = None;
    let mut last_read_ms: Option<i64> = None;
    // Non-sticky: background ticks retry on the normal backoff, so a repaired
    // or unlocked Keychain recovers without forcing the user to retry.
    let mut plan_unavailable = false;

    // Stale-while-revalidate: the widget hides until a snapshot has data, and
    // the first cold scan of the transcript dirs can take tens of seconds, so
    // surface the previous run's local snapshot immediately. Plan percentages
    // have their own cache with an independent freshness timestamp, so never
    // revive the snapshot's embedded copy; the first shared-cache read below
    // supplies it when it is still current.
    let mut last_stored = snapshot_cache::load(&paths.snapshot_cache, Utc::now());
    let mut last_plan: Option<PlanLimits> = None;
    if let Some(cached) = &mut last_stored {
        cached.claude.plan = None;
        // Recompute current credential availability on the first tick.
        cached.claude.plan_unavailable = false;
        if block_on(tx.send(ProducerUpdate {
            snapshot: cached.clone(),
            refresh_attempt_finished: false,
        }))
        .is_err()
        {
            return;
        }
    }

    loop {
        let now = Utc::now();
        let now_ms = now.timestamp_millis();
        let enabled_now = enabled.load(Ordering::Relaxed);
        let previous_plan = last_plan;
        let previous_unavailable = plan_unavailable;
        let mut refresh_attempt_finished = false;
        let mut refreshed_token = false;

        if !enabled_now {
            // Gauge disabled: never touch the Keychain. Drop any cached token and
            // last-good plan so the gauges clear immediately and re-enabling
            // forces a fresh read.
            cached_token = None;
            last_read_ms = None;
            last_plan = None;
            plan_unavailable = false;
        } else {
            // Token acquisition happens OUTSIDE the shared fetch throttle so an
            // Retry click acts on the next tick instead of waiting out the
            // 5-minute fetch cadence. The gesture is consumed only while
            // enabled, so a click racing the settings flag survives to the
            // next tick rather than being dropped.
            let gesture = refresh.swap(false, Ordering::Relaxed);
            let read_due = should_read_keychain(
                cached_token.as_ref(),
                last_read_ms,
                now_ms,
                REREAD_BACKOFF_MS,
            );
            if gesture || read_due {
                let acquisition = acquire_claude_token(&keychain, &paths.os_account);
                refreshed_token = gesture && matches!(&acquisition, TokenAcquisition::Token(_));
                match acquisition {
                    TokenAcquisition::Token(token) => {
                        cached_token = Some(token);
                        plan_unavailable = false;
                    }
                    TokenAcquisition::ItemMissing | TokenAcquisition::RetryLater => {
                        cached_token = None;
                        plan_unavailable = true;
                    }
                    TokenAcquisition::Unavailable => {
                        cached_token = None;
                        plan_unavailable = true;
                    }
                }
                last_read_ms = Some(now_ms);
                refresh_attempt_finished = gesture;
            }

            last_plan = if plan_unavailable {
                // Nothing to fetch, and skipping refresh_shared keeps this
                // process from burning the shared attempt cadence that a
                // healthy sibling process may be using.
                None
            } else {
                let fetch_plan = || {
                    let mut outcome = cached_token
                        .as_ref()
                        .map(|token| {
                            fetch_plan_for_token_outcome(&fetch as &dyn FetchUsage, token, now_ms)
                        })
                        .unwrap_or(PlanFetchOutcome::Unavailable);

                    // Claude Code may rotate a still-nominally-valid token. A 401
                    // is the one signal that our cached copy is no longer usable;
                    // boundedly re-read the Keychain and retry only when the token
                    // actually changed. The same silent, bounded read is used.
                    if matches!(outcome, PlanFetchOutcome::Unauthorized)
                        && last_read_ms
                            .map(|last| now_ms.saturating_sub(last) >= REREAD_BACKOFF_MS)
                            .unwrap_or(true)
                    {
                        let refreshed = match acquire_claude_token(&keychain, &paths.os_account) {
                            TokenAcquisition::Token(token) => {
                                plan_unavailable = false;
                                Some(token)
                            }
                            TokenAcquisition::ItemMissing | TokenAcquisition::RetryLater => {
                                plan_unavailable = true;
                                None
                            }
                            TokenAcquisition::Unavailable => {
                                plan_unavailable = true;
                                None
                            }
                        };
                        let changed = match (&cached_token, &refreshed) {
                            (Some(old), Some(new)) => old.access_token != new.access_token,
                            (None, Some(_)) => true,
                            _ => false,
                        };
                        cached_token = refreshed;
                        last_read_ms = Some(now_ms);
                        if changed {
                            outcome = cached_token
                                .as_ref()
                                .map(|token| {
                                    fetch_plan_for_token_outcome(
                                        &fetch as &dyn FetchUsage,
                                        token,
                                        now_ms,
                                    )
                                })
                                .unwrap_or(PlanFetchOutcome::Unavailable);
                        }
                    }
                    outcome
                };
                let refreshed_plan = if refreshed_token {
                    claude_plan_cache::refresh_shared_after_retry(
                        &paths.snapshot_cache,
                        now,
                        fetch_plan,
                    )
                } else {
                    claude_plan_cache::refresh_shared(&paths.snapshot_cache, now, fetch_plan)
                };
                if plan_unavailable {
                    // A 401-triggered refresh can discover unavailable credentials.
                    // Clear stale gauges while retaining the local usage totals.
                    None
                } else {
                    refreshed_plan
                }
            };
        }

        // Plan refreshes (and availability changes) should not sit behind a
        // 10s+ recursive transcript scan. Push them with the last local
        // snapshot immediately; the live scan below replaces the local totals
        // on the same loop.
        if last_plan != previous_plan
            || plan_unavailable != previous_unavailable
            || refresh_attempt_finished
        {
            let mut preview = last_stored.clone().unwrap_or_default();
            preview.claude.plan = last_plan;
            preview.claude.plan_unavailable = plan_unavailable;
            if block_on(tx.send(ProducerUpdate {
                snapshot: preview,
                refresh_attempt_finished,
            }))
            .is_err()
            {
                return;
            }
        }

        let mut snap = scan_local(&paths, &mut caches, now);
        snap.claude.plan = last_plan;
        snap.claude.plan_unavailable = plan_unavailable;
        if last_stored.as_ref() != Some(&snap) {
            snapshot_cache::store(&paths.snapshot_cache, &snap, now);
            last_stored = Some(snap.clone());
        }
        if block_on(tx.send(ProducerUpdate {
            snapshot: snap,
            refresh_attempt_finished: false,
        }))
        .is_err()
        {
            break; // receiver dropped (model gone) => exit cleanly
        }
        std::thread::park_timeout(FILE_POLL);
    }
}
