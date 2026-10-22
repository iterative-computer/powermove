//! Whole-clip generated media and representation selection (plan section 14).
//!
//! The project records proxy *intent*. This crate stores derived bytes by the
//! source's full content hash and a versioned recipe; paths to source files
//! never enter the store. Analysis and normal exports resolve the original.

mod generate;
mod policy;
mod sequence;
mod store;

pub use generate::{
    GeneratedProxy, GenerationControl, GenerationRequest, PauseReason, generate,
    generate_with_disk_probe,
};
pub use policy::{
    AutomaticInput, DecodeMeasurement, MachineProfile, MediaAvailability, MediaUse, PlaybackMoment,
    Representation, RepresentationChoice, RepresentationIntent, ResolutionIntent,
    choose_representation, probe_machine_profile, source_for,
};
pub use sequence::{SequenceFrame, SequenceManifest, SequenceRate};
pub use store::{
    ProxyKey, ProxyRecipe, ProxyStore, StoredProxy, TEST_DISK_RESERVE_ENV, disk_guard, free_bytes,
};
#[cfg(feature = "test-hooks")]
pub use store::set_test_disk_reserve;

#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid media: {0}")]
    InvalidMedia(String),
    #[error("proxy validation failed: {0}")]
    Validation(String),
    #[error("codec process failed: {0}")]
    Codec(String),
    #[error("proxy generation cancelled")]
    Cancelled,
    #[error("store data is corrupt: {0}")]
    CorruptStore(String),
}

pub type Result<T> = std::result::Result<T, ProxyError>;
