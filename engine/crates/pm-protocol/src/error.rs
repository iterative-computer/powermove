//! Errors and isolation (contract 7; owner: Engine and platform).
//!
//! An error names what failed (`scope`), why (`code`), and whether trying
//! again can help (`retry`). Scopes map to isolation rules: a failing layer
//! renders transparent, a failing effect is skipped with its data kept, a
//! failing request fails alone, and only device loss or a crash touches a
//! whole session. Crash attribution marks an asset or effect failed after two
//! crashes attributed to it (section 19).

use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::common::{
    AssetId, DefinitionId, EffectInstanceId, JobId, LayerId, RequestId, Revision, SessionId,
};
use crate::version::ContractId;

/// What failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    /// The bytes did not decode, or a value broke a contract invariant.
    InvalidMessage,
    /// Version negotiation failed or a message used an unnegotiated contract.
    UnsupportedVersion,
    /// The session does not exist (closed, or the engine restarted).
    UnknownSession,
    /// The request needs a revision the engine has not applied and the wait
    /// bound passed, or a patch batch did not follow the applied revision.
    StaleRevision,
    /// A capability handle was unknown, expired, or bound to another session or port.
    CapabilityRejected,
    /// A definition has no snapshot and is not already marked missing.
    DefinitionMissing,
    /// GLSL failed to compile or translate; diagnostics carry user line numbers.
    ShaderCompile,
    /// An effect or shader exceeded its GPU time budget and was disabled.
    WatchdogTimeout,
    /// A source could not be probed or decoded.
    DecodeFailed,
    /// A source's file is missing.
    MediaOffline,
    /// A lease could not cover the work.
    BudgetExceeded,
    /// The GPU device was lost; it is being rebuilt.
    DeviceLost,
    /// Blender is not installed or is older than 4.5.
    BlenderMissing,
    /// A Blender job failed, timed out, or a required surface is missing.
    BlenderFailed,
    /// The project needs the renderer (a `raster` override or a shader that
    /// fell back to the GLSL runner) and the host has none.
    NotHeadlessRenderable,
    /// Encoding, muxing, or verification of an output failed.
    ExportFailed,
    /// The request was cancelled.
    Cancelled,
    /// The request's deadline passed.
    Timeout,
    /// A feature the engine does not support yet (capability routing sends the
    /// work to the legacy engine while it exists).
    Unsupported,
    /// Repeated crashes were attributed to this item; it stays failed for the session.
    CrashAttributed,
    /// A bug. Always reported.
    Internal,
}

/// The thing an error is about; decides the isolation rule.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum ErrorScope {
    /// One request failed; nothing else is affected.
    Request { id: RequestId },
    /// A layer failed: it outputs transparency (section 8, isolation).
    Layer { layer: LayerId },
    /// An effect instance failed: it is skipped with its data kept.
    Effect {
        layer: LayerId,
        effect: EffectInstanceId,
        definition: DefinitionId,
    },
    /// A definition failed everywhere it is used (compile failures stay per definition).
    Definition { definition: DefinitionId },
    /// An asset failed to decode or is offline.
    Asset { asset: AssetId },
    /// A job failed; other jobs continue.
    Job { job: JobId },
    /// The whole session (device loss, crash). Renderers re-send snapshots.
    Session { session: SessionId },
    /// The connection itself (bad bytes, version mismatch).
    Connection,
}

/// What the engine does about an error in this scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum Isolation {
    FailRequest,
    TransparentLayer,
    SkipEffect,
    MarkDefinitionFailed,
    MarkAssetFailed,
    FailJob,
    RestartSession,
    CloseConnection,
}

impl ErrorScope {
    pub const fn isolation(&self) -> Isolation {
        match self {
            ErrorScope::Request { .. } => Isolation::FailRequest,
            ErrorScope::Layer { .. } => Isolation::TransparentLayer,
            ErrorScope::Effect { .. } => Isolation::SkipEffect,
            ErrorScope::Definition { .. } => Isolation::MarkDefinitionFailed,
            ErrorScope::Asset { .. } => Isolation::MarkAssetFailed,
            ErrorScope::Job { .. } => Isolation::FailJob,
            ErrorScope::Session { .. } => Isolation::RestartSession,
            ErrorScope::Connection => Isolation::CloseConnection,
        }
    }
}

/// Whether the same request can succeed later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum Retry {
    Never,
    /// Retry after this many milliseconds.
    After {
        ms: u32,
    },
    /// Retry after a canonical resync of the document.
    AfterResync,
    /// Retry after the user acts (install Blender, locate media).
    AfterUserAction,
}

/// A user-facing line number in a shader diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ShaderDiagnostic {
    /// 1-based line in the user's source (wrapping offsets removed).
    pub line: u32,
    pub column: Option<u32>,
    pub message: String,
}

/// Structured details for codes that need them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum ErrorDetail {
    ShaderCompile {
        diagnostics: Vec<ShaderDiagnostic>,
    },
    StaleRevision {
        applied: Revision,
        wanted: Revision,
    },
    UnsupportedVersion {
        contract: Option<ContractId>,
        local: Vec<u16>,
        remote: Vec<u16>,
    },
    MissingSurfaces {
        count: u32,
    },
    Offline {
        assets: Vec<AssetId>,
    },
}

/// An error as it crosses the protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EngineError {
    pub code: ErrorCode,
    pub scope: ErrorScope,
    /// One sentence saying what failed and what to do. No paths the engine
    /// was not given, no secrets.
    pub message: String,
    pub retry: Retry,
    pub detail: Option<ErrorDetail>,
}

impl EngineError {
    pub fn new(code: ErrorCode, scope: ErrorScope, message: impl Into<String>) -> Self {
        Self {
            code,
            scope,
            message: message.into(),
            retry: Retry::Never,
            detail: None,
        }
    }

    pub fn with_retry(mut self, retry: Retry) -> Self {
        self.retry = retry;
        self
    }

    pub fn with_detail(mut self, detail: ErrorDetail) -> Self {
        self.detail = Some(detail);
        self
    }
}

/// Longest error message, in UTF-8 bytes.
pub const MAX_ERROR_MESSAGE_BYTES: usize = 4096;

impl crate::validate::Validate for EngineError {
    fn validate(&self) -> Result<(), crate::validate::Violation> {
        crate::validate::ensure(
            !self.message.is_empty() && self.message.len() <= MAX_ERROR_MESSAGE_BYTES,
            ContractId::Errors,
            || format!("error messages are 1 to {MAX_ERROR_MESSAGE_BYTES} bytes"),
        )
    }
}

impl core::fmt::Display for EngineError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for EngineError {}

/// An isolated failure reported as an event while work continues (a layer
/// renders transparent, an effect is skipped). Filled into
/// `uiState.getShaderMeta` and the status bar at the frame's revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FailureReport {
    pub revision: Revision,
    pub error: EngineError,
}

/// Which engine process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum EngineProcess {
    Interactive,
    Job,
}

/// Sent by the host after an engine process exits unexpectedly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CrashReport {
    pub process: EngineProcess,
    /// Consecutive crashes without a healthy interval.
    pub attempt: u32,
    /// Exit code or signal, as text for the log.
    pub exit: String,
    /// Items in flight when it crashed (from the engine's last breadcrumb).
    pub suspects: Vec<ErrorScope>,
    /// Items now marked failed for the session (after two attributed crashes).
    pub marked_failed: Vec<ErrorScope>,
    /// When the restart happens.
    pub restart_in_ms: u32,
}

/// Crashes attributed to one item before it is marked failed (section 19).
pub const CRASHES_BEFORE_MARKED_FAILED: u32 = 2;

/// Restart backoff: 250 ms doubling per attempt, capped at 30 s.
pub fn restart_backoff_ms(attempt: u32) -> u32 {
    const BASE_MS: u32 = 250;
    const CAP_MS: u32 = 30_000;
    let shift = attempt.saturating_sub(1).min(16);
    BASE_MS.saturating_mul(1 << shift).min(CAP_MS)
}

/// Host-side crash attribution: counts crashes per suspect and reports the
/// items that cross [`CRASHES_BEFORE_MARKED_FAILED`].
#[derive(Debug, Default, Clone)]
pub struct CrashLedger {
    counts: Vec<(ErrorScope, u32)>,
}

impl CrashLedger {
    /// Records one crash with its suspects and returns the scopes that became
    /// failed with this crash. Only asset, effect, definition, and layer scopes
    /// are attributable; others are ignored.
    pub fn record(&mut self, suspects: &[ErrorScope]) -> Vec<ErrorScope> {
        let mut newly_failed = Vec::new();
        for suspect in suspects {
            if !matches!(
                suspect,
                ErrorScope::Asset { .. }
                    | ErrorScope::Effect { .. }
                    | ErrorScope::Definition { .. }
                    | ErrorScope::Layer { .. }
            ) {
                continue;
            }
            let count = match self.counts.iter_mut().find(|(scope, _)| scope == suspect) {
                Some((_, count)) => {
                    *count += 1;
                    *count
                }
                None => {
                    self.counts.push((suspect.clone(), 1));
                    1
                }
            };
            if count == CRASHES_BEFORE_MARKED_FAILED {
                newly_failed.push(suspect.clone());
            }
        }
        newly_failed
    }

    pub fn is_failed(&self, scope: &ErrorScope) -> bool {
        self.counts
            .iter()
            .any(|(s, count)| s == scope && *count >= CRASHES_BEFORE_MARKED_FAILED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_map_to_isolation_rules() {
        assert_eq!(
            ErrorScope::Layer { layer: "l".into() }.isolation(),
            Isolation::TransparentLayer
        );
        let effect = ErrorScope::Effect {
            layer: "l".into(),
            effect: "e".into(),
            definition: "d".into(),
        };
        assert_eq!(effect.isolation(), Isolation::SkipEffect);
        assert_eq!(
            ErrorScope::Connection.isolation(),
            Isolation::CloseConnection
        );
    }

    #[test]
    fn two_attributed_crashes_mark_an_item_failed() {
        let asset = ErrorScope::Asset { asset: "a1".into() };
        let session = ErrorScope::Session {
            session: crate::common::SessionId::ZERO,
        };
        let mut ledger = CrashLedger::default();
        assert!(ledger.record(&[asset.clone(), session.clone()]).is_empty());
        assert!(!ledger.is_failed(&asset));
        assert_eq!(
            ledger.record(std::slice::from_ref(&asset)),
            vec![asset.clone()]
        );
        assert!(ledger.is_failed(&asset));
        // Reported once, not on every later crash.
        assert!(ledger.record(std::slice::from_ref(&asset)).is_empty());
        assert!(!ledger.is_failed(&session));
    }

    #[test]
    fn backoff_doubles_and_caps() {
        assert_eq!(restart_backoff_ms(0), 250);
        assert_eq!(restart_backoff_ms(1), 250);
        assert_eq!(restart_backoff_ms(2), 500);
        assert_eq!(restart_backoff_ms(3), 1000);
        assert_eq!(restart_backoff_ms(8), 30_000);
        assert_eq!(restart_backoff_ms(u32::MAX), 30_000);
    }
}
