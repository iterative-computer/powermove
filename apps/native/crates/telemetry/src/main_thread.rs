//! The main thread's heartbeat and a one-shot stack sample of it, shared by
//! the hang watchdog (telemetry) and the stall sampler of the diagnostics
//! trace (`PM_NATIVE_MAIN_TRACE`). A beat is one relaxed atomic store; a
//! sample suspends the main thread for one frame-pointer walk (no allocation
//! while it is suspended).
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::Instant;

pub const DEPTH: usize = 48;

static MAIN: AtomicU32 = AtomicU32::new(0);
static STACK_LOW: AtomicU64 = AtomicU64::new(0);
static STACK_HIGH: AtomicU64 = AtomicU64::new(0);
static BEAT: AtomicU64 = AtomicU64::new(0);
static QUEUE_BEAT: AtomicU64 = AtomicU64::new(0);
static QUEUE_POSTED: AtomicBool = AtomicBool::new(false);

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
    QUEUE_BEAT.store(now_ms(), Ordering::Relaxed);
    #[cfg(target_os = "macos")]
    let _ = std::thread::Builder::new()
        .name("pm-main-queue-beat".into())
        .spawn(post_queue_beats);
}

/// How long ago the main thread last drained its dispatch queue. The app's
/// heartbeat ([`beat`]) is a GPUI task woken by a timer that passes through
/// the shared background queues; this one is posted straight to the main
/// queue from a thread of its own, so the two disagree only when the main
/// thread answers but the background queues are held up.
pub fn since_queue_beat_ms() -> u64 {
    now_ms().saturating_sub(QUEUE_BEAT.load(Ordering::Relaxed))
}

#[cfg(target_os = "macos")]
fn post_queue_beats() {
    extern "C" fn answered(_: *mut std::ffi::c_void) {
        QUEUE_BEAT.store(now_ms(), Ordering::Relaxed);
        QUEUE_POSTED.store(false, Ordering::Relaxed);
    }
    loop {
        std::thread::sleep(std::time::Duration::from_millis(250));
        // One post at a time: a hung main thread never gathers a backlog.
        if !QUEUE_POSTED.swap(true, Ordering::Relaxed) {
            // SAFETY: libdispatch's main queue lives for the process; the
            // callback takes no context.
            unsafe {
                dispatch_async_f(
                    std::ptr::addr_of!(_dispatch_main_q).cast_mut().cast(),
                    std::ptr::null_mut(),
                    answered,
                )
            };
        }
    }
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
    static _dispatch_main_q: u8;
    fn dispatch_async_f(
        queue: *mut std::ffi::c_void,
        context: *mut std::ffi::c_void,
        work: extern "C" fn(*mut std::ffi::c_void),
    );
    static mach_task_self_: u32;
    fn mach_vm_read_overwrite(
        task: u32,
        address: u64,
        size: u64,
        data: u64,
        out_size: *mut u64,
    ) -> i32;
}

/// Walks frame pointers from `fp` within the stack `[low, high)`, reading
/// each frame record through `read` (which may fail), into `out`, innermost
/// first after `pc` and `lr`. Never dereferences, never overflows, always
/// ends: each next frame must be higher up the stack than the last.
pub fn walk(
    pc: u64,
    lr: u64,
    mut fp: u64,
    low: u64,
    high: u64,
    mut read: impl FnMut(u64) -> Option<(u64, u64)>,
    out: &mut [u64; DEPTH],
) -> usize {
    let strip = |a: u64| a & 0x0000_7FFF_FFFF_FFFF;
    out[0] = strip(pc);
    out[1] = strip(lr);
    let mut n = 2;
    while n < DEPTH
        && fp >= low
        && fp.is_multiple_of(8)
        && fp.checked_add(16).is_some_and(|end| end <= high)
    {
        let Some((next, ret)) = read(fp) else { break };
        let ret = strip(ret);
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
    n
}

/// Resumes the thread however the sample ends.
#[cfg(target_os = "macos")]
struct Suspended(u32);
#[cfg(target_os = "macos")]
impl Drop for Suspended {
    fn drop(&mut self) {
        // SAFETY: the thread was suspended by this guard's owner.
        unsafe {
            thread_resume(self.0);
        }
    }
}

/// One stack of the main thread into `out`, innermost first; returns its
/// depth (0 before [`install`] or off arm64 macOS). The thread is suspended
/// only for the walk (no allocation, no lock), every read is a fallible Mach
/// read, and it is resumed on every path.
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
        // SAFETY: plain Mach calls; the guard resumes the thread.
        unsafe {
            if thread_suspend(thread) != 0 {
                return 0;
            }
            let _resume = Suspended(thread);
            if thread_get_state(
                thread,
                ARM_THREAD_STATE64,
                (&mut state as *mut ArmThreadState64).cast(),
                &mut count,
            ) != 0
            {
                return 0;
            }
            let task = mach_task_self_;
            walk(
                state.pc,
                state.lr,
                state.fp,
                low,
                high,
                |fp| {
                    let mut record = [0u64; 2];
                    let mut got = 0u64;
                    let ok =
                        mach_vm_read_overwrite(task, fp, 16, record.as_mut_ptr() as u64, &mut got)
                            == 0;
                    (ok && got == 16).then_some((record[0], record[1]))
                },
                out,
            )
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Sol P1 (w99 review): a corrupt stack ends the walk, never a fault or
    /// an overflow, and an unreadable frame stops it.
    #[test]
    fn a_corrupt_stack_ends_the_walk_safely() {
        let mut out = [0u64; DEPTH];
        // A frame pointer near the top of the address space can't wrap.
        let n = walk(1, 2, u64::MAX - 8, 0, u64::MAX, |_| Some((0, 3)), &mut out);
        assert_eq!(n, 2);
        // A loop (next not higher) ends after one frame.
        let n = walk(1, 2, 0x1000, 0x1000, 0x2000, |fp| Some((fp, 7)), &mut out);
        assert_eq!(n, 3);
        // An unreadable frame stops the walk.
        let n = walk(1, 2, 0x1000, 0x1000, 0x2000, |_| None, &mut out);
        assert_eq!(n, 2);
        // A real chain is followed to the end of the stack.
        let n = walk(
            1,
            2,
            0x1000,
            0x1000,
            0x1040,
            |fp| Some((fp + 16, fp)),
            &mut out,
        );
        assert_eq!(&out[..n], &[1, 2, 0x1000, 0x1010, 0x1020, 0x1030]);
    }

    /// A main thread that never drains its dispatch queue (libtest's waits in
    /// a join) reads stale on the queue heartbeat.
    #[test]
    fn the_queue_heartbeat_goes_stale_when_the_main_queue_is_not_drained() {
        install();
        std::thread::sleep(std::time::Duration::from_millis(700));
        assert!(since_queue_beat_ms() >= 600, "{}", since_queue_beat_ms());
    }

    /// The sampler suspends and resumes: the main thread keeps running.
    #[test]
    fn sampling_a_running_thread_resumes_it() {
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            install();
            tx.send(()).unwrap();
            let mut spins = 0u64;
            for _ in 0..200 {
                spins = spins.wrapping_add(std::hint::black_box(1));
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            spins
        });
        rx.recv().unwrap();
        let mut out = [0u64; DEPTH];
        for _ in 0..20 {
            let _ = sample(&mut out);
        }
        assert_eq!(worker.join().unwrap(), 200);
    }
}
