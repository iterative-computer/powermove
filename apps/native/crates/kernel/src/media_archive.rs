//! Worker-owned immutable originals. A receipt certifies a complete copy and
//! fsync, never a metadata-only import or a proxy substituted for the original.
use crate::{
    ProjectIoError,
    error::io_err,
    project_io::{MediaEntry, MediaSource},
};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Debug)]
pub struct Archive {
    root: PathBuf,
}

/// Not serializable or constructible from saved project data. Only the worker
/// archive can produce one; the actor separately checks the attached profile.
#[derive(Clone, Debug)]
pub struct Original {
    root: PathBuf,
    hash: String,
    length: u64,
    mime: String,
    version: String,
    observed: Option<MediaEntry>,
}

impl Original {
    pub fn hash(&self) -> &str {
        &self.hash
    }
    pub fn length(&self) -> u64 {
        self.length
    }
    pub fn mime(&self) -> &str {
        &self.mime
    }
    /// Logical versions are unique even when their physical bytes deduplicate.
    /// Reusing a receipt keeps its version; another retention creates a fresh
    /// identity which cannot overwrite a previous asset or MIME interpretation.
    pub fn version(&self) -> &str {
        &self.version
    }
    /// An explicit read capability for downstream native probe/decode workers.
    pub fn path(&self) -> PathBuf {
        self.root.join(&self.hash)
    }
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }
    pub(crate) fn observed(&self) -> Option<&MediaEntry> {
        self.observed.as_ref()
    }
    pub(crate) fn entry(&self, id: &str) -> MediaEntry {
        MediaEntry {
            id: id.into(),
            media_type: self.mime.as_str().into(),
            revision: self.hash.as_str().into(),
            source: MediaSource::File {
                path: self.root.join(&self.hash),
                length: self.length,
                sha256: Some(self.hash.clone()),
            },
        }
    }
}

impl Archive {
    /// Explicit native profile capability, supplied by the application worker.
    /// Kernel admission checks this against its own attached library profile.
    pub fn open(profile: &Path) -> Result<Self, ProjectIoError> {
        let mut root = profile.canonicalize().map_err(|e| io_err(profile, e))?;
        for component in ["media", "retained"] {
            let next = root.join(component);
            match std::fs::create_dir(&next) {
                Ok(()) => sync_directory(&root)?,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if !std::fs::symlink_metadata(&next)
                        .map_err(|e| io_err(&next, e))?
                        .is_dir()
                    {
                        return invalid("The original archive must use ordinary directories");
                    }
                }
                Err(error) => return Err(io_err(&next, error)),
            }
            root = next;
        }
        Ok(Self { root })
    }

    /// Recovered ownership grants a bounded read capability, never readiness.
    /// The consumer streams and verifies length/SHA before using these bytes.
    pub(crate) fn reference(
        &self,
        id: &str,
        hash: &str,
        length: u64,
        mime: &str,
    ) -> Option<MediaEntry> {
        valid_hash(hash).then(|| MediaEntry {
            id: id.into(),
            media_type: mime.into(),
            revision: hash.into(),
            source: MediaSource::File {
                path: self.root.join(hash),
                length,
                sha256: Some(hash.into()),
            },
        })
    }

    fn temporary(&self) -> Result<(PathBuf, File), ProjectIoError> {
        loop {
            let path = self.root.join(crate::new_id("pending-original"));
            let mut options = OpenOptions::new();
            options.create_new(true).write(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(file) => return Ok((path, file)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(io_err(&path, error)),
            }
        }
    }

    /// Copy an explicitly selected local original on a worker, observing the
    /// opened source's identity before and after the complete streamed copy.
    pub fn retain_path(
        &self,
        source: &Path,
        mime: &str,
        cancel: &AtomicBool,
    ) -> Result<Original, ProjectIoError> {
        #[cfg(target_os = "macos")]
        {
            use std::os::macos::fs::MetadataExt;
            let observed = std::fs::metadata(source).map_err(|e| io_err(source, e))?;
            if observed.st_flags() & 0x40000000 != 0 {
                return invalid(
                    "The original is not available locally; choose its download explicitly",
                );
            }
        }
        let mut input = File::open(source).map_err(|e| io_err(source, e))?;
        let before = input.metadata().map_err(|e| io_err(source, e))?;
        if !before.is_file() {
            return invalid("Original media must be an ordinary file");
        }
        let (path, mut output) = self.temporary()?;
        let mut digest = Sha256::new();
        let mut length = 0u64;
        let mut buffer = vec![0; 1 << 20];
        loop {
            if cancel.load(Ordering::Acquire) {
                return invalid("Original retention was cancelled; its source was not changed");
            }
            let count = input.read(&mut buffer).map_err(|e| io_err(source, e))?;
            if count == 0 {
                break;
            }
            output
                .write_all(&buffer[..count])
                .map_err(|e| io_err(&path, e))?;
            digest.update(&buffer[..count]);
            length = length
                .checked_add(count as u64)
                .ok_or_else(|| ProjectIoError::Invalid("Original media is too large".into()))?;
        }
        let after = input.metadata().map_err(|e| io_err(source, e))?;
        if length != before.len() || !same_source(&before, &after) {
            return invalid("Original media changed while its bytes were being retained");
        }
        crate::journal::full_sync(&output).map_err(|e| io_err(&path, e))?;
        drop(output);
        self.publish(path, hex(&digest.finalize()), length, mime, None)
    }

    /// Preserve the prior original's actual read capability before relinking.
    /// Embedded container versions and known checksums are rechecked by copy_to.
    pub fn retain_entry(&self, source: &MediaEntry) -> Result<Original, ProjectIoError> {
        if let MediaSource::File {
            path,
            length,
            sha256,
        } = &source.source
        {
            if sha256.is_none() {
                return invalid(
                    "The prior original has no verified byte identity; its historical bytes cannot be certified",
                );
            }
            let mime = source.media_type.as_str().ok_or_else(|| {
                ProjectIoError::Invalid("The original media type cannot be interpreted".into())
            })?;
            let mut original = self.retain_path(path, mime, &AtomicBool::new(false))?;
            if original.length != *length
                || sha256.as_ref().is_some_and(|hash| hash != &original.hash)
            {
                return invalid("The prior original changed before it could be retained");
            }
            original.observed = Some(source.clone());
            return Ok(original);
        }
        // copy_to owns exclusive destination creation and its streaming fsync.
        let path = self.root.join(crate::new_id("pending-original"));
        source.copy_to(&path)?;
        let (hash, length) = digest_file(&path)?;
        self.publish(
            path,
            hash,
            length,
            source.media_type.as_str().ok_or_else(|| {
                ProjectIoError::Invalid("The original media type cannot be interpreted".into())
            })?,
            Some(source.clone()),
        )
    }

    /// Worker recovery verifies a bounded content identity inside this explicit
    /// archive. Saved metadata never grants an arbitrary filesystem path.
    pub fn recover(&self, hash: &str, length: u64, mime: &str) -> Result<Original, ProjectIoError> {
        if !valid_hash(hash) {
            return invalid("Invalid retained original identity");
        }
        let path = self.root.join(hash);
        let (actual, count) = digest_file(&path)?;
        if actual != hash || count != length {
            return invalid("A retained original is missing or changed");
        }
        Ok(Original {
            root: self.root.clone(),
            hash: hash.into(),
            length,
            mime: mime.into(),
            version: crate::new_id("retained-original"),
            observed: None,
        })
    }

    fn publish(
        &self,
        temporary: PathBuf,
        hash: String,
        length: u64,
        mime: &str,
        observed: Option<MediaEntry>,
    ) -> Result<Original, ProjectIoError> {
        crate::journal::full_sync(&File::open(&temporary).map_err(|e| io_err(&temporary, e))?)
            .map_err(|e| io_err(&temporary, e))?;
        let destination = self.root.join(&hash);
        match std::fs::hard_link(&temporary, &destination) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let (actual, count) = digest_file(&destination)?;
                if actual != hash || count != length {
                    return invalid(
                        "A retained original conflicts with its content identity; both copies were preserved",
                    );
                }
            }
            Err(error) => return Err(io_err(&destination, error)),
        }
        sync_directory(&self.root)?;
        // The complete content-addressed original is durable before retiring
        // this worker's duplicate temporary name. Error paths retain that copy.
        std::fs::remove_file(&temporary).map_err(|e| io_err(&temporary, e))?;
        sync_directory(&self.root)?;
        Ok(Original {
            root: self.root.clone(),
            hash,
            length,
            mime: mime.into(),
            version: crate::new_id("retained-original"),
            observed,
        })
    }
}

fn invalid<T>(message: &str) -> Result<T, ProjectIoError> {
    Err(ProjectIoError::Invalid(message.into()))
}
pub(crate) fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn digest_file(path: &Path) -> Result<(String, u64), ProjectIoError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path).map_err(|e| io_err(path, e))?;
    let before = file.metadata().map_err(|e| io_err(path, e))?;
    if !before.is_file() {
        return invalid("Retained originals must be ordinary files");
    }
    let mut digest = Sha256::new();
    let mut length = 0u64;
    let mut buffer = vec![0; 1 << 20];
    loop {
        let count = file.read(&mut buffer).map_err(|e| io_err(path, e))?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
        length += count as u64;
    }
    let after = file.metadata().map_err(|e| io_err(path, e))?;
    if length != before.len() || !same_source(&before, &after) {
        return invalid("Retained original changed while it was being verified");
    }
    Ok((hex(&digest.finalize()), length))
}
#[cfg(unix)]
fn same_source(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    a.dev() == b.dev()
        && a.ino() == b.ino()
        && a.len() == b.len()
        && a.mtime() == b.mtime()
        && a.mtime_nsec() == b.mtime_nsec()
        && a.ctime() == b.ctime()
        && a.ctime_nsec() == b.ctime_nsec()
}
#[cfg(not(unix))]
fn same_source(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    a.len() == b.len() && a.modified().ok() == b.modified().ok()
}
fn sync_directory(path: &Path) -> Result<(), ProjectIoError> {
    File::open(path)
        .and_then(|file| crate::journal::full_sync(&file))
        .map_err(|e| io_err(path, e))
}
