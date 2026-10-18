//! Media facts for Interpret Footage (plan section 9 "Per-source
//! interpretation", appendix D): the host asks the job engine what a file
//! is and how it reads, with every value's origin, so the sheet can show
//! values with where they came from.
//!
//! The request names the file by a `mediaRead` capability handle the host
//! minted for the job engine's host port (contract 14: the engine accepts no
//! raw paths); the engine resolves it with `resolveHandle`.

use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::host::CapabilityHandle;
use crate::validate::{Validate, Violation, ensure};
use crate::version::ContractId;

/// Longest `interpret` member text a probe carries.
pub const MAX_INTERPRET_BYTES: usize = 16 * 1024;
/// Most fact lines and settings in an answer.
pub const MAX_FACT_LINES: usize = 64;
pub const MAX_SETTINGS: usize = 32;
/// Longest string in an answer.
pub const MAX_FACT_TEXT: usize = 1024;

/// Host → job engine: the facts of the file a handle names. Answer:
/// `mediaFacts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProbeMedia {
    /// A `mediaRead` handle bound to the job engine's host port and
    /// session 0 (no engine session).
    pub handle: CapabilityHandle,
    /// The asset's `interpret` member as JSON text, when it has one: the
    /// answer then shows the overridden values.
    pub interpret: Option<String>,
}

impl Validate for ProbeMedia {
    fn validate(&self) -> Result<(), Violation> {
        ensure(
            self.interpret
                .as_ref()
                .is_none_or(|t| t.len() <= MAX_INTERPRET_BYTES),
            ContractId::HostServices,
            || format!("an interpret member is at most {MAX_INTERPRET_BYTES} bytes"),
        )
    }
}

/// What kind of media a file holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum MediaKind {
    Video,
    Still,
    Audio,
}

/// Where a value of the reading came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum FactOrigin {
    /// Read from the file (`source` names the tag: "colr nclx", "pasp", ...).
    File { source: String },
    /// The file says nothing; the default rule that applied (section 9).
    Default { rule: String },
    /// Set in Interpret Footage.
    Override,
}

/// One read-only fact ("Codec", "ProRes 422 HQ").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactLine {
    pub label: String,
    pub value: String,
}

/// One setting Interpret Footage can override.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct InterpretSetting {
    /// The `interpret` member it maps to (`primaries`, `frameRate`, ...).
    pub member: String,
    /// The value in effect, in the member's stored form (`bt709`,
    /// `24000/1001`, `10:11`, `true`).
    pub value: String,
    /// The value without an override (what "Automatic" means).
    pub automatic: String,
    pub origin: FactOrigin,
}

/// The job engine's answer to [`ProbeMedia`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MediaFacts {
    pub kind: MediaKind,
    /// Read-only facts, in display order.
    pub facts: Vec<FactLine>,
    /// The settings Interpret Footage shows for this file, in display order.
    pub settings: Vec<InterpretSetting>,
    /// `interpret` members that cannot be used (they read as automatic).
    pub issues: Vec<String>,
    /// The size the picture shows at with this reading (pixel aspect, clean
    /// aperture, rotation): the asset's `w` and `h`.
    pub display_size: Option<[u32; 2]>,
    /// The program length in seconds with this reading (a conformed rate
    /// changes it): the asset's `dur`.
    pub duration: Option<f64>,
}

impl Validate for MediaFacts {
    fn validate(&self) -> Result<(), Violation> {
        const C: ContractId = ContractId::HostServices;
        ensure(self.facts.len() <= MAX_FACT_LINES, C, || {
            "too many fact lines".into()
        })?;
        ensure(self.settings.len() <= MAX_SETTINGS, C, || {
            "too many settings".into()
        })?;
        let texts = self
            .facts
            .iter()
            .flat_map(|f| [&f.label, &f.value])
            .chain(
                self.settings
                    .iter()
                    .flat_map(|s| [&s.member, &s.value, &s.automatic]),
            )
            .chain(&self.issues);
        for t in texts {
            ensure(t.len() <= MAX_FACT_TEXT, C, || {
                "a fact is at most 1024 bytes".into()
            })?;
        }
        ensure(
            self.duration.is_none_or(|d| d.is_finite() && d >= 0.0),
            C,
            || "a duration is finite and not negative".into(),
        )
    }
}
