//! Powermove P-FEEL: opt-in input-to-photon probe (`PM_FEEL_TRACE=<file>`).
//!
//! The window notes the timestamp of each input event it handles; the next
//! drawable it paints carries the oldest one and, when it reaches the glass,
//! appends one NDJSON line: `{"ui":kind,"input":t,"draw":t,"presented":t}`
//! in host seconds (the clock of `NSEvent.timestamp` and
//! `CAMetalDrawable.presentedTime`). Off unless the variable is set: no
//! handler, no file, no clock reads. On, the file is opened and written on
//! its own thread: a slow disk or a FIFO nobody reads never stalls input or
//! drawing (lines past [`QUEUE`] waiting are dropped).
use parking_lot::Mutex;
use std::{
    io::Write,
    sync::{OnceLock, mpsc},
};

unsafe extern "C" {
    fn CACurrentMediaTime() -> f64;
}

/// Host seconds now, on the clock input and presented times use.
pub fn now() -> f64 {
    // SAFETY: a QuartzCore clock read with no arguments.
    unsafe { CACurrentMediaTime() }
}

/// Lines waiting for the writer thread.
pub const QUEUE: usize = 4096;

/// Whether the probe is on: the variable alone decides, read once.
pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("PM_FEEL_TRACE").is_some_and(|p| !p.is_empty()))
}

/// The writer thread's queue, started by the first line.
fn sink() -> Option<&'static mpsc::SyncSender<String>> {
    static SINK: OnceLock<Option<mpsc::SyncSender<String>>> = OnceLock::new();
    SINK.get_or_init(|| {
        let path = std::env::var_os("PM_FEEL_TRACE").filter(|_| enabled())?;
        let (sender, lines) = mpsc::sync_channel::<String>(QUEUE);
        std::thread::Builder::new()
            .name("pm-feel-trace".into())
            .spawn(move || {
                let Ok(mut file) = std::fs::File::options()
                    .create(true)
                    .append(true)
                    .open(path)
                else {
                    return;
                };
                for mut line in lines {
                    line.push('\n');
                    if file.write_all(line.as_bytes()).is_err() {
                        return;
                    }
                }
            })
            .ok()?;
        Some(sender)
    })
    .as_ref()
}

/// Queues one line for the trace file; never blocks.
pub fn write(line: &str) {
    if enabled()
        && let Some(sink) = sink()
    {
        let _ = sink.try_send(line.to_owned());
    }
}

/// The oldest input not yet painted, and what it was. One per process: the
/// window an input changes is the one that paints next.
static PENDING: Mutex<Option<(f64, &'static str)>> = Mutex::new(None);

/// Notes an input event (`input`: its host-seconds timestamp).
pub fn note(input: f64, kind: &'static str) {
    if enabled() && input > 0. {
        PENDING.lock().get_or_insert((input, kind));
    }
}

/// Notes an input handled now; off, the clock is not read.
pub fn note_now(kind: &'static str) {
    if enabled() {
        note(now(), kind);
    }
}

pub(crate) fn take() -> Option<(f64, &'static str)> {
    if !enabled() {
        return None;
    }
    PENDING.lock().take()
}

/// The display link's target presentation time for the update being drawn.
static TARGET: Mutex<f64> = Mutex::new(0.);
pub fn set_target(t: f64) {
    if enabled() {
        *TARGET.lock() = t;
    }
}
pub(crate) fn target() -> f64 {
    *TARGET.lock()
}

/// When the frame being drawn began (layout, prepaint and paint come first).
static FRAME_START: Mutex<f64> = Mutex::new(0.);
pub fn set_frame_start() {
    if enabled() {
        *FRAME_START.lock() = now();
    }
}
pub(crate) fn frame_start() -> f64 {
    *FRAME_START.lock()
}
