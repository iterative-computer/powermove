//! Headless V8 realms. All JavaScript and disposal run on owner workers.
//! No kernel, engine, UI, Node, or Chromium dependency is linked here.
mod cache;
mod compiler;
pub use compiler::*;
mod host;
mod runtime;
mod vm;
pub use cache::{CacheKey, CodeCache, SnapshotKind};
pub use host::*;
pub use runtime::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    JavaScript(String),
    Terminated,
    Quota(&'static str),
    Host(String),
    Closed,
    Busy,
    Invalid(&'static str),
    StaleGeneration,
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
