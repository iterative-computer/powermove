//! Store policy and worker-side lifecycle. No JS, WebKit, UI, or kernel writer.
//! Platform adapters enqueue commands; none of these I/O methods belong on main.
pub mod adapter;
pub mod artifact;
pub mod identity;
pub mod install;
pub mod permission;
pub mod view;

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Invalid(&'static str),
    Integrity(&'static str),
    Conflict,
    NotInstalled,
    Denied,
    Stale,
    Backpressure,
    Platform(String),
    Io(String),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}
pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
