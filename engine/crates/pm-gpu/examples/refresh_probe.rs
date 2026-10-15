//! Does a refresh-rate request reach the ProMotion panel? (plan section
//! 15; docs/engine/bridge.md "ProMotion"). Watches the main display's
//! refreshes with a display link for a few seconds without a request, then
//! with requests for multiples of common content rates, and prints the
//! refresh intervals seen. Opens no window.
//!
//! ```sh
//! source engine/env.sh && cd engine && cargo run --release -p pm-gpu --no-default-features --features remote-layer --example refresh_probe
//! ```

// NSScreen lives in AppKit, which a plain Rust binary does not load (the
// engine's utility process does: Electron's framework links it).
#[cfg(target_os = "macos")]
#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}

#[cfg(target_os = "macos")]
fn main() {
    use pm_gpu::bridge_out::remote_layer::{RatePreference, take_display_ticks};
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
    let mut out = Vec::new();
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
        let link = match want {
            Some(hz) => RatePreference::request(hz),
            None => RatePreference::observe(),
        }
        .expect("a display link on the main screen");
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
        out.push(line);
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {}
