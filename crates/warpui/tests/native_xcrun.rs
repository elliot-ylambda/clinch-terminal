use std::ffi::OsStr;

#[path = "../build_support.rs"]
mod build_support;

#[test]
fn prefers_native_apple_silicon_xcrun_with_an_intel_fallback() {
    let mut command = build_support::native_xcrun();
    command.args(["-sdk", "macosx", "metal", "--version"]);

    assert_eq!(command.get_program(), OsStr::new("/usr/bin/arch"));
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        [
            "-arm64",
            "-x86_64",
            "/usr/bin/xcrun",
            "-sdk",
            "macosx",
            "metal",
            "--version",
        ]
        .map(OsStr::new)
    );
}
