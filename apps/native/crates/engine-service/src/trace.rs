//! Monotonic host seconds, matching NSEvent and drawable presentedTime.
//! Zero CPU/GPU fields mean unobserved; physical display stays Option.
#[derive(Default, Debug)]
pub struct Trace {
    pub worker_start: f64,
    pub target: f64,
    /// `presentDrawable:atTime:` argument for timed (playing) presents; 0 for
    /// untimed and legacy presents.
    pub requested: f64,
    /// requested + measured present lag: when this frame is expected on glass.
    pub predicted: f64,
    pub render_start: f64,
    pub render_submitted: f64,
    /// The render thread's own CPU time from render start to submission
    /// (ms): the frame's CPU cost without time spent preempted.
    pub render_cpu_ms: f64,
    /// Whether this frame counts its work (only a traced viewer does).
    pub count_work: bool,
    /// The render's work counters (`pm_perf`, the render thread's own).
    pub work: pm_perf::Counters,
    pub drawable_asked: f64,
    pub drawable_acquired: f64,
    pub encode_end: f64,
    pub present_requested: f64,
    pub commit: f64,
    pub scheduled: f64,
    pub gpu_start: f64,
    pub gpu_done: f64,
    pub completed: f64,
    pub display: Option<f64>,
    pub display_callback: f64,
    pub cache_hit: bool,
    /// The disk render cache: 0 not looked at, 1 missed, 2 read.
    pub disk: u8,
    /// Milliseconds this frame waited for the disk reader.
    pub disk_wait_ms: f64,
    pub gpu_ok: bool,
}
pub fn now() -> f64 {
    objc2_quartz_core::CACurrentMediaTime()
}

/// Where the viewer diagnostic trace goes, read once: `PM_ENGINE_TRACE`, or
/// for a development bundle launched without a custom environment
/// (`com.powermove.native.dev.*`) its own `PMEngineTrace` default; a shipped
/// app never reads the default.
pub fn path() -> Option<&'static std::ffi::OsStr> {
    static PATH: std::sync::OnceLock<Option<std::ffi::OsString>> = std::sync::OnceLock::new();
    PATH.get_or_init(|| {
        std::env::var_os("PM_ENGINE_TRACE").or_else(|| {
            let bundle = objc2_foundation::NSBundle::mainBundle().bundleIdentifier()?;
            if !bundle.to_string().starts_with("com.powermove.native.dev.") {
                return None;
            }
            let key = objc2_foundation::NSString::from_str("PMEngineTrace");
            // The app's own domain only, never an inherited (global) default.
            objc2_foundation::NSUserDefaults::standardUserDefaults()
                .persistentDomainForName(&bundle)?
                .objectForKey(&key)?
                .downcast::<objc2_foundation::NSString>()
                .ok()
                .map(|path| path.to_string().into())
        })
    })
    .as_deref()
}
/// Whether viewer frames are traced. Untraced, callers skip input stamps
/// (event timestamps and [`crate::Viewer::input_event`]) altogether.
pub fn enabled() -> bool {
    path().is_some()
}

/// This thread's CPU time in milliseconds.
pub fn thread_cpu_ms() -> f64 {
    unsafe extern "C" {
        fn clock_gettime_nsec_np(clock: u32) -> u64;
    }
    /// `CLOCK_THREAD_CPUTIME_ID` on Darwin.
    const THREAD_CPUTIME: u32 = 16;
    // SAFETY: a libSystem call with a valid clock id; it only reads the clock.
    unsafe { clock_gettime_nsec_np(THREAD_CPUTIME) as f64 / 1e6 }
}

/// Opt-in diagnostic writer. It never blocks the presenter and streams NDJSON
/// so AppKit termination cannot strand the entire report behind service Drop.
pub(crate) struct Recorder {
    sender: Option<std::sync::mpsc::SyncSender<serde_json::Value>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Recorder {
    pub fn open(path: std::ffi::OsString) -> anyhow::Result<Self> {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(path)?;
        let (sender, rx) = std::sync::mpsc::sync_channel::<serde_json::Value>(1024);
        let thread = std::thread::Builder::new()
            .name("pm-viewer-trace".into())
            .spawn(move || {
                use std::io::Write;
                for row in rx {
                    if serde_json::to_writer(&mut file, &row).is_err()
                        || file.write_all(b"\n").and_then(|_| file.flush()).is_err()
                    {
                        eprintln!("Viewer diagnostic trace write failed");
                        break;
                    }
                }
            })?;
        Ok(Self {
            sender: Some(sender),
            thread: Some(thread),
        })
    }
    pub fn record(&self, row: serde_json::Value) -> bool {
        self.sender.as_ref().unwrap().try_send(row).is_ok()
    }
}
impl Drop for Recorder {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Pipeline {
    pub request: f64,
    pub actor_ms: f64,
    pub publication_ms: f64,
}
