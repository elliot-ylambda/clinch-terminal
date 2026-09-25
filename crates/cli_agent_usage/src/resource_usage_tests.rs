use std::collections::HashSet;

use chrono::{Duration, TimeZone, Utc};

use super::*;

#[test]
fn aggregate_skips_expired_dedup_keys_and_keeps_the_month_boundary() {
    let now = Utc.with_ymd_and_hms(2026, 9, 23, 12, 0, 0).unwrap();
    let entry = |ts, dedup, output| Entry {
        ts,
        dedup,
        model: "gpt-5.5".to_string(),
        tokens: TokenCounts {
            output,
            ..Default::default()
        },
    };
    let mut entries: Vec<_> = (0..1024)
        .map(|index| entry(now - Duration::days(31), format!("expired-{index}"), 100))
        .collect();
    entries.push(entry(now - Duration::days(30), "boundary".into(), 4));
    entries.push(entry(now, "current".into(), 6));
    entries.push(entry(now, "current".into(), 6));

    let mut seen = HashSet::new();
    let (mut today, mut week, mut month) = Default::default();
    aggregate_windows(&entries, now, &mut seen, &mut today, &mut week, &mut month);

    assert_eq!(today.tokens.output, 6);
    assert_eq!(week.tokens.output, 6);
    assert_eq!(month.tokens.output, 10);
    assert_eq!(
        seen,
        HashSet::from(["boundary".to_string(), "current".to_string()])
    );
}

#[test]
fn repeated_provider_scans_release_deleted_sources_without_losing_session_totals() {
    let root = std::env::temp_dir().join(format!("cau_deleted_sources_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let claude_root = root.join("claude");
    let codex_root = root.join("codex");
    std::fs::create_dir_all(&claude_root).unwrap();
    std::fs::create_dir_all(&codex_root).unwrap();
    let claude_path = claude_root.join("session.jsonl");
    let codex_path = codex_root.join("rollout-session.jsonl");
    std::fs::write(&claude_path, r#"{"type":"assistant","timestamp":"2026-01-01T12:00:00Z","requestId":"request","message":{"id":"message","model":"claude-haiku","usage":{"output_tokens":5}}}"#).unwrap();
    std::fs::write(&codex_path, r#"{"timestamp":"2026-01-01T12:00:00Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":0,"output_tokens":7}}}}"#).unwrap();
    let paths = Paths {
        claude_projects: claude_root,
        codex_sessions: codex_root,
        os_account: String::new(),
        snapshot_cache: root.join("unused.json"),
    };
    let mut caches = Caches::new();
    let now = Utc.with_ymd_and_hms(2026, 9, 23, 12, 0, 0).unwrap();
    let first = scan_local(&paths, &mut caches, now);
    assert_eq!(first.claude.session.tokens.output, 5);
    assert_eq!(first.codex.session.tokens.output, 7);
    assert_eq!(first.claude.month.tokens.output, 0);
    assert_eq!(first.codex.month.tokens.output, 0);
    assert_eq!(scan_local(&paths, &mut caches, now), first);

    // Losing access to the optional plan credential must not lose local
    // Claude/Codex totals or try an authenticated network request.
    struct Denied;
    impl keychain::ReadSecret for Denied {
        fn read(&self, _: &str, _: &str) -> keychain::SecretRead {
            keychain::SecretRead::Unavailable
        }
    }
    struct NoFetch;
    impl http::FetchUsage for NoFetch {
        fn fetch(&self, _: &str) -> Result<String, http::FetchError> {
            panic!("denied credential must not make a usage request")
        }
    }
    assert_eq!(refresh(&paths, &mut caches, now, &Denied, &NoFetch), first);

    let claude_metadata = std::fs::metadata(&claude_path).unwrap();
    let codex_metadata = std::fs::metadata(&codex_path).unwrap();
    std::fs::remove_file(&claude_path).unwrap();
    std::fs::remove_file(&codex_path).unwrap();
    assert_eq!(
        scan_local(&paths, &mut caches, now),
        UsageSnapshot::default()
    );

    let mut claude_reparsed = false;
    caches.claude.get_or_parse(
        &claude_path,
        claude_metadata.modified().unwrap(),
        claude_metadata.len(),
        |_| {
            claude_reparsed = true;
            Vec::new()
        },
    );
    let mut codex_reparsed = false;
    caches.codex.get_or_parse(
        &codex_path,
        codex_metadata.modified().unwrap(),
        codex_metadata.len(),
        |_| {
            codex_reparsed = true;
            codex::RollupFile::default()
        },
    );
    assert!(
        claude_reparsed && codex_reparsed,
        "deleted sources stayed cached"
    );
    std::fs::remove_dir_all(root).unwrap();
}
