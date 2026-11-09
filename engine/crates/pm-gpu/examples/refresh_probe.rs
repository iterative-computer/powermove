//! Does a refresh-rate request reach the ProMotion panel? (plan section
//! 15; docs/engine/bridge.md "ProMotion"). Watches the main display's
//! refreshes with a display link for a few seconds without a request, then
//! with requests for multiples of common content rates, and prints the
//! refresh intervals seen. Opens no window.
//!
//! ```sh
//! source engine/env.sh && cd engine && cargo run --release -p pm-gpu --example refresh_probe
//! ```

// NSScreen lives in AppKit, which a plain Rust binary does not load.
#[cfg(target_os = "macos")]
#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}

#[cfg(target_os = "macos")]
mod display_link {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, mpsc};

    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject};

    objc2::define_class!(
        /// The display link's target: records each refresh.
        #[unsafe(super(objc2_foundation::NSObject))]
        struct RateTarget;

        impl RateTarget {
            #[unsafe(method(tick:))]
            fn tick(&self, link: &AnyObject) {
                // SAFETY: CADisplayLink's timestamp properties.
                let (now, target): (f64, f64) = unsafe { (msg_send![link, timestamp], msg_send![link, targetTimestamp]) };
                if let Ok(mut ticks) = TICKS.lock()
                    && ticks.len() < MAX_TICKS
                {
                    ticks.push((now, target));
                }
            }
        }
    );

    /// Display link callbacks seen (`timestamp`, `targetTimestamp`, seconds
    /// on the host clock); capped.
    static TICKS: Mutex<Vec<(f64, f64)>> = Mutex::new(Vec::new());
    const MAX_TICKS: usize = 20_000;

    /// The display link callbacks recorded so far, emptied: when the display
    /// actually refreshed while a [`RatePreference`] lived.
    pub fn take_display_ticks() -> Vec<(f64, f64)> {
        std::mem::take(&mut *TICKS.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// A display link on the main display that, while it lives, asks for a
    /// refresh rate (`-[NSScreen displayLinkWithTarget:selector:]` and
    /// `preferredFrameRateRange`, macOS 14) or only observes. It runs on its
    /// own run-loop thread.
    pub struct RatePreference {
        stop: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl RatePreference {
        /// Asks for `hz` exactly (minimum = maximum = preferred), or nothing.
        pub fn start(want: Option<f64>) -> Result<Self, &'static str> {
            let stop = Arc::new(AtomicBool::new(false));
            let flag = stop.clone();
            let (tx, rx) = mpsc::channel::<Result<(), &'static str>>();
            let thread = std::thread::Builder::new()
                .name("refresh-probe-link".into())
                .spawn(move || {
                    let Some(screen_class) = AnyClass::get(c"NSScreen") else {
                        let _ = tx.send(Err("NSScreen"));
                        return;
                    };
                    // SAFETY: class methods and properties of AppKit and Core
                    // Animation, used on this thread only.
                    unsafe {
                        let screen: Option<Retained<AnyObject>> = msg_send![screen_class, mainScreen];
                        let Some(screen) = screen else {
                            let _ = tx.send(Err("a main screen"));
                            return;
                        };
                        let target: Retained<RateTarget> = msg_send![<RateTarget as objc2::ClassType>::class(), new];
                        let link: Option<Retained<AnyObject>> = msg_send![&*screen, displayLinkWithTarget: &*target, selector: objc2::sel!(tick:)];
                        let Some(link) = link else {
                            let _ = tx.send(Err("a display link"));
                            return;
                        };
                        if let Some(hz) = want {
                            let range = objc2_quartz_core::CAFrameRateRange {
                                minimum: hz as f32,
                                maximum: hz as f32,
                                preferred: hz as f32,
                            };
                            let _: () = msg_send![&*link, setPreferredFrameRateRange: range];
                        }
                        let run_loop: Retained<AnyObject> = msg_send![AnyClass::get(c"NSRunLoop").expect("NSRunLoop"), currentRunLoop];
                        let _: () = msg_send![&*link, addToRunLoop: &*run_loop, forMode: objc2_foundation::NSRunLoopCommonModes];
                        let _ = tx.send(Ok(()));
                        while !flag.load(Ordering::Relaxed) {
                            objc2_core_foundation::CFRunLoop::run_in_mode(objc2_core_foundation::kCFRunLoopDefaultMode, 0.25, false);
                        }
                        let _: () = msg_send![&*link, invalidate];
                    }
                })
                .map_err(|_| "a thread")?;
            match rx.recv() {
                Ok(Ok(())) => Ok(Self {
                    stop,
                    thread: Some(thread),
                }),
                Ok(Err(missing)) => Err(missing),
                Err(_) => Err("the display link thread"),
            }
        }
    }

    impl Drop for RatePreference {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn main() {
    use display_link::{RatePreference, take_display_ticks};
    let seconds = std::env::args()
        .nth(1)
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(3.0);
    // A WindowServer connection as an accessory-less background process
    // (activation policy prohibited: no Dock icon, no menu bar, no focus).
    // SAFETY: AppKit class methods on the main thread.
    unsafe {
        use objc2::msg_send;
        use objc2::runtime::{AnyClass, AnyObject};
        let app: *mut AnyObject = msg_send![
            AnyClass::get(c"NSApplication").expect("AppKit"),
            sharedApplication
        ];
        let _: bool = msg_send![app, setActivationPolicy: 2_isize];
    }
    // Multiples of 25 fps (100, 75, 50) and of 24 fps (120, 96, 72, 48), and
    // 60 Hz, between two unrequested windows.
    for want in [
        None,
        Some(100.0),
        Some(75.0),
        Some(50.0),
        Some(120.0),
        Some(96.0),
        Some(72.0),
        Some(60.0),
        Some(48.0),
        None,
    ] {
        let _ = take_display_ticks();
        let link = RatePreference::start(want).expect("a display link on the main screen");
        std::thread::sleep(std::time::Duration::from_secs_f64(seconds));
        drop(link);
        let ticks = take_display_ticks();
        let mut intervals: Vec<f64> = ticks
            .windows(2)
            .map(|w| (w[1].0 - w[0].0) * 1000.0)
            .filter(|d| *d > 0.0)
            .collect();
        intervals.sort_by(f64::total_cmp);
        let pct = |p: f64| {
            intervals
                .get(((intervals.len().saturating_sub(1)) as f64 * p) as usize)
                .copied()
                .unwrap_or(f64::NAN)
        };
        // Intervals rounded to 0.1 ms, most frequent first.
        let mut histogram: Vec<(i64, usize)> = Vec::new();
        for d in &intervals {
            let key = (d * 10.0).round() as i64;
            match histogram.iter_mut().find(|(k, _)| *k == key) {
                Some((_, n)) => *n += 1,
                None => histogram.push((key, 1)),
            }
        }
        histogram.sort_by_key(|b| std::cmp::Reverse(b.1));
        let histogram = histogram
            .iter()
            .take(4)
            .map(|(k, n)| format!("\"{:.1}\":{n}", *k as f64 / 10.0))
            .collect::<Vec<_>>()
            .join(",");
        let line = format!(
            "{{\"request\":{},\"callbacks\":{},\"intervalMs\":{{\"p05\":{:.3},\"p50\":{:.3},\"p95\":{:.3}}},\"histogramMs\":{{{histogram}}}}}",
            want.map_or("null".into(), |h| format!("{h}")),
            ticks.len(),
            pct(0.05),
            pct(0.5),
            pct(0.95)
        );
        println!("{line}");
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {}
