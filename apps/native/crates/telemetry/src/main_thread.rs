//! The main thread's heartbeat and a one-shot stack sample of it, shared by
//! the hang watchdog (telemetry) and the stall sampler of the diagnostics
//! trace (`PM_NATIVE_MAIN_TRACE`). A beat is one relaxed atomic store; a
//! sample suspends the main thread for one frame-pointer walk (no allocation
//! while it is suspended).
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Instant;

pub const DEPTH: usize = 48;

static MAIN: AtomicU32 = AtomicU32::new(0);
static STACK_LOW: AtomicU64 = AtomicU64::new(0);
static STACK_HIGH: AtomicU64 = AtomicU64::new(0);
static BEAT: AtomicU64 = AtomicU64::new(0);

fn clock() -> Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    *START.get_or_init(Instant::now)
}

/// Milliseconds on this module's clock.
pub fn now_ms() -> u64 {
    clock().elapsed().as_millis() as u64
}

/// The main thread is alive now.
pub fn beat() {
    BEAT.store(now_ms(), Ordering::Relaxed);
}

/// How long ago the main thread last beat.
pub fn since_beat_ms() -> u64 {
    now_ms().saturating_sub(BEAT.load(Ordering::Relaxed))
}

/// Records the calling thread as the main thread. Call on the main thread;
/// later calls keep the first.
pub fn install() {
    if MAIN.load(Ordering::Relaxed) != 0 {
        return;
    }
    #[cfg(target_os = "macos")]
    // SAFETY: plain libSystem queries about the calling thread.
    unsafe {
        let me = libc::pthread_self();
        let top = libc::pthread_get_stackaddr_np(me) as u64;
        let size = libc::pthread_get_stacksize_np(me) as u64;
        STACK_LOW.store(top - size, Ordering::Relaxed);
        STACK_HIGH.store(top, Ordering::Relaxed);
        MAIN.store(libc::pthread_mach_thread_np(me), Ordering::Relaxed);
    }
    beat();
}

pub fn installed() -> bool {
    MAIN.load(Ordering::Relaxed) != 0
}

/// The main executable's load address (for `atos -l`).
pub fn load_address() -> u64 {
    #[cfg(target_os = "macos")]
    // SAFETY: a dyld query with no preconditions.
    return 0x1_0000_0000u64.wrapping_add(unsafe { _dyld_get_image_vmaddr_slide(0) } as u64);
    #[cfg(not(target_os = "macos"))]
    0
}

#[cfg(target_os = "macos")]
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

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn thread_suspend(thread: u32) -> i32;
    fn thread_resume(thread: u32) -> i32;
    fn thread_get_state(thread: u32, flavor: i32, state: *mut u32, count: *mut u32) -> i32;
    fn _dyld_get_image_vmaddr_slide(index: u32) -> isize;
}

/// One stack of the suspended main thread into `out`, innermost first;
/// returns its depth (0 before [`install`] or off arm64 macOS).
pub fn sample(out: &mut [u64; DEPTH]) -> usize {
    let thread = MAIN.load(Ordering::Relaxed);
    if thread == 0 {
        return 0;
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        const ARM_THREAD_STATE64: i32 = 6;
        let (low, high) = (
            STACK_LOW.load(Ordering::Relaxed),
            STACK_HIGH.load(Ordering::Relaxed),
        );
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
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        let _ = (out, &STACK_LOW, &STACK_HIGH);
        0
    }
}

/// A frame by its exported symbol and binary name (`dladdr`), or the address
/// when the loader has no name for it (the app's own symbols are not
/// exported). Never a path.
pub fn frame_name(address: u64) -> String {
    #[cfg(target_os = "macos")]
    {
        let mut info: libc::Dl_info = unsafe { std::mem::zeroed() };
        // SAFETY: dladdr only reads the loader's tables for this address.
        let found = unsafe { libc::dladdr(address as *const libc::c_void, &mut info) } != 0;
        let main = load_address();
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
    #[cfg(not(target_os = "macos"))]
    format!("{address:#x}")
}
