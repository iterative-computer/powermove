//! Bounded probe/index prewarm cache.
//!
//! A prepared video is identified by its file stamp (device, inode, size,
//! modification and change time) and its sampled fingerprint (a header and
//! three 64 KiB samples, [`crate::hash::sampled_fingerprint`]). Preparing
//! or finding a source never reads the whole file: a multi-gigabyte camera
//! original costs what a small clip does. Any edit or replacement changes the
//! stamp, so a stale probe or index is never served.

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

/// The stamp half of a file's identity. FFmpeg uses the filename as a
/// demux hint for ambiguous byte streams, so the extension is part of it.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct Stamp {
    version: Version,
    extension: String,
}

impl Stamp {
    fn of(path: &Path) -> Result<Stamp> {
        Ok(Stamp {
            version: Version::from(&std::fs::metadata(path)?),
            extension: path
                .extension()
                .map(|value| value.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default(),
        })
    }
}

/// Reuse is valid only for containers whose streams live in the file
/// itself: a playlist's segments can change without its own stamp changing.
fn self_contained(path: &Path) -> Result<bool> {
    let mut header = [0u8; 16];
    let n = File::open(path)?.read(&mut header)?;
    let ext = path
        .extension()
        .map(|value| value.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    Ok(match ext.as_str() {
        "mp4" | "m4v" | "mov" | "m4a" => n >= 8 && &header[4..8] == b"ftyp",
        "mkv" | "webm" => n >= 4 && header[..4] == [0x1a, 0x45, 0xdf, 0xa3],
        "avi" => n >= 12 && &header[..4] == b"RIFF" && &header[8..12] == b"AVI ",
        _ => false,
    })
}

#[derive(Clone)]
pub(crate) struct Prepared {
    pub probe: MediaProbe,
    pub index: Arc<FrameIndex>,
    pub extradata: Vec<u8>,
    /// The probe's open demuxer, handed to the first decoder session.
    pub input: Arc<Mutex<Option<Input>>>,
}

struct Entry {
    prepared: Prepared,
    bytes: usize,
}

#[derive(Default)]
struct Cache {
    entries: HashMap<Stamp, Entry>,
    recent: VecDeque<Stamp>,
    bytes: usize,
}

static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();

fn cache() -> &'static Mutex<Cache> {
    CACHE.get_or_init(|| Mutex::new(Cache::default()))
}

impl Cache {
    fn touch(&mut self, key: &Stamp) {
        self.recent.retain(|item| item != key);
        self.recent.push_back(key.clone());
    }

    fn remove(&mut self, key: &Stamp) {
        if let Some(entry) = self.entries.remove(key) {
            self.bytes -= entry.bytes;
        }
        self.recent.retain(|item| item != key);
    }

    fn insert(&mut self, key: Stamp, prepared: Prepared) {
        self.remove(&key);
        let bytes = estimate_bytes(&prepared);
        if bytes > MAX_BYTES {
            return;
        }
        while self.entries.len() >= MAX_ENTRIES || self.bytes + bytes > MAX_BYTES {
            let Some(old) = self.recent.pop_front() else {
                break;
            };
            if let Some(entry) = self.entries.remove(&old) {
                self.bytes -= entry.bytes;
            }
        }
        self.bytes += bytes;
        self.entries.insert(key.clone(), Entry { prepared, bytes });
        self.touch(&key);
    }
}

fn estimate_bytes(p: &Prepared) -> usize {
    let config = &p.index.config;
    p.index.samples.capacity() * size_of::<crate::index::Sample>()
        + p.index.frames.capacity() * size_of::<crate::index::Frame>()
        + config
            .atoms
            .iter()
            .map(|(_, data)| data.capacity())
            .sum::<usize>()
        + config.codec_private.as_ref().map_or(0, Vec::capacity)
        + config.strip_prefix.as_ref().map_or(0, Vec::capacity)
        + config.codec_id.as_ref().map_or(0, String::capacity)
        + p.extradata.capacity()
        + 4096
}

/// The prepared probe and index of `path` as it is now, if a prewarm
/// prepared this file version: same stamp and same sampled fingerprint.
pub(crate) fn lookup(path: &Path) -> Result<Option<Prepared>> {
    if !self_contained(path)? {
        return Ok(None);
    }
    let key = Stamp::of(path)?;
    let Some(mut prepared) = cache()
        .lock()
        .unwrap()
        .entries
        .get(&key)
        .map(|e| e.prepared.clone())
    else {
        return Ok(None);
    };
    // The bounded content check catches a rewrite that kept every stamp
    // field (file systems without change times, restored metadata).
    if crate::hash::sampled_fingerprint(path)? != prepared.probe.sampled_fingerprint
        || Stamp::of(path)? != key
    {
        cache().lock().unwrap().remove(&key);
        return Ok(None);
    }
    cache().lock().unwrap().touch(&key);
    prepared.probe.path = path.to_path_buf();
    Ok(Some(prepared))
}

/// Prepare a stable source at import/open time: probe, index and an open
/// demuxer for its first decoder, without reading the whole file.
pub fn prewarm(path: &Path) -> Result<()> {
    prewarm_with_cancel(path, None)
}

/// Like `prewarm`, but a closing renderer can stop it between stages.
pub fn prewarm_cancelled(path: &Path, cancel: &AtomicBool) -> Result<()> {
    prewarm_with_cancel(path, Some(cancel))
}

fn prewarm_with_cancel(path: &Path, cancel: Option<&AtomicBool>) -> Result<()> {
    let cancelled = || cancel.is_some_and(|flag| flag.load(Ordering::Relaxed));
    if !self_contained(path)? || lookup(path)?.is_some() {
        return Ok(());
    }
    if cancelled() {
        return Err(MediaError::invalid("prewarm canceled"));
    }
    let key = Stamp::of(path)?;
    let (Probed { probe, index }, input) = super::probe_with_input(path)?;
    let stream = probe
        .video_stream
        .ok_or_else(|| MediaError::invalid("no video stream"))?;
    let index = Arc::new(index.ok_or_else(|| MediaError::invalid("no video index"))?);
    let extradata = input.extradata(stream);
    if cancelled() {
        return Err(MediaError::invalid("prewarm canceled"));
    }
    // Publish only what was read from one unchanged file version.
    if Stamp::of(path)? != key {
        return Err(MediaError::invalid("source changed while preparing"));
    }
    cache().lock().unwrap().insert(
        key,
        Prepared {
            probe,
            index,
            extradata,
            input: Arc::new(Mutex::new(Some(input))),
        },
    );
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::decode::service::MediaSource;
    use std::io::{Seek, SeekFrom, Write};
    use std::path::PathBuf;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Scratch {
            let dir = std::env::temp_dir().join(format!(
                "pm-media-{name}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn fixtures() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../apps/desktop/e2e/fixtures")
    }

    #[test]
    fn playlists_never_enter_the_cache() {
        let scratch = Scratch::new("playlist-cache");
        let manifest = b"#EXTM3U\n#EXTINF:1,\nseg0.ts\n";
        for name in ["index.m3u8", "index.mp4"] {
            let playlist = scratch.0.join(name);
            std::fs::write(&playlist, manifest).unwrap();
            assert!(!self_contained(&playlist).unwrap());
            prewarm(&playlist).unwrap();
            assert!(lookup(&playlist).unwrap().is_none());
        }
    }

    #[test]
    fn identity_is_the_stamp_and_sampled_fingerprint() {
        let scratch = Scratch::new("probe-cache");
        let original = scratch.0.join("source.mp4");
        let alias = scratch.0.join("alias.mp4");
        let copy = scratch.0.join("copy.mp4");
        std::fs::copy(fixtures().join("h264-aac.mp4"), &original).unwrap();
        std::os::unix::fs::symlink(&original, &alias).unwrap();
        std::fs::copy(&original, &copy).unwrap();

        assert!(lookup(&original).unwrap().is_none());
        prewarm(&original).unwrap();
        let first = lookup(&original).unwrap().unwrap();
        // A link reaches the same file version; a copy is another file.
        let through_alias = lookup(&alias).unwrap().unwrap();
        assert!(Arc::ptr_eq(&first.index, &through_alias.index));
        assert_eq!(through_alias.probe.path, alias);
        assert!(lookup(&copy).unwrap().is_none());

        // The first session takes the probe's demuxer once.
        assert!(first.input.lock().unwrap().is_some());
        assert_eq!(MediaSource::open(&original).unwrap().info.codec, "h264");

        // A same-length edit in place changes the stamp.
        prewarm(&copy).unwrap();
        assert!(lookup(&copy).unwrap().is_some());
        let mut edit = std::fs::OpenOptions::new().write(true).open(&copy).unwrap();
        edit.seek(SeekFrom::Start(32)).unwrap();
        edit.write_all(&[0x5a]).unwrap();
        edit.sync_all().unwrap();
        drop(edit);
        assert!(lookup(&copy).unwrap().is_none());

        // So does a replacement at the same path and inode.
        std::fs::copy(fixtures().join("vp9-opus.webm"), &original).unwrap();
        assert!(lookup(&original).unwrap().is_none());
        assert_eq!(MediaSource::open(&alias).unwrap().info.codec, "vp9");
    }

    #[test]
    fn a_rewrite_that_restores_every_stamp_field_but_ctime_is_still_caught() {
        // A change time can only move forward; restoring mtime and length
        // never brings back a prepared entry.
        let scratch = Scratch::new("probe-cache-mtime");
        let path = scratch.0.join("clip.mp4");
        std::fs::copy(fixtures().join("h264-aac.mp4"), &path).unwrap();
        prewarm(&path).unwrap();
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        std::fs::write(&path, &bytes).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
        assert!(lookup(&path).unwrap().is_none());
    }

    #[test]
    fn entries_and_bytes_are_bounded() {
        let scratch = Scratch::new("probe-cache-bound");
        let source = fixtures().join("h264-aac.mp4");
        let mut cache = Cache::default();
        let (Probed { probe, index }, input) = super::super::probe_with_input(&source).unwrap();
        let prepared = Prepared {
            extradata: input.extradata(probe.video_stream.unwrap()),
            probe,
            index: Arc::new(index.unwrap()),
            input: Arc::new(Mutex::new(None)),
        };
        for i in 0..(MAX_ENTRIES + 4) {
            let path = scratch.0.join(format!("{i}.mp4"));
            std::fs::copy(&source, &path).unwrap();
            cache.insert(Stamp::of(&path).unwrap(), prepared.clone());
        }
        assert_eq!(cache.entries.len(), MAX_ENTRIES);
        assert_eq!(cache.recent.len(), MAX_ENTRIES);
        assert!(cache.bytes <= MAX_BYTES);
        assert!(
            !cache
                .entries
                .contains_key(&Stamp::of(&scratch.0.join("0.mp4")).unwrap())
        );
    }
}
