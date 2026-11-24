//! Aero's eval suite (DESIGN.md section 8): the versioned task file and its
//! fixtures, programmatic graders, the runner that drives a harness against the
//! headless `powermove-mcp` host, run records with 90 % intervals, and the
//! model-stream recorder/replayer that lets pm-gate catch harness regressions
//! without a paid model. See `docs/native/aero-eval.md`.
pub mod artifacts;
pub mod graders;
pub mod image;
pub mod pmv;
pub mod record;
pub mod replay;
pub mod runner;
pub mod stats;
pub mod suite;
pub mod verdict;

use std::path::PathBuf;

/// The crate's `data/` folder (suite, fixtures, rubrics, recordings).
pub fn data_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data")
}

/// Lowercase hex SHA-256.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
