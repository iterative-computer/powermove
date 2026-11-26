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
    use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize};
    use std::ffi::c_void;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGSize {
        width: f64,
        height: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGRect {
        origin: CGPoint,
        size: CGSize,
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventCreate(source: *const c_void) -> *const c_void;
        fn CGEventGetLocation(event: *const c_void) -> CGPoint;
        fn CGAssociateMouseAndMouseCursorPosition(connected: i32) -> i32;
        fn CGWarpMouseCursorPosition(new_cursor_position: CGPoint) -> i32;
        fn CGMainDisplayID() -> u32;
        fn CGDisplayBounds(display: u32) -> CGRect;
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

    fn current_pointer_event() -> Option<objc2::rc::Retained<objc2_app_kit::NSEvent>> {
        let mtm = MainThreadMarker::new()?;
        let app = NSApplication::sharedApplication(mtm);
        let event = app.currentEvent()?;
        matches!(
            event.r#type(),
            NSEventType::MouseMoved
                | NSEventType::LeftMouseDragged
                | NSEventType::RightMouseDragged
                | NSEventType::OtherMouseDragged
        )
        .then_some(event)
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

    pub fn engage_pointer_lock(window_point: Option<(f64, f64)>) -> Option<(f64, f64)> {
        let mtm = MainThreadMarker::new()?;
        let app = NSApplication::sharedApplication(mtm);
        if !app.isActive() {
            return None;
        }
        let target = app.keyWindow().and_then(|window| {
            let content_rect = window
                .contentView()
                .map(|v| v.frame())
                .unwrap_or_else(|| window.contentLayoutRect());
            let screen_rect = window.convertRectToScreen(NSRect::new(
                NSPoint::new(content_rect.origin.x, content_rect.origin.y),
                NSSize::new(content_rect.size.width, content_rect.size.height),
            ));
            let primary_h = unsafe { CGDisplayBounds(CGMainDisplayID()).size.height };
            if !primary_h.is_finite() || primary_h <= 0. {
                return None;
            }
            let (vx, vy) = window_point.unwrap_or((
                screen_rect.size.width * 0.5,
                screen_rect.size.height * 0.5,
            ));
            let cg_x = screen_rect.origin.x + vx.clamp(8., (screen_rect.size.width - 8.).max(8.));
            let cg_y = primary_h
                - (screen_rect.origin.y + screen_rect.size.height)
                + vy.clamp(8., (screen_rect.size.height - 8.).max(8.));
            (cg_x.is_finite() && cg_y.is_finite()).then_some(CGPoint { x: cg_x, y: cg_y })
        });
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
            NSCursor::hide();
            if let Some(pt) = target {
                let _ = CGWarpMouseCursorPosition(pt);
            }
            if CGAssociateMouseAndMouseCursorPosition(0) != 0 {
                let _ = CGWarpMouseCursorPosition(loc);
                NSCursor::unhide();
                return None;
            }
            Some((loc.x, loc.y))
        }
    }

    pub fn delta_x() -> Option<f64> {
        let dx = current_drag_event()?.deltaX();
        dx.is_finite().then_some(dx)
    }

    pub fn delta_xy() -> Option<(f64, f64)> {
        let ev = current_pointer_event()?;
        let (dx, dy) = (ev.deltaX(), ev.deltaY());
        (dx.is_finite() && dy.is_finite()).then_some((dx, dy))
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

/// Modal pointer lock for 3D Fly navigation on macOS.
///
/// Hides and disassociates the hardware cursor while parked inside the viewer
/// so mouse-look reads unbounded raw hardware `(deltaX, deltaY)` without moving
/// the actual OS cursor across panels or screen edges. Dropping the guard always
/// restores the original cursor position, re-associates motion, and unhides the
/// cursor. Under `#[cfg(test)]` or non-macOS targets, this is a safe no-op.
#[derive(Debug, Default)]
pub struct PointerLock {
    anchor: Option<(f64, f64)>,
}

impl PointerLock {
    pub fn new() -> Self {
        Self { anchor: None }
    }

    pub fn engage(&mut self, window_point: Option<(f64, f64)>) {
        if self.anchor.is_some() {
            return;
        }
        #[cfg(all(target_os = "macos", not(test)))]
        {
            self.anchor = macos::engage_pointer_lock(window_point);
        }
        #[cfg(not(all(target_os = "macos", not(test))))]
        let _ = window_point;
    }

    pub fn is_locked(&self) -> bool {
        self.anchor.is_some()
    }

    pub fn take_delta(&self) -> Option<(f64, f64)> {
        self.anchor?;
        #[cfg(all(target_os = "macos", not(test)))]
        {
            macos::delta_xy()
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

impl Drop for PointerLock {
    fn drop(&mut self) {
        self.release();
    }
}
