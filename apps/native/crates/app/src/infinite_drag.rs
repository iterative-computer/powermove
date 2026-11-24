//! Pointer-locked infinite horizontal scrubbing for numeric controls on macOS.
//!
//! While engaged, the cursor is disassociated and hidden so the user can scrub
//! continuously past display edges using raw hardware `NSEvent.deltaX` motion.
//! Dropping the guard always warps the pointer back to its anchor, re-associates
//! mouse motion, and unhides the cursor. Under `#[cfg(test)]` or non-macOS
//! targets, this is a safe no-op so synthetic tests never touch the OS cursor.

#[derive(Debug, Default)]
pub struct InfiniteDrag {
    anchor: Option<(f64, f64)>,
}

#[cfg(all(target_os = "macos", not(test)))]
mod macos {
    use objc2_app_kit::{NSApplication, NSCursor, NSEventType};
    use objc2_foundation::MainThreadMarker;
    use std::ffi::c_void;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventCreate(source: *const c_void) -> *const c_void;
        fn CGEventGetLocation(event: *const c_void) -> CGPoint;
        fn CGAssociateMouseAndMouseCursorPosition(connected: i32) -> i32;
        fn CGWarpMouseCursorPosition(new_cursor_position: CGPoint) -> i32;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(cf: *const c_void);
    }

    fn current_drag_event() -> Option<objc2::rc::Retained<objc2_app_kit::NSEvent>> {
        let mtm = MainThreadMarker::new()?;
        let app = NSApplication::sharedApplication(mtm);
        let event = app.currentEvent()?;
        (event.r#type() == NSEventType::LeftMouseDragged).then_some(event)
    }

    pub fn engage() -> Option<(f64, f64)> {
        // Only lock the hardware cursor during a genuine interactive drag.
        current_drag_event()?;
        unsafe {
            let ev = CGEventCreate(std::ptr::null());
            if ev.is_null() {
                return None;
            }
            let loc = CGEventGetLocation(ev);
            CFRelease(ev);
            if !loc.x.is_finite() || !loc.y.is_finite() {
                return None;
            }
            if CGAssociateMouseAndMouseCursorPosition(0) != 0 {
                return None;
            }
            NSCursor::hide();
            Some((loc.x, loc.y))
        }
    }

    pub fn delta_x() -> Option<f64> {
        let dx = current_drag_event()?.deltaX();
        dx.is_finite().then_some(dx)
    }

    pub fn release(anchor: (f64, f64)) {
        unsafe {
            let _ = CGWarpMouseCursorPosition(CGPoint {
                x: anchor.0,
                y: anchor.1,
            });
            let _ = CGAssociateMouseAndMouseCursorPosition(1);
            NSCursor::unhide();
        }
    }
}

impl InfiniteDrag {
    pub fn new() -> Self {
        Self { anchor: None }
    }

    pub fn engage(&mut self) {
        if self.anchor.is_some() {
            return;
        }
        #[cfg(all(target_os = "macos", not(test)))]
        {
            self.anchor = macos::engage();
        }
    }

    pub fn take_delta_x(&self) -> Option<f64> {
        self.anchor?;
        #[cfg(all(target_os = "macos", not(test)))]
        {
            macos::delta_x()
        }
        #[cfg(not(all(target_os = "macos", not(test))))]
        {
            None
        }
    }

    pub fn release(&mut self) {
        if let Some(anchor) = self.anchor.take() {
            #[cfg(all(target_os = "macos", not(test)))]
            macos::release(anchor);
            #[cfg(not(all(target_os = "macos", not(test))))]
            let _ = anchor;
        }
    }
}

impl Drop for InfiniteDrag {
    fn drop(&mut self) {
        self.release();
    }
}
