//! Manual check of the Keychain ACL trust probe: reports whether the poller
//! would read Claude Code's credentials silently, wait for an Authorize
//! gesture, or skip because no item exists. Metadata-only — never prompts.
//! Run: cargo run -p cli_agent_usage --example probe_trust [service]

use cli_agent_usage::keychain::{plan_token_read, MacKeychain, ReadSecret, CLAUDE_SERVICE};

fn main() {
    let service = std::env::args()
        .nth(1)
        .unwrap_or_else(|| CLAUDE_SERVICE.to_string());
    let account = std::env::var("USER").unwrap_or_default();
    let trust = MacKeychain.probe_trust(&service, &account);
    println!("service:  {service}");
    println!("account:  {account}");
    println!("trust:    {trust:?}");
    println!("poller:   {:?}", plan_token_read(trust, false));
    println!("gesture:  {:?}", plan_token_read(trust, true));
}
