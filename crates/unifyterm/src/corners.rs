//! The window's own corners (`theme.corners`).
//!
//! The system draws the window's outline, not UnifyTerm: a program can only
//! say which of the system's shapes it wants, and only Windows 11 asks.
//! There it is one of three: round, round with a smaller radius, square.
//! On Windows 10, macOS and Linux nothing is done: the corners stay as the
//! system makes them.

use gpui::Window;
use unifyterm_config::Corners;

/// Tells the system which corners `window` has; `None`: the system's own choice.
#[cfg(windows)]
pub(crate) fn set(window: &Window, corners: Option<Corners>) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    /// `DWMWA_WINDOW_CORNER_PREFERENCE`, and its values (dwmapi.h).
    const CORNER_PREFERENCE: u32 = 33;
    const DEFAULT: u32 = 0;
    const DO_NOT_ROUND: u32 = 1;
    const ROUND: u32 = 2;
    const ROUND_SMALL: u32 = 3;
    /// The size of the value, in bytes: one `u32`.
    const SIZE: u32 = 4;

    #[link(name = "dwmapi")]
    unsafe extern "system" {
        fn DwmSetWindowAttribute(
            window: *mut std::ffi::c_void,
            attribute: u32,
            value: *const std::ffi::c_void,
            size: u32,
        ) -> i32;
    }

    // GPUI's window has a method of its own with this name: the trait's is asked for by its whole name.
    let Ok(handle) = HasWindowHandle::window_handle(window) else { return };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else { return };
    let wanted = match corners {
        None => DEFAULT,
        Some(Corners::Round) => ROUND,
        Some(Corners::Small) => ROUND_SMALL,
        Some(Corners::Square) => DO_NOT_ROUND,
    };
    // A window handle is a pointer-sized number that Windows gave: it is passed back as it came.
    let window = std::ptr::with_exposed_provenance_mut(handle.hwnd.get().cast_unsigned());
    // SAFETY: `window` is this program's own window, alive while `handle` is; `wanted` is a `u32`
    // that lives through the call, and the call is told its size.
    let result = unsafe { DwmSetWindowAttribute(window, CORNER_PREFERENCE, (&raw const wanted).cast(), SIZE) };
    if result < 0 {
        // Windows 10 does not know the attribute: the corners stay as they are.
        log::debug!("theme: the window's corners were not set: the system answered {result:#x}");
    }
}

/// Elsewhere the system does not ask: nothing is done.
#[cfg(not(windows))]
pub(crate) fn set(_window: &Window, _corners: Option<Corners>) {}
