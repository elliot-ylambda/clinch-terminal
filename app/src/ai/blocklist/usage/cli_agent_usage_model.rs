//! Singleton that keeps the latest CLI-agent (Claude Code + Codex) usage snapshot
//! fresh for the footer. All blocking work (file IO + the Claude usage HTTP call)
//! runs on ONE dedicated `std::thread` — never the gpui background executor, which
//! is Tokio-backed and would make `reqwest::blocking` panic.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use cli_agent_usage::claude_account::ClaudeAccountWatcher;
use cli_agent_usage::claude_plan_cache::RefreshPolicy;
use cli_agent_usage::codex_live::{find_codex_binary, read_limits, LiveCodexLimits};
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

/// One-shot signals that the user is looking at the numbers, so the numbers should be current.
#[derive(Default)]
struct LookingFlags {
    /// Clinch came to the front, or the usage panel opened.
    any: AtomicBool,
    /// The usage panel opened. Claude's endpoint is rate-limited, so only this shortens its wait.
    panel: AtomicBool,
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
    looking: Arc<LookingFlags>,
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
        let looking = Arc::new(LookingFlags::default());

        let producer_thread = if let Some(paths) = Paths::detect() {
            // Dedicated OS thread => guaranteed no Tokio runtime context.
            let enabled = enabled.clone();
            let refresh = refresh.clone();
            let looking = looking.clone();
            std::thread::Builder::new()
                .name("cli-agent-usage".to_string())
                .spawn(move || producer_loop(paths, tx, enabled, refresh, looking))
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
            looking,
            producer_thread,
            refresh_pending: false,
        }
    }

    /// The user is looking at Clinch, or at the usage panel when `panel_opened`, so bring the
    /// numbers up to date now rather than on the next background check.
    pub fn refresh_for_viewer(&self, panel_opened: bool) {
        self.looking.any.store(true, Ordering::Relaxed);
        if panel_opened {
            self.looking.panel.store(true, Ordering::Relaxed);
        }
        if let Some(thread) = &self.producer_thread {
            thread.unpark();
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
            looking: Arc::default(),
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

/// One bounded, silent Keychain read. `None` covers every way of not getting a token.
fn read_claude_token(keychain: &MacKeychain, os_account: &str) -> Option<ClaudeToken> {
    match acquire_claude_token(keychain, os_account) {
        TokenAcquisition::Token(token) => Some(token),
        TokenAcquisition::ItemMissing
        | TokenAcquisition::RetryLater
        | TokenAcquisition::Unavailable => None,
    }
}

/// Runs on the dedicated thread. Local scans run every `FILE_POLL`; the shared
/// Claude plan cache independently throttles endpoint requests by wall-clock
/// time, retaining the last good `PlanLimits` across transient failures and
/// coordinating Stable/Dev processes. Exits when the receiver is dropped.
///
/// The Claude OAuth token is re-read from the Keychain before each real request
/// to the endpoint, because logging in to another account replaces it while the
/// old token keeps working. Between requests the plan cache answers. When
/// `enabled` is false the Keychain is never read at all and the plan gauges
/// clear, and `codex app-server` is never started.
///
/// Every credential read is non-interactive and bounded. Retry clicks bypass
/// the normal cadence but cannot display a system credential dialog.
fn producer_loop(
    paths: Paths,
    tx: async_channel::Sender<ProducerUpdate>,
    enabled: Arc<AtomicBool>,
    refresh: Arc<AtomicBool>,
    looking: Arc<LookingFlags>,
) {
    let mut caches = Caches::new();
    let keychain = MacKeychain;
    let fetch = ReqwestUsage;
    let mut cached_token: Option<ClaudeToken> = None;
    let mut last_read_ms: Option<i64> = None;
    // Non-sticky: background ticks retry on the normal backoff, so a repaired
    // or unlocked Keychain recovers without forcing the user to retry.
    let mut plan_unavailable = false;
    let mut claude_account = ClaudeAccountWatcher::new(paths.claude_config_file.clone());
    let mut last_account: Option<String> = None;
    let codex_binary = find_codex_binary(&paths.home, std::env::var_os("PATH").as_deref());
    let mut live_codex = LiveCodexLimits::default();

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
        let mut codex_live_attempted = false;
        let user_looking = looking.any.swap(false, Ordering::Relaxed);
        let panel_opened = looking.panel.swap(false, Ordering::Relaxed);

        if !enabled_now {
            // Gauge disabled: never touch the Keychain. Drop any cached token and
            // last-good plan so the gauges clear immediately and re-enabling
            // forces a fresh read.
            cached_token = None;
            last_read_ms = None;
            last_plan = None;
            plan_unavailable = false;
            live_codex.clear();
        } else {
            let account = claude_account.current().map(str::to_owned);
            if account != last_account {
                // Forces the Keychain read below onto this tick. The plan cache is keyed by
                // account, so the old account's limits are gone as well.
                cached_token = None;
                last_read_ms = None;
            }

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
                cached_token = read_claude_token(&keychain, &paths.os_account);
                plan_unavailable = cached_token.is_none();
                refreshed_token = gesture && cached_token.is_some();
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
                    cached_token = read_claude_token(&keychain, &paths.os_account);
                    plan_unavailable = cached_token.is_none();
                    last_read_ms = Some(now_ms);
                    cached_token
                        .as_ref()
                        .map(|token| {
                            fetch_plan_for_token_outcome(&fetch as &dyn FetchUsage, token, now_ms)
                        })
                        .unwrap_or(PlanFetchOutcome::Unavailable)
                };
                let refreshed_plan = claude_plan_cache::refresh_shared(
                    &paths.snapshot_cache,
                    now,
                    RefreshPolicy {
                        account: account.as_deref(),
                        retry_empty: refreshed_token,
                        user_looking: panel_opened,
                    },
                    fetch_plan,
                );
                if plan_unavailable {
                    // The read before a request can discover unavailable credentials.
                    // Clear stale gauges while retaining the local usage totals.
                    None
                } else {
                    refreshed_plan
                }
            };
            last_account = account;

            if let Some(binary) = codex_binary.as_deref() {
                // Skip machines that have never used Codex, even with a stray binary around.
                let codex_in_use = paths.codex_sessions.parent().is_some_and(Path::is_dir);
                if codex_in_use && live_codex.is_due(now, user_looking) {
                    live_codex.record_attempt(read_limits(binary, &paths.home), now);
                    codex_live_attempted = true;
                }
            }
        }

        // Plan refreshes (and availability changes) should not sit behind a
        // 10s+ recursive transcript scan. Push them with the last local
        // snapshot immediately; the live scan below replaces the local totals
        // on the same loop.
        if last_plan != previous_plan
            || plan_unavailable != previous_unavailable
            || refresh_attempt_finished
            || codex_live_attempted
        {
            let mut preview = last_stored.clone().unwrap_or_default();
            preview.claude.plan = last_plan;
            preview.claude.plan_unavailable = plan_unavailable;
            preview.codex.apply_live_plan(live_codex.current(now));
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
        snap.codex.apply_live_plan(live_codex.current(now));
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
