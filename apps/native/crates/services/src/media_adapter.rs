//! Main-reference flat-file/source-link adapter. Explicit caller-owned index
//! roots only; no profile discovery and no source writes or cloud hydration.
use crate::{Result, ServiceError, ServiceErrorCode};
use pm_native_migrate::library_media::{DiskMediaRecord, SourceStatus, read_index};
use std::{
    io::{Read, Seek, SeekFrom},
    path::Path,
};

pub struct FileMedia {
    records: std::collections::BTreeMap<String, DiskMediaRecord>,
}
impl FileMedia {
    pub fn open(root: &Path) -> Result<Self> {
        Ok(Self {
            records: read_index(root)?
                .into_iter()
                .map(|r| (r.key.clone(), r))
                .collect(),
        })
    }
    /// Metadata access never dereferences source links. Complete unknown fields,
    /// MIME, revision, size, checksum and relink intent remain available offline.
    pub fn record(&self, key: &str) -> Option<&DiskMediaRecord> {
        self.records.get(key)
    }
    pub fn status(&self, key: &str) -> Result<SourceStatus> {
        Ok(self.require(key)?.source_status()?)
    }
    fn require(&self, key: &str) -> Result<&DiskMediaRecord> {
        self.record(key)
            .ok_or_else(|| ServiceError::message(ServiceErrorCode::NotFound, "Unknown media key"))
    }
    /// Bounded range read through the same verified descriptor. No mtime-only
    /// fast path can accidentally accept a changed original with old metadata.
    pub fn read_range(&self, key: &str, offset: u64, length: usize) -> Result<Vec<u8>> {
        let r = self.require(key)?;
        if length > 8 * 1024 * 1024
            || offset
                .checked_add(length as u64)
                .is_none_or(|end| end > r.size)
        {
            return Err(ServiceError::message(
                ServiceErrorCode::InvalidInput,
                "Invalid media range",
            ));
        }
        use std::os::unix::fs::MetadataExt;
        let path = r
            .owned_path
            .as_ref()
            .or(r.source_path.as_ref())
            .ok_or_else(|| {
                ServiceError::message(ServiceErrorCode::InvalidData, "Missing payload")
            })?;
        let stamp = |m: &std::fs::Metadata| {
            (
                m.dev(),
                m.ino(),
                m.len(),
                m.mtime(),
                m.mtime_nsec(),
                m.ctime(),
                m.ctime_nsec(),
            )
        };
        let before = stamp(&std::fs::symlink_metadata(path)?);
        let mut file = r.open_verified()?;
        file.seek(SeekFrom::Start(offset))?;
        let mut bytes = vec![0; length];
        file.read_exact(&mut bytes)?;
        if before != stamp(&file.metadata()?) || before != stamp(&std::fs::symlink_metadata(path)?)
        {
            return Err(ServiceError::message(
                ServiceErrorCode::InvalidData,
                "Media changed during range read",
            ));
        }
        Ok(bytes)
    }
}

/// Complete worker metadata; unknown authored fields are retained verbatim.
pub struct ImportMetadata {
    pub asset: pm_model::Value,
    pub mime: String,
    pub revision: String,
}

/// The queue owner calls this after worker probing. Copy into an explicit private
/// owned root BEFORE kernel admission; deleting an external source cannot break
/// the admitted import even before the first Save. No source is changed.
pub fn admit_import(
    store: &mut pm_kernel::DocumentStore,
    job: &str,
    request: &crate::media::ImportRequest,
    probe: &crate::media::Probe,
    metadata: ImportMetadata,
    owned_root: &Path,
) -> Result<()> {
    probe.validate()?;
    if store
        .root()
        .and_then(|r| r.get("id"))
        .map(|n| n.to_value())
        .and_then(|v| v.as_str().map(str::to_owned))
        .as_deref()
        != Some(&request.project)
    {
        return Err(ServiceError::message(
            ServiceErrorCode::InvalidProject,
            "Import belongs to another project",
        ));
    }
    let prepared = prepare_import(job, request, probe, metadata, owned_root)?;
    store
        .execute(prepared.command(), pm_kernel::Origin::Interface)
        .map_err(|e| ServiceError::message(ServiceErrorCode::InvalidData, e.to_string()))?;
    Ok(())
}

/// Worker-only immutable output preparation, usable with the live KernelHandle.
pub fn prepare_import(
    job: &str,
    request: &crate::media::ImportRequest,
    probe: &crate::media::Probe,
    mut metadata: ImportMetadata,
    owned_root: &Path,
) -> Result<pm_kernel::import::PreparedImport> {
    probe.validate()?;
    if request.provenance == crate::media::Provenance::ProxyOnly {
        metadata
            .asset
            .as_object_mut()
            .ok_or_else(|| {
                ServiceError::message(ServiceErrorCode::InvalidData, "Invalid asset metadata")
            })?
            .insert("playbackProxy", pm_model::Value::Bool(true));
    }
    let owned_path = own_output(owned_root, request, probe, &metadata)?;
    pm_kernel::import::PreparedImport::capture(
        job.into(),
        vec![pm_kernel::import::ImportInput {
            asset_id: request.asset.clone(),
            asset: metadata.asset,
            owned_path,
            mime: metadata.mime,
            revision: metadata.revision,
            size: probe.bytes,
            sha256: probe.content_sha256.clone(),
        }],
    )
    .map_err(|e| ServiceError::message(ServiceErrorCode::InvalidData, e.to_string()))
}

/// Concrete queue-to-kernel binding. The host's worker supplies the probe and
/// complete metadata; `ImportQueue::finish` owns cancellation/generation fencing.
pub struct KernelImportHook<'a, P, M> {
    pub store: &'a mut pm_kernel::DocumentStore,
    pub owned_root: &'a Path,
    pub probe: P,
    pub metadata: M,
}
impl<P, M> crate::media::MediaHook for KernelImportHook<'_, P, M>
where
    P: FnMut(&Path) -> Result<crate::media::Probe>,
    M: FnMut(
        &crate::media::ImportRequest,
        &crate::media::Probe,
    ) -> Result<(pm_model::Value, String, String)>,
{
    fn probe(&mut self, source: &Path) -> Result<crate::media::Probe> {
        (self.probe)(source)
    }
    fn admit(
        &mut self,
        job: &str,
        request: &crate::media::ImportRequest,
        probe: &crate::media::Probe,
    ) -> Result<()> {
        let (mut asset, mime, revision) = (self.metadata)(request, probe)?;
        if request.provenance == crate::media::Provenance::ProxyOnly {
            let map = asset.as_object_mut().ok_or_else(|| {
                ServiceError::message(ServiceErrorCode::InvalidData, "Invalid asset metadata")
            })?;
            map.insert("playbackProxy", pm_model::Value::Bool(true));
        }
        admit_import(
            self.store,
            job,
            request,
            probe,
            ImportMetadata {
                asset,
                mime,
                revision,
            },
            self.owned_root,
        )
    }
}

fn own_output(
    root: &Path,
    request: &crate::media::ImportRequest,
    probe: &crate::media::Probe,
    metadata: &ImportMetadata,
) -> Result<std::path::PathBuf> {
    use std::{
        fs::{self, OpenOptions},
        io::Read,
        os::unix::fs::{OpenOptionsExt, PermissionsExt},
    };
    crate::storage::component(&request.asset)?;
    if !root.is_absolute()
        || root.canonicalize()? != root
        || !fs::symlink_metadata(root)?.is_dir()
        || fs::metadata(root)?.permissions().mode() & 0o077 != 0
    {
        return Err(ServiceError::message(
            ServiceErrorCode::InvalidInput,
            "Import output root must be an existing private directory",
        ));
    }
    let mut record = DiskMediaRecord::decode(
        serde_json::json!({"key":format!("media:{}",request.asset), "revision":metadata.revision,"type":metadata.mime,"size":probe.bytes,"sha256":probe.content_sha256,"mtime":0,"sourcePath":request.source}),
        root,
    )?;
    let target = root.join(format!("{}.media", probe.content_sha256));
    if fs::symlink_metadata(&target).is_ok() {
        record.owned_path = Some(target.clone());
        record.source_path = None;
        record.verify_owned()?;
        return Ok(target);
    }
    // A partial never masquerades as the complete content-addressed output.
    // Create-new attempts and hard-link publication retain both failed and
    // completed scratch bytes; no user's source or prior attempt is removed.
    let mut source =
        record
            .open_verified()?
            .take(probe.bytes.checked_add(1).ok_or_else(|| {
                ServiceError::message(ServiceErrorCode::TooLarge, "Import too large")
            })?);
    if fs2::available_space(root)?.saturating_sub(probe.bytes) < 4 * 1024 * 1024 * 1024 {
        return Err(ServiceError::message(
            ServiceErrorCode::DiskFull,
            "Insufficient import space",
        ));
    }
    let attempt = root.join(format!(
        "{}.{}.part",
        probe.content_sha256,
        pm_kernel::commands::new_id("import")
    ));
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&attempt)?;
    let count = std::io::copy(&mut source, &mut output)?;
    output.sync_all()?;
    if count != probe.bytes {
        return Err(ServiceError::message(
            ServiceErrorCode::InvalidData,
            "Import changed while copying",
        ));
    }
    record.owned_path = Some(attempt.clone());
    record.source_path = None;
    record.verify_owned()?;
    match fs::hard_link(&attempt, &target) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.into()),
    }
    record.owned_path = Some(target.clone());
    record.verify_owned()?;
    fs::File::open(root)?.sync_all()?;
    Ok(target)
}

/// Live-application hook: preparation runs on the worker, and admission runs on
/// the existing kernel actor. `document` is captured when the import is queued.
pub struct ActorImportHook<'a, P, M> {
    pub kernel: &'a pm_kernel::KernelHandle,
    pub document: u64,
    pub owned_root: &'a Path,
    pub probe: P,
    pub metadata: M,
}
impl<P, M> crate::media::MediaHook for ActorImportHook<'_, P, M>
where
    P: FnMut(&Path) -> Result<crate::media::Probe>,
    M: FnMut(
        &crate::media::ImportRequest,
        &crate::media::Probe,
    ) -> Result<(pm_model::Value, String, String)>,
{
    fn probe(&mut self, source: &Path) -> Result<crate::media::Probe> {
        (self.probe)(source)
    }
    fn admit(
        &mut self,
        job: &str,
        request: &crate::media::ImportRequest,
        probe: &crate::media::Probe,
    ) -> Result<()> {
        let (asset, mime, revision) = (self.metadata)(request, probe)?;
        let prepared = prepare_import(
            job,
            request,
            probe,
            ImportMetadata {
                asset,
                mime,
                revision,
            },
            self.owned_root,
        )?;
        let ack = self
            .kernel
            .admit_import(&request.project, self.document, prepared)
            .wait()
            .ok_or_else(|| {
                ServiceError::message(
                    ServiceErrorCode::Unavailable,
                    "Kernel stopped during import",
                )
            })?
            .map_err(|e| ServiceError::message(ServiceErrorCode::InvalidProject, e.to_string()))?;
        match ack.reply {
            pm_kernel::Reply::Applied { .. } => Ok(()),
            other => Err(ServiceError::message(
                ServiceErrorCode::InvalidData,
                format!("Kernel refused import: {other:?}"),
            )),
        }
    }
}
