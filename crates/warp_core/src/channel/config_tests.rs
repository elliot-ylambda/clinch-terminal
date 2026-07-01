use super::*;
use crate::AppId;

#[test]
fn no_backend_urls_parse_and_are_unroutable() {
    let config = ChannelConfig::no_backend(AppId::new("sh", "clinch", "Clinch"), "clinch.log");

    // Several call sites do `Url::parse(...).unwrap()`/`.expect(...)` on these, so every URL
    // MUST parse or the app panics at startup. Empty strings would not parse — that is the whole
    // reason `offline()` uses black-hole URLs instead of "".
    for url in [
        config.server_config.server_root_url.as_ref(),
        config.server_config.rtc_server_url.as_ref(),
        config.oz_config.oz_root_url.as_ref(),
    ] {
        url::Url::parse(url).unwrap_or_else(|e| panic!("URL {url:?} must parse: {e}"));
    }

    // No real backend, credentials, or optional subsystems.
    assert_eq!(config.server_config.firebase_auth_api_key, "");
    assert!(config.server_config.session_sharing_server_url.is_none());
    assert!(config.server_config.iap_config.is_none());
    assert!(config.oz_config.workload_audience_url.is_none());
    assert!(config.telemetry_config.is_none());
    assert!(config.autoupdate_config.is_none());
    assert!(config.crash_reporting_config.is_none());
    assert!(config.mcp_static_config.is_none());

    assert_eq!(config.app_id.to_string(), "sh.clinch.Clinch");
    assert_eq!(config.logfile_name, "clinch.log");
}

#[test]
fn no_backend_round_trips_through_serde() {
    let config =
        ChannelConfig::no_backend(AppId::new("dev", "warp", "Warp-Local"), "warp-local.log");

    let json = serde_json::to_string(&config).expect("serialize");
    let back: ChannelConfig = serde_json::from_str(&json).expect("deserialize");

    assert_eq!(back.app_id.to_string(), "dev.warp.Warp-Local");
    assert_eq!(back.logfile_name, "warp-local.log");
    assert_eq!(back.server_config.server_root_url, "http://192.0.2.0:9");
    assert_eq!(back.oz_config.oz_root_url, "http://192.0.2.0:9");
}
