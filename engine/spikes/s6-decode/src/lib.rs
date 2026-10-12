//! Spike S6 (plan section 26, Phase 0): decode the e2e fixture matrix and a
//! seek corpus into wgpu textures without CPU copies, and prove random access
//! to frame N equals sequential decode of frame N.

pub mod apple;
pub mod ff;
pub mod frame;
pub mod gpu;
pub mod index;
pub mod pool;
pub mod time;
pub mod vt;
pub mod run;
pub mod sw;
