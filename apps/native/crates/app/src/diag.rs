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

pub fn line(value: serde_json::Value) {
    if enabled() {
        let mut value = value;
        if let Some(map) = value.as_object_mut() {
            map.insert(
                "t".into(),
                (start().elapsed().as_secs_f64() * 1000.).round().into(),
            );
        }
        eprintln!("PM_MAIN {value}");
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
        self.0 = enabled().then(Instant::now);
    }
    pub fn mark(&self, stage: &str, extra: serde_json::Value) {
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
                line(serde_json::json!({"span":self.label,"ms":ms}));
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
pub fn install_probe(cx: &mut gpui::App) {
    if !enabled() {
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
                line(serde_json::json!({"probe_worst_ms":worst}));
                worst = 0.;
                reported = Instant::now();
            }
        }
    })
    .detach();
}

/// Where the main thread is during a stall (`PM_MAIN {"stall_stacks":..}`):
/// a watchdog thread that, while the probe's heartbeat is over 50 ms late,
/// suspends the main thread every 20 ms for a frame-pointer walk (no
/// allocation while it is suspended), then reports the return addresses with
/// the image's load address for `atos -l`. Tracing runs only.
mod stall_sampler {
    use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
    use std::time::Duration;

    static MAIN: AtomicU32 = AtomicU32::new(0);
    static BEAT: AtomicU64 = AtomicU64::new(0);
    const DEPTH: usize = 48;

    #[repr(C)]
    #[derive(Default)]
    struct ArmThreadState64 {
        x: [u64; 29],
        fp: u64,
        lr: u64,
        sp: u64,
        pc: u64,
        cpsr: u32,
        flags: u32,
    }
    const ARM_THREAD_STATE64: i32 = 6;
    unsafe extern "C" {
        fn thread_suspend(thread: u32) -> i32;
        fn thread_resume(thread: u32) -> i32;
        fn thread_get_state(thread: u32, flavor: i32, state: *mut u32, count: *mut u32) -> i32;
        fn _dyld_get_image_vmaddr_slide(index: u32) -> isize;
    }

    fn now_ms() -> u64 {
        super::start().elapsed().as_millis() as u64
    }
    pub fn beat() {
        BEAT.store(now_ms(), Ordering::Relaxed);
    }
    /// Call on the main thread.
    pub fn start() {
        // SAFETY: plain libSystem queries about the calling thread.
        let (port, top, size) = unsafe {
            let me = libc::pthread_self();
            (
                libc::pthread_mach_thread_np(me),
                libc::pthread_get_stackaddr_np(me) as u64,
                libc::pthread_get_stacksize_np(me) as u64,
            )
        };
        MAIN.store(port, Ordering::Relaxed);
        beat();
        let _ = std::thread::Builder::new()
            .name("pm-stall-sampler".into())
            .spawn(move || watch(top - size, top));
    }

    /// One stack of the suspended main thread, into `out`; returns its depth.
    fn sample(thread: u32, low: u64, high: u64, out: &mut [u64; DEPTH]) -> usize {
        let mut state = ArmThreadState64::default();
        let mut count = (std::mem::size_of::<ArmThreadState64>() / 4) as u32;
        // SAFETY: the thread stays suspended while its stack is read; every
        // frame pointer is bounds-checked against its stack before a read.
        unsafe {
            if thread_suspend(thread) != 0 {
                return 0;
            }
            let mut n = 0;
            if thread_get_state(
                thread,
                ARM_THREAD_STATE64,
                (&mut state as *mut ArmThreadState64).cast(),
                &mut count,
            ) == 0
            {
                let strip = |a: u64| a & 0x0000_7FFF_FFFF_FFFF;
                out[0] = strip(state.pc);
                out[1] = strip(state.lr);
                n = 2;
                let mut fp = state.fp;
                while n < DEPTH && fp >= low && fp + 16 <= high && fp % 8 == 0 {
                    let frame = fp as *const u64;
                    let next = *frame;
                    let ret = strip(*frame.add(1));
                    if ret == 0 {
                        break;
                    }
                    out[n] = ret;
                    n += 1;
                    if next <= fp {
                        break;
                    }
                    fp = next;
                }
            }
            thread_resume(thread);
            n
        }
    }

    /// A system frame by its exported symbol (`dladdr`); the app's own frames
    /// stay addresses for `atos -l`, as its symbols aren't exported.
    fn frame_name(address: u64) -> String {
        let mut info: libc::Dl_info = unsafe { std::mem::zeroed() };
        // SAFETY: dladdr only reads the loader's tables for this address.
        let found = unsafe { libc::dladdr(address as *const libc::c_void, &mut info) } != 0;
        let main = unsafe { _dyld_get_image_vmaddr_slide(0) } as u64 + 0x1_0000_0000;
        if !found || info.dli_sname.is_null() || info.dli_fbase as u64 == main {
            return format!("{address:#x}");
        }
        // SAFETY: dladdr returned NUL-terminated loader strings.
        let (symbol, image) = unsafe {
            (
                std::ffi::CStr::from_ptr(info.dli_sname).to_string_lossy(),
                std::ffi::CStr::from_ptr(info.dli_fname).to_string_lossy(),
            )
        };
        let image = image.rsplit('/').next().unwrap_or_default().to_owned();
        format!("{symbol} ({image})")
    }

    fn watch(low: u64, high: u64) {
        let thread = MAIN.load(Ordering::Relaxed);
        // SAFETY: a dyld query with no preconditions.
        let load = 0x1_0000_0000u64.wrapping_add(unsafe { _dyld_get_image_vmaddr_slide(0) } as u64);
        let mut stacks: Vec<Vec<String>> = vec![];
        let mut buf = [0u64; DEPTH];
        loop {
            std::thread::sleep(Duration::from_millis(20));
            let late = now_ms().saturating_sub(BEAT.load(Ordering::Relaxed));
            if late > 55 {
                if stacks.len() < 8 {
                    let n = sample(thread, low, high, &mut buf);
                    stacks.push(buf[..n].iter().map(|a| frame_name(*a)).collect());
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
