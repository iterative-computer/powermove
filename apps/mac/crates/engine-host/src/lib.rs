//! Native, in-process render worker. No Node/WASM ABI, GPUI, or main-thread GPU waits.
//! See README.md for ownership, font pumping, and frame leases.
mod pixels;
mod worker;

pub use pm_gpu::bridge_out::surface::ExportedSurface;
pub use pm_node::fonts::render::FontBroker;
pub use pm_protocol::color::{ViewerColorSpace, WorkingEncoding};
pub use pm_protocol::common::{Size, Ticks};
pub use worker::{EngineHost, RenderRequest};

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

/// Create and pump on AppKit's main thread. A render worker can ask for fonts;
/// main must keep pumping and must never synchronously wait for that worker.
pub struct MainThreadFonts {
    service: pm_node::fonts::FontService,
    broker: FontBroker,
}
impl MainThreadFonts {
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Result<Self, String> {
        let service = pm_node::fonts::FontService::new()?;
        let broker = FontBroker::new();
        broker.set_wake(wake);
        broker.set_generation(service.generation());
        Ok(Self { service, broker })
    }
    pub fn broker(&self) -> FontBroker {
        self.broker.clone()
    }
    /// Allows project fonts to be installed through the engine's existing font API.
    pub fn service_mut(&mut self) -> &mut pm_node::fonts::FontService {
        &mut self.service
    }
    pub fn pump(&mut self) {
        self.broker.set_generation(self.service.generation());
        pm_node::fonts::render::serve(&mut self.service, self.broker.take_asks());
    }
}
impl Drop for MainThreadFonts {
    fn drop(&mut self) {
        self.broker.close();
    }
}

/// Immutable producer-complete pixels. Clones are pool leases. The GPUI adapter
/// retains one through scene retirement and each Metal consumer's completion.
#[derive(Clone)]
pub struct FrameHandle {
    pub surface: Arc<ExportedSurface>,
    pub revision: u64,
    pub time: Ticks,
    pub size: Size,
    /// Pool generation (changes on output-size changes).
    pub generation: u64,
    pub slot: usize,
    /// Monotonic request identity; stale work is never published.
    pub request_id: u64,
    pub metal_registry_id: u64,
    pub color_space: ViewerColorSpace,
    pub requested_at: Instant,
    pub ready_at: Instant,
    displayed_ns: Arc<AtomicU64>,
}
impl std::fmt::Debug for FrameHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameHandle")
            .field("revision", &self.revision)
            .field("time", &self.time)
            .field("generation", &self.generation)
            .field("slot", &self.slot)
            .field("request_id", &self.request_id)
            .finish()
    }
}
impl FrameHandle {
    /// Called by the consumer's Metal completion handler; records the first
    /// completed draw. This measures request→consumer GPU completion, not scanout.
    pub fn consumer_completed(&self) {
        let elapsed = self.requested_at.elapsed().as_nanos().min(u64::MAX as u128) as u64;
        if self
            .displayed_ns
            .compare_exchange(0, elapsed.max(1), Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            eprintln!(
                "frame {} r{} t={} slot={}: request→consumer GPU {:.2} ms (producer {:.2} ms)",
                self.request_id,
                self.revision,
                self.time.get(),
                self.slot,
                elapsed as f64 / 1e6,
                self.ready_at
                    .duration_since(self.requested_at)
                    .as_secs_f64()
                    * 1000.0
            );
        }
    }
    pub fn display_latency(&self) -> Option<Duration> {
        let ns = self.displayed_ns.load(Ordering::Acquire);
        (ns != 0).then(|| Duration::from_nanos(ns))
    }
}

/// A slot is writable only when the ring is its sole owner. In particular,
/// dropping the UI frame does not free a scene/command-buffer lease.
fn slot_free<T>(slot: &Arc<T>) -> bool {
    Arc::strong_count(slot) == 1
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scene_and_gpu_leases_both_block_reuse() {
        let slot = Arc::new(());
        let frame = slot.clone();
        let scene = frame.clone();
        let gpu = scene.clone();
        drop(frame);
        assert!(!slot_free(&slot));
        drop(scene);
        assert!(!slot_free(&slot));
        drop(gpu);
        assert!(slot_free(&slot));
    }
}
