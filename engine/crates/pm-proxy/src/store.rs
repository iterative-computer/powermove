use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use pm_protocol::priority::{ProxyFormat, ProxySize};
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use crate::{ProxyError, Result};

/// A recipe changes identity whenever encoded bytes or their interpretation
/// may change. An alpha or >10-bit source uses 4444 regardless of preference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyRecipe {
    pub version: u32,
    pub requested_format: ProxyFormat,
    pub requested_size: ProxySize,
    pub width: u32,
    pub height: u32,
    pub source_depth: u8,
    pub source_has_alpha: bool,
}

impl ProxyRecipe {
    pub const VERSION: u32 = 2;

    pub fn new(
        requested_format: ProxyFormat,
        requested_size: ProxySize,
        source_width: u32,
        source_height: u32,
        source_depth: u8,
        source_has_alpha: bool,
    ) -> Result<Self> {
        if source_width == 0 || source_height == 0 || source_depth == 0 {
            return Err(ProxyError::InvalidMedia(
                "invalid source size or depth".into(),
            ));
        }
        let max_height = match requested_size {
            ProxySize::Automatic => (source_height / 2).max(540),
            ProxySize::Half => (source_height / 2).max(1),
            ProxySize::Quarter => (source_height / 4).max(1),
            ProxySize::P1080 => 1080,
            ProxySize::P720 => 720,
        };
        let height = max_height.min(source_height);
        // 4:2:2 and 4:2:0 encoders need even dimensions. Cropping would
        // discard a source pixel; round up instead, preserving the whole view.
        let mut width = u32::try_from(
            (u64::from(source_width) * u64::from(height)).div_ceil(u64::from(source_height)),
        )
        .map_err(|_| ProxyError::InvalidMedia("proxy width overflows".into()))?;
        if width % 2 != 0 {
            width = width
                .checked_add(1)
                .ok_or_else(|| ProxyError::InvalidMedia("proxy width overflows".into()))?;
        }
        let height = if height % 2 != 0 {
            height
                .checked_add(1)
                .ok_or_else(|| ProxyError::InvalidMedia("proxy height overflows".into()))?
        } else {
            height
        };
        Ok(Self {
            version: Self::VERSION,
            requested_format,
            requested_size,
            width,
            height,
            source_depth,
            source_has_alpha,
        })
    }

    pub fn codec(&self) -> &'static str {
        if self.source_has_alpha || self.source_depth > 10 {
            "prores_4444"
        } else {
            match self.requested_format {
                ProxyFormat::ProResProxy => "prores_proxy",
                ProxyFormat::ProResLt => "prores_lt",
                ProxyFormat::Hevc10 => "hevc_10",
            }
        }
    }
}

/// Full BLAKE3 source identity plus the canonical serialized recipe.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProxyKey(pub String);

impl ProxyKey {
    pub fn new(source_hash: &str, recipe: &ProxyRecipe) -> Result<Self> {
        if source_hash.len() != 64 || !source_hash.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(ProxyError::InvalidMedia(
                "source hash must be BLAKE3 hex".into(),
            ));
        }
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"powermove-proxy-v1\0");
        hasher.update(source_hash.to_ascii_lowercase().as_bytes());
        hasher.update(
            &serde_json::to_vec(recipe).map_err(|e| ProxyError::CorruptStore(e.to_string()))?,
        );
        Ok(Self(hasher.finalize().to_hex().to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredProxy {
    pub key: ProxyKey,
    pub source_hash: String,
    pub recipe: ProxyRecipe,
    pub path: PathBuf,
    pub byte_size: u64,
    pub blake3: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredRecord {
    source_hash: String,
    recipe: ProxyRecipe,
    byte_size: u64,
    blake3: String,
}

/// Persistent generated media. The host supplies `root` from a
/// `CacheKind::GeneratedMedia` read/write capability.
#[derive(Debug, Clone)]
pub struct ProxyStore {
    root: PathBuf,
}

impl ProxyStore {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        fs::create_dir_all(root.as_ref())?;
        Ok(Self {
            root: root.as_ref().canonicalize()?,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn paths(&self, key: &ProxyKey) -> (PathBuf, PathBuf) {
        let sub = self.root.join(&key.0[..2]);
        (
            sub.join(format!("{}.mov", key.0)),
            sub.join(format!("{}.json", key.0)),
        )
    }

    /// Returns only a complete, verified object. A crash between writing
    /// media and metadata is a miss; changed/corrupt bytes are never played.
    pub fn lookup(&self, source_hash: &str, recipe: &ProxyRecipe) -> Result<Option<StoredProxy>> {
        self.lookup_cancel(source_hash, recipe, &|| false)
    }

    fn lookup_cancel(
        &self,
        source_hash: &str,
        recipe: &ProxyRecipe,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Option<StoredProxy>> {
        let key = ProxyKey::new(source_hash, recipe)?;
        let (media, info) = self.paths(&key);
        let bytes = match fs::read(&info) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let record: StoredRecord =
            serde_json::from_slice(&bytes).map_err(|e| ProxyError::CorruptStore(e.to_string()))?;
        if !record.source_hash.eq_ignore_ascii_case(source_hash) || record.recipe != *recipe {
            return Err(ProxyError::CorruptStore("record identity mismatch".into()));
        }
        let mut file = match File::open(&media) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        if file.metadata()?.len() != record.byte_size
            || hash_reader_cancel(&mut file, cancelled)? != record.blake3
        {
            return Err(ProxyError::CorruptStore(
                "generated media checksum mismatch".into(),
            ));
        }
        Ok(Some(StoredProxy {
            key,
            source_hash: source_hash.to_ascii_lowercase(),
            recipe: recipe.clone(),
            path: media,
            byte_size: record.byte_size,
            blake3: record.blake3,
        }))
    }

    /// Rebinds generated media after a move, relink or relaunch using the
    /// original's full hash. Newest validated matching recipe wins.
    pub fn find_for_source(
        &self,
        source_hash: &str,
        format: Option<ProxyFormat>,
        size: Option<ProxySize>,
    ) -> Result<Option<StoredProxy>> {
        self.find_for_source_cancel(source_hash, format, size, || false)
    }

    /// As `find_for_source`, but interruption is checked during directory
    /// scanning and after each hash chunk. Viewer-owned workers use this to
    /// stop promptly when their view closes.
    pub fn find_for_source_cancel(
        &self,
        source_hash: &str,
        format: Option<ProxyFormat>,
        size: Option<ProxySize>,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<StoredProxy>> {
        if source_hash.len() != 64 || !source_hash.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(ProxyError::InvalidMedia(
                "source hash must be BLAKE3 hex".into(),
            ));
        }
        let mut matches = Vec::new();
        for shard in fs::read_dir(&self.root)? {
            if cancelled() {
                return Err(ProxyError::Cancelled);
            }
            let shard = shard?;
            if !shard.file_type()?.is_dir() {
                continue;
            }
            for entry in fs::read_dir(shard.path())? {
                if cancelled() {
                    return Err(ProxyError::Cancelled);
                }
                let entry = entry?;
                if entry.path().extension().is_none_or(|ext| ext != "json") {
                    continue;
                }
                let Ok(data) = fs::read(entry.path()) else {
                    continue;
                };
                let Ok(record) = serde_json::from_slice::<StoredRecord>(&data) else {
                    continue;
                };
                if record.recipe.version != ProxyRecipe::VERSION
                    || !record.source_hash.eq_ignore_ascii_case(source_hash)
                    || format.is_some_and(|v| record.recipe.requested_format != v)
                    || size.is_some_and(|v| record.recipe.requested_size != v)
                {
                    continue;
                }
                let modified = entry.metadata()?.modified()?;
                matches.push((modified, record.recipe));
            }
        }
        matches.sort_by(|a, b| b.0.cmp(&a.0));
        for (_, recipe) in matches {
            if cancelled() {
                return Err(ProxyError::Cancelled);
            }
            match self.lookup_cancel(source_hash, &recipe, &cancelled) {
                Ok(Some(stored)) => return Ok(Some(stored)),
                Err(ProxyError::Cancelled) => return Err(ProxyError::Cancelled),
                _ => {}
            }
        }
        Ok(None)
    }

    /// Commits a validated temporary MOV by atomic rename, then writes the
    /// record atomically. `media` must be in this store's filesystem.
    pub fn commit(
        &self,
        source_hash: &str,
        recipe: &ProxyRecipe,
        media: NamedTempFile,
    ) -> Result<StoredProxy> {
        let key = ProxyKey::new(source_hash, recipe)?;
        let (destination, info) = self.paths(&key);
        fs::create_dir_all(destination.parent().expect("parent"))?;
        let mut src = File::open(media.path())?;
        let byte_size = src.metadata()?.len();
        let blake3 = hash_reader(&mut src)?;
        let record = StoredRecord {
            source_hash: source_hash.to_ascii_lowercase(),
            recipe: recipe.clone(),
            byte_size,
            blake3: blake3.clone(),
        };
        media
            .persist(&destination)
            .map_err(|e| ProxyError::Io(e.error))?;
        let mut tmp = NamedTempFile::new_in(destination.parent().expect("parent"))?;
        serde_json::to_writer(&mut tmp, &record)
            .map_err(|e| ProxyError::CorruptStore(e.to_string()))?;
        tmp.flush()?;
        tmp.as_file().sync_all()?;
        tmp.persist(&info).map_err(|e| ProxyError::Io(e.error))?;
        File::open(destination.parent().expect("parent"))?.sync_all()?;
        Ok(StoredProxy {
            key,
            source_hash: source_hash.to_ascii_lowercase(),
            recipe: recipe.clone(),
            path: destination,
            byte_size,
            blake3,
        })
    }

    pub fn remove(&self, source_hash: &str, recipe: &ProxyRecipe) -> Result<()> {
        let key = ProxyKey::new(source_hash, recipe)?;
        let (media, info) = self.paths(&key);
        for path in [info, media] {
            match fs::remove_file(path) {
                Ok(()) => (),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }
}

pub(crate) fn hash_reader(reader: &mut impl Read) -> Result<String> {
    hash_reader_cancel(reader, &|| false)
}

fn hash_reader_cancel(reader: &mut impl Read, cancelled: &impl Fn() -> bool) -> Result<String> {
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        if cancelled() {
            return Err(ProxyError::Cancelled);
        }
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

#[cfg(test)]
mod cancel_tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn checksum_can_stop_between_chunks() {
        let bytes = vec![7_u8; 3 * 1024 * 1024];
        let mut input = std::io::Cursor::new(bytes);
        let checks = Cell::new(0);
        let result = hash_reader_cancel(&mut input, &|| {
            checks.set(checks.get() + 1);
            checks.get() > 1
        });
        assert!(matches!(result, Err(ProxyError::Cancelled)));
        assert!(input.position() < 3 * 1024 * 1024);
    }
}

/// (available, capacity) on the volume containing `path`.
pub fn free_bytes(path: &Path) -> Result<(u64, u64)> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| ProxyError::InvalidMedia("NUL in store path".into()))?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: path is NUL terminated; stat points to writable storage.
    if unsafe { libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: successful statvfs initializes the result.
    let stat = unsafe { stat.assume_init() };
    Ok((
        u64::from(stat.f_bavail).saturating_mul(stat.f_frsize),
        u64::from(stat.f_blocks).saturating_mul(stat.f_frsize),
    ))
}

/// Generation pauses when free bytes are below max(20 GiB, 10% capacity).
pub fn disk_guard(available: u64, capacity: u64) -> bool {
    available >= (20u64 << 30).max(capacity / 10)
}
