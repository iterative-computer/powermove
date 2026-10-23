//! Host services (contract 14): the arbiter, capability handle minting,
//! output destinations, Blender supervision, and the definition-snapshot
//! source. Implemented by Electron main (TypeScript, through these messages)
//! and by the CLI host (`pm-host`, through [`HostServices`]).
//!
//! The engine accepts no raw paths. Every file it reads or writes is named by
//! a capability handle that the host minted, bound to one session and one
//! port, and rejects anywhere else (section 19, security).

use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::blender::{BlenderJobRequest, BlenderStatus};
use crate::common::{AssetId, ContentHash, JobId, SessionId};
use crate::definitions::DefinitionSetSnapshot;
use crate::error::EngineError;
use crate::lease::{Lease, LeaseRequest};
use crate::validate::{Validate, Violation, ensure};
use crate::version::ContractId;

/// An unguessable 128-bit capability, written as 32 lowercase hex digits.
/// Only the host mints handles, from a cryptographic RNG.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(try_from = "String", into = "String")]
#[cfg_attr(feature = "ts", ts(type = "string"))]
pub struct CapabilityHandle(String);

/// A string that is not 32 lowercase hex digits.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("a capability handle is 32 lowercase hex digits")]
pub struct MalformedHandle;

impl CapabilityHandle {
    /// Builds a handle from 16 random bytes supplied by the host's RNG.
    pub fn from_random_bytes(bytes: [u8; 16]) -> Self {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut text = String::with_capacity(32);
        for byte in bytes {
            text.push(HEX[(byte >> 4) as usize] as char);
            text.push(HEX[(byte & 0xf) as usize] as char);
        }
        Self(text)
    }

    pub fn parse(text: &str) -> Result<Self, MalformedHandle> {
        if text.len() == 32
            && text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            Ok(Self(text.to_owned()))
        } else {
            Err(MalformedHandle)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for CapabilityHandle {
    type Error = MalformedHandle;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<CapabilityHandle> for String {
    fn from(value: CapabilityHandle) -> String {
        value.0
    }
}

/// Identifies one MessagePort (or pipe) between the host and an engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PortId(pub u32);

/// Derived caches the engine may read or write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum CacheKind {
    MediaIndex,
    Thumbnails,
    GeneratedMedia,
    RenderCache,
    BlenderSurfaces,
    ShaderCache,
    FontSubsets,
    LargeStillMips,
    Scratch,
}

/// What a handle grants.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum CapabilityKind {
    /// Read an asset's bytes (original, linked file, or media-store copy).
    MediaRead { asset: AssetId },
    /// Read the source monitor's browser-visible rendition, or the original
    /// when no separate rendition exists.
    SourceRead { asset: AssetId },
    /// Read and write one cache area.
    Cache { cache: CacheKind },
    /// Write one job output (a temporary file in the destination folder).
    OutputWrite { job: JobId },
    /// Read a project file or profile store entry (document host persistence).
    ProjectStore,
    /// Read an extension bundle (document host extension runtime).
    ExtensionBundle,
}

/// Ask the host for a handle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MintHandle {
    pub kind: CapabilityKind,
    pub session: SessionId,
}

/// A minted handle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct HandleGrant {
    pub handle: CapabilityHandle,
    pub kind: CapabilityKind,
    /// For media reads: the content hash the host verified, when known.
    pub content_hash: Option<ContentHash>,
}

/// How an engine may open the file a handle names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum HandleAccess {
    /// Media reads, project store, extension bundles.
    Read,
    /// Cache areas.
    ReadWrite,
    /// One job's temporary output (created by the host, written by the engine).
    Write,
}

impl CapabilityKind {
    /// The access a handle of this kind grants.
    pub const fn access(&self) -> HandleAccess {
        match self {
            CapabilityKind::MediaRead { .. }
            | CapabilityKind::SourceRead { .. }
            | CapabilityKind::ProjectStore
            | CapabilityKind::ExtensionBundle => HandleAccess::Read,
            CapabilityKind::Cache { .. } => HandleAccess::ReadWrite,
            CapabilityKind::OutputWrite { .. } => HandleAccess::Write,
        }
    }
}

/// Engine to host, on the engine's host port only: what file does this
/// handle name? Renderers pass handles, never paths; the engine resolves
/// each one with the host before it opens anything. The host checks the
/// binding against the session and the port the request arrived on, so a
/// handle minted for another engine launch or session is refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ResolveHandle {
    pub handle: CapabilityHandle,
    pub session: SessionId,
}

/// The host's answer: the absolute path a handle names and how it may be
/// opened. Only the host sends it, only on an engine's host port, and only
/// for a handle bound to that port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct HandleResolved {
    pub handle: CapabilityHandle,
    pub kind: CapabilityKind,
    pub access: HandleAccess,
    /// Absolute, at most 4096 bytes, no NUL. A directory for cache areas.
    pub path: String,
}

/// Longest path a [`HandleResolved`] may carry (macOS `PATH_MAX`).
pub const MAX_RESOLVED_PATH_BYTES: usize = 4096;

/// What happens when an output's name is taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum OverwritePolicy {
    /// Pick a unique name (agents always).
    UniqueName,
    /// Replace the existing file after verification (user choice).
    Replace,
    /// Fail.
    Fail,
}

/// Who chooses where an output goes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum OutputChoice {
    /// MCP and agent tools: the host's automatic destination, unique name,
    /// no dialog, owner-only file created exclusively (`open(wx, 0600)`).
    /// Agents never choose output paths.
    AgentAutomatic {
        suggested_name: String,
        extension: String,
    },
    /// The export sheet or the CLI: a destination the user chose, already
    /// turned into a handle by the host.
    UserChosen { destination: CapabilityHandle },
}

/// Engine (job) to host: where should this job's output go?
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OutputDestinationRequest {
    pub job: JobId,
    pub choice: OutputChoice,
    pub overwrite: OverwritePolicy,
    /// The output is a folder of files (PNG sequences): the host creates an
    /// empty owner-only temporary folder instead of a file.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(as = "Option<bool>", optional))]
    pub folder: bool,
}

/// The host's answer: a handle to a temporary file in the destination folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OutputDestination {
    pub job: JobId,
    pub temporary: CapabilityHandle,
}

/// Facts the engine verified about an output before it is moved into place
/// (section 17, verification).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OutputVerification {
    pub codec: String,
    pub frames: u32,
    /// Total audio samples per channel after edit-list trimming; equals
    /// round(duration × sample rate) (section 2, audio bar).
    pub audio_samples: crate::common::Count,
    pub bytes: crate::common::ByteSize,
}

/// Engine to host: the output is encoded and verified; move it into place.
/// Empty or unverified files are removed instead (`commit: false`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CommitOutput {
    pub job: JobId,
    pub temporary: CapabilityHandle,
    pub commit: bool,
    pub verification: Option<OutputVerification>,
}

/// The host's answer: the real path, returned to the agent after encoding
/// and verification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OutputCommitted {
    pub job: JobId,
    pub path: Option<String>,
}

/// Hardware codec sessions arbitrated by main (section 13, decode service).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum HardwareSessionKind {
    Decode,
    Encode,
    ProResDecode,
    ProResEncode,
}

/// Ask for, or give back, hardware codec sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct HardwareSessionRequest {
    pub kind: HardwareSessionKind,
    /// Positive to acquire, negative to release.
    pub delta: i16,
}

/// The host's grant: sessions now held by the requester.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct HardwareSessionGrant {
    pub kind: HardwareSessionKind,
    pub held: u16,
}

/// Host services as a Rust interface, implemented by `pm-host` for the CLI.
/// Electron main implements the same operations over the protocol messages
/// that carry these types.
pub trait HostServices {
    type Error: std::error::Error;

    fn mint_handle(
        &mut self,
        request: &MintHandle,
        port: PortId,
    ) -> Result<HandleGrant, Self::Error>;
    /// Checks a handle presented on `port` by `session`; never trusts the engine's word.
    fn check_handle(
        &self,
        handle: &CapabilityHandle,
        session: SessionId,
        port: PortId,
    ) -> Result<CapabilityKind, Self::Error>;
    /// Resolves a handle presented on `port` to the file it names.
    fn resolve_handle(
        &self,
        request: &ResolveHandle,
        port: PortId,
    ) -> Result<HandleResolved, Self::Error>;
    fn revoke_session(&mut self, session: SessionId);
    fn output_destination(
        &mut self,
        request: &OutputDestinationRequest,
    ) -> Result<OutputDestination, Self::Error>;
    fn commit_output(&mut self, request: &CommitOutput) -> Result<OutputCommitted, Self::Error>;
    fn request_lease(&mut self, request: &LeaseRequest) -> Result<Lease, Self::Error>;
    fn hardware_sessions(
        &mut self,
        request: &HardwareSessionRequest,
    ) -> Result<HardwareSessionGrant, Self::Error>;
    fn blender_status(&self) -> BlenderStatus;
    fn submit_blender_job(&mut self, request: &BlenderJobRequest) -> Result<(), Self::Error>;
    /// The current winning definitions, for jobs and the CLI.
    fn definition_snapshots(&self) -> DefinitionSetSnapshot;
}

/// Why a handle was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HandleRejection {
    #[error("unknown capability handle")]
    Unknown,
    #[error("capability handle bound to another session")]
    WrongSession,
    #[error("capability handle bound to another port")]
    WrongPort,
}

impl From<HandleRejection> for EngineError {
    fn from(rejection: HandleRejection) -> Self {
        EngineError::new(
            crate::error::ErrorCode::CapabilityRejected,
            crate::error::ErrorScope::Connection,
            rejection.to_string(),
        )
    }
}

/// The binding table every host keeps: handle → (kind, session, port).
#[derive(Debug, Default, Clone)]
pub struct HandleTable {
    entries: std::collections::BTreeMap<CapabilityHandle, (CapabilityKind, SessionId, PortId)>,
}

impl HandleTable {
    /// Binds a freshly minted handle. Returns false if it already exists
    /// (a duplicate means the RNG is broken; the caller must not use it).
    pub fn bind(
        &mut self,
        handle: CapabilityHandle,
        kind: CapabilityKind,
        session: SessionId,
        port: PortId,
    ) -> bool {
        if self.entries.contains_key(&handle) {
            return false;
        }
        self.entries.insert(handle, (kind, session, port));
        true
    }

    pub fn check(
        &self,
        handle: &CapabilityHandle,
        session: SessionId,
        port: PortId,
    ) -> Result<&CapabilityKind, HandleRejection> {
        let (kind, bound_session, bound_port) =
            self.entries.get(handle).ok_or(HandleRejection::Unknown)?;
        if *bound_session != session {
            return Err(HandleRejection::WrongSession);
        }
        if *bound_port != port {
            return Err(HandleRejection::WrongPort);
        }
        Ok(kind)
    }

    /// Drops one handle (an output committed or removed).
    pub fn revoke(&mut self, handle: &CapabilityHandle) -> bool {
        self.entries.remove(handle).is_some()
    }

    /// Drops every handle bound to a port (the engine launch ended).
    pub fn revoke_port(&mut self, port: PortId) -> usize {
        let before = self.entries.len();
        self.entries.retain(|_, (_, _, p)| *p != port);
        before - self.entries.len()
    }

    /// Drops every handle of a session on a port (the session closed).
    pub fn revoke_session_on(&mut self, session: SessionId, port: PortId) -> usize {
        let before = self.entries.len();
        self.entries
            .retain(|_, (_, s, p)| !(*s == session && *p == port));
        before - self.entries.len()
    }

    /// Whether a handle is bound.
    pub fn contains(&self, handle: &CapabilityHandle) -> bool {
        self.entries.contains_key(handle)
    }

    /// Drops every handle of a session (session closed or engine restarted).
    pub fn revoke_session(&mut self, session: SessionId) -> usize {
        let before = self.entries.len();
        self.entries.retain(|_, (_, s, _)| *s != session);
        before - self.entries.len()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Validate for OutputDestinationRequest {
    fn validate(&self) -> Result<(), Violation> {
        const C: ContractId = ContractId::HostServices;
        ensure(self.job.is_valid(), C, || "invalid job id".into())?;
        if let OutputChoice::AgentAutomatic {
            suggested_name,
            extension,
        } = &self.choice
        {
            ensure(self.overwrite == OverwritePolicy::UniqueName, C, || {
                "agent outputs always get unique names".into()
            })?;
            let clean = |s: &str| {
                !s.is_empty()
                    && s.len() <= 255
                    && !s.contains(['/', '\\', '\0'])
                    && s != "."
                    && s != ".."
            };
            ensure(clean(suggested_name), C, || {
                "suggested name must be a bare file name".into()
            })?;
            ensure(
                clean(extension) && extension.bytes().all(|b| b.is_ascii_alphanumeric()),
                C,
                || "extension must be alphanumeric".into(),
            )?;
        }
        Ok(())
    }
}

impl Validate for HandleResolved {
    fn validate(&self) -> Result<(), Violation> {
        const C: ContractId = ContractId::HostServices;
        ensure(self.access == self.kind.access(), C, || {
            "the access must be the one the handle's kind grants".into()
        })?;
        ensure(
            self.path.starts_with('/')
                && self.path.len() <= MAX_RESOLVED_PATH_BYTES
                && !self.path.contains('\0'),
            C,
            || {
                format!(
                    "a resolved path is absolute, at most {MAX_RESOLVED_PATH_BYTES} bytes, without NUL"
                )
            },
        )
    }
}

impl Validate for HardwareSessionRequest {
    fn validate(&self) -> Result<(), Violation> {
        ensure(self.delta != 0, ContractId::HostServices, || {
            "a session request changes the count".into()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_are_32_lowercase_hex() {
        let handle = CapabilityHandle::from_random_bytes([0xab; 16]);
        assert_eq!(handle.as_str(), "abababababababababababababababab");
        assert!(CapabilityHandle::parse(handle.as_str()).is_ok());
        assert!(CapabilityHandle::parse("ABABABABABABABABABABABABABABABAB").is_err());
        assert!(CapabilityHandle::parse("abc").is_err());
        assert!(CapabilityHandle::parse("/Users/someone/movie.mov").is_err());
        let bytes = rmp_serde::to_vec(&"zz".repeat(16)).unwrap();
        assert!(rmp_serde::from_slice::<CapabilityHandle>(&bytes).is_err());
    }

    #[test]
    fn handles_are_rejected_on_another_session_or_port() {
        let mut table = HandleTable::default();
        let handle = CapabilityHandle::from_random_bytes([7; 16]);
        let session = SessionId::new(1).unwrap();
        let kind = CapabilityKind::MediaRead { asset: "a".into() };
        assert!(table.bind(handle.clone(), kind.clone(), session, PortId(3)));
        assert!(!table.bind(handle.clone(), kind.clone(), session, PortId(3)));
        assert_eq!(table.check(&handle, session, PortId(3)), Ok(&kind));
        assert_eq!(
            table.check(&handle, SessionId::new(2).unwrap(), PortId(3)),
            Err(HandleRejection::WrongSession)
        );
        assert_eq!(
            table.check(&handle, session, PortId(4)),
            Err(HandleRejection::WrongPort)
        );
        let other = CapabilityHandle::from_random_bytes([8; 16]);
        assert_eq!(
            table.check(&other, session, PortId(3)),
            Err(HandleRejection::Unknown)
        );
        let second = CapabilityHandle::from_random_bytes([9; 16]);
        assert!(table.bind(second.clone(), kind.clone(), session, PortId(5)));
        assert_eq!(table.revoke_port(PortId(5)), 1);
        assert!(!table.contains(&second));
        assert_eq!(table.revoke_session_on(session, PortId(4)), 0);
        assert_eq!(table.revoke_session(session), 1);
        assert_eq!(
            table.check(&handle, session, PortId(3)),
            Err(HandleRejection::Unknown)
        );
    }

    #[test]
    fn resolved_handles_carry_an_absolute_path_and_their_kind_s_access() {
        let resolved = HandleResolved {
            handle: CapabilityHandle::from_random_bytes([1; 16]),
            kind: CapabilityKind::MediaRead { asset: "a".into() },
            access: HandleAccess::Read,
            path: "/Users/someone/Movies/clip.mov".into(),
        };
        assert!(resolved.validate().is_ok());
        let writable = HandleResolved {
            access: HandleAccess::Write,
            ..resolved.clone()
        };
        assert!(writable.validate().is_err(), "media reads are read-only");
        let relative = HandleResolved {
            path: "Movies/clip.mov".into(),
            ..resolved.clone()
        };
        assert!(relative.validate().is_err());
        let nul = HandleResolved {
            path: "/tmp/a\0b".into(),
            ..resolved
        };
        assert!(nul.validate().is_err());
        assert_eq!(
            CapabilityKind::OutputWrite { job: "j".into() }.access(),
            HandleAccess::Write
        );
        assert_eq!(
            CapabilityKind::Cache {
                cache: CacheKind::Scratch
            }
            .access(),
            HandleAccess::ReadWrite
        );
    }

    #[test]
    fn agent_outputs_never_choose_paths() {
        let request = OutputDestinationRequest {
            job: "j".into(),
            choice: OutputChoice::AgentAutomatic {
                suggested_name: "../../etc/passwd".into(),
                extension: "mp4".into(),
            },
            overwrite: OverwritePolicy::UniqueName,
            folder: false,
        };
        assert!(request.validate().is_err());
        let request = OutputDestinationRequest {
            choice: OutputChoice::AgentAutomatic {
                suggested_name: "Launch cut".into(),
                extension: "mp4".into(),
            },
            overwrite: OverwritePolicy::Replace,
            ..request
        };
        assert!(request.validate().is_err(), "agents cannot overwrite");
        let request = OutputDestinationRequest {
            overwrite: OverwritePolicy::UniqueName,
            ..request
        };
        assert!(request.validate().is_ok());
    }
}
