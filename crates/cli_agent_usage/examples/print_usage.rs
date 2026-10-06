//! Manual end-to-end check: prints real usage for the current machine.
//! Run: cargo run -p cli_agent_usage --example print_usage
//! Optional Claude plan limits use existing credentials only when silently
//! accessible; this diagnostic never requests a computer password.

use cli_agent_usage::http::ReqwestUsage;
use cli_agent_usage::keychain::MacKeychain;
use cli_agent_usage::{refresh, Caches, Paths, Provider};

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
    #[cfg(target_os = "macos")]
    {
        // This standalone process owns its permanent no-UI policy, just as
        // Clinch does at startup. Never restore interaction after the read.
        let status =
            unsafe { security_framework_sys::keychain::SecKeychainSetUserInteractionAllowed(0) };
        if status != security_framework_sys::base::errSecSuccess {
            eprintln!("Could not disable Keychain interaction; skipping the diagnostic.");
            std::process::exit(1);
        }
    }
    let paths = Paths::detect().expect("HOME set");
    let mut caches = Caches::new();
    let now = chrono::Utc::now();
    let snap = refresh(&paths, &mut caches, now, &MacKeychain, &ReqwestUsage);
    fmt_provider("Claude Code", &snap.claude);
    fmt_provider("Codex", &snap.codex);
}
