//! Main-reference flat-file/source-link adapter. Explicit caller-owned index
//! roots only; no profile discovery and no source writes or cloud hydration.
use crate::library_media::{DiskMediaRecord, SourceStatus, read_index};
use anyhow::{Result, anyhow, bail};
use std::{
    io::{Read, Seek, SeekFrom},
    path::Path,
};

type Identity = (u64, u64, u64, i64, i64, i64, i64, u32);
fn identity(m: &std::fs::Metadata) -> Identity {
    use std::os::unix::fs::MetadataExt;
    #[cfg(target_os = "macos")]
    let flags = {
        use std::os::macos::fs::MetadataExt;
        m.st_flags()
    };
    #[cfg(not(target_os = "macos"))]
    let flags = 0;
    (
        m.dev(),
        m.ino(),
        m.len(),
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec(),
        flags,
    )
}
/// Worker-owned adapter. Retain at most 64 verified descriptors for playback;
/// a cache hit needs the full device/inode/length/mtime/ctime/flags identity.
pub struct FileMedia {
    records: std::collections::BTreeMap<String, DiskMediaRecord>,
    verified: std::cell::RefCell<std::collections::BTreeMap<String, (Identity, std::fs::File)>>,
}
impl FileMedia {
    pub fn open(root: &Path) -> Result<Self> {
        Ok(Self {
            records: read_index(root)?
                .into_iter()
                .map(|r| (r.key.clone(), r))
                .collect(),
            verified: Default::default(),
        })
    }
    /// Metadata access never dereferences source links. Complete unknown fields,
    /// MIME, revision, size, checksum and relink intent remain available offline.
    pub fn record(&self, key: &str) -> Option<&DiskMediaRecord> {
        self.records.get(key)
    }
    pub fn status(&self, key: &str) -> Result<SourceStatus> {
        self.require(key)?.source_status()
    }
    fn require(&self, key: &str) -> Result<&DiskMediaRecord> {
        self.record(key).ok_or_else(|| anyhow!("Unknown media key"))
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
            bail!("Invalid media range");
        }
        let path = r
            .owned_path
            .as_ref()
            .or(r.source_path.as_ref())
            .ok_or_else(|| anyhow!("Missing payload"))?;
        let before = identity(&std::fs::symlink_metadata(path)?);
        let mut cache = self.verified.borrow_mut();
        if cache.get(key).is_some_and(|(stamp, file)| {
            *stamp != before || file.metadata().map_or(true, |m| identity(&m) != before)
        }) {
            cache.remove(key);
        }
        if !cache.contains_key(key) {
            let file = r.open_verified()?;
            if cache.len() >= 64 {
                cache.pop_first();
            }
            cache.insert(key.into(), (before, file));
        }
        let (_, file) = cache.get_mut(key).expect("verified media descriptor");
        file.seek(SeekFrom::Start(offset))?;
        let mut bytes = vec![0; length];
        file.read_exact(&mut bytes)?;
        if before != identity(&file.metadata()?)
            || before != identity(&std::fs::symlink_metadata(path)?)
        {
            cache.remove(key);
            bail!("Media changed during range read");
        }
        Ok(bytes)
    }
}
