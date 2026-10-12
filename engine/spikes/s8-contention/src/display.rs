//! Display-link timing: vsync host times from a `CVDisplayLink` on the main display.
//!
//! The interactive process paces its frames on these ticks, so its deadlines are the
//! real vsyncs of the panel Jude is looking at (120 Hz ProMotion on the reference
//! machine), not a free-running timer. `CVDisplayLink` is deprecated in favour of
//! `NSView.displayLink`, but it is still functional and needs no window or run loop,
//! which is what an engine process (no UI) has.

use std::ffi::c_void;
use std::sync::mpsc;

type CVDisplayLinkRef = *mut c_void;
type CVReturn = i32;

/// Leading fields of `CVTimeStamp` (CoreVideo/CVBase.h). The full struct is 80 bytes;
/// only the prefix is read through the pointer CoreVideo passes.
#[repr(C)]
struct CVTimeStamp {
    version: u32,
    video_time_scale: i32,
    video_time: i64,
    host_time: u64,
    rate_scalar: f64,
    video_refresh_period: i64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CVTime {
    time_value: i64,
    time_scale: i32,
    flags: i32,
}

type OutputCallback = extern "C" fn(
    link: CVDisplayLinkRef,
    now: *const CVTimeStamp,
    output: *const CVTimeStamp,
    flags_in: u64,
    flags_out: *mut u64,
    ctx: *mut c_void,
) -> CVReturn;

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    fn CVDisplayLinkCreateWithCGDisplay(display: u32, out: *mut CVDisplayLinkRef) -> CVReturn;
    fn CVDisplayLinkSetOutputCallback(link: CVDisplayLinkRef, cb: OutputCallback, ctx: *mut c_void) -> CVReturn;
    fn CVDisplayLinkStart(link: CVDisplayLinkRef) -> CVReturn;
    fn CVDisplayLinkStop(link: CVDisplayLinkRef) -> CVReturn;
    fn CVDisplayLinkRelease(link: CVDisplayLinkRef);
    fn CVDisplayLinkGetNominalOutputVideoRefreshPeriod(link: CVDisplayLinkRef) -> CVTime;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGMainDisplayID() -> u32;
    fn CGDisplayIsAsleep(display: u32) -> u32;
}

/// Whether the main display is asleep. While it is, CVDisplayLink still ticks at the
/// panel rate (synthesised), but nothing reaches the glass and WindowServer composites
/// nothing, so on-screen presentation cannot be measured.
pub fn main_display_asleep() -> bool {
    // SAFETY: plain query.
    unsafe { CGDisplayIsAsleep(CGMainDisplayID()) != 0 }
}

/// Whether the console session's screen is locked (from the IORegistry root's
/// `IOConsoleUsers`). `None` if it cannot be read.
pub fn screen_locked() -> Option<bool> {
    let out = std::process::Command::new("/usr/sbin/ioreg")
        .args(["-n", "Root", "-d1"])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    if s.contains("\"CGSSessionScreenIsLocked\"=Yes") {
        Some(true)
    } else if s.contains("IOConsoleUsers") {
        Some(false)
    } else {
        None
    }
}

/// One vsync: host time (ns) of the vsync that just occurred and of the one the
/// display link is preparing for.
#[derive(Clone, Copy, Debug)]
pub struct Vsync {
    pub now_ns: u64,
    pub output_ns: u64,
}

struct Ctx {
    tx: mpsc::SyncSender<Vsync>,
}

extern "C" fn on_vsync(
    _link: CVDisplayLinkRef,
    now: *const CVTimeStamp,
    output: *const CVTimeStamp,
    _flags_in: u64,
    _flags_out: *mut u64,
    ctx: *mut c_void,
) -> CVReturn {
    // SAFETY: ctx is the Box<Ctx> leaked in `DisplayLink::start` and freed only after
    // CVDisplayLinkStop returns (which waits for an in-flight callback); the time
    // stamps are valid for the duration of the callback.
    unsafe {
        let ctx = &*(ctx as *const Ctx);
        let v = Vsync {
            now_ns: crate::clock::ticks_to_ns((*now).host_time),
            output_ns: crate::clock::ticks_to_ns((*output).host_time),
        };
        // A full channel means the consumer is behind; it reads the newest ticks it
        // needs from `now_ns`, so dropping one here loses nothing it would use.
        let _ = ctx.tx.try_send(v);
    }
    0
}

pub struct DisplayLink {
    link: CVDisplayLinkRef,
    ctx: *mut Ctx,
    pub rx: mpsc::Receiver<Vsync>,
    /// Nominal refresh period of the display (ns).
    pub refresh_ns: f64,
}

// SAFETY: the link handle is only used to stop and release it.
unsafe impl Send for DisplayLink {}

impl DisplayLink {
    /// Start a display link on the main display.
    pub fn start() -> Result<Self, String> {
        let (tx, rx) = mpsc::sync_channel(16);
        let ctx = Box::into_raw(Box::new(Ctx { tx }));
        let mut link: CVDisplayLinkRef = std::ptr::null_mut();
        // SAFETY: out-parameter calls; ctx stays alive until Drop.
        unsafe {
            let rc = CVDisplayLinkCreateWithCGDisplay(CGMainDisplayID(), &mut link);
            if rc != 0 || link.is_null() {
                drop(Box::from_raw(ctx));
                return Err(format!("CVDisplayLinkCreateWithCGDisplay = {rc}"));
            }
            let t = CVDisplayLinkGetNominalOutputVideoRefreshPeriod(link);
            let refresh_ns = if t.time_scale > 0 && t.time_value > 0 {
                t.time_value as f64 * 1e9 / t.time_scale as f64
            } else {
                f64::NAN
            };
            let rc = CVDisplayLinkSetOutputCallback(link, on_vsync, ctx as *mut c_void);
            if rc != 0 {
                CVDisplayLinkRelease(link);
                drop(Box::from_raw(ctx));
                return Err(format!("CVDisplayLinkSetOutputCallback = {rc}"));
            }
            let rc = CVDisplayLinkStart(link);
            if rc != 0 {
                CVDisplayLinkRelease(link);
                drop(Box::from_raw(ctx));
                return Err(format!("CVDisplayLinkStart = {rc}"));
            }
            Ok(Self {
                link,
                ctx,
                rx,
                refresh_ns,
            })
        }
    }

    /// Measure the actual refresh period over `n` callbacks: total span divided by the
    /// number of refreshes it covers (each gap rounded to whole median periods, so a
    /// missed callback does not bias the result).
    pub fn measure_refresh(&self, n: usize) -> f64 {
        let mut times = Vec::with_capacity(n + 1);
        while times.len() <= n {
            let Ok(v) = self.rx.recv() else { break };
            times.push(v.now_ns);
        }
        let gaps: Vec<f64> = times.windows(2).map(|w| w[1].saturating_sub(w[0]) as f64).collect();
        let median = crate::stats::dist(&gaps).p50;
        let refreshes: f64 = gaps.iter().map(|g| (g / median).round().max(1.0)).sum();
        (times[times.len() - 1] - times[0]) as f64 / refreshes
    }
}

impl Drop for DisplayLink {
    fn drop(&mut self) {
        // SAFETY: stop waits for any running callback, after which ctx is unused.
        unsafe {
            CVDisplayLinkStop(self.link);
            CVDisplayLinkRelease(self.link);
            drop(Box::from_raw(self.ctx));
        }
    }
}

/// How many display refreshes one content frame spans (`fps` on a `refresh_hz` panel).
/// Returns `None` unless the frame period is a whole number of refreshes within 0.05%
/// (a 0.1% mismatch, 23.976 on 120 Hz, would slip a refresh every 42 s).
pub fn refreshes_per_frame(refresh_ns: f64, fps: f64) -> Option<u64> {
    let k = (1e9 / fps) / refresh_ns;
    let r = k.round();
    (r >= 1.0 && (k - r).abs() / r < 5e-4).then_some(r as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_multiples() {
        assert_eq!(refreshes_per_frame(1e9 / 120.0, 60.0), Some(2));
        assert_eq!(refreshes_per_frame(1e9 / 120.0, 120.0), Some(1));
        assert_eq!(refreshes_per_frame(1e9 / 60.0, 60.0), Some(1));
        assert_eq!(refreshes_per_frame(1e9 / 120.0, 23.976), None);
        assert_eq!(refreshes_per_frame(1e9 / 59.94, 59.94), Some(1));
    }

    #[test]
    fn display_link_ticks_at_the_panel_rate() {
        let dl = DisplayLink::start().expect("display link");
        let measured = dl.measure_refresh(30);
        assert!(measured > 4e6 && measured < 34e6, "refresh {measured} ns");
        if dl.refresh_ns.is_finite() {
            assert!(
                (measured - dl.refresh_ns).abs() / dl.refresh_ns < 0.05,
                "{measured} vs {}",
                dl.refresh_ns
            );
        }
    }
}
