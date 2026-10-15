//! Host services for the CLI (plan section 19, contract 14).
//!
//! Engines never touch the outside world directly; every host gives them
//! the same services. Electron main implements them in TypeScript
//! (`apps/desktop/src/main/engine/services.ts`); this crate is the
//! implementation `pm-cli` and, from Phase 3c, the hidden MCP host use:
//! capability handles bound to a session and a port, owner-only exclusive
//! output destinations with atomic commits, memory leases, hardware codec
//! sessions, and the definition-snapshot source.
//!
//! [`CliHost::serve`] answers an engine's host request exactly as main
//! does; the shared contract sequence in `tests/fixtures/host-contract.json`
//! runs against both (contract 14 invariant 9).

// Errors are protocol `EngineError`s: they go back to the engine as is.
#![allow(clippy::result_large_err)]

pub mod arbiter;
pub mod outputs;

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use pm_protocol::Validate;
use pm_protocol::blender::{BlenderJobRequest, BlenderStatus};
use pm_protocol::common::{ContentHash, SessionId};
use pm_protocol::definitions::{DefinitionSet, DefinitionSetSnapshot, DefinitionsUpdate};
use pm_protocol::envelope::{Request, Response};
use pm_protocol::error::{EngineError, EngineProcess, ErrorCode, ErrorScope};
use pm_protocol::host::{
    CacheKind, CapabilityHandle, CapabilityKind, CommitOutput, HandleGrant, HandleRejection,
    HandleResolved, HandleTable, HardwareSessionGrant, HardwareSessionRequest, HostServices,
    MintHandle, OutputChoice, OutputCommitted, OutputDestination, OutputDestinationRequest, PortId,
    ResolveHandle,
};
use pm_protocol::lease::{Lease, LeaseRequest};

use arbiter::{HardwareArbiter, MemoryArbiter, global_budget};
use outputs::{Placement, create_temporary, place};

/// The directory name of each cache area (same as main's).
pub fn cache_dir_name(kind: CacheKind) -> &'static str {
    match kind {
        CacheKind::MediaIndex => "Media Index",
        CacheKind::Thumbnails => "Thumbnails",
        CacheKind::GeneratedMedia => "Generated Media",
        CacheKind::RenderCache => "Render Cache",
        CacheKind::BlenderSurfaces => "Blender Surfaces",
        CacheKind::ShaderCache => "Shader Cache",
        CacheKind::FontSubsets => "Font Subsets",
        CacheKind::LargeStillMips => "Large Still Mips",
        CacheKind::Scratch => "Scratch",
    }
}

/// 16 bytes from the OS CSPRNG.
pub fn os_random() -> std::io::Result<[u8; 16]> {
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn error(code: ErrorCode, message: impl Into<String>) -> EngineError {
    EngineError::new(code, ErrorScope::Connection, message)
}

/// Resolves an asset to a readable file and its verified content hash.
pub type AssetResolver = Box<dyn Fn(&str) -> Option<(PathBuf, Option<ContentHash>)> + Send>;

pub struct CliHostConfig {
    pub cache_directory: PathBuf,
    /// Where agent outputs go (`~/Powermove` on the desktop).
    pub automatic_output_directory: PathBuf,
    pub physical_memory: u64,
}

struct PendingOutput {
    job: String,
    session: SessionId,
    port: PortId,
    temporary: PathBuf,
    placement: Placement,
}

/// The CLI host.
pub struct CliHost {
    config: CliHostConfig,
    table: HandleTable,
    paths: HashMap<CapabilityHandle, PathBuf>,
    pending: HashMap<CapabilityHandle, PendingOutput>,
    memory: MemoryArbiter,
    hardware: HardwareArbiter,
    definitions: DefinitionSet,
    resolve_asset: Option<AssetResolver>,
    random: Box<dyn FnMut() -> std::io::Result<[u8; 16]> + Send>,
}

impl CliHost {
    pub fn new(config: CliHostConfig) -> Self {
        let memory = MemoryArbiter::new(global_budget(config.physical_memory));
        Self {
            config,
            table: HandleTable::default(),
            paths: HashMap::new(),
            pending: HashMap::new(),
            memory,
            hardware: HardwareArbiter::default(),
            definitions: DefinitionSet::default(),
            resolve_asset: None,
            random: Box::new(os_random),
        }
    }

    pub fn set_asset_resolver(&mut self, resolver: AssetResolver) {
        self.resolve_asset = Some(resolver);
    }

    /// Replaces the random source (tests).
    pub fn set_random(&mut self, random: Box<dyn FnMut() -> std::io::Result<[u8; 16]> + Send>) {
        self.random = random;
    }

    pub fn memory(&self) -> &MemoryArbiter {
        &self.memory
    }

    pub fn memory_mut(&mut self) -> &mut MemoryArbiter {
        &mut self.memory
    }

    pub fn hardware(&self) -> &HardwareArbiter {
        &self.hardware
    }

    pub fn handle_count(&self) -> usize {
        self.table.len()
    }

    pub fn pending_outputs(&self) -> usize {
        self.pending.len()
    }

    /// The kernel's definition updates (the source for jobs and pm-cli).
    pub fn update_definitions(&mut self, update: &DefinitionsUpdate) -> Result<(), EngineError> {
        self.definitions
            .apply(update)
            .map_err(|e| error(ErrorCode::InvalidMessage, e.to_string()))
    }

    /// Mints and binds a handle naming `path`.
    pub fn mint(
        &mut self,
        kind: CapabilityKind,
        session: SessionId,
        port: PortId,
        path: PathBuf,
    ) -> Result<CapabilityHandle, EngineError> {
        if !path.is_absolute()
            || path.as_os_str().len() > pm_protocol::host::MAX_RESOLVED_PATH_BYTES
        {
            return Err(error(
                ErrorCode::Internal,
                "A handle names an absolute path of at most 4096 bytes.",
            ));
        }
        let bytes = (self.random)()
            .map_err(|e| error(ErrorCode::Internal, format!("no random source ({e})")))?;
        let handle = CapabilityHandle::from_random_bytes(bytes);
        if !self.table.bind(handle.clone(), kind, session, port) {
            return Err(error(
                ErrorCode::Internal,
                "The random source repeated a capability handle.",
            ));
        }
        self.paths.insert(handle.clone(), path);
        Ok(handle)
    }

    /// The export sheet or the command line: a chosen destination file.
    pub fn choose_destination(
        &mut self,
        file: PathBuf,
        job: &str,
        session: SessionId,
        port: PortId,
    ) -> Result<CapabilityHandle, EngineError> {
        self.mint(
            CapabilityKind::OutputWrite { job: job.into() },
            session,
            port,
            file,
        )
    }

    fn rejected(rejection: HandleRejection) -> EngineError {
        EngineError::from(rejection)
    }

    fn forget(&mut self, handle: &CapabilityHandle) {
        self.paths.remove(handle);
        self.table.revoke(handle);
    }

    fn path_for(
        &self,
        kind: &CapabilityKind,
    ) -> Result<(PathBuf, Option<ContentHash>), EngineError> {
        match kind {
            CapabilityKind::Cache { cache } => {
                let dir = self.config.cache_directory.join(cache_dir_name(*cache));
                std::fs::create_dir_all(&dir).map_err(|e| {
                    error(
                        ErrorCode::Internal,
                        format!("The cache area could not be created ({}).", e.kind()),
                    )
                })?;
                Ok((dir, None))
            }
            CapabilityKind::MediaRead { asset } => {
                if !asset.is_valid() {
                    return Err(error(ErrorCode::InvalidMessage, "invalid asset id"));
                }
                self.resolve_asset
                    .as_ref()
                    .and_then(|resolve| resolve(asset.as_str()))
                    .ok_or_else(|| {
                        error(
                            ErrorCode::MediaOffline,
                            "The asset's file is not available.",
                        )
                    })
            }
            CapabilityKind::OutputWrite { .. } => Err(error(
                ErrorCode::CapabilityRejected,
                "Output handles come from outputDestination.",
            )),
            CapabilityKind::ProjectStore | CapabilityKind::ExtensionBundle => Err(error(
                ErrorCode::Unsupported,
                "Document host handles are served by the document host (Phase 3c).",
            )),
        }
    }

    /// Serves one engine → host request from `process` on `port`, as main
    /// does. `session` is the envelope's session.
    pub fn serve(
        &mut self,
        process: EngineProcess,
        port: PortId,
        request: &Request,
        session: Option<SessionId>,
    ) -> Result<Response, EngineError> {
        let mut bound = Bound {
            host: self,
            process,
            port,
            session: session.unwrap_or(SessionId::ZERO),
        };
        match request {
            Request::MintHandle(mint) => bound.mint_handle(mint, port).map(Response::Handle),
            Request::ResolveHandle(resolve) => bound
                .resolve_handle(resolve, port)
                .map(Response::HandleResolved),
            Request::OutputDestination(destination) => bound
                .output_destination(destination)
                .map(Response::OutputDestination),
            Request::CommitOutput(commit) => {
                bound.commit_output(commit).map(Response::OutputCommitted)
            }
            Request::RequestLease(lease) => bound.request_lease(lease).map(Response::Lease),
            Request::HardwareSessions(hw) => {
                bound.hardware_sessions(hw).map(Response::HardwareSessions)
            }
            Request::BlenderStatus | Request::BlenderRender(_) | Request::BlenderModel(_) => {
                Err(error(
                    ErrorCode::Unsupported,
                    "Blender jobs from the new engine are not connected yet; the current engine runs Blender.",
                ))
            }
            other => Err(error(
                ErrorCode::InvalidMessage,
                format!("Engines do not send {:?} to the host.", other.expects()),
            )),
        }
    }

    /// An engine launch ended: its handles go, its unfinished outputs are
    /// removed, its codec sessions are released. Returns removed outputs.
    pub fn end_launch(&mut self, process: EngineProcess, port: PortId) -> usize {
        let orphaned: Vec<CapabilityHandle> = self
            .pending
            .iter()
            .filter(|(_, p)| p.port == port)
            .map(|(h, _)| h.clone())
            .collect();
        for handle in &orphaned {
            if let Some(pending) = self.pending.remove(handle) {
                outputs::remove_temporary(&pending.temporary);
            }
        }
        self.table.revoke_port(port);
        self.paths.retain(|handle, _| self.table.contains(handle));
        self.hardware.release_all(process);
        orphaned.len()
    }

    fn check(
        &self,
        handle: &CapabilityHandle,
        session: SessionId,
        port: PortId,
    ) -> Result<&CapabilityKind, EngineError> {
        self.table
            .check(handle, session, port)
            .map_err(Self::rejected)
    }
}

/// The host services seen by one engine connection: the `HostServices`
/// trait with the process, port, and envelope session filled in.
pub struct Bound<'a> {
    pub host: &'a mut CliHost,
    pub process: EngineProcess,
    pub port: PortId,
    pub session: SessionId,
}

impl HostServices for Bound<'_> {
    type Error = EngineError;

    fn mint_handle(
        &mut self,
        request: &MintHandle,
        port: PortId,
    ) -> Result<HandleGrant, EngineError> {
        let (path, content_hash) = self.host.path_for(&request.kind)?;
        let handle = self
            .host
            .mint(request.kind.clone(), request.session, port, path)?;
        Ok(HandleGrant {
            handle,
            kind: request.kind.clone(),
            content_hash,
        })
    }

    fn check_handle(
        &self,
        handle: &CapabilityHandle,
        session: SessionId,
        port: PortId,
    ) -> Result<CapabilityKind, EngineError> {
        self.host.check(handle, session, port).cloned()
    }

    fn resolve_handle(
        &self,
        request: &ResolveHandle,
        port: PortId,
    ) -> Result<HandleResolved, EngineError> {
        let kind = self
            .host
            .check(&request.handle, request.session, port)?
            .clone();
        let path = self
            .host
            .paths
            .get(&request.handle)
            .ok_or_else(|| Self::Error::from(HandleRejection::Unknown))?;
        let resolved = HandleResolved {
            handle: request.handle.clone(),
            access: kind.access(),
            kind,
            path: path.to_string_lossy().into_owned(),
        };
        resolved
            .validate()
            .map_err(|v| error(ErrorCode::Internal, v.to_string()))?;
        Ok(resolved)
    }

    fn revoke_session(&mut self, session: SessionId) {
        self.host.table.revoke_session_on(session, self.port);
        let table = &self.host.table;
        self.host.paths.retain(|handle, _| table.contains(handle));
    }

    fn output_destination(
        &mut self,
        request: &OutputDestinationRequest,
    ) -> Result<OutputDestination, EngineError> {
        if self.process != EngineProcess::Job {
            return Err(error(
                ErrorCode::CapabilityRejected,
                "Only the job engine writes outputs.",
            ));
        }
        request
            .validate()
            .map_err(|v| error(ErrorCode::InvalidMessage, v.message))?;
        let chosen = match &request.choice {
            OutputChoice::UserChosen { destination } => {
                let kind = self.host.check(destination, self.session, self.port)?;
                if !matches!(kind, CapabilityKind::OutputWrite { job } if *job == request.job) {
                    return Err(error(
                        ErrorCode::CapabilityRejected,
                        "The destination handle is not this job's output.",
                    ));
                }
                self.host.paths.get(destination).cloned()
            }
            OutputChoice::AgentAutomatic { .. } => None,
        };
        let placement = Placement::for_request(
            request,
            &self.host.config.automatic_output_directory,
            chosen.as_deref(),
        )
        .ok_or_else(|| error(ErrorCode::InvalidMessage, "The destination is not a file."))?;
        let mut temporary = None;
        for _ in 0..8 {
            let bytes = (self.host.random)()
                .map_err(|e| error(ErrorCode::Internal, format!("no random source ({e})")))?;
            let hex: String = bytes[..8].iter().map(|b| format!("{b:02x}")).collect();
            match create_temporary(&placement.directory, &hex) {
                Ok(path) => {
                    temporary = Some(path);
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => {
                    return Err(error(
                        ErrorCode::ExportFailed,
                        format!("The output could not be created ({}).", e.kind()),
                    ));
                }
            }
        }
        let temporary =
            temporary.ok_or_else(|| error(ErrorCode::ExportFailed, "No free temporary name."))?;
        let handle = self.host.mint(
            CapabilityKind::OutputWrite {
                job: request.job.clone(),
            },
            self.session,
            self.port,
            temporary.clone(),
        )?;
        self.host.pending.insert(
            handle.clone(),
            PendingOutput {
                job: request.job.as_str().to_owned(),
                session: self.session,
                port: self.port,
                temporary,
                placement,
            },
        );
        Ok(OutputDestination {
            job: request.job.clone(),
            temporary: handle,
        })
    }

    fn commit_output(&mut self, request: &CommitOutput) -> Result<OutputCommitted, EngineError> {
        if self.process != EngineProcess::Job {
            return Err(error(
                ErrorCode::CapabilityRejected,
                "Only the job engine writes outputs.",
            ));
        }
        let kind = self
            .host
            .check(&request.temporary, self.session, self.port)?
            .clone();
        let owned = matches!(&kind, CapabilityKind::OutputWrite { job } if *job == request.job)
            && self
                .host
                .pending
                .get(&request.temporary)
                .is_some_and(|p| p.job == request.job.as_str() && p.session == self.session);
        if !owned {
            return Err(error(
                ErrorCode::CapabilityRejected,
                "The handle is not this job's temporary output.",
            ));
        }
        let pending = self
            .host
            .pending
            .remove(&request.temporary)
            .expect("checked");
        self.host.forget(&request.temporary);
        let discard = |p: &Path| outputs::remove_temporary(p);
        if !request.commit {
            discard(&pending.temporary);
            return Ok(OutputCommitted {
                job: request.job.clone(),
                path: None,
            });
        }
        let size = std::fs::metadata(&pending.temporary).map_or(0, |m| m.len());
        if size == 0 {
            discard(&pending.temporary);
            return Err(error(
                ErrorCode::ExportFailed,
                "The output is empty, so it was removed.",
            ));
        }
        if request.verification.as_ref().map(|v| v.bytes.get()) != Some(size) {
            discard(&pending.temporary);
            return Err(error(
                ErrorCode::ExportFailed,
                "The output was not verified, so it was removed.",
            ));
        }
        match place(&pending.temporary, &pending.placement) {
            Ok(path) => Ok(OutputCommitted {
                job: request.job.clone(),
                path: Some(path.to_string_lossy().into_owned()),
            }),
            Err(e) => {
                discard(&pending.temporary);
                let message = match e {
                    outputs::PlaceError::Exists => {
                        "A file with that name already exists.".to_owned()
                    }
                    other => format!("{}.", capitalize(&other.to_string())),
                };
                Err(error(ErrorCode::ExportFailed, message))
            }
        }
    }

    fn request_lease(&mut self, request: &LeaseRequest) -> Result<Lease, EngineError> {
        if request.process != self.process {
            return Err(error(
                ErrorCode::InvalidMessage,
                "a lease request names its own process and a byte count",
            ));
        }
        self.host
            .memory
            .request(request)
            .map_err(|e| error(ErrorCode::InvalidMessage, e.to_string()))
    }

    fn hardware_sessions(
        &mut self,
        request: &HardwareSessionRequest,
    ) -> Result<HardwareSessionGrant, EngineError> {
        request
            .validate()
            .map_err(|v| error(ErrorCode::InvalidMessage, v.message))?;
        Ok(self.host.hardware.request(self.process, request))
    }

    fn blender_status(&self) -> BlenderStatus {
        BlenderStatus {
            available: false,
            version: None,
            error: Some("Blender jobs from the new engine are not connected yet.".into()),
        }
    }

    fn submit_blender_job(&mut self, _: &BlenderJobRequest) -> Result<(), EngineError> {
        Err(error(
            ErrorCode::Unsupported,
            "Blender jobs from the new engine are not connected yet; the current engine runs Blender.",
        ))
    }

    fn definition_snapshots(&self) -> DefinitionSetSnapshot {
        self.host.definitions.snapshot()
    }
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}
