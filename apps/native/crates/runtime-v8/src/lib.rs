//! Headless V8 realms. All JavaScript and disposal run on owner workers.
//! No kernel, engine, UI, Node, or Chromium dependency is linked here.
mod cache;
mod compiler;
mod store;
pub use compiler::*;
pub use store::*;
mod host;
mod modules;
mod runtime;
mod vm;
pub use cache::{CacheKey, CodeCache, SnapshotKind};
pub use host::*;
pub use runtime::*;
pub use vm::HEAP_FULL;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    JavaScript(String),
    Terminated,
    Quota(&'static str),
    Host(String),
    Closed,
    Busy,
    /// A newer observation replaced this pending work before evaluation.
    Superseded,
    Invalid(&'static str),
    StaleGeneration,
}
impl Error {
    /// The realm ran out of heap or external memory: a fresh realm may succeed.
    pub fn is_memory(&self) -> bool {
        matches!(self, Error::Quota(q) if *q == "heap" || *q == HEAP_FULL || *q == "external memory")
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests;
