//! Blender jobs for the engine (plan sections 8, 9, 14.7, 17; contract 11,
//! `pm_protocol::blender`; contract 14's Blender supervision; amendments 11
//! and 14). Design notes: `docs/engine/blender.md`.
//!
//! Compositions with `render3d.enabled` get each 3D object layer's pixels
//! from a user-installed Blender. This crate holds everything about that
//! which needs neither the GPU nor the evaluator:
//!
//! - [`snapshot`]: the Blender snapshot (today's `renderSnapshot` JSON,
//!   `SnapshotEncoding::RenderSnapshotJsonV1`), its canonical bytes and
//!   hash, and the frame it belongs to. `pm-render` builds snapshots from
//!   `pm-eval` and `pm-geom3d` (`pm_render::blender::snapshot`).
//! - [`compare`]: the section 25 "Blender snapshots" parity class.
//! - [`surface`]: decoding a surface (16-bit RGBA PNG, AgX codes = E,
//!   straight alpha) exactly as `code / 65535`, ignoring PNG color chunks,
//!   premultiplied in E at f32, with today's crop rectangle.
//! - [`key`]: the section 14.7 surface cache key.
//! - [`cache`]: the surface cache (memory under a lease, and disk).
//! - [`worker`]: the pinned worker script and its hash.
//! - [`supervisor`]: Blender discovery and the supervised worker process
//!   (scrubbed environment, pinned flags, timeout, cancel), used by the CLI
//!   host and tests; Electron main runs the same script with the same rules
//!   (`apps/desktop/src/main/engine/blender.ts`).
//! - [`job`]: turning the frames a render needs into contract 11 requests,
//!   and the batching rule.

pub mod cache;
pub mod compare;
pub mod job;
pub mod key;
pub mod snapshot;
pub mod supervisor;
pub mod surface;
pub mod worker;

pub use cache::SurfaceCache;
pub use snapshot::{AssetBytes, BlenderFrame, Snapshot, SnapshotAsset};
pub use surface::{DecodedSurface, SurfaceError, decode_surface};

/// Errors of Blender work, mapped to `ErrorCode::BlenderMissing` and
/// `ErrorCode::BlenderFailed` by the job engine.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BlenderError {
    /// Blender is not installed, cannot start, or is older than 4.5.
    #[error("{0}")]
    Missing(String),
    /// A job failed, timed out, or did not deliver a surface.
    #[error("{0}")]
    Failed(String),
    /// The job was cancelled.
    #[error("Blender request cancelled")]
    Cancelled,
}

/// The sentence shown when a composition needs Blender and none is usable
/// (appendix D, "Offline media and Blender at export").
pub const BLENDER_MISSING_MESSAGE: &str = "This composition renders with Blender. Install Blender 4.5 or later, or set its renderer to Native.";
