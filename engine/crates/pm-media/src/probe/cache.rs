//! Bounded, content-hash-keyed video probe/index prewarm cache.
//!
//! A prewarm hashes the whole file and validates its inode, size, mtime and
//! ctime before and after reading/probing. Repeated opens use that stat tuple
//! as a cheap route to the verified content hash. Any edit or replacement
//! invalidates that route; a new prewarm then verifies the new bytes.

use super::{MediaProbe, Probed};
use crate::error::{MediaError, Result};
use crate::ff::Input;
use crate::index::FrameIndex;
use std::collections::{HashMap, VecDeque};
use std::fs::{File, Metadata};
use std::io::Read;
use std::mem::size_of;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
#[cfg(windows)]
use std::os::windows::fs::MetadataExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

const MAX_ENTRIES: usize = 16;
const MAX_BYTES: usize = 64 << 20;
const MAX_VERSIONS: usize = 64;
// Tiny inputs stay inline to avoid a thread per small imported clip.
const PARALLEL_HASH_MIN_BYTES: u64 = 128 << 10;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct Version {
    len: u64,
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    ino: u64,
    #[cfg(unix)]
    mtime: (i64, i64),
    #[cfg(unix)]
    ctime: (i64, i64),
    #[cfg(windows)]
    created: u64,
    #[cfg(windows)]
    modified: u64,
    #[cfg(windows)]
    attributes: u32,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct CacheKey {
    hash: [u8; 32],
    /// FFmpeg uses the filename as a demux hint for ambiguous byte streams.
    extension: String,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct Route {
    version: Version,
    extension: String,
}

fn extension(path: &Path) -> String {
    path.extension()
        .map(|value| value.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

/// Content-addressed probe/index reuse is valid only for containers whose
/// streams live in the file itself. A playlist with identical bytes can
/// resolve relative segment names differently at another path, and its
/// segments can change without the manifest's stat changing.
fn self_contained(path: &Path) -> Result<bool> {
    let ext = extension(path);
    let mut header = [0u8; 16];
    let mut file = File::open(path)?;
    let n = file.read(&mut header)?;
    Ok(match ext.as_str() {
        "mp4" | "m4v" | "mov" | "m4a" => n >= 8 && &header[4..8] == b"ftyp",
        "mkv" | "webm" => n >= 4 && header[..4] == [0x1a, 0x45, 0xdf, 0xa3],
        "avi" => n >= 12 && &header[..4] == b"RIFF" && &header[8..12] == b"AVI ",
        _ => false,
    })
}

impl From<&Metadata> for Version {
    fn from(m: &Metadata) -> Self {
        #[cfg(unix)]
        {
            Self {
                len: m.len(),
                dev: m.dev(),
                ino: m.ino(),
                mtime: (m.mtime(), m.mtime_nsec()),
                ctime: (m.ctime(), m.ctime_nsec()),
            }
        }
        #[cfg(windows)]
        {
            Self {
                len: m.len(),
                created: m.creation_time(),
                modified: m.last_write_time(),
                attributes: m.file_attributes(),
            }
        }
    }
}

/// Hash bytes only after checking that the opened file and pathname refer
/// to one unchanged version. Windows verifies this hash on cache lookup too:
/// its stable metadata APIs cannot expose the change time and file ID.
fn hash_stable(path: &Path, before: Version, cancel: Option<&AtomicBool>) -> Result<[u8; 32]> {
    let mut file = File::open(path)?;
    if Version::from(&file.metadata()?) != before {
        return Err(MediaError::invalid("source changed before hashing"));
    }
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0; 1 << 20];
    loop {
        if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return Err(MediaError::invalid("prewarm canceled"));
        }
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    if Version::from(&file.metadata()?) != before
        || Version::from(&std::fs::metadata(path)?) != before
    {
        return Err(MediaError::invalid("source changed while hashing"));
    }
    Ok(*hasher.finalize().as_bytes())
}

#[derive(Clone)]
pub(crate) struct Prepared {
    pub probe: MediaProbe,
    pub index: Arc<FrameIndex>,
    pub extradata: Vec<u8>,
    pub input: Arc<Mutex<Option<Input>>>,
}

struct Entry {
    prepared: Prepared,
    input_version: Version,
    bytes: usize,
}

#[derive(Default)]
struct Cache {
    entries: HashMap<CacheKey, Entry>,
    versions: HashMap<Route, CacheKey>,
    version_recent: VecDeque<Route>,
    recent: VecDeque<CacheKey>,
    bytes: usize,
}

static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();

fn cache() -> &'static Mutex<Cache> {
    CACHE.get_or_init(|| Mutex::new(Cache::default()))
}

impl Cache {
    fn bind_version(&mut self, route: Route, key: CacheKey) {
        self.versions.insert(route.clone(), key);
        self.version_recent.retain(|item| *item != route);
        self.version_recent.push_back(route);
        while self.versions.len() > MAX_VERSIONS {
            if let Some(old) = self.version_recent.pop_front() {
                self.versions.remove(&old);
            }
        }
    }

    fn touch(&mut self, key: &CacheKey) {
        self.recent.retain(|item| item != key);
        self.recent.push_back(key.clone());
    }

    fn insert(&mut self, route: Route, key: CacheKey, prepared: Prepared) {
        if self.entries.contains_key(&key) {
            self.bind_version(route, key.clone());
            self.touch(&key);
            return;
        }
        let bytes = estimate_bytes(&prepared);
        if bytes > MAX_BYTES {
            return;
        }
        while self.entries.len() >= MAX_ENTRIES || self.bytes.saturating_add(bytes) > MAX_BYTES {
            let Some(old) = self.recent.pop_front() else {
                break;
            };
            if let Some(entry) = self.entries.remove(&old) {
                self.bytes -= entry.bytes;
            }
            self.versions.retain(|_, value| value != &old);
            let versions = &self.versions;
            self.version_recent
                .retain(|version| versions.contains_key(version));
        }
        self.bytes += bytes;
        self.entries.insert(
            key.clone(),
            Entry {
                prepared,
                input_version: route.version,
                bytes,
            },
        );
        self.bind_version(route, key.clone());
        self.touch(&key);
    }
}

fn estimate_bytes(p: &Prepared) -> usize {
    p.index.samples.capacity() * size_of::<crate::index::Sample>()
        + p.index.frames.capacity() * size_of::<crate::index::Frame>()
        + p.index
            .config
            .atoms
            .iter()
            .map(|(_, data)| data.capacity())
            .sum::<usize>()
        + p.index
            .config
            .codec_private
            .as_ref()
            .map_or(0, Vec::capacity)
        + p.index
            .config
            .strip_prefix
            .as_ref()
            .map_or(0, Vec::capacity)
        + p.index.config.codec_id.as_ref().map_or(0, String::capacity)
        + p.extradata.capacity()
        + 4096
}

/// Reuse a verified content hash's probe/index, rebasing its path to the
/// caller's current path. A changed file is a miss, never stale metadata.
pub(crate) fn lookup(path: &Path) -> Result<Option<Prepared>> {
    if !self_contained(path)? {
        return Ok(None);
    }
    let before = Version::from(&std::fs::metadata(path)?);
    let route = Route {
        version: before,
        extension: extension(path),
    };
    let mut cache = cache().lock().unwrap();
    let Some(key) = cache.versions.get(&route).cloned() else {
        return Ok(None);
    };
    let Some(entry) = cache.entries.get(&key) else {
        cache.versions.remove(&route);
        cache.version_recent.retain(|item| *item != route);
        return Ok(None);
    };
    let mut prepared = entry.prepared.clone();
    // Probe/index metadata can be shared by identical bytes at different
    // inodes, but an open demuxer must never be borrowed from another file.
    if entry.input_version != before || cfg!(windows) {
        prepared.input = Arc::new(Mutex::new(None));
    }
    cache.touch(&key);
    cache.bind_version(route, key.clone());
    drop(cache);
    if Version::from(&std::fs::metadata(path)?) != before {
        return Ok(None);
    }
    #[cfg(windows)]
    if hash_stable(path, before, None)? != key.hash {
        return Ok(None);
    }
    prepared.probe.path = path.to_path_buf();
    Ok(Some(prepared))
}

/// Hash and prepare a stable source at import/open time. Input opening for
/// the eventual decoder runs alongside index construction. Decoder creation
/// itself still waits for the index's configuration and timebase.
pub fn prewarm(path: &Path) -> Result<()> {
    prewarm_with_cancel(path, None)
}

/// Like `prewarm`, but let a closing renderer interrupt a large file hash.
pub fn prewarm_cancelled(path: &Path, cancel: &AtomicBool) -> Result<()> {
    prewarm_with_cancel(path, Some(cancel))
}

fn prewarm_with_cancel(path: &Path, cancel: Option<&AtomicBool>) -> Result<()> {
    if !self_contained(path)? {
        return Ok(());
    }
    if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
        return Err(MediaError::invalid("prewarm canceled"));
    }
    if lookup(path)?.is_some() {
        return Ok(());
    }
    let before = Version::from(&std::fs::metadata(path)?);
    let route = Route {
        version: before,
        extension: extension(path),
    };
    // A second path to already-prepared bytes should pay only for a hash.
    // When no cached entry could possibly have this size and container hint,
    // hash the file alongside the probe/index and the decoder's input open.
    // The hash and probe each validate the same file version before any result
    // is published, so an in-place edit cannot combine two file generations.
    let maybe_copy = {
        let cache = cache().lock().unwrap();
        cache.entries.iter().any(|(key, entry)| {
            key.extension == route.extension && entry.prepared.probe.size == before.len
        })
    };
    if !maybe_copy && before.len >= PARALLEL_HASH_MIN_BYTES {
        let (hash, prepared) = std::thread::scope(|scope| {
            let hashing = scope.spawn(|| hash_stable(path, before, cancel));
            let prepared = probe_prepared(path, before);
            let hash = hashing
                .join()
                .map_err(|_| MediaError::invalid("prewarm hash worker panicked"))?;
            Ok::<_, MediaError>((hash?, prepared?))
        })?;
        if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return Err(MediaError::invalid("prewarm canceled"));
        }
        if Version::from(&std::fs::metadata(path)?) != before {
            return Err(MediaError::invalid("source changed while preparing"));
        }
        let key = CacheKey {
            hash,
            extension: route.extension.clone(),
        };
        cache().lock().unwrap().insert(route, key, prepared);
        return Ok(());
    }
    let hash = hash_stable(path, before, cancel)?;
    let key = CacheKey {
        hash,
        extension: route.extension.clone(),
    };
    {
        let mut cache = cache().lock().unwrap();
        if cache.entries.contains_key(&key) {
            cache.bind_version(route, key.clone());
            cache.touch(&key);
            return Ok(());
        }
    }
    let prepared = probe_prepared(path, before)?;
    if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
        return Err(MediaError::invalid("prewarm canceled"));
    }
    if Version::from(&std::fs::metadata(path)?) != before {
        return Err(MediaError::invalid("source changed while preparing"));
    }
    cache().lock().unwrap().insert(route, key, prepared);
    Ok(())
}

fn probe_prepared(path: &Path, before: Version) -> Result<Prepared> {
    let (probed, input) = std::thread::scope(|scope| {
        let input = scope.spawn(|| Input::open(path));
        let probed = super::probe(path);
        let input = input
            .join()
            .map_err(|_| MediaError::invalid("prewarm input worker panicked"))?;
        Ok::<_, MediaError>((probed?, input?))
    })?;
    let Probed { probe, index } = probed;
    let stream = probe
        .video_stream
        .ok_or_else(|| MediaError::invalid("no video stream"))?;
    let index = Arc::new(index.ok_or_else(|| MediaError::invalid("no video index"))?);
    let extradata = input.extradata(stream);
    if Version::from(&std::fs::metadata(path)?) != before {
        return Err(MediaError::invalid("source changed while probing"));
    }
    Ok(Prepared {
        probe,
        index,
        extradata,
        input: Arc::new(Mutex::new(Some(input))),
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::decode::service::MediaSource;
    use std::io::{Seek, SeekFrom, Write};
    use std::path::PathBuf;

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn relative_playlists_and_renamed_manifests_never_enter_content_cache() {
        let root = std::env::temp_dir().join(format!(
            "pm-media-playlist-cache-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("a")).unwrap();
        std::fs::create_dir_all(root.join("b")).unwrap();
        let _scratch = Scratch(root.clone());
        let manifest = b"#EXTM3U\n#EXTINF:1,\nseg0.ts\n";
        for folder in ["a", "b"] {
            let playlist = root.join(folder).join("index.m3u8");
            std::fs::write(&playlist, manifest).unwrap();
            assert!(!self_contained(&playlist).unwrap());
            prewarm(&playlist).unwrap();
            assert!(lookup(&playlist).unwrap().is_none());
            let renamed = root.join(folder).join("index.mp4");
            std::fs::write(&renamed, manifest).unwrap();
            assert!(!self_contained(&renamed).unwrap());
            prewarm(&renamed).unwrap();
            assert!(lookup(&renamed).unwrap().is_none());
        }
    }

    #[test]
    fn content_hash_routes_have_a_fixed_budget() {
        let mut cache = Cache::default();
        let version = |ino| Version {
            dev: 1,
            ino,
            len: 1,
            mtime: (1, 0),
            ctime: (1, 0),
        };
        let route = |ino| Route {
            version: version(ino),
            extension: "mp4".into(),
        };
        let key = CacheKey {
            hash: [0; 32],
            extension: "mp4".into(),
        };
        for ino in 0..(MAX_VERSIONS as u64 + 10) {
            cache.bind_version(route(ino), key.clone());
        }
        assert_eq!(cache.versions.len(), MAX_VERSIONS);
        assert_eq!(cache.version_recent.len(), MAX_VERSIONS);
        assert!(!cache.versions.contains_key(&route(0)));
        assert!(cache.versions.contains_key(&route(MAX_VERSIONS as u64 + 9)));
    }

    #[test]
    fn shared_hash_rebases_paths_and_file_change_invalidates() {
        let root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../apps/desktop/e2e/fixtures");
        let dir = std::env::temp_dir().join(format!(
            "pm-media-probe-cache-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        let _scratch = Scratch(dir.clone());
        let original = dir.join("source.mp4");
        let alias = dir.join("alias.mp4");
        let copy = dir.join("copy.mp4");
        let different_hint = dir.join("copy.bin");
        std::fs::copy(root.join("h264-aac.mp4"), &original).unwrap();
        std::os::unix::fs::symlink(&original, &alias).unwrap();
        std::fs::copy(&original, &copy).unwrap();
        std::os::unix::fs::symlink(&original, &different_hint).unwrap();

        assert!(lookup(&original).unwrap().is_none());
        prewarm(&original).unwrap();
        let first = lookup(&original).unwrap().unwrap();
        let through_alias = lookup(&alias).unwrap().unwrap();
        assert!(Arc::ptr_eq(&first.index, &through_alias.index));
        assert_eq!(through_alias.probe.path, alias);
        assert!(lookup(&different_hint).unwrap().is_none());
        prewarm(&copy).unwrap();
        let through_copy = lookup(&copy).unwrap().unwrap();
        assert!(Arc::ptr_eq(&first.index, &through_copy.index));
        assert_eq!(through_copy.probe.path, copy);
        assert!(through_copy.input.lock().unwrap().is_none());
        assert_eq!(MediaSource::open(&alias).unwrap().info.codec, "h264");

        // In-place replacement with unchanged length must invalidate too.
        let mut edit = std::fs::OpenOptions::new().write(true).open(&copy).unwrap();
        edit.seek(SeekFrom::Start(32)).unwrap();
        edit.write_all(&[0x5a]).unwrap();
        edit.sync_all().unwrap();
        assert!(lookup(&copy).unwrap().is_none());

        // Overwrite in place. The path and inode stay the same, while the
        // content, size and change time identify a different file.
        std::fs::copy(root.join("vp9-opus.webm"), &original).unwrap();
        assert!(lookup(&original).unwrap().is_none());
        assert_eq!(MediaSource::open(&original).unwrap().info.codec, "vp9");
        prewarm(&original).unwrap();
        assert_eq!(MediaSource::open(&alias).unwrap().info.codec, "vp9");
    }

    #[test]
    fn parallel_hash_and_probe_publish_only_validated_source() {
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../pm-render/tests/fixtures/video-parity/h264-hd-709-matrix-only.mp4");
        let dir = std::env::temp_dir().join(format!(
            "pm-media-parallel-probe-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        let _scratch = Scratch(dir.clone());
        let first = dir.join("first.mp4");
        let copy = dir.join("copy.mp4");
        std::fs::copy(&fixture, &first).unwrap();
        assert!(std::fs::metadata(&first).unwrap().len() >= PARALLEL_HASH_MIN_BYTES);
        prewarm(&first).unwrap();
        let prepared = lookup(&first).unwrap().unwrap();
        let opened = MediaSource::open(&first).unwrap();
        assert_eq!(opened.info, prepared.probe.video.as_ref().unwrap().codec);
        assert_eq!(opened.index.frames.len(), prepared.index.frames.len());

        std::fs::copy(&first, &copy).unwrap();
        prewarm(&copy).unwrap();
        let copied = lookup(&copy).unwrap().unwrap();
        assert!(Arc::ptr_eq(&prepared.index, &copied.index));
        assert_eq!(copied.probe.path, copy);
        let mut changed = std::fs::OpenOptions::new().write(true).open(&copy).unwrap();
        changed.seek(SeekFrom::Start(32)).unwrap();
        changed.write_all(&[0x5a]).unwrap();
        changed.sync_all().unwrap();
        assert!(lookup(&copy).unwrap().is_none());
    }
}
