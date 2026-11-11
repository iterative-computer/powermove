//! Immutable, checksummed disk render cache. Only derived frames can be evicted.
use crate::storage::{invalid, no_links, owned_root, publish, regular};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderKey {
    /// Complete semantic graph digest, including colour/effects/fonts/versions.
    pub graph: [u8; 32],
    /// Rotates on any relink, content or proxy selection change.
    pub media: [u8; 32],
    pub ticks: i64,
    pub width: u32,
    pub height: u32,
    pub format: String,
    pub renderer_version: String,
}
impl RenderKey {
    pub fn digest(&self) -> io::Result<String> {
        if self.width == 0
            || self.height == 0
            || self.format.is_empty()
            || self.renderer_version.is_empty()
        {
            return Err(invalid("incomplete render key"));
        }
        Ok(blake3::hash(&serde_json::to_vec(self)?)
            .to_hex()
            .to_string())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarState {
    Missing,
    Rendering,
    Rendered,
    Kept,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderBar {
    pub first_tick: i64,
    pub end_tick: i64,
    pub state: BarState,
}
pub struct RenderCache {
    root: PathBuf,
    quota: u64,
}
const MAGIC: &[u8; 8] = b"PMFRAME1";
const HEADER: usize = 48;
const MAX_ENTRY_BYTES: u64 = 1024 * 1024 * 1024;
impl RenderCache {
    pub fn open(root: &Path, quota_bytes: u64) -> io::Result<Self> {
        Ok(Self {
            root: owned_root(root, ".pm-render-v1")?,
            quota: quota_bytes,
        })
    }
    fn path(&self, key: &RenderKey) -> io::Result<PathBuf> {
        no_links(&self.root)?;
        Ok(self.root.join(format!("{}.frame", key.digest()?)))
    }
    fn keep_path(&self, key: &RenderKey) -> io::Result<PathBuf> {
        Ok(self.root.join(format!("{}.keep", key.digest()?)))
    }
    /// Publication is idempotent only for identical bytes. Different bytes for the
    /// same semantic key indicate a caller determinism/key error, never overwrite.
    pub fn put(&self, key: &RenderKey, payload: &[u8]) -> io::Result<()> {
        let path = self.path(key)?;
        if path.exists() {
            if self.get(key)?.as_deref() == Some(payload) {
                return Ok(());
            }
            return Err(invalid("render key already has different bytes"));
        }
        let size = (payload.len() as u64)
            .checked_add(HEADER as u64)
            .ok_or_else(|| invalid("size overflow"))?;
        if size > self.quota || size > MAX_ENTRY_BYTES {
            return Err(invalid("frame exceeds cache quota"));
        }
        self.trim_to(self.quota - size)?;
        let mut data = Vec::with_capacity(HEADER + payload.len());
        data.extend_from_slice(MAGIC);
        data.extend_from_slice(blake3::hash(payload).as_bytes());
        data.extend_from_slice(&(payload.len() as u64).to_le_bytes());
        data.extend_from_slice(payload);
        publish(&path, &data)
    }
    pub fn get(&self, key: &RenderKey) -> io::Result<Option<Vec<u8>>> {
        let path = self.path(key)?;
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
            Ok(_) => regular(&path)?,
        }
        let mut f = File::open(path)?;
        let mut header = [0u8; HEADER];
        f.read_exact(&mut header)?;
        if &header[..8] != MAGIC {
            return Err(invalid("corrupt cache header"));
        }
        let size = u64::from_le_bytes(header[40..48].try_into().expect("fixed header"));
        if size > MAX_ENTRY_BYTES || size.checked_add(HEADER as u64) != Some(f.metadata()?.len()) {
            return Err(invalid("corrupt cache size or entry exceeds safety bound"));
        }
        let mut payload = Vec::new();
        f.read_to_end(&mut payload)?;
        if blake3::hash(&payload).as_bytes() != &header[8..40] {
            return Err(invalid("cache checksum mismatch"));
        }
        Ok(Some(payload))
    }
    /// Marks a frame as just used: eviction goes least recently used first.
    pub fn touch(&self, key: &RenderKey) -> io::Result<()> {
        let path = self.path(key)?;
        regular(&path)?;
        fs::File::options()
            .write(true)
            .open(path)?
            .set_modified(std::time::SystemTime::now())
    }
    pub fn keep_rendered(&self, key: &RenderKey, keep: bool) -> io::Result<()> {
        if self.get(key)?.is_none() {
            return Err(invalid("cannot keep missing render"));
        }
        let path = self.keep_path(key)?;
        if keep {
            if !path.exists() {
                publish(&path, b"keep\n")?;
            } else {
                regular(&path)?;
            }
        } else {
            match fs::symlink_metadata(&path) {
                Ok(_) => {
                    regular(&path)?;
                    fs::remove_file(&path)?;
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
            File::open(&self.root)?.sync_all()?;
        }
        Ok(())
    }
    /// Whether a frame is on disk (a regular file of a plausible size), and
    /// whether it is kept, from metadata alone: indexing a composition's
    /// frames must not read them. `get` still verifies a frame it reads.
    pub fn presence(&self, key: &RenderKey) -> io::Result<Option<bool>> {
        let path = self.path(key)?;
        let meta = match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
            Ok(meta) => meta,
        };
        if !meta.file_type().is_file() || meta.len() < HEADER as u64 {
            return Ok(None);
        }
        Ok(Some(
            fs::symlink_metadata(self.keep_path(key)?).is_ok_and(|m| m.file_type().is_file()),
        ))
    }
    /// Deletes a frame and its Keep Rendered mark (Delete Rendered Frames).
    pub fn remove(&self, key: &RenderKey) -> io::Result<()> {
        for path in [self.keep_path(key)?, self.path(key)?] {
            match fs::symlink_metadata(&path) {
                Ok(_) => {
                    regular(&path)?;
                    fs::remove_file(&path)?;
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
    /// Always query with the current semantic graph/media digest. A changed
    /// digest immediately returns Missing (or Rendering for its own queued job),
    /// even while old kept bytes remain on disk. Never reuse an old bar snapshot
    /// after kernel publication; recompute it before presenting the new graph.
    pub fn state(&self, key: &RenderKey, rendering: bool) -> io::Result<BarState> {
        if self.get(key)?.is_some() {
            let keep = self.keep_path(key)?;
            if keep.exists() {
                regular(&keep)?;
                return Ok(BarState::Kept);
            }
            return Ok(BarState::Rendered);
        }
        Ok(if rendering {
            BarState::Rendering
        } else {
            BarState::Missing
        })
    }
    /// Removes every frame not marked Keep Rendered; returns the bytes freed.
    pub fn clear_unkept(&self) -> io::Result<u64> {
        no_links(&self.root)?;
        let mut kept = 0u64;
        for entry in fs::read_dir(&self.root)? {
            let path = entry?.path();
            if path.extension().is_some_and(|s| s == "frame")
                && path.with_extension("keep").exists()
            {
                kept = kept.saturating_add(fs::metadata(&path)?.len());
            }
        }
        self.trim_to(kept)
    }
    /// Least recently used first (written or [`Self::touch`]ed). Kept entries may make quota
    /// impossible; refusal leaves them intact. Check the whole plan before IO.
    pub fn trim_to(&self, bytes: u64) -> io::Result<u64> {
        no_links(&self.root)?;
        let mut entries = Vec::new();
        let mut total = 0u64;
        for entry in fs::read_dir(&self.root)? {
            let path = entry?.path();
            if path.extension().is_none_or(|s| s != "frame") {
                continue;
            }
            regular(&path)?;
            let m = fs::metadata(&path)?;
            total = total
                .checked_add(m.len())
                .ok_or_else(|| invalid("cache size overflow"))?;
            let keep = path.with_extension("keep");
            if keep.exists() {
                regular(&keep)?;
            } else {
                entries.push((m.modified()?, path, m.len()));
            }
        }
        entries.sort();
        let need = total.saturating_sub(bytes);
        if entries.iter().map(|e| e.2).sum::<u64>() < need {
            return Err(io::Error::other("kept renders exceed quota"));
        }
        let mut removed = 0;
        for (_, path, size) in entries {
            if removed >= need {
                break;
            }
            fs::remove_file(path)?;
            removed += size;
        }
        File::open(&self.root)?.sync_all()?;
        Ok(removed)
    }
}
/// Collapse adjacent equal states; ranges are exact half-open engine ticks.
pub fn render_bar(segments: &[(i64, i64, BarState)]) -> io::Result<Vec<RenderBar>> {
    let mut out: Vec<RenderBar> = Vec::new();
    for &(first_tick, end_tick, state) in segments {
        if first_tick < 0
            || end_tick <= first_tick
            || out.last().is_some_and(|s| s.end_tick > first_tick)
        {
            return Err(invalid("invalid or overlapping render range"));
        }
        if let Some(last) = out.last_mut()
            && last.end_tick == first_tick
            && last.state == state
        {
            last.end_tick = end_tick;
            continue;
        }
        out.push(RenderBar {
            first_tick,
            end_tick,
            state,
        });
    }
    Ok(out)
}

/// Map half-open engine-tick segments to composition frames in one place.
/// A frame belongs to the segment containing its exact `Rate::frame_start`;
/// subframe intervals with no frame start disappear. End frames are exclusive.
/// This preserves adjacent boundaries at fractional rates without float rounding
/// or assigning two states to one frame. Views must use this helper, not convert
/// ticks ad hoc. The input must be ordered and nonoverlapping, as in `render_bar`.
pub fn render_bar_frames(
    segments: &[RenderBar],
    rate: pm_time::Rate,
) -> io::Result<Vec<(i64, i64, BarState)>> {
    let ticks: Vec<_> = segments
        .iter()
        .map(|s| (s.first_tick, s.end_tick, s.state))
        .collect();
    let segments = render_bar(&ticks)?;
    let boundary = |tick| -> io::Result<i64> {
        let frame = rate.frame_of(tick);
        let start = rate
            .frame_start(frame)
            .map_err(|_| invalid("frame boundary overflow"))?;
        Ok(frame + i64::from(start < tick))
    };
    let mut out: Vec<(i64, i64, BarState)> = Vec::new();
    for segment in segments {
        let first = boundary(segment.first_tick)?;
        let end = boundary(segment.end_tick)?;
        if first == end {
            continue;
        }
        if let Some(last) = out.last_mut()
            && last.1 == first
            && last.2 == segment.state
        {
            last.1 = end;
        } else {
            out.push((first, end, segment.state));
        }
    }
    Ok(out)
}
