//! # `pm-mac-core`: the document authority of Powermove for Mac
//!
//! Pure Rust, no GPUI. Every panel, the MCP server and extensions read and
//! change the project through this crate; nothing else mutates the
//! document (docs/gpui/engine-boundary.md, sections 1, 2.2, 5.2 and 6).
//! The short version for panel authors is `docs/gpui/core-api.md`.
//!
//! ## The contract
//!
//! * **One store per open project**: [`DocumentStore`]. It lives on the
//!   thread that owns the window (the GPUI main thread) and is `Send`, not
//!   shared: wrap it in a GPUI entity, never in a lock other threads wait on.
//! * **Reading**: [`DocumentStore::snapshot`] returns a [`Snapshot`], an
//!   `Arc` of the immutable document at one revision. Hand snapshots to
//!   render, audio and export threads; they never change underneath you.
//!   [`DocumentStore::layers`] lists the open composition's layers;
//!   [`DocumentStore::queries`] answers evaluated values, bounds and picks
//!   synchronously at the current revision ([`QueryContext`]).
//! * **Changing**: only through typed [`Command`]s:
//!   [`DocumentStore::execute`] (one command, one undo step),
//!   [`DocumentStore::transact`] (many commands, one undo step), or a
//!   gesture ([`DocumentStore::begin_gesture`] → [`update_gesture`](DocumentStore::update_gesture)
//!   → [`commit_gesture`](DocumentStore::commit_gesture) /
//!   [`cancel_gesture`](DocumentStore::cancel_gesture)) for drags, which
//!   publish live revisions but make one undo step. Every change is atomic:
//!   staged on a scratch tree, validated as a whole, published only on
//!   success. Commands address layers by **id**, never by index.
//! * **Undo/redo**: [`DocumentStore::undo`] / [`redo`](DocumentStore::redo)
//!   publish *new, higher* revisions that restore content. Revisions never
//!   go backwards, so any reader that keys caches by revision stays correct.
//! * **Observing**: [`DocumentStore::subscribe`] calls you synchronously with
//!   a [`StoreEvent`] after the store (and its query mirror) is fully
//!   updated: [`StoreEvent::Changed`] carries the [`Change`] (base revision,
//!   revision, operations) for incremental readers;
//!   [`DocumentStore::changes_since`] catches up a reader that lagged.
//! * **Selection**: [`DocumentStore::select_layers`] and friends; ids, so
//!   reordering never changes what is selected. Not undoable, not saved.
//! * **Files**: [`project_io::open_project`] (blocking: run it on a
//!   background executor) → [`DocumentStore::open`]. Saving:
//!   [`DocumentStore::prepare_save`] (cheap, main thread) →
//!   [`SavePlan::run`] (blocking, background) →
//!   [`DocumentStore::finish_save`] (main thread). Edits during a save stay
//!   dirty; a failed save changes neither the file nor the dirty state.
//!
//! ## Threading model
//!
//! | Thread | May call |
//! |---|---|
//! | Owner (GPUI main) | everything on [`DocumentStore`] |
//! | Background executor | [`project_io::open_project`], [`SavePlan::run`] |
//! | Render / audio / export | read [`Snapshot`]s they were given |
//!
//! ## Modules
//!
//! * [`document`]: the store, snapshots, changes, gestures, save hand-off.
//! * [`commands`]: typed commands and their validation.
//! * [`history`]: undo records and the saved history format.
//! * [`selection`]: selected layers, keys and channel.
//! * [`queries`]: the synchronous `pm-wasm` query mirror.
//! * [`project_io`]: open (JSON, PMV3, PMV4) and the PMV4 writer.
//! * [`model`]: read helpers over the canonical tree (layer lookup, lock).

pub mod commands;
pub mod document;
pub mod error;
pub mod history;
pub mod model;
pub mod project_io;
pub mod queries;
pub mod selection;

pub use commands::{Command, Origin};
pub use document::{
    Change, ChangeCause, Committed, DocumentStore, GestureId, Snapshot, StoreEvent, Subscription,
    TransactOptions,
};
pub use error::{CommandError, ProjectIoError, TransactionError};
pub use history::{History, HistoryMark, HistoryRecord};
pub use model::LayerInfo;
pub use project_io::{
    OpenOptions, OpenedProject, ProjectFile, SaveOptions, SavePlan, SaveReceipt, SaveTarget,
    open_project,
};
pub use queries::{Bounds, PickHit, PickOptions, QueryContext, QueryError};
pub use selection::Selection;

/// Compile-time checks of the threading contract.
#[allow(dead_code)]
fn assert_thread_bounds() {
    fn send<T: Send>() {}
    fn send_sync<T: Send + Sync>() {}
    send::<DocumentStore>();
    send::<SavePlan>();
    send::<SaveReceipt>();
    send::<OpenedProject>();
    send_sync::<Snapshot>();
    send_sync::<Change>();
}
