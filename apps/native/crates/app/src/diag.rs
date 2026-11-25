//! Opt-in main-thread diagnostics (`PM_NATIVE_MAIN_TRACE=1`). Off by default:
//! no timer, probe or output in normal runs. Lines go to stderr as
//! `PM_MAIN {json}` so stdout stays reserved for MCP and the perf gate.
use std::{
    sync::OnceLock,
    time::{Duration, Instant},
};

pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("PM_NATIVE_MAIN_TRACE").as_deref() == Ok("1"))
}

/// The process clock the lines carry (`t`, ms since the first diagnostic
/// call, which `main` makes first).
fn start() -> Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    *START.get_or_init(Instant::now)
}

/// Wall-clock milliseconds, to line these up with other processes' traces.
fn wall_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

pub fn line(value: serde_json::Value) {
    if enabled() {
        let mut value = value;
        if let Some(map) = value.as_object_mut() {
            map.insert(
                "t".into(),
                (start().elapsed().as_secs_f64() * 1000.).round().into(),
            );
            map.insert("w".into(), wall_ms().into());
        }
        // A writer thread: stderr is a file, and a write that waited on a
        // busy disk held the main thread for a second (soak, 2026-11-09).
        static WRITER: OnceLock<Option<Queue>> = OnceLock::new();
        let writer = WRITER.get_or_init(|| {
            let (queue, rx) = Queue::new(QUEUE_LINES);
            std::thread::Builder::new()
                .name("pm-diag".into())
                .spawn(move || {
                    for line in rx {
                        eprintln!("{line}");
                    }
                })
                .ok()
                .map(|_| queue)
        });
        let line = format!("PM_MAIN {value}");
        match writer {
            Some(queue) => queue.push(line),
            None => eprintln!("{line}"),
        }
    }
}

/// Lines waiting for a slow or blocked stderr before new ones are dropped.
const QUEUE_LINES: usize = 4096;

/// The writer thread's bounded queue. A full queue drops the line and counts
/// it; the next line that fits is preceded by one `{"dropped":N}` line, in
/// the place the lines went missing.
struct Queue {
    tx: std::sync::mpsc::SyncSender<String>,
    dropped: std::sync::atomic::AtomicU64,
}
impl Queue {
    fn new(lines: usize) -> (Queue, std::sync::mpsc::Receiver<String>) {
        let (tx, rx) = std::sync::mpsc::sync_channel(lines);
        let dropped = Default::default();
        (Queue { tx, dropped }, rx)
    }
    fn push(&self, line: String) {
        use std::sync::atomic::Ordering::Relaxed;
        let missing = self.dropped.swap(0, Relaxed);
        if missing > 0
            && self
                .tx
                .try_send(format!(r#"PM_MAIN {{"dropped":{missing}}}"#))
                .is_err()
        {
            self.dropped.fetch_add(missing + 1, Relaxed);
            return;
        }
        if self.tx.try_send(line).is_err() {
            self.dropped.fetch_add(1, Relaxed);
        }
    }
}

/// Starts the process clock (call first thing in `main`).
pub fn mark_start() {
    if enabled() {
        start();
    }
}

/// Milestones of one open (`{"open": stage, "ms": since the open began}`).
#[derive(Default)]
pub struct OpenTimer(Option<Instant>);
impl OpenTimer {
    pub fn begin(&mut self) {
        self.0 = (enabled() || gpui_macos::launch::enabled()).then(Instant::now);
    }
    pub fn mark(&self, stage: &str, extra: serde_json::Value) {
        // P-LAUNCH-TRACE: the project on screen, and its first picture.
        if gpui_macos::launch::enabled() {
            let label = match stage {
                "opened" => Some("opened"),
                "first_frame" => Some("picture"),
                _ => None,
            };
            if let Some(label) = label {
                gpui_macos::launch::mark(label);
                gpui_macos::launch::arm_present(label);
            }
        }
        if !enabled() {
            return;
        }
        if let Some(t) = self.0 {
            let mut v = serde_json::json!({"open": stage, "ms": (t.elapsed().as_secs_f64() * 1000.).round()});
            if let (Some(m), serde_json::Value::Object(e)) = (v.as_object_mut(), extra) {
                m.extend(e);
            }
            line(v);
        }
    }
    pub fn active(&self) -> bool {
        self.0.is_some()
    }
}

/// Times a main-thread span; reports it when tracing is on.
pub struct Span {
    label: &'static str,
    start: Option<Instant>,
}
pub fn span(label: &'static str) -> Span {
    Span {
        label,
        start: enabled().then(Instant::now),
    }
}
impl Drop for Span {
    fn drop(&mut self) {
        if let Some(start) = self.start {
            let ms = start.elapsed().as_secs_f64() * 1000.;
            if ms >= 1. {
                // `end`: host seconds, the clock of input events and frames.
                line(serde_json::json!({"span":self.label,"ms":ms,"end":gpui_macos::feel::now()}));
            }
        }
    }
}

/// When the app becomes the active app, and the event that did it (a
/// person's click reads `mouse`; anything else, e.g. another app quitting
/// and the system choosing this one, reads its event type or `none`), so an
/// unattended run can tell a focus steal from a person.
fn report_activation() {
    use objc2_app_kit::{NSApplication, NSApplicationDidBecomeActiveNotification, NSEventType};
    let block = block2::RcBlock::new(|_: std::ptr::NonNull<objc2_foundation::NSNotification>| {
        let event = objc2::MainThreadMarker::new()
            .and_then(|mtm| NSApplication::sharedApplication(mtm).currentEvent())
            .map(|e| {
                let t = e.r#type();
                if t == NSEventType::LeftMouseDown || t == NSEventType::RightMouseDown {
                    "mouse".to_owned()
                } else {
                    format!("{t:?}")
                }
            })
            .unwrap_or_else(|| "none".into());
        line(serde_json::json!({"app": "became_active", "event": event}));
    });
    // SAFETY: a notification name constant; the observer lives for the process.
    let token = unsafe {
        objc2_foundation::NSNotificationCenter::defaultCenter()
            .addObserverForName_object_queue_usingBlock(
                Some(NSApplicationDidBecomeActiveNotification),
                None,
                None,
                &block,
            )
    };
    std::mem::forget(token);
}

/// Main-thread responsiveness: a foreground task asks for a 5 ms timer and
/// reports how late it resumes. A late resume is time the UI thread could
/// not paint or handle input.
/// Live editor controllers, for the probe line: a closed window's editor
/// must go (stability soak).
pub static EDITORS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub fn install_probe(cx: &mut gpui::App) {
    // High frequency latency sampling is opt-in, even when trace logging is on.
    if !enabled() || std::env::var_os("PM_NATIVE_RESPONSIVENESS_PROBE").is_none() {
        return;
    }
    let timer = cx.background_executor().clone();
    stall_sampler::start();
    report_activation();
    cx.spawn(async move |_| {
        let mut worst = 0f64;
        let mut reported = Instant::now();
        loop {
            stall_sampler::beat();
            let asked = Instant::now();
            timer.timer(Duration::from_millis(5)).await;
            let late = (asked.elapsed().as_secs_f64() - 0.005).max(0.) * 1000.;
            worst = worst.max(late);
            if late >= 50. {
                line(serde_json::json!({"stall_ms":late}));
            }
            if reported.elapsed() >= Duration::from_secs(1) {
                line(serde_json::json!({"probe_worst_ms":worst,
                    "editors":EDITORS.load(std::sync::atomic::Ordering::Relaxed)}));
                worst = 0.;
                reported = Instant::now();
            }
        }
    })
    .detach();
}

/// Where the main thread is during a stall (`PM_MAIN {"stall_stacks":..}`):
/// a watchdog thread that, while the main thread's heartbeat
/// (`pm_telemetry::main_thread`, shared with the hang watchdog) is over 50 ms
/// late, samples the main thread every 20 ms, then reports the return
/// addresses with the image's load address for `atos -l`. Tracing runs only.
mod stall_sampler {
    use pm_telemetry::main_thread as main;
    use std::time::Duration;

    pub fn beat() {
        main::beat();
    }
    /// Call on the main thread.
    pub fn start() {
        main::install();
        let _ = std::thread::Builder::new()
            .name("pm-stall-sampler".into())
            .spawn(watch);
    }

    fn watch() {
        let load = main::load_address();
        let mut stacks: Vec<Vec<String>> = vec![];
        let mut buf = [0u64; main::DEPTH];
        loop {
            std::thread::sleep(Duration::from_millis(20));
            if main::since_beat_ms() > 55 {
                if stacks.len() < 8 {
                    let n = main::sample(&mut buf);
                    stacks.push(buf[..n].iter().map(|a| main::frame_name(*a)).collect());
                }
            } else if !stacks.is_empty() {
                super::line(serde_json::json!({
                    "stall_stacks": std::mem::take(&mut stacks),
                    "load": format!("{load:#x}"),
                }));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Queue;

    /// A blocked stderr never grows the queue past its bound: the overflow
    /// is counted and reported once, where it happened.
    #[test]
    fn a_blocked_writer_drops_and_counts_lines() {
        let (queue, rx) = Queue::new(2);
        for n in 0..5 {
            queue.push(format!("line {n}"));
        }
        let drained: Vec<String> = rx.try_iter().collect();
        assert_eq!(drained, ["line 0", "line 1"]);
        queue.push("line 5".into());
        let drained: Vec<String> = rx.try_iter().collect();
        assert_eq!(drained, [r#"PM_MAIN {"dropped":3}"#, "line 5"]);
        queue.push("line 6".into());
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), ["line 6"]);
    }
}

/// PM-401: what showed the app again after the person hid it. AppKit unhides
/// a hidden app the moment one of its windows is ordered in, so the main
/// thread's stack at that moment names the caller (an ordering of ours, or the
/// person's click on the Dock). Logged only with the diagnostics on.
#[cfg(target_os = "macos")]
pub fn trace_unhide() {
    if !enabled() {
        return;
    }
    let block = block2::RcBlock::new(|_: std::ptr::NonNull<objc2_foundation::NSNotification>| {
        let stack = std::backtrace::Backtrace::force_capture().to_string();
        let frames: Vec<&str> = stack
            .lines()
            .map(str::trim)
            .filter(|l| !l.starts_with("at ") && !l.is_empty())
            .take(48)
            .collect();
        line(serde_json::json!({ "unhide": frames }));
    });
    let token = unsafe {
        objc2_foundation::NSNotificationCenter::defaultCenter()
            .addObserverForName_object_queue_usingBlock(
                Some(objc2_app_kit::NSApplicationWillUnhideNotification),
                None,
                None,
                &block,
            )
    };
    // Kept for the life of the process.
    std::mem::forget(token);
}
