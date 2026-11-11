mod bench;
mod memory_bench;
// Match the app allocator so headless footprint comparisons measure the
// same page-return policy, including project-close pressure relief.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

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
    unsafe { mi_option_set(15, 0) }; // mi_option_purge_delay
    pm_memory::set_allocator_relief(collect_allocator);
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
