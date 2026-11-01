//! CPU-side application services. All I/O requires an explicit host capability.
//! Construct and drive these services on a worker, never on the application main thread.
//! No profile discovery, JavaScript runtime, network client or updater installer lives here.
pub mod diagnostics;
pub mod fonts;
pub mod library;
pub mod media;
pub mod settings;
pub mod storage;
pub mod worker;

#[cfg(test)]
mod tests;
