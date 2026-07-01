//! Manual end-to-end check: prints real usage for the current machine.
//! Run: cargo run -p cli_agent_usage --example print_usage

use cli_agent_usage::{
    http::ReqwestUsage, keychain::MacKeychain, refresh, Caches, Paths, Provider,
};

fn fmt_provider(name: &str, p: &Provider) {
    println!("== {name} ==");
    let tok = |w: &cli_agent_usage::WindowTotals| {
        format!("{} tok  ~${:.2}", w.tokens.total(), w.cost_usd)
    };
    println!("  session: {}", tok(&p.session));
    println!("  today:   {}", tok(&p.today));
    println!("  week:    {}", tok(&p.week));
    println!("  month:   {}", tok(&p.month));
    match &p.plan {
        Some(pl) => {
            if let Some(s) = pl.session {
                println!(
                    "  5h limit:   {:.0}%  (resets {:?})",
                    s.percent, s.resets_at
                );
            }
            if let Some(w) = pl.weekly {
                println!(
                    "  weekly lim: {:.0}%  (resets {:?})",
                    w.percent, w.resets_at
                );
            }
        }
        None => println!("  plan-%: (unavailable)"),
    }
}

fn main() {
    let paths = Paths::detect().expect("HOME set");
    let mut caches = Caches::new();
    let now = chrono::Utc::now();
    let snap = refresh(&paths, &mut caches, now, &MacKeychain, &ReqwestUsage);
    fmt_provider("Claude Code", &snap.claude);
    fmt_provider("Codex", &snap.codex);
}
