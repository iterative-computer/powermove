//! Engine protocol messages and contract types (sections 19 and 27).
//!
//! `pm-protocol` holds the fifteen Phase 0 contracts as Rust types with
//! serde, MessagePack (rmp-serde), and TypeScript generation (ts-rs). Each
//! contract has an RFC in `docs/engine/rfcs/` (see [`ContractId::rfc`]);
//! the plan is `docs/rust-engine.md`. All contracts are drafts (version 0)
//! until their consumer phase starts.
//!
//! | # | Contract | Module |
//! |---|---|---|
//! | 1 | `EvaluatedScene` | [`scene`] |
//! | 2 | `FrameRequest` and `FrameDescriptor` | [`frame`] |
//! | 3 | The `pm-gpu` interop trait | [`interop`] |
//! | 4 | Color | [`color`] |
//! | 5 | Patches (message envelope) | [`patch`] |
//! | 6 | The playback clock | [`clock`] |
//! | 7 | Errors and isolation | [`error`] |
//! | 8 | Budget leases | [`lease`] |
//! | 9 | Protocol codec and code generation | [`envelope`], [`codec`], [`version`], [`common`] |
//! | 10 | Definition snapshots | [`definitions`] |
//! | 11 | Blender jobs | [`blender`] |
//! | 12 | Priority classes and agent frames | [`priority`] |
//! | 13 | GPU time slicing | [`slicing`] |
//! | 14 | Host services | [`host`] |
//! | 15 | The headless document host | [`document_host`] |
//!
//! [`media`] (Interpret Footage's media facts) extends contract 14.

pub mod blender;
pub mod clock;
pub mod codec;
pub mod color;
pub mod common;
pub mod definitions;
pub mod document_host;
pub mod envelope;
pub mod error;
pub mod frame;
pub mod host;
pub mod interop;
pub mod lease;
pub mod media;
pub mod patch;
pub mod priority;
pub mod scene;
pub mod slicing;
pub mod validate;
pub mod version;

#[doc(hidden)]
pub mod samples;

#[cfg(feature = "ts")]
pub mod ts;

pub use codec::{CodecError, decode, encode};
pub use envelope::{Envelope, Event, Message, Request, Response};
pub use validate::{Validate, Violation};
pub use version::{ContractId, WIRE_VERSION};
