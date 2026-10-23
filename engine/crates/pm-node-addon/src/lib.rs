//! The engine processes' Node-API addon (`native/pm_node.node`).
//!
//! Everything lives in [`pm_node`]; with the `node` feature its `#[napi]`
//! items register themselves (napi-rs constructors) and the `napi` crate
//! exports `napi_register_module_v1`, the one symbol Node looks up. This
//! crate only makes the cdylib, so the release profile's LTO applies to it
//! (docs/engine/release-build.md).

// Links pm-node (and with it the napi registrations) into the cdylib.
use pm_node as _;

/// Measurement builds only: Rust allocations through DHAT (`dhat-heap`).
#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOCATOR: dhat::Alloc = dhat::Alloc;

#[cfg(feature = "dhat-heap")]
mod heap_profile {
    use std::time::Duration;

    /// Starts the profiler when the dylib loads (a Mach-O initializer, as
    /// napi-rs registers its exports), so allocations from the first engine
    /// call on are attributed, and writes the profile after the delay.
    extern "C" fn start() {
        let Some(directory) = std::env::var_os("POWERMOVE_ENGINE_DHAT") else {
            return;
        };
        let after = std::env::var("POWERMOVE_ENGINE_DHAT_AFTER_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30_000);
        let file = std::path::PathBuf::from(directory)
            .join(format!("dhat-{}.json", std::process::id()));
        let profiler = dhat::Profiler::builder().file_name(file).build();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(after));
            drop(profiler);
        });
    }

    #[used]
    #[unsafe(link_section = "__DATA,__mod_init_func")]
    static START: extern "C" fn() = start;
}
