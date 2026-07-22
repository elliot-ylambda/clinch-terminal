//! Singleton that keeps the latest CLI-agent (Claude Code + Codex) usage snapshot
//! fresh for the footer. All blocking work (file IO + the Claude usage HTTP call)
//! runs on ONE dedicated `std::thread` — never the gpui background executor, which
//! is Tokio-backed and would make `reqwest::blocking` panic.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
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
/// most this often. Unsanctioned reads are ACL-probed first and happen only
/// when provably silent, so this backoff bounds probe/read *work*, not
/// prompts — prompts only ever follow a Turn on / Authorize click.
const REREAD_BACKOFF_MS: i64 = 5 * 60 * 1000;

pub enum CliAgentUsageModelEvent {
    Updated,
}

struct ProducerUpdate {
    snapshot: UsageSnapshot,
    authorization_attempt_finished: bool,
}

pub struct CliAgentUsageModel {
    latest: UsageSnapshot,
    /// One-shot gesture flag consumed by the producer thread: the user just
    /// clicked Turn on / Authorize, sanctioning one Keychain read even if
    /// macOS will raise its credential prompt for it.
    authorize: Arc<AtomicBool>,
    /// Wakes the producer out of its idle wait so an explicit gesture does not
    /// sit behind the normal file-poll interval.
    producer_thread: Option<std::thread::Thread>,
    /// UI-only acknowledgement shown from the click until the producer has
    /// completed that specific Keychain attempt and any immediate fetch.
    authorization_pending: bool,
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
        // so the credential prompt never fires.
        let enabled = Arc::new(AtomicBool::new(
            *CliAgentUsageSettings::as_ref(ctx).show_plan_limits,
        ));
        let authorize = Arc::new(AtomicBool::new(false));

        let producer_thread = if let Some(paths) = Paths::detect() {
            // Dedicated OS thread => guaranteed no Tokio runtime context.
            let enabled = enabled.clone();
            let authorize = authorize.clone();
            std::thread::Builder::new()
                .name("cli-agent-usage".to_string())
                .spawn(move || producer_loop(paths, tx, enabled, authorize))
                .ok()
                .map(|handle| handle.thread().clone())
        } else {
            None
        };

        // Track setting changes (Settings UI or Command Palette toggle). The
        // producer observes the new value on its next tick. Turning the
        // gauges ON is a user gesture from any of those surfaces, so it also
        // sanctions the Keychain read (and its prompt, if macOS raises one).
        ctx.subscribe_to_model(&CliAgentUsageSettings::handle(ctx), {
            let enabled = enabled.clone();
            let authorize = authorize.clone();
            let producer_thread = producer_thread.clone();
            let mut was_enabled = *CliAgentUsageSettings::as_ref(ctx).show_plan_limits;
            move |_model, _handle, _event, ctx| {
                let is_enabled = *CliAgentUsageSettings::as_ref(ctx).show_plan_limits;
                let turned_on = is_enabled && !was_enabled;
                if turned_on {
                    authorize.store(true, Ordering::Relaxed);
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
            authorize,
            producer_thread,
            authorization_pending: false,
        }
    }

    /// Sanction one Keychain read in direct response to a user click (the
    /// usage widget's Turn on / Authorize affordance). If reading requires
    /// the macOS credential prompt, it appears now — never unbidden at launch.
    pub fn request_authorization(&mut self, ctx: &mut ModelContext<Self>) {
        self.authorize.store(true, Ordering::Relaxed);
        if let Some(thread) = &self.producer_thread {
            thread.unpark();
        }
        if !self.authorization_pending {
            self.authorization_pending = true;
            ctx.emit(CliAgentUsageModelEvent::Updated);
            ctx.notify();
        }
    }

    pub fn authorization_pending(&self) -> bool {
        self.authorization_pending
    }

    /// Test-only constructor: skips the producer thread (which reads the macOS
    /// keychain and makes a blocking HTTP call) so workspace tests that build
    /// the footer can register and subscribe to this singleton without touching
    /// the network or keychain. Holds a default snapshot forever.
    #[cfg(test)]
    pub fn new_for_test() -> Self {
        Self {
            latest: UsageSnapshot::default(),
            authorize: Arc::new(AtomicBool::new(false)),
            producer_thread: None,
            authorization_pending: false,
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
                authorization_attempt_finished: false,
            },
            ctx,
        );
    }

    pub fn latest(&self) -> &UsageSnapshot {
        &self.latest
    }

    fn on_update(&mut self, update: ProducerUpdate, ctx: &mut ModelContext<Self>) {
        let pending_changed = update.authorization_attempt_finished && self.authorization_pending;
        if pending_changed {
            self.authorization_pending = false;
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
/// Prompt policy: reads the poller decides on its own first probe the item's
/// ACL and proceed only when the read is provably silent. A metadata probe has
/// a hard deadline and is retried on the normal backoff, so temporary Keychain
/// trouble cannot strand the UI. The widget's Authorize click (the `authorize`
/// flag) bypasses that probe and is the only thing that sanctions a prompting
/// read.
fn producer_loop(
    paths: Paths,
    tx: async_channel::Sender<ProducerUpdate>,
    enabled: Arc<AtomicBool>,
    authorize: Arc<AtomicBool>,
) {
    let mut caches = Caches::new();
    let keychain = MacKeychain;
    let fetch = ReqwestUsage;
    let mut cached_token: Option<ClaudeToken> = None;
    let mut last_read_ms: Option<i64> = None;
    // Whether the most recent bounded attempt needs a user-sanctioned read.
    // This is intentionally non-sticky: background ticks re-probe on the
    // normal backoff so a repaired/unlocked Keychain self-heals.
    let mut needs_authorization = false;

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
        // Recomputed within the first tick (the probe is fast); never revive
        // last run's Authorize affordance ahead of a fresh ACL probe.
        cached.claude.plan_needs_authorization = false;
        if block_on(tx.send(ProducerUpdate {
            snapshot: cached.clone(),
            authorization_attempt_finished: false,
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
        let previous_needs_auth = needs_authorization;
        let mut authorization_attempt_finished = false;
        let mut authorized_token = false;

        if !enabled_now {
            // Gauge disabled: never touch the Keychain. Drop any cached token and
            // last-good plan so the gauges clear immediately and re-enabling
            // forces a fresh read.
            cached_token = None;
            last_read_ms = None;
            last_plan = None;
            needs_authorization = false;
        } else {
            // Token acquisition happens OUTSIDE the shared fetch throttle so an
            // Authorize click acts on the next tick instead of waiting out the
            // 5-minute fetch cadence. The gesture is consumed only while
            // enabled, so a click racing the settings flag survives to the
            // next tick rather than being dropped.
            let gesture = authorize.swap(false, Ordering::Relaxed);
            let read_due = should_read_keychain(
                cached_token.as_ref(),
                last_read_ms,
                now_ms,
                REREAD_BACKOFF_MS,
            );
            if gesture || read_due {
                let acquisition = acquire_claude_token(&keychain, &paths.os_account, gesture);
                authorized_token = gesture && matches!(&acquisition, TokenAcquisition::Token(_));
                match acquisition {
                    TokenAcquisition::Token(token) => {
                        cached_token = Some(token);
                        needs_authorization = false;
                    }
                    TokenAcquisition::ItemMissing | TokenAcquisition::RetryLater => {
                        cached_token = None;
                        needs_authorization = false;
                    }
                    TokenAcquisition::NeedsAuthorization => {
                        cached_token = None;
                        needs_authorization = true;
                    }
                }
                last_read_ms = Some(now_ms);
                authorization_attempt_finished = gesture;
            }

            last_plan = if needs_authorization {
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
                    // actually changed. The shared acquisition path still proves
                    // this background re-read silent and applies hard deadlines.
                    if matches!(outcome, PlanFetchOutcome::Unauthorized)
                        && last_read_ms
                            .map(|last| now_ms.saturating_sub(last) >= REREAD_BACKOFF_MS)
                            .unwrap_or(true)
                    {
                        let refreshed =
                            match acquire_claude_token(&keychain, &paths.os_account, false) {
                                TokenAcquisition::Token(token) => {
                                    needs_authorization = false;
                                    Some(token)
                                }
                                TokenAcquisition::ItemMissing | TokenAcquisition::RetryLater => {
                                    needs_authorization = false;
                                    None
                                }
                                TokenAcquisition::NeedsAuthorization => {
                                    needs_authorization = true;
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
                let refreshed_plan = if authorized_token {
                    claude_plan_cache::refresh_shared_after_authorization(
                        &paths.snapshot_cache,
                        now,
                        fetch_plan,
                    )
                } else {
                    claude_plan_cache::refresh_shared(&paths.snapshot_cache, now, fetch_plan)
                };
                if needs_authorization {
                    // A 401-triggered refresh may discover that the current
                    // credential now needs a gesture. Never leave a stale plan
                    // beside the Authorize affordance.
                    None
                } else {
                    refreshed_plan
                }
            };
        }

        // Plan refreshes (and Authorize-state flips) should not sit behind a
        // 10s+ recursive transcript scan. Push them with the last local
        // snapshot immediately; the live scan below replaces the local totals
        // on the same loop.
        if last_plan != previous_plan
            || needs_authorization != previous_needs_auth
            || authorization_attempt_finished
        {
            let mut preview = last_stored.clone().unwrap_or_default();
            preview.claude.plan = last_plan;
            preview.claude.plan_needs_authorization = needs_authorization;
            if block_on(tx.send(ProducerUpdate {
                snapshot: preview,
                authorization_attempt_finished,
            }))
            .is_err()
            {
                return;
            }
        }

        let mut snap = scan_local(&paths, &mut caches, now);
        snap.claude.plan = last_plan;
        snap.claude.plan_needs_authorization = needs_authorization;
        if last_stored.as_ref() != Some(&snap) {
            snapshot_cache::store(&paths.snapshot_cache, &snap, now);
            last_stored = Some(snap.clone());
        }
        if block_on(tx.send(ProducerUpdate {
            snapshot: snap,
            authorization_attempt_finished: false,
        }))
        .is_err()
        {
            break; // receiver dropped (model gone) => exit cleanly
        }
        std::thread::park_timeout(FILE_POLL);
    }
}
