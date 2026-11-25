//! The window's three buttons on macOS: close, minimize, zoom.
//!
//! When the row of tabs is the title bar, the system draws them in that row,
//! over whatever the window draws there. A row that keeps out of the way
//! (`window.tab_bar = "auto-hide"`) takes them with it: they are hidden
//! while the row is away, or they would lie on the first line of text, and
//! shown again when the row is. The system hides them itself in full screen.
//! On other systems there is nothing to do: UnifyTerm draws its own buttons
//! in the row (Windows), or the window manager has them in its bar (Linux).

use gpui::Window;

/// Whether the three buttons show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lights {
    Shown,
    Hidden,
}

/// Shows or hides the three buttons of `window`. What the system holds
/// after is read back and written to the log (`UNIFYTERM_LOG=info`).
#[cfg(target_os = "macos")]
pub(crate) fn set(window: &Window, lights: Lights) {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    /// `NSWindowCloseButton`, `NSWindowMiniaturizeButton`, `NSWindowZoomButton`.
    const BUTTONS: [usize; 3] = [0, 1, 2];

    // GPUI's window has a method of its own with this name: the trait's is asked for by its whole name.
    let Ok(handle) = HasWindowHandle::window_handle(window) else { return };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else { return };
    let view: *mut AnyObject = handle.ns_view.as_ptr().cast();
    let hidden = lights == Lights::Hidden;
    // SAFETY: `view` is this window's own NSView, alive while `handle` is, and this is the main
    // thread (a window is drawn there). `window` answers the NSWindow the view is in, or nil.
    let ns_window: *mut AnyObject = unsafe { msg_send![view, window] };
    if ns_window.is_null() {
        return;
    }
    let mut now_hidden = 0;
    for button in BUTTONS {
        // SAFETY: `ns_window` is that view's window, not nil. `standardWindowButton:` takes an
        // NSUInteger and answers an NSButton, or nil for a window without that button.
        let button: *mut AnyObject = unsafe { msg_send![ns_window, standardWindowButton: button] };
        if button.is_null() {
            continue;
        }
        // SAFETY: `button` is an NSButton, not nil. `setHidden:` takes a BOOL and answers nothing.
        let () = unsafe { msg_send![button, setHidden: hidden] };
        // SAFETY: the same button. `isHidden` takes nothing and answers a BOOL.
        let is: bool = unsafe { msg_send![button, isHidden] };
        now_hidden += usize::from(is);
    }
    log::info!("window: the system's three buttons asked to be {lights:?}; hidden now: {now_hidden} of 3");
}

/// Elsewhere the system draws no buttons over the row: nothing is done.
#[cfg(not(target_os = "macos"))]
pub(crate) fn set(_window: &Window, _lights: Lights) {}
