//! One version of a file on disk, as everything that keeps something read
//! from a file tells that the file is still the one it read: the font
//! registry's files, the renderer's media, and the keys of frames rendered
//! from them.

use std::path::Path;

/// A file's length, modification time, device, inode and status-change
/// time. A save in place, a replacement (another inode) and a rewrite of
/// the same length with its modification time put back each give another
/// version: the status-change time moves with every change of a file's
/// data or metadata, and a program cannot set it back.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct FileVersion {
    pub len: u64,
    /// Nanoseconds since the Unix epoch.
    pub modified: u64,
    pub device: u64,
    pub inode: u64,
    /// Seconds and nanoseconds.
    pub changed: (i64, i64),
}

impl FileVersion {
    /// The version of the file at `path` now; `None` when it cannot be
    /// looked at.
    pub fn of(path: &Path) -> Option<Self> {
        std::fs::metadata(path).ok().map(|meta| Self::read(&meta))
    }

    pub fn read(meta: &std::fs::Metadata) -> Self {
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX));
        #[cfg(unix)]
        let (device, inode, changed) = {
            use std::os::unix::fs::MetadataExt;
            (meta.dev(), meta.ino(), (meta.ctime(), meta.ctime_nsec()))
        };
        #[cfg(not(unix))]
        let (device, inode, changed) = (0, 0, (0, 0));
        Self {
            len: meta.len(),
            modified,
            device,
            inode,
            changed,
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// A rewrite of the same length with its modification time put back,
    /// and a replacement by another file with the same length and time,
    /// are other versions.
    #[test]
    fn a_same_length_rewrite_with_its_time_restored_is_another_version() {
        let dir = std::env::temp_dir().join(format!("pm-file-version-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.bin");
        std::fs::write(&path, b"first").unwrap();
        let before = FileVersion::of(&path).unwrap();
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        // The status-change time has the file system's resolution.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&path, b"other").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        let rewritten = FileVersion::of(&path).unwrap();
        assert_eq!(
            (rewritten.len, rewritten.modified, rewritten.inode),
            (before.len, before.modified, before.inode),
            "the rewrite is invisible to length, time and inode"
        );
        assert_ne!(rewritten, before);
        let other = dir.join("b.bin");
        std::fs::write(&other, b"third").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&other)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        std::fs::rename(&other, &path).unwrap();
        let replaced = FileVersion::of(&path).unwrap();
        assert_eq!(
            (replaced.len, replaced.modified),
            (before.len, before.modified)
        );
        assert_ne!(replaced, rewritten);
        assert_eq!(FileVersion::of(&dir.join("missing")), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
