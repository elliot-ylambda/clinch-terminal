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
    read_claude_token, should_read_keychain, ClaudeToken, MacKeychain, ReadSecret,
};
use cli_agent_usage::{fetch_plan_for_token, scan_local, Caches, Paths, PlanLimits, UsageSnapshot};
use warpui::r#async::block_on;
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::settings::CliAgentUsageSettings;

/// How often the producer thread re-scans local files.
const FILE_POLL: Duration = Duration::from_secs(5);
/// Fetch the Claude usage endpoint every Nth tick (~60s at FILE_POLL = 5s).
const ENDPOINT_EVERY: u64 = 12;
/// While we lack a fresh, valid Keychain token, re-read the Keychain at most
/// this often. Reading the Keychain is what triggers the macOS credential
/// prompt, so this bounds prompts to ~one per 5 min in the worst case (Claude
/// Code's stored token itself expired); normally it is one read per launch.
const REREAD_BACKOFF_MS: i64 = 5 * 60 * 1000;

pub enum CliAgentUsageModelEvent {
    Updated,
}

pub struct CliAgentUsageModel {
    latest: UsageSnapshot,
}

impl Entity for CliAgentUsageModel {
    type Event = CliAgentUsageModelEvent;
}

impl SingletonEntity for CliAgentUsageModel {}

impl CliAgentUsageModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let (tx, rx) = async_channel::unbounded::<UsageSnapshot>();

        // Bridge the main-thread-only `show_plan_limits` setting to the
        // off-thread producer with a lock-free atomic (same pattern as
        // FeatureFlag). Seeded from the current value; kept live by the
        // subscription below. When false, the producer never reads the Keychain,
        // so the credential prompt never fires.
        let enabled = Arc::new(AtomicBool::new(
            *CliAgentUsageSettings::as_ref(ctx).show_plan_limits,
        ));

        if let Some(paths) = Paths::detect() {
            // Dedicated OS thread => guaranteed no Tokio runtime context.
            let enabled = enabled.clone();
            let _ = std::thread::Builder::new()
                .name("cli-agent-usage".to_string())
                .spawn(move || producer_loop(paths, tx, enabled));
        }

        // Track setting changes (Settings UI or Command Palette toggle). The
        // producer observes the new value on its next tick.
        ctx.subscribe_to_model(&CliAgentUsageSettings::handle(ctx), {
            let enabled = enabled.clone();
            move |_model, _handle, _event, ctx| {
                enabled.store(
                    *CliAgentUsageSettings::as_ref(ctx).show_plan_limits,
                    Ordering::Relaxed,
                );
            }
        });

        // Deliver each snapshot on the main thread; store it and notify observers.
        ctx.spawn_stream_local(rx, Self::on_snapshot, |_, _| {});
        Self {
            latest: UsageSnapshot::default(),
        }
    }

    /// Test-only constructor: skips the producer thread (which reads the macOS
    /// keychain and makes a blocking HTTP call) so workspace tests that build
    /// the footer can register and subscribe to this singleton without touching
    /// the network or keychain. Holds a default snapshot forever.
    #[cfg(test)]
    pub fn new_for_test() -> Self {
        Self {
            latest: UsageSnapshot::default(),
        }
    }

    pub fn latest(&self) -> &UsageSnapshot {
        &self.latest
    }

    fn on_snapshot(&mut self, snap: UsageSnapshot, ctx: &mut ModelContext<Self>) {
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
        if snap == self.latest && !countdown_is_live {
            return;
        }
        self.latest = snap;
        ctx.emit(CliAgentUsageModelEvent::Updated);
        ctx.notify();
    }
}

/// Runs on the dedicated thread. Split cadence: local scans every `FILE_POLL`,
/// the Claude usage endpoint every `ENDPOINT_EVERY` ticks, retaining the last good
/// `PlanLimits` across transient failures. Exits when the receiver is dropped.
///
/// The Claude OAuth token is cached in `cached_token`: the Keychain (which
/// triggers the macOS credential prompt) is read only when we lack a usable
/// token, not on every endpoint tick. When `enabled` is false the Keychain is
/// never read at all and the plan gauges clear.
fn producer_loop(paths: Paths, tx: async_channel::Sender<UsageSnapshot>, enabled: Arc<AtomicBool>) {
    let mut caches = Caches::new();
    let keychain = MacKeychain;
    let fetch = ReqwestUsage;
    let mut last_plan: Option<PlanLimits> = None;
    let mut cached_token: Option<ClaudeToken> = None;
    let mut last_read_ms: Option<i64> = None;
    let mut tick: u64 = 0;
    loop {
        let now = Utc::now();
        let now_ms = now.timestamp_millis();
        let mut snap = scan_local(&paths, &mut caches, now);

        if !enabled.load(Ordering::Relaxed) {
            // Gauge disabled: never touch the Keychain. Drop any cached token and
            // last-good plan so the gauges clear immediately and re-enabling
            // forces a fresh read.
            cached_token = None;
            last_read_ms = None;
            last_plan = None;
        } else if tick.is_multiple_of(ENDPOINT_EVERY) {
            // Read the Keychain (the prompt-triggering call) only when we lack a
            // usable token; otherwise reuse the cached one.
            if should_read_keychain(
                cached_token.as_ref(),
                last_read_ms,
                now_ms,
                REREAD_BACKOFF_MS,
            ) {
                cached_token = read_claude_token(&keychain as &dyn ReadSecret, &paths.os_account);
                last_read_ms = Some(now_ms);
            }
            if let Some(token) = &cached_token {
                if let Some(fresh) = fetch_plan_for_token(&fetch as &dyn FetchUsage, token, now_ms)
                {
                    last_plan = Some(fresh); // overwrite only on success => last-good retained
                }
            }
        }

        snap.claude.plan = last_plan;
        if block_on(tx.send(snap)).is_err() {
            break; // receiver dropped (model gone) => exit cleanly
        }
        tick = tick.wrapping_add(1);
        std::thread::sleep(FILE_POLL);
    }
}
