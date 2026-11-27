//! Pointer-locked infinite horizontal scrubbing for numeric controls on macOS.
//!
//! While engaged, the cursor is disassociated and hidden so the user can scrub
//! continuously past display edges using raw hardware `NSEvent.deltaX` motion.
//! Dropping the guard always warps the pointer back to its anchor, re-associates
//! mouse motion, and unhides the cursor. Under `#[cfg(test)]` or non-macOS
//! targets, this is a safe no-op so synthetic tests never touch the OS cursor.

use std::cell::Cell;

/// Identifies a specific AppKit `NSEvent` instance so synthetic GPUI mouse-move
/// repaints while `NSApp.currentEvent` still points to the last hardware drag
/// event never re-consume its `deltaX`/`deltaY` in a runaway loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EventStamp {
    timestamp_bits: u64,
    event_number: isize,
    ptr: usize,
}

#[derive(Debug, Default)]
pub struct InfiniteDrag {
    anchor: Option<(f64, f64)>,
    last_event: Cell<Option<EventStamp>>,
}

fn dedup_delta<T: Copy>(
    last: &Cell<Option<EventStamp>>,
    sample: Option<(EventStamp, T)>,
    zero: T,
) -> T {
    let Some((stamp, delta)) = sample else {
        return zero;
    };
    if last.get() == Some(stamp) {
        return zero;
    }
    last.set(Some(stamp));
    delta
}

#[cfg(all(target_os = "macos", not(test)))]
mod macos {
    use super::EventStamp;
    use objc2::rc::Retained;
    use objc2_app_kit::{NSApplication, NSCursor, NSEvent, NSEventType};
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

    fn event_stamp(event: &NSEvent) -> EventStamp {
        EventStamp {
            timestamp_bits: event.timestamp().to_bits(),
            event_number: event.eventNumber(),
            ptr: event as *const NSEvent as usize,
        }
    }

    fn current_drag_event() -> Option<Retained<NSEvent>> {
        let mtm = MainThreadMarker::new()?;
        let app = NSApplication::sharedApplication(mtm);
        let event = app.currentEvent()?;
        (event.r#type() == NSEventType::LeftMouseDragged).then_some(event)
    }

    fn current_pointer_event() -> Option<Retained<NSEvent>> {
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

    pub fn engage() -> Option<((f64, f64), EventStamp)> {
        // Only lock the hardware cursor during a genuine interactive drag.
        let trigger = current_drag_event()?;
        let stamp = event_stamp(&trigger);
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
            Some(((loc.x, loc.y), stamp))
        }
    }

    pub fn engage_pointer_lock(
        window_point: Option<(f64, f64)>,
    ) -> Option<((f64, f64), Option<EventStamp>)> {
        let mtm = MainThreadMarker::new()?;
        let app = NSApplication::sharedApplication(mtm);
        if !app.isActive() {
            return None;
        }
        let stamp = current_pointer_event().as_deref().map(event_stamp);
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
            Some(((loc.x, loc.y), stamp))
        }
    }

    pub fn delta_x() -> Option<(EventStamp, f64)> {
        let ev = current_drag_event()?;
        let dx = ev.deltaX();
        dx.is_finite().then_some((event_stamp(&ev), dx))
    }

    pub fn delta_xy() -> Option<(EventStamp, (f64, f64))> {
        let ev = current_pointer_event()?;
        let (dx, dy) = (ev.deltaX(), ev.deltaY());
        (dx.is_finite() && dy.is_finite()).then_some((event_stamp(&ev), (dx, dy)))
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
        Self {
            anchor: None,
            last_event: Cell::new(None),
        }
    }

    pub fn engage(&mut self) {
        if self.anchor.is_some() {
            return;
        }
        #[cfg(all(target_os = "macos", not(test)))]
        {
            if let Some((anchor, stamp)) = macos::engage() {
                self.anchor = Some(anchor);
                // The drag event that crossed the threshold was already sampled
                // in screen coordinates before engaging; never double-apply it.
                self.last_event.set(Some(stamp));
            }
        }
    }

    pub fn take_delta_x(&self) -> Option<f64> {
        self.anchor?;
        #[cfg(all(target_os = "macos", not(test)))]
        {
            Some(dedup_delta(&self.last_event, macos::delta_x(), 0.0))
        }
        #[cfg(not(all(target_os = "macos", not(test))))]
        {
            None
        }
    }

    pub fn release(&mut self) {
        self.last_event.set(None);
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
    last_event: Cell<Option<EventStamp>>,
}

impl PointerLock {
    pub fn new() -> Self {
        Self {
            anchor: None,
            last_event: Cell::new(None),
        }
    }

    pub fn engage(&mut self, window_point: Option<(f64, f64)>) {
        if self.anchor.is_some() {
            return;
        }
        #[cfg(all(target_os = "macos", not(test)))]
        {
            if let Some((anchor, stamp)) = macos::engage_pointer_lock(window_point) {
                self.anchor = Some(anchor);
                self.last_event.set(stamp);
            }
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
            Some(dedup_delta(&self.last_event, macos::delta_xy(), (0.0, 0.0)))
        }
        #[cfg(not(all(target_os = "macos", not(test))))]
        {
            None
        }
    }

    pub fn release(&mut self) {
        self.last_event.set(None);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp(ts: f64, num: isize, ptr: usize) -> EventStamp {
        EventStamp {
            timestamp_bits: ts.to_bits(),
            event_number: num,
            ptr,
        }
    }

    #[test]
    fn dedup_delta_ignores_repeated_current_event_when_mouse_stops() {
        let last = Cell::new(Some(stamp(1.0, 10, 0x1000)));
        // Triggering event already consumed when engaging.
        assert_eq!(dedup_delta(&last, Some((stamp(1.0, 10, 0x1000), 4.0)), 0.0), 0.0);
        // New hardware drag event applies once.
        assert_eq!(dedup_delta(&last, Some((stamp(1.016, 11, 0x2000), 3.0)), 0.0), 3.0);
        // Subsequent synthetic GPUI MouseMoveEvents see the same stale NSEvent -> 0.0.
        assert_eq!(dedup_delta(&last, Some((stamp(1.016, 11, 0x2000), 3.0)), 0.0), 0.0);
        assert_eq!(dedup_delta(&last, Some((stamp(1.016, 11, 0x2000), 3.0)), 0.0), 0.0);
        // Non-drag currentEvent while still locked -> 0.0 (stays locked, no screen jump).
        assert_eq!(dedup_delta(&last, None, 0.0), 0.0);
        // Next genuine motion applies cleanly.
        assert_eq!(dedup_delta(&last, Some((stamp(1.032, 12, 0x2000), -2.5)), 0.0), -2.5);
    }

    #[test]
    fn dedup_delta_tracks_xy_pairs_for_pointer_lock() {
        let last = Cell::new(None);
        let s1 = stamp(2.0, 20, 0x3000);
        assert_eq!(dedup_delta(&last, Some((s1, (5.0, -3.0))), (0.0, 0.0)), (5.0, -3.0));
        assert_eq!(dedup_delta(&last, Some((s1, (5.0, -3.0))), (0.0, 0.0)), (0.0, 0.0));
        let s2 = stamp(2.016, 21, 0x3000);
        assert_eq!(dedup_delta(&last, Some((s2, (0.0, 1.5))), (0.0, 0.0)), (0.0, 1.5));
    }
}
