// We can use `std::process::Command` here because this module is only used by
// WarpUI's build script and its regression test.
#![allow(clippy::disallowed_types)]

use std::process::Command;

/// Run Xcode command-line tools in the Mac's native architecture.
///
/// The architecture order makes Apple Silicon Macs select arm64 even when the
/// parent build is producing an x86_64 target. Intel Macs skip the unavailable
/// arm64 slice and use x86_64.
pub(crate) fn native_xcrun() -> Command {
    let mut command = Command::new("/usr/bin/arch");
    command.args(["-arm64", "-x86_64", "/usr/bin/xcrun"]);
    command
}
