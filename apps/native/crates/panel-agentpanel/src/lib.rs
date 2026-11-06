//! Agent panel model and concrete provider worker bridge. No renderer, JS,
//! native dialogs, or independent filesystem writer.
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
pub mod runtime;
pub mod store;
pub mod v4;
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
/// What a conversation shows when a run could not start or act.
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Host(message) => f.write_str(message),
            Error::Busy => f.write_str("The agent is busy. Try again in a moment."),
            Error::Stale => f.write_str("This run changed before the request reached it."),
            Error::UnknownId(_) | Error::Invalid(_) | Error::MissingPayload(_) => {
                f.write_str("The agent could not take this request.")
            }
        }
    }
}

pub mod work_groups;
