//! Agent panel MODEL only. No renderer, JS, native dialogs, or filesystem writer.
//! Commands and effects address project/thread/run/row ids. Hosts execute effects
//! on workers and acknowledge them; kernel writes return nonblocking futures.
//! See docs/native/panels/agentpanel.md for tagged evidence and integration limits.
pub mod activity;
pub mod composer;
pub mod input;
pub mod interaction;
pub mod kernel;
pub mod model;
pub mod placement;
pub mod questions;
pub mod view;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    UnknownId(String),
    Invalid(&'static str),
    Busy,
    Stale,
    MissingPayload(String),
    Host(String),
}
pub type Result<T> = std::result::Result<T, Error>;
