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
        ChannelConfig::no_backend(AppId::new("sh", "clinch", "ClinchDev"), "clinch-dev.log");

    let json = serde_json::to_string(&config).expect("serialize");
    let back: ChannelConfig = serde_json::from_str(&json).expect("deserialize");

    assert_eq!(back.app_id.to_string(), "sh.clinch.ClinchDev");
    assert_eq!(back.logfile_name, "clinch-dev.log");
    assert_eq!(back.server_config.server_root_url, "http://192.0.2.0:9");
    assert_eq!(back.oz_config.oz_root_url, "http://192.0.2.0:9");
    assert!(!back.has_backend);
}

#[test]
fn no_backend_has_backend_is_false() {
    let config = ChannelConfig::no_backend(AppId::new("sh", "clinch", "Clinch"), "clinch.log");

    assert!(!config.has_backend);
}

#[test]
fn clinch_enables_only_the_signed_github_updater() {
    let config = ChannelConfig::clinch(AppId::new("sh", "clinch", "Clinch"), "clinch.log");

    assert!(!config.has_backend);
    assert!(config.telemetry_config.is_none());
    let updater = config.autoupdate_config.expect("Clinch updater config");
    assert_eq!(updater.provider, AutoupdateProvider::ClinchGithub);
    assert!(updater.show_autoupdate_menu_items);
    assert_eq!(
        updater.releases_base_url,
        "https://api.github.com/repos/elliot-ylambda/clinch-terminal/releases"
    );
}

#[test]
fn deserializing_config_missing_has_backend_defaults_to_true() {
    // Upstream's generated dev/preview JSON channel configs predate the
    // `has_backend` field, so deserializing JSON that omits it entirely must
    // still yield `has_backend: true` (a real-backend build). Note: this test
    // builds the JSON by string surgery, not via `serde_json::Value`, because
    // `AppId`'s `Deserialize` impl requires a borrowed `&str` (see
    // `AppId::deserialize`), which `Value`-based deserialization can't supply.
    let config = ChannelConfig::no_backend(AppId::new("dev", "warp", "WarpDev"), "warp-dev.log");
    let json = serde_json::to_string(&config).expect("serialize");
    let without_has_backend = json.replace(r#","has_backend":false"#, "");
    assert_ne!(
        json, without_has_backend,
        "expected serialized JSON to contain a `has_backend` key to strip"
    );

    let back: ChannelConfig = serde_json::from_str(&without_has_backend).expect("deserialize");

    assert!(back.has_backend);
}
