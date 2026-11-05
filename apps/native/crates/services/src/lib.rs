//! CPU-side application services. All I/O requires an explicit host capability.
//! Construct and drive these services on a worker, never on the application main thread.
//! No profile discovery, JavaScript runtime, network client or updater installer lives here.
pub mod error;
pub use error::{Result, ServiceError, ServiceErrorCode};
pub mod diagnostics;
pub mod entrypoints;
pub mod fonts;
pub mod library;
pub mod media;
pub mod settings;
pub mod storage;
pub mod store;
pub mod surfaces;
pub mod updater;
pub mod worker;

#[cfg(test)]
mod tests;
