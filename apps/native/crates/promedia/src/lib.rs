//! Worker-side pro media contracts. The host kernel owns and publishes models.
//! Filesystem operations and codecs must run on workers, never on the UI thread.
pub mod cache;
pub mod cache_frame;
pub mod codec;
pub mod export_panel;
pub mod media;
pub mod proxy;
pub mod render;
pub mod render_job;
pub mod source_frames;
pub mod storage;
pub mod tasks;
pub mod worker;

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
