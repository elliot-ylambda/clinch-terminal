//! Singleton that keeps the latest CLI-agent (Claude Code + Codex) usage snapshot
//! fresh for the footer. All blocking work (file IO + the Claude usage HTTP call)
//! runs on ONE dedicated `std::thread` — never the gpui background executor, which
//! is Tokio-backed and would make `reqwest::blocking` panic.

use std::time::Duration;

use chrono::Utc;
use cli_agent_usage::http::{FetchUsage, ReqwestUsage};
use cli_agent_usage::keychain::{MacKeychain, ReadSecret};
use cli_agent_usage::{fetch_claude_plan, scan_local, Caches, Paths, PlanLimits, UsageSnapshot};
use warpui::{Entity, ModelContext, SingletonEntity};

/// How often the producer thread re-scans local files.
const FILE_POLL: Duration = Duration::from_secs(5);
/// Fetch the Claude usage endpoint every Nth tick (~60s at FILE_POLL = 5s).
const ENDPOINT_EVERY: u64 = 12;

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
        if let Some(paths) = Paths::detect() {
            // Dedicated OS thread => guaranteed no Tokio runtime context.
            let _ = std::thread::Builder::new()
                .name("cli-agent-usage".to_string())
                .spawn(move || producer_loop(paths, tx));
        }
        // Deliver each snapshot on the main thread; store it and notify observers.
        ctx.spawn_stream_local(rx, Self::on_snapshot, |_, _| {});
        Self {
            latest: UsageSnapshot::default(),
        }
    }

    pub fn latest(&self) -> &UsageSnapshot {
        &self.latest
    }

    fn on_snapshot(&mut self, snap: UsageSnapshot, ctx: &mut ModelContext<Self>) {
        self.latest = snap;
        ctx.emit(CliAgentUsageModelEvent::Updated);
        ctx.notify();
    }
}

/// Runs on the dedicated thread. Split cadence: local scans every `FILE_POLL`,
/// the Claude usage endpoint every `ENDPOINT_EVERY` ticks, retaining the last good
/// `PlanLimits` across transient failures. Exits when the receiver is dropped.
fn producer_loop(paths: Paths, tx: async_channel::Sender<UsageSnapshot>) {
    let mut caches = Caches::new();
    let keychain = MacKeychain;
    let fetch = ReqwestUsage;
    let mut last_plan: Option<PlanLimits> = None;
    let mut tick: u64 = 0;
    loop {
        let now = Utc::now();
        let mut snap = scan_local(&paths, &mut caches, now);
        if tick % ENDPOINT_EVERY == 0 {
            if let Some(fresh) = fetch_claude_plan(
                &keychain as &dyn ReadSecret,
                &fetch as &dyn FetchUsage,
                &paths,
                now,
            ) {
                last_plan = Some(fresh); // overwrite only on success => last-good retained
            }
        }
        snap.claude.plan = last_plan;
        if tx.send_blocking(snap).is_err() {
            break; // receiver dropped (model gone) => exit cleanly
        }
        tick = tick.wrapping_add(1);
        std::thread::sleep(FILE_POLL);
    }
}
