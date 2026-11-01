//! Pure caption data and operations. Times are seconds in layer space.
//! No renderer, clock, ID generator, project writer or UI dependency. Callers
//! supply new cue IDs, so headless edits are deterministic and retryable (N7).

pub mod formats;
pub mod model;
pub mod style;
pub mod text;
pub use formats::*;
pub use model::*;
pub use style::*;
pub use text::*;
