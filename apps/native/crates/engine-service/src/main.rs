mod bench;
mod memory_bench;
// Match the app allocator so headless footprint comparisons measure the
// same page-return policy, including project-close pressure relief.
//
// Benchmark switch: `PM_BENCH_SYSTEM_ALLOC=1` uses the system allocator, as
// the app now does (run it with `MallocLargeCache=0`, as the bundle is).
// Decided once, before the first allocation, from the raw environment.
#[global_allocator]
static GLOBAL: Switch = Switch;

struct Switch;
static SYSTEM: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
fn system() -> bool {
    use std::sync::atomic::Ordering;
    match SYSTEM.load(Ordering::Relaxed) {
        1 => true,
        2 => false,
        _ => {
            unsafe extern "C" {
                fn getenv(name: *const std::ffi::c_char) -> *const std::ffi::c_char;
            }
            // SAFETY: getenv reads the environment without allocating.
            let on = unsafe {
                let v = getenv(c"PM_BENCH_SYSTEM_ALLOC".as_ptr());
                !v.is_null() && *v == b'1' as std::ffi::c_char
            };
            SYSTEM.store(if on { 1 } else { 2 }, Ordering::Relaxed);
            on
        }
    }
}
// SAFETY: every block is freed by the allocator that made it (the choice
// never changes after the first call).
unsafe impl std::alloc::GlobalAlloc for Switch {
    unsafe fn alloc(&self, l: std::alloc::Layout) -> *mut u8 {
        unsafe {
            if system() {
                std::alloc::System.alloc(l)
            } else {
                mimalloc::MiMalloc.alloc(l)
            }
        }
    }
    unsafe fn alloc_zeroed(&self, l: std::alloc::Layout) -> *mut u8 {
        unsafe {
            if system() {
                std::alloc::System.alloc_zeroed(l)
            } else {
                mimalloc::MiMalloc.alloc_zeroed(l)
            }
        }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: std::alloc::Layout) {
        unsafe {
            if system() {
                std::alloc::System.dealloc(p, l)
            } else {
                mimalloc::MiMalloc.dealloc(p, l)
            }
        }
    }
    unsafe fn realloc(&self, p: *mut u8, l: std::alloc::Layout, n: usize) -> *mut u8 {
        unsafe {
            if system() {
                std::alloc::System.realloc(p, l, n)
            } else {
                mimalloc::MiMalloc.realloc(p, l, n)
            }
        }
    }
}

fn collect_allocator() {
    unsafe extern "C" {
        fn mi_collect(force: bool);
    }
    unsafe { mi_collect(true) };
}

fn main() -> anyhow::Result<()> {
    unsafe extern "C" {
        fn mi_option_set(option: i32, value: std::ffi::c_long);
    }
    if !system() {
        unsafe { mi_option_set(15, 0) }; // mi_option_purge_delay
        pm_memory::set_allocator_relief(collect_allocator);
    }
    if std::env::args().any(|a| a == "--memory-bench") {
        return memory_bench::run();
    }
    if std::env::args().nth(1).as_deref() == Some("--engine-job") {
        return pm_engine_service::jobs::run_child();
    }
    if std::env::args().any(|a| a == "--bench") {
        return bench::run();
    }
    anyhow::bail!("pm-engine-service is a library; this binary handles --engine-job")
}
