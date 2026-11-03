use objc2_app_kit::{NSAlert, NSApplication};

/// Prepares an `NSAlert` to be shown.
///
/// Exported via `#[no_mangle]` so the AppKit `showModal:modalId:` dispatch path in `app.m` can call
/// it by its C symbol.
#[no_mangle]
pub extern "C-unwind" fn configureModal(alert: &NSAlert, app: &NSApplication) {
    alert.setShowsSuppressionButton(true);

    // It is generally frowned-upon to be overly assertive about putting our windows in
    // the user's face. However, it is reasonable to do this before showing our modal. If
    // we don't make ourselves the top active app, our modal might show up behind another
    // app's window.
    app.activateIgnoringOtherApps(true);
}
