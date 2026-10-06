//! Whether anyone could be looking at the Mac's screen right now.

/// True while the Mac's screen is locked, its display is asleep, or another user owns the console.
/// macOS keeps the frontmost app "active" through all of these, so app activation alone would claim
/// the desktop is watching a pane that nobody can see.
#[cfg(target_os = "macos")]
pub(super) fn desktop_unattended() -> bool {
    use std::ffi::c_void;
    use std::ptr::NonNull;

    use objc2_core_foundation::{CFBoolean, CFDictionary, CFNumber, CFRetained, CFString, CFType};

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGSessionCopyCurrentDictionary() -> *mut CFDictionary;
        fn CGMainDisplayID() -> u32;
        fn CGDisplayIsAsleep(display: u32) -> u32;
    }

    // SAFETY: plain CoreGraphics queries. The session dictionary follows the Create rule, so
    // `CFRetained::from_raw` takes over the +1 reference, and values read from it are borrowed only
    // while it is alive.
    unsafe {
        if CGDisplayIsAsleep(CGMainDisplayID()) != 0 {
            return true;
        }
        // No window-server session at all (for example a headless login): nobody is watching.
        let Some(session) = NonNull::new(CGSessionCopyCurrentDictionary()) else {
            return true;
        };
        let session = CFRetained::from_raw(session);
        let flag = |key: &'static str| -> Option<bool> {
            let key = CFString::from_static_str(key);
            let value = session.value((&*key as *const CFString).cast::<c_void>());
            if value.is_null() {
                return None;
            }
            // Documented as booleans, but accept a number too rather than silently miss a lock.
            let value = &*value.cast::<CFType>();
            value
                .downcast_ref::<CFBoolean>()
                .map(CFBoolean::value)
                .or_else(|| value.downcast_ref::<CFNumber>()?.as_i64().map(|n| n != 0))
        };
        flag("CGSSessionScreenIsLocked") == Some(true)
            || flag("kCGSSessionOnConsoleKey") == Some(false)
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn desktop_unattended() -> bool {
    false
}
