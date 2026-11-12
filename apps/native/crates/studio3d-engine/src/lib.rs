//! Native worker glue around the pinned shared Studio renderer.
//! This crate contains no UI and does not own authored document state.
#[cfg(feature = "oidn")]
pub use pm_studio::oidn;
pub use pm_studio::{bvh, environment, gpu, sampling, math, mesh, nature, scatter, scene, terrain};
pub mod compositor;
pub mod extract;
pub mod service;
pub mod splats;
pub mod status;
pub mod swatch;

pub mod packing;
pub use packing::{MaterialExt, SceneExt};
