//! Tiny platform services used by controls; no editor/document state lives here.
#[cfg(all(target_os = "macos", not(test)))]
mod mac {
    use objc2::{MainThreadMarker, msg_send};
    use objc2_app_kit::{
        NSApplication, NSHapticFeedbackManager, NSHapticFeedbackPattern,
        NSHapticFeedbackPerformanceTime, NSHapticFeedbackPerformer, NSWorkspace,
    };
    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn CGAssociateMouseAndMouseCursorPosition(connected: bool) -> i32;
        fn CGMainDisplayID() -> u32;
        fn CGDisplayHideCursor(display: u32) -> i32;
        fn CGDisplayShowCursor(display: u32) -> i32;
    }
    pub fn alignment() {
        NSHapticFeedbackManager::defaultPerformer().performFeedbackPattern_performanceTime(
            NSHapticFeedbackPattern::Alignment,
            NSHapticFeedbackPerformanceTime::Now,
        );
    }
    pub fn reduce_motion() -> bool {
        unsafe {
            msg_send![
                &*NSWorkspace::sharedWorkspace(),
                accessibilityDisplayShouldReduceMotion
            ]
        }
    }
    /// Owns the cursor association. Drop restores it on every exit path.
    pub struct InfiniteScrub {
        display: u32,
    }
    impl InfiniteScrub {
        pub fn begin() -> Option<Self> {
            MainThreadMarker::new()?;
            unsafe {
                if CGAssociateMouseAndMouseCursorPosition(false) != 0 {
                    return None;
                }
                let display = CGMainDisplayID();
                CGDisplayHideCursor(display);
                Some(Self { display })
            }
        }
        pub fn delta(&self) -> Option<(f64, f64)> {
            let mtm = MainThreadMarker::new()?;
            let event = NSApplication::sharedApplication(mtm).currentEvent()?;
            Some((event.deltaX() as f64, event.deltaY() as f64))
        }
    }
    impl Drop for InfiniteScrub {
        fn drop(&mut self) {
            unsafe {
                CGAssociateMouseAndMouseCursorPosition(true);
                CGDisplayShowCursor(self.display);
            }
        }
    }
}
#[cfg(all(target_os = "macos", not(test)))]
pub use mac::*;
#[cfg(any(not(target_os = "macos"), test))]
mod portable {
    pub fn alignment() {}
    pub fn reduce_motion() -> bool {
        false
    }
    pub struct InfiniteScrub;
    impl InfiniteScrub {
        pub fn begin() -> Option<Self> {
            None
        }
        pub fn delta(&self) -> Option<(f64, f64)> {
            None
        }
    }
}
#[cfg(any(not(target_os = "macos"), test))]
pub use portable::*;
