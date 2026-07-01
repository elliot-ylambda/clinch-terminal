// On Windows, we don't want to display a console window when the application is running in release
// builds. See https://doc.rust-lang.org/reference/runtime.html#the-windows_subsystem-attribute.
#![cfg_attr(feature = "release_bundle", windows_subsystem = "windows")]

use anyhow::Result;
use warp_core::channel::{Channel, ChannelConfig, ChannelState};
use warp_core::AppId;

// Simple wrapper around warp::run() for the stable (Clinch) channel. The Clinch fork ships with
// no backend, so the channel config is constructed inline (like oss.rs) instead of being loaded
// from the private `warp-channel-config` generator.
fn main() -> Result<()> {
    ChannelState::set(ChannelState::new(
        Channel::Stable,
        ChannelConfig::no_backend(AppId::new("sh", "clinch", "Clinch"), "clinch.log"),
    ));

    warp::run()
}
