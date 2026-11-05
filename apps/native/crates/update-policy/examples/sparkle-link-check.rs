//! Link/guard probe only: starts no updater and performs no network/install work.
#[cfg(all(target_os = "macos", feature = "macos-sparkle"))]
fn main() {
    use std::sync::{Arc, atomic::AtomicBool};
    assert!(
        pm_update_policy::sparkle_native::NativeSession::new(Arc::new(AtomicBool::new(false)))
            .is_err()
    );
    println!(
        "Sparkle bridge linked; missing termination guard refused. Runtime installation remains unverified."
    );
}
#[cfg(not(all(target_os = "macos", feature = "macos-sparkle")))]
fn main() {
    eprintln!("BLOCKED: requires macOS and --features macos-sparkle");
    std::process::exit(2);
}
