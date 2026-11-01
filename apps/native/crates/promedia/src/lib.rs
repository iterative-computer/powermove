//! Worker-side pro media contracts. The host kernel owns and publishes models.
//! Filesystem operations and codecs must run on workers, never on the UI thread.
pub mod cache;
pub mod codec;
pub mod media;
pub mod storage;
pub mod tasks;

// Lane ownership excludes pm-color/lib.rs. Compile the actual engine sources
// here until the colour owner exports these modules from pm-color.
#[path = "../../../../../engine/crates/pm-color/src/lut.rs"]
pub mod lut;
#[path = "../../../../../engine/crates/pm-color/src/scopes.rs"]
pub mod scopes;

// Process-local lifetimes fence late work after a catalog/queue is replaced.
pub(crate) fn next_epoch() -> u64 {
    static EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    EPOCH
        .fetch_update(
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
            |n| n.checked_add(1),
        )
        .expect("process lifetime namespace exhausted")
}
