//! Powermove P-LAUNCH-TRACE: opt-in launch timing (`PM_LAUNCH_TRACE=<file>`).
//!
//! Appends one NDJSON line per milestone, `{"m":name,"w":wall_ms}` (Unix
//! milliseconds, the clock of the process start the platform writes first),
//! so a launch can be timed from exec to the first frame on the glass. Each
//! window's first drawable, and the next drawable after [`arm_present`],
//! write `{"m":"present","of":label,"w":presented_wall_ms}` from their
//! presented handler. Off unless the variable is set: one cached flag read,
//! no handler, file or clock reads.
use parking_lot::Mutex;
use std::{
    io::Write,
    sync::OnceLock,
    time::{SystemTime, UNIX_EPOCH},
};

fn path() -> Option<&'static std::ffi::OsStr> {
    static PATH: OnceLock<Option<std::ffi::OsString>> = OnceLock::new();
    PATH.get_or_init(|| std::env::var_os("PM_LAUNCH_TRACE").filter(|p| !p.is_empty()))
        .as_deref()
}

/// Whether the trace is on: the variable alone decides, read once.
pub fn enabled() -> bool {
    path().is_some()
}

/// Unix milliseconds now.
pub fn wall_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0., |d| d.as_secs_f64() * 1000.)
}

/// Appends one milestone line with extra JSON members (`extra` is either
/// empty or starts with a comma).
pub fn write(name: &str, wall_ms: f64, extra: &str) {
    let Some(path) = path() else { return };
    static FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);
    let mut file = FILE.lock();
    if file.is_none() {
        *file = std::fs::File::options()
            .create(true)
            .append(true)
            .open(path)
            .ok();
    }
    if let Some(file) = file.as_mut() {
        let _ = writeln!(file, r#"{{"m":"{name}","w":{wall_ms:.3}{extra}}}"#);
    }
}

/// A milestone at this moment.
pub fn mark(name: &str) {
    if enabled() {
        write(name, wall_ms(), "");
    }
}

static ARMED: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

/// The next drawable any window presents writes a `present` line for `label`.
pub fn arm_present(label: &'static str) {
    if enabled() {
        ARMED.lock().push(label);
    }
}

/// Labels the drawable being encoded reports when it reaches the glass.
pub(crate) fn take_armed(first_of_window: bool) -> Vec<&'static str> {
    let mut labels = std::mem::take(&mut *ARMED.lock());
    if first_of_window {
        labels.insert(0, "window");
    }
    labels
}

/// From the presented handler: host-seconds presented time to wall time.
pub(crate) fn presented(labels: &[&'static str], presented_host: f64) {
    let now_host = crate::feel::now();
    let now = wall_ms();
    // Zero when the drawable never reached the glass (covered, or a display
    // path without timestamps): the handler's own time stands in, marked.
    let (wall, exact) = if presented_host > 0. {
        (now - (now_host - presented_host) * 1000., true)
    } else {
        (now, false)
    };
    for label in labels {
        write("present", wall, &format!(r#","of":"{label}","exact":{exact}"#));
    }
    IDLE_WANTED.store(true, std::sync::atomic::Ordering::Release);
}

static IDLE_WANTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The platform's main run loop is about to wait for events: the first time
/// after a traced present it writes `idle` (input is taken from then on).
pub fn before_waiting() {
    if IDLE_WANTED.swap(false, std::sync::atomic::Ordering::AcqRel) {
        mark("idle");
    }
}
