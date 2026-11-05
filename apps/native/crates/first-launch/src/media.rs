//! Worker-side flat-file/source-link media admission. Uses main's actual index
//! and kernel MediaTable; never invents payloads, grants or original provenance.
use anyhow::{Result, ensure};
use pm_kernel::project_io::{MediaEntry, MediaSource, MediaTable};
pub use pm_native_migrate::library_media::{DiskMediaRecord, ResidentSources, SourceAccess};
use std::{
    fs::File,
    path::{Path, PathBuf},
};

/// A handle to the bytes verified during admission, with their full raw metadata.
/// Keep any security-scoped access lease alongside this capability in the host.
pub struct AdmittedMedia {
    record: DiskMediaRecord,
    stamp: std::fs::Metadata,
    file: File,
}
impl AdmittedMedia {
    pub fn record(&self) -> &DiskMediaRecord {
        &self.record
    }
    /// Bounded positional reads use the verified descriptor, never a URL/path
    /// reopen. Range arithmetic cannot overflow or escape the payload.
    pub fn read_range(&self, offset: u64, length: usize) -> Result<Vec<u8>> {
        use std::os::unix::fs::FileExt;
        ensure!(length <= 4 * 1024 * 1024, "media range exceeds 4 MiB");
        ensure!(
            offset <= self.record.size && length as u64 <= self.record.size - offset,
            "media range exceeds payload"
        );
        self.unchanged()?;
        let mut bytes = vec![0; length];
        self.file.read_exact_at(&mut bytes, offset)?;
        self.unchanged()?;
        Ok(bytes)
    }
    /// Kernel admission is explicit by asset id. Source links survive ordinary
    /// saves; owned flat-file media stays embeddable. Proxy identity is authored
    /// asset metadata and must be preserved by the actor's AddAsset command.
    pub fn attach(&self, asset_id: &str, table: &mut MediaTable) -> Result<()> {
        self.unchanged()?;
        let path = self
            .record
            .source_path
            .as_ref()
            .or(self.record.owned_path.as_ref())
            .ok_or_else(|| anyhow::anyhow!("missing media path"))?;
        let entry = MediaEntry {
            id: asset_id.into(),
            media_type: self.record.mime.as_str().into(),
            revision: self.record.revision.as_str().into(),
            source: MediaSource::File {
                path: path.clone(),
                length: self.record.size,
                sha256: Some(self.record.sha256.clone()),
            },
        };
        if self.record.source_path.is_some() {
            table.insert_link(entry)?;
        } else {
            table.insert(entry);
        }
        Ok(())
    }
    fn unchanged(&self) -> Result<()> {
        use std::os::unix::fs::MetadataExt;
        let now = self.file.metadata()?;
        ensure!(
            now.dev() == self.stamp.dev()
                && now.ino() == self.stamp.ino()
                && now.len() == self.stamp.len()
                && now.mtime() == self.stamp.mtime()
                && now.mtime_nsec() == self.stamp.mtime_nsec()
                && now.ctime() == self.stamp.ctime()
                && now.ctime_nsec() == self.stamp.ctime_nsec(),
            "admitted media changed"
        );
        Ok(())
    }
}

/// Construct with an explicit, canonical profile Media Store on a worker. This
/// adapter is read-only and holds no second kernel writer or profile discovery.
pub struct FileMedia<A> {
    root: PathBuf,
    access: A,
}
impl<A: SourceAccess> FileMedia<A> {
    pub fn open(root: &Path, access: A) -> Result<Self> {
        ensure!(root.is_absolute(), "media root must be absolute");
        let root = root.canonicalize()?;
        ensure!(root.is_dir(), "media root is not a directory");
        Ok(Self { root, access })
    }
    pub fn metadata(&self, key: &str) -> Result<Option<DiskMediaRecord>> {
        pm_native_migrate::library_media::read_record(&self.root, key)
    }
    /// Missing key/offline link yields None. Damaged bytes, refused access and
    /// malformed records are errors, allowing the owner to retain/relink them.
    pub fn resolve(&mut self, key: &str) -> Result<Option<AdmittedMedia>> {
        let Some(record) = self.metadata(key)? else {
            return Ok(None);
        };
        let Some(file) = record.open_verified(&mut self.access)? else {
            return Ok(None);
        };
        let stamp = file.metadata()?;
        Ok(Some(AdmittedMedia {
            record,
            file,
            stamp,
        }))
    }
}
