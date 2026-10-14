//! Native dialogs attached to the editor window.
//!
//! The host runs beside the WebView and has no window handle of its own, so
//! dialogs are parented to the foreground window: the editor window the user
//! just clicked in. That keeps them modal to it instead of opening behind.

use rfd::{AsyncFileDialog, AsyncMessageDialog};

/// A file dialog owned by the current foreground window.
pub fn file() -> AsyncFileDialog {
    let dialog = AsyncFileDialog::new();
    match Foreground::current() {
        Some(parent) => dialog.set_parent(&parent),
        None => dialog,
    }
}

/// A message dialog owned by the current foreground window.
pub fn message() -> AsyncMessageDialog {
    let dialog = AsyncMessageDialog::new();
    match Foreground::current() {
        Some(parent) => dialog.set_parent(&parent),
        None => dialog,
    }
}

struct Foreground(raw_window_handle::RawWindowHandle);

impl Foreground {
    #[cfg(windows)]
    fn current() -> Option<Self> {
        use std::num::NonZeroIsize;
        // SAFETY: GetForegroundWindow has no preconditions; it may return null.
        let hwnd = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
        let hwnd = NonZeroIsize::new(hwnd as isize)?;
        Some(Self(raw_window_handle::Win32WindowHandle::new(hwnd).into()))
    }

    #[cfg(not(windows))]
    fn current() -> Option<Self> {
        None
    }
}

impl raw_window_handle::HasWindowHandle for Foreground {
    fn window_handle(&self) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        // SAFETY: the handle names a live top-level window for the dialog's
        // duration; if it closes, the dialog simply loses its owner.
        Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(self.0) })
    }
}

impl raw_window_handle::HasDisplayHandle for Foreground {
    fn display_handle(&self) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        Ok(raw_window_handle::DisplayHandle::windows())
    }
}
