//! Rust-allocation attribution only: native Metal/CoreText and Node heaps
//! require footprint/vmmap/Instruments, not subtraction from these counts.
//! cargo run --release -p pm-node --example startup_heap -- /outside/repo/dhat.json

#[global_allocator]
static ALLOCATOR: dhat::Alloc = dhat::Alloc;

fn checkpoint(name: &str) {
    let stats = dhat::HeapStats::get();
    println!(
        "{name}: live_bytes={} live_blocks={} peak_bytes={} total_bytes={}",
        stats.curr_bytes, stats.curr_blocks, stats.max_bytes, stats.total_bytes
    );
}

fn main() {
    let output = std::path::PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("an external dhat output path"),
    );
    let checkout = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .unwrap();
    assert!(
        output.is_absolute() && !output.starts_with(checkout),
        "profiling evidence stays outside Git"
    );
    let _profile = dhat::Profiler::builder().file_name(output).build();
    checkpoint("before host");
    {
        let host = pm_node::EngineHost::with_sink(
            pm_protocol::error::EngineProcess::Interactive,
            Some(std::sync::Arc::new(|_| {})),
            |_| None,
        )
        .expect("interactive engine host");
        checkpoint("interactive host without a viewer");
        drop(host);
    }
    checkpoint("after interactive host drop");
    {
        let host = pm_node::EngineHost::with_sink(
            pm_protocol::error::EngineProcess::Job,
            Some(std::sync::Arc::new(|_| {})),
            |_| None,
        )
        .expect("job engine host");
        checkpoint("job host with idle lanes");
        drop(host);
    }
    checkpoint("after job host drop");
    #[cfg(target_os = "macos")]
    {
        let gpu = pm_gpu::Gpu::new("startup heap attribution").expect("Metal device");
        checkpoint("Metal device");
        let renderer = pm_render::Renderer::new(gpu.device.clone(), gpu.queue.clone());
        checkpoint("renderer without content");
        drop(renderer);
        drop(gpu);
    }
    checkpoint("after renderer drop");
}
