//! Targeted source scrub and filmstrip counters plus wall-clock in JSON.
//!
//! Build with `cargo build --release -p pm-perfbench --example source_media`,
//! then run `pm-electron-lock target/release/examples/source_media`.
//! `PM_SOURCE_BENCH_MEDIA=/path/to/clip.mp4` replaces the public fixture.
//! The generated filmstrip is removed on drop.

#[global_allocator]
static ALLOC: pm_perf::CountingAlloc = pm_perf::CountingAlloc;

#[cfg(target_os = "macos")]
fn main() {
    for bench in [
        pm_perfbench::source_media::source_scrub_24,
        pm_perfbench::source_media::source_filmstrip_24,
    ] {
        println!("{}", bench().to_json());
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {}
