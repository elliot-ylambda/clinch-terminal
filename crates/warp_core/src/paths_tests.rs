use dirs::home_dir;

use super::*;

#[test]
fn test_data_dir_path() {
    let home_dir = home_dir().expect("Should be able to compute home directory");
    // ChannelState, by default, is configured for Channel::Oss.
    cfg_if::cfg_if! {
        if #[cfg(target_os = "macos")] {
            assert_eq!(data_dir(), home_dir.join(".warp-oss"));
        } else if #[cfg(any(target_os = "linux", target_os = "freebsd"))] {
            assert_eq!(data_dir(), home_dir.join(".local/share/warp-oss"));
        } else if #[cfg(windows)] {
            assert_eq!(data_dir(), home_dir.join("AppData\\Roaming\\warp\\WarpOss\\data"));
        } else {
            unimplemented!("Need to update tests for current platform!");
        }
    }
}

#[test]
fn test_config_local_dir_path() {
    let home_dir = home_dir().expect("Should be able to compute home directory");
    // ChannelState, by default, is configured for Channel::Oss.
    cfg_if::cfg_if! {
        if #[cfg(target_os = "macos")] {
            assert_eq!(config_local_dir(), home_dir.join(".warp-oss"));
        } else if #[cfg(any(target_os = "linux", target_os = "freebsd"))] {
            assert_eq!(config_local_dir(), home_dir.join(".config/warp-oss"));
        } else if #[cfg(windows)] {
            assert_eq!(config_local_dir(), home_dir.join("AppData\\Local\\warp\\WarpOss\\config"));
        } else {
            unimplemented!("Need to update tests for current platform!");
        }
    }
}

#[test]
fn test_warp_home_config_dir_path() {
    let home_dir = home_dir().expect("Should be able to compute home directory");
    let expected_dir_name = match ChannelState::data_profile() {
        Some(data_profile) => format!(".warp-oss-{data_profile}"),
        None => ".warp-oss".to_string(),
    };

    assert_eq!(
        warp_home_config_dir(),
        Some(home_dir.join(expected_dir_name))
    );
}

#[test]
fn test_warp_home_skills_and_mcp_paths() {
    let Some(config_dir) = warp_home_config_dir() else {
        panic!("Should be able to compute Warp home config directory");
    };

    assert_eq!(warp_home_skills_dir(), Some(config_dir.join("skills")));
    assert_eq!(
        warp_home_mcp_config_file_path(),
        Some(config_dir.join(".mcp.json"))
    );
}

#[test]
fn test_local_home_config_dir_name_is_scoped_to_the_app_owner() {
    assert_eq!(
        local_home_config_dir_name_for_app_id(&AppId::new("sh", "clinch", "ClinchDev")),
        ".clinch-local"
    );
    assert_eq!(
        local_home_config_dir_name_for_app_id(&AppId::new("dev", "warp", "Warp-Local")),
        ".warp-local"
    );
}

#[test]
fn test_cache_dir_path() {
    let home_dir = home_dir().expect("Should be able to compute home directory");
    // ChannelState, by default, is configured for Channel::Oss.
    cfg_if::cfg_if! {
        if #[cfg(target_os = "macos")] {
            assert_eq!(cache_dir(), home_dir.join("Library/Application Support/dev.warp.WarpOss"));
        } else if #[cfg(any(target_os = "linux", target_os = "freebsd"))] {
            assert_eq!(cache_dir(), home_dir.join(".cache/warp-oss"));
        } else if #[cfg(windows)] {
            assert_eq!(cache_dir(), home_dir.join("AppData\\Local\\warp\\WarpOss\\cache"));
        } else {
            unimplemented!("Need to update tests for current platform!");
        }
    }
}

#[test]
fn test_state_dir_path() {
    let home_dir = home_dir().expect("Should be able to compute home directory");
    cfg_if::cfg_if! {
        // ChannelState, by default, is configured for Channel::Oss.
        if #[cfg(target_os = "macos")] {
            assert_eq!(state_dir(), home_dir.join("Library/Application Support/dev.warp.WarpOss"));
        } else if #[cfg(any(target_os = "linux", target_os = "freebsd"))] {
            assert_eq!(state_dir(), home_dir.join(".local/state/warp-oss"));
        } else if #[cfg(windows)] {
            assert_eq!(state_dir(), home_dir.join("AppData\\Local\\warp\\WarpOss\\data"));
        } else {
            unimplemented!("Need to update tests for current platform!");
        }
    }
}

#[test]
fn test_project_path_for_warp_app_id() {
    let project_dirs = project_dirs_for_app_id(AppId::new("dev", "warp", "Warp"), None)
        .expect("should be able to compute project dirs");
    cfg_if::cfg_if! {
        if #[cfg(target_os = "macos")] {
            assert_eq!(project_dirs.project_path(), "dev.warp.Warp");
        } else if #[cfg(any(target_os = "linux", target_os = "freebsd"))] {
            assert_eq!(project_dirs.project_path(), "warp-terminal");
        } else if #[cfg(windows)] {
            assert_eq!(project_dirs.project_path(), "warp\\Warp");
        } else {
            unimplemented!("Need to update tests for current platform!");
        }
    }
}

#[test]
fn test_project_path_for_warp_dev_app_id() {
    let project_dirs = project_dirs_for_app_id(AppId::new("dev", "warp", "WarpDev"), None)
        .expect("should be able to compute project dirs");
    cfg_if::cfg_if! {
        if #[cfg(target_os = "macos")] {
            assert_eq!(project_dirs.project_path(), "dev.warp.WarpDev");
        } else if #[cfg(any(target_os = "linux", target_os = "freebsd"))] {
            assert_eq!(project_dirs.project_path(), "warp-terminal-dev");
        } else if #[cfg(windows)] {
            assert_eq!(project_dirs.project_path(), "warp\\WarpDev");
        } else {
            unimplemented!("Need to update tests for current platform!");
        }
    }
}

#[test]
fn test_project_path_for_oss_app_id() {
    let project_dirs = project_dirs_for_app_id(AppId::new("dev", "warp", "WarpOss"), None)
        .expect("should be able to compute project dirs");
    cfg_if::cfg_if! {
        if #[cfg(target_os = "macos")] {
            assert_eq!(project_dirs.project_path(), "dev.warp.WarpOss");
        } else if #[cfg(any(target_os = "linux", target_os = "freebsd"))] {
            assert_eq!(project_dirs.project_path(), "warp-oss");
        } else if #[cfg(windows)] {
            assert_eq!(project_dirs.project_path(), "warp\\WarpOss");
        } else {
            unimplemented!("Need to update tests for current platform!");
        }
    }
}

/// Only official Warp Stable/Preview builds (bundle id `dev.warp.*`, signed by
/// Apple team `2BBY89MBSN`) are entitled to the `<APPLE_TEAM_ID>.dev.warp` app
/// group. Self-signed forks such as Clinch (`sh.clinch.*`) must fall back to
/// per-app storage so macOS does not prompt on every launch.
#[cfg(target_os = "macos")]
#[test]
fn test_should_use_warp_app_group() {
    let warp_stable = AppId::new("dev", "warp", "Warp-Stable");
    let warp_preview = AppId::new("dev", "warp", "Warp-Preview");
    let warp_oss = AppId::new("dev", "warp", "WarpOss");
    let clinch = AppId::new("sh", "clinch", "Clinch");

    // Official Warp Stable/Preview → entitled.
    assert!(should_use_warp_app_group(&warp_stable, Channel::Stable));
    assert!(should_use_warp_app_group(&warp_preview, Channel::Preview));

    // Clinch is a self-signed fork on the Stable channel with a `sh.clinch.*`
    // bundle id → NOT entitled, even though its channel is Stable.
    assert!(!should_use_warp_app_group(&clinch, Channel::Stable));
    assert!(!should_use_warp_app_group(&clinch, Channel::Preview));

    // Non-Stable/Preview channels are never entitled, even for `dev.warp.*`.
    assert!(!should_use_warp_app_group(&warp_oss, Channel::Oss));
    assert!(!should_use_warp_app_group(&warp_stable, Channel::Local));
    assert!(!should_use_warp_app_group(&warp_stable, Channel::Dev));
}
