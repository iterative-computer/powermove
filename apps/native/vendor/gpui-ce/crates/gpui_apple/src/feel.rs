//! Powermove P-FEEL: opt-in input-to-photon probe (`PM_FEEL_TRACE=<file>`).
//!
//! The window notes the timestamp of each input event it handles; the next
//! drawable it paints carries the oldest one and, when it reaches the glass,
//! appends one NDJSON line: `{"ui":kind,"input":t,"draw":t,"presented":t}`
//! in host seconds (the clock of `NSEvent.timestamp` and
//! `CAMetalDrawable.presentedTime`). Off unless the variable is set: no
//! handler, no file, no clock reads.
use parking_lot::Mutex;
use std::{fs::File, io::Write, sync::OnceLock};

unsafe extern "C" {
    fn CACurrentMediaTime() -> f64;
}

/// Host seconds now, on the clock input and presented times use.
pub fn now() -> f64 {
    // SAFETY: a QuartzCore clock read with no arguments.
    unsafe { CACurrentMediaTime() }
}

fn sink() -> Option<&'static Mutex<File>> {
    static SINK: OnceLock<Option<Mutex<File>>> = OnceLock::new();
    SINK.get_or_init(|| {
        let path = std::env::var_os("PM_FEEL_TRACE")?;
        File::options()
            .create(true)
            .append(true)
            .open(path)
            .ok()
            .map(Mutex::new)
    })
    .as_ref()
}

pub fn enabled() -> bool {
    sink().is_some()
}

/// Appends one line (presented handlers run on a Metal thread, never main).
pub fn write(line: &str) {
    if let Some(file) = sink() {
        let mut file = file.lock();
        let _ = file.write_all(line.as_bytes());
        let _ = file.write_all(b"\n");
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
