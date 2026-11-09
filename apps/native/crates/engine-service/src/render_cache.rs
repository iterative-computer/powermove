//! Disk render cache (plan section 1, pro media): a composition's frames as
//! the paused viewer draws them, rendered in the background while the viewer
//! rests (Render Selection), kept in Storage › Renders, and shown by playback
//! and scrubbing instead of rendering them again.
//!
//! Frames are keyed by a fingerprint of the whole composition (the document,
//! the contributed definitions, the media files and the renderer): any edit
//! misses every frame, and frames of an earlier state stay on disk until the
//! cache's quota or Clear… removes them. Keep Rendered pins frames against
//! both. A project whose extensions override rasters keys its frames to the
//! session (override code has no identity across launches).

use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use pm_promedia::cache::{BarState, RenderCache, RenderKey};
use pm_protocol::common::{Size, Ticks};
use pm_time::Rate;

use crate::render::Scene;

/// Frames on disk: RGBA16F rows, top-down, premultiplied working values.
pub const FORMAT: &str = "rgba16f-viewer-v1";
const RENDERER: &str = concat!("powermove-native ", env!("CARGO_PKG_VERSION"));

/// The composition state a fingerprint was taken of.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity {
    doc: usize,
    revision: u64,
    content: u64,
    comp: String,
}
impl Identity {
    fn of(scene: &Scene) -> Self {
        Self {
            doc: Arc::as_ptr(&scene.doc) as usize,
            revision: scene.doc.revision(),
            content: scene.content,
            comp: scene.comp.clone(),
        }
    }
}

/// The open composition's frames.
struct Frames {
    identity: Identity,
    /// `None` while it is being taken.
    fingerprint: Option<[u8; 32]>,
    size: Size,
    rate: Rate,
    count: i64,
    present: HashSet<i64>,
    kept: HashSet<i64>,
}
impl Frames {
    fn ticks(&self, n: i64) -> Option<Ticks> {
        Ticks::new(self.rate.frame_start(n).ok()?).ok()
    }
    /// The frame starting exactly at `time`.
    fn index(&self, time: Ticks) -> Option<i64> {
        let n = self.rate.frame_of(time.get());
        (self.ticks(n)? == time && (0..self.count).contains(&n)).then_some(n)
    }
    fn key(&self, n: i64) -> Option<RenderKey> {
        let fingerprint = self.fingerprint?;
        Some(RenderKey {
            graph: fingerprint,
            media: fingerprint,
            ticks: self.ticks(n)?.get(),
            width: self.size.width,
            height: self.size.height,
            format: FORMAT.into(),
            renderer_version: RENDERER.into(),
        })
    }
}

/// What the timeline's render bar shows of the open composition.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RenderBars {
    /// Rises whenever anything below changes.
    pub revision: u64,
    pub composition: Option<String>,
    /// Seconds, half-open, ordered; unrendered stretches are left out.
    pub bars: Vec<(f64, f64, BarState)>,
    /// Frames queued and frames rendered of the current Render Selection.
    pub queued: u64,
    pub done: u64,
}

/// The same storage is looked at again at most this often.
const REINDEX_EVERY: std::time::Duration = std::time::Duration::from_secs(2);

type Notify = Arc<dyn Fn() + Send + Sync>;

/// Something the render bar shows changed: tell whoever listens.
fn bump(state: &mut State) {
    state.revision += 1;
    if let Some(notify) = &state.notify {
        notify();
    }
    // The engine renders queued frames when it next looks.
    if let Some(wake) = &state.wake {
        let _ = wake.try_send(());
    }
}

#[derive(Default)]
struct State {
    notify: Option<Notify>,
    wake: Option<std::sync::mpsc::SyncSender<()>>,
    root: Option<(PathBuf, u64)>,
    indexed: Option<std::time::Instant>,
    cache: Option<Arc<RenderCache>>,
    frames: Option<Frames>,
    queued: BTreeSet<i64>,
    total: u64,
    done: u64,
    revision: u64,
    /// The extension wrappers drawing raster overrides, when any do: their
    /// frames are this session's only.
    session: Option<u64>,
}

/// The engine's render cache, shared by the engine thread (fill, playback)
/// and the app (commands, the render bar).
#[derive(Clone, Default)]
pub struct DiskFrames {
    state: Arc<Mutex<State>>,
    writes: Arc<Mutex<Option<std::sync::mpsc::SyncSender<Write>>>>,
}
type Write = (RenderKey, i64, u64, Vec<u8>);

impl DiskFrames {
    /// Wakes the engine thread when frames are queued or a fingerprint lands.
    pub(crate) fn set_wake(&self, wake: std::sync::mpsc::SyncSender<()>) {
        self.state.lock().unwrap().wake = Some(wake);
    }

    /// Called (on any thread, never blocking) whenever the render bar
    /// changes.
    pub fn set_notify(&self, notify: Option<Notify>) {
        self.state.lock().unwrap().notify = notify;
    }

    /// Storage › Renders: where frames go and the most they may take
    /// (`None`: no render cache).
    /// The same folder again finds its frames again (Clear… may have removed
    /// some). Disk work: call off the main thread.
    pub fn set_storage(&self, root: Option<(PathBuf, u64)>) -> std::io::Result<()> {
        let mut state = self.state.lock().unwrap();
        if state.root == root {
            if state.indexed.is_some_and(|at| at.elapsed() < REINDEX_EVERY) {
                return Ok(());
            }
            state.indexed = Some(std::time::Instant::now());
            if let (Some(cache), Some(frames)) = (state.cache.clone(), state.frames.as_mut()) {
                let n_frames = frames.count;
                let keys: Vec<(i64, RenderKey)> = (0..n_frames)
                    .filter_map(|n| Some((n, frames.key(n)?)))
                    .collect();
                drop(state);
                let (present, kept) = index(&cache, &keys);
                let mut state = self.state.lock().unwrap();
                if let Some(frames) = state.frames.as_mut()
                    && keys.first().map(|k| Some(&k.1) == frames.key(k.0).as_ref()) != Some(false)
                {
                    frames.present = present;
                    frames.kept = kept;
                    bump(&mut state);
                }
            }
            return Ok(());
        }
        let cache = match &root {
            Some((root, quota)) => Some(Arc::new(RenderCache::open(root, *quota)?)),
            None => None,
        };
        state.root = root;
        state.cache = cache;
        if let Some(frames) = state.frames.as_mut() {
            // Taken again against the new folder.
            frames.fingerprint = None;
            frames.identity.content = u64::MAX;
        }
        bump(&mut state);
        Ok(())
    }

    /// The engine thread's current scene: a new composition state takes a
    /// new fingerprint (off this thread) and finds its frames on disk.
    /// Returns whether it did.
    pub(crate) fn observe(
        &self,
        scene: &Scene,
        definitions: &[pm_protocol::definitions::DefinitionSnapshot],
        overrides: Option<u64>,
    ) -> bool {
        let identity = Identity::of(scene);
        let mut state = self.state.lock().unwrap();
        if state.cache.is_none()
            || (state
                .frames
                .as_ref()
                .is_some_and(|f| f.identity == identity)
                && state.session == overrides)
        {
            return false;
        }
        let Some((size, rate, count)) = composition(scene) else {
            state.frames = None;
            return true;
        };
        if state
            .frames
            .as_ref()
            .is_some_and(|f| f.identity.comp != identity.comp)
        {
            state.queued.clear();
        }
        state.session = overrides;
        state.frames = Some(Frames {
            identity: identity.clone(),
            fingerprint: None,
            size,
            rate,
            count,
            present: HashSet::new(),
            kept: HashSet::new(),
        });
        bump(&mut state);
        let cache = state.cache.clone().expect("checked above");
        drop(state);
        let (doc, media, definitions) = (
            scene.doc.clone(),
            scene.media.clone(),
            serde_json::to_vec(definitions).unwrap_or_default(),
        );
        let this = self.clone();
        std::thread::Builder::new()
            .name("pm-render-cache".into())
            .spawn(move || {
                let fingerprint =
                    fingerprint(&doc, &identity.comp, &media, &definitions, overrides);
                let mut found = Frames {
                    identity: identity.clone(),
                    fingerprint: Some(fingerprint),
                    size,
                    rate,
                    count,
                    present: HashSet::new(),
                    kept: HashSet::new(),
                };
                let keys: Vec<(i64, RenderKey)> = (0..count)
                    .filter_map(|n| Some((n, found.key(n)?)))
                    .collect();
                (found.present, found.kept) = index(&cache, &keys);
                let mut state = this.state.lock().unwrap();
                if state
                    .frames
                    .as_ref()
                    .is_some_and(|f| f.identity == identity)
                {
                    state.frames = Some(found);
                    bump(&mut state);
                }
            })
            .ok();
        true
    }

    /// The composition's frames in `[from, to)` (seconds) go to the queue;
    /// the engine renders them while the viewer rests.
    pub fn render_range(&self, from: f64, to: f64) -> u64 {
        let mut state = self.state.lock().unwrap();
        let Some(frames) = state.frames.as_ref() else {
            return 0;
        };
        let range = frame_range(frames, from, to);
        let missing: Vec<i64> = range.filter(|n| !frames.present.contains(n)).collect();
        if state.queued.is_empty() {
            state.total = 0;
            state.done = 0;
        }
        let added = missing
            .into_iter()
            .filter(|n| state.queued.insert(*n))
            .count() as u64;
        state.total += added;
        bump(&mut state);
        added
    }

    /// Stops rendering the queued frames (frames on disk stay).
    pub fn cancel(&self) {
        let mut state = self.state.lock().unwrap();
        state.queued.clear();
        bump(&mut state);
    }

    /// Keep Rendered: the rendered frames in `[from, to)` are pinned against
    /// the quota and Clear… (`keep`), or unpinned. Returns frames changed.
    pub fn keep_range(&self, from: f64, to: f64, keep: bool) -> std::io::Result<u64> {
        let mut state = self.state.lock().unwrap();
        let Some(cache) = state.cache.clone() else {
            return Ok(0);
        };
        let Some(frames) = state.frames.as_mut() else {
            return Ok(0);
        };
        let mut changed = 0;
        for n in frame_range(frames, from, to) {
            if !frames.present.contains(&n) || frames.kept.contains(&n) == keep {
                continue;
            }
            let Some(key) = frames.key(n) else { continue };
            cache.keep_rendered(&key, keep)?;
            if keep {
                frames.kept.insert(n);
            } else {
                frames.kept.remove(&n);
            }
            changed += 1;
        }
        bump(&mut state);
        Ok(changed)
    }

    /// Delete Rendered Frames: every frame of the open composition's current
    /// state, kept ones too. Returns frames deleted.
    pub fn delete_all(&self) -> std::io::Result<u64> {
        let mut state = self.state.lock().unwrap();
        state.queued.clear();
        let Some(cache) = state.cache.clone() else {
            return Ok(0);
        };
        let Some(frames) = state.frames.as_mut() else {
            return Ok(0);
        };
        let mut deleted = 0;
        for n in frames.present.drain().collect::<Vec<_>>() {
            if let Some(key) = frames.key(n) {
                cache.remove(&key)?;
                deleted += 1;
            }
        }
        frames.kept.clear();
        bump(&mut state);
        Ok(deleted)
    }

    pub fn bars(&self) -> RenderBars {
        let state = self.state.lock().unwrap();
        let Some(frames) = state.frames.as_ref() else {
            return RenderBars {
                revision: state.revision,
                ..RenderBars::default()
            };
        };
        let mut bars: Vec<(f64, f64, BarState)> = Vec::new();
        let seconds = |n: i64| {
            frames.ticks(n).map_or(0., |t| {
                t.get() as f64 / pm_protocol::common::TICKS_PER_SECOND as f64
            })
        };
        for n in 0..frames.count {
            let bar = if frames.kept.contains(&n) {
                BarState::Kept
            } else if frames.present.contains(&n) {
                BarState::Rendered
            } else if state.queued.contains(&n) {
                BarState::Rendering
            } else {
                continue;
            };
            let (from, to) = (seconds(n), seconds(n + 1));
            match bars.last_mut() {
                Some(last) if last.2 == bar && (last.1 - from).abs() < 1e-9 => last.1 = to,
                _ => bars.push((from, to, bar)),
            }
        }
        RenderBars {
            revision: state.revision,
            composition: Some(frames.identity.comp.clone()),
            bars,
            queued: state.total,
            done: state.done,
        }
    }

    /// A frame on disk for `scene` at `time`: its size and RGBA16F bytes.
    pub(crate) fn read(&self, scene: &Scene, time: Ticks) -> Option<(Size, Vec<u8>)> {
        let state = self.state.lock().unwrap();
        let frames = state.frames.as_ref()?;
        if frames.identity != Identity::of(scene) {
            return None;
        }
        let n = frames.index(time)?;
        if !frames.present.contains(&n) {
            return None;
        }
        let (key, size, cache) = (frames.key(n)?, frames.size, state.cache.clone()?);
        drop(state);
        let bytes = cache.get(&key).ok().flatten()?;
        (bytes.len() as u64 == u64::from(size.width) * u64::from(size.height) * 8)
            .then_some((size, bytes))
    }

    /// The next queued frame of `scene`'s current state: its index, time and
    /// size. `None`: nothing to render, or the fingerprint is being taken.
    pub(crate) fn next(&self, scene: &Scene) -> Option<(i64, Ticks, Size)> {
        let mut state = self.state.lock().unwrap();
        let identity = Identity::of(scene);
        let frames = state.frames.as_ref()?;
        if frames.identity != identity || frames.fingerprint.is_none() {
            return None;
        }
        let (size, present) = (frames.size, frames.present.clone());
        while let Some(n) = state.queued.first().copied() {
            if present.contains(&n) {
                state.queued.remove(&n);
                state.done += 1;
                continue;
            }
            let time = state.frames.as_ref()?.ticks(n)?;
            return Some((n, time, size));
        }
        None
    }

    /// The engine rendered frame `n` of `scene`'s state: written to disk off
    /// the engine thread, then shown in the render bar.
    pub(crate) fn rendered(&self, scene: &Scene, n: i64, rgba: &[f32]) {
        let mut state = self.state.lock().unwrap();
        state.queued.remove(&n);
        let Some(frames) = state.frames.as_ref() else {
            return;
        };
        if frames.identity != Identity::of(scene) {
            return;
        }
        let Some(key) = frames.key(n) else { return };
        let generation = state.revision;
        drop(state);
        let bytes: Vec<u8> = rgba
            .iter()
            .flat_map(|v| half::f16::from_f32(*v).to_le_bytes())
            .collect();
        let sender = self.writer();
        if sender.send((key, n, generation, bytes)).is_err() {
            *self.writes.lock().unwrap() = None;
        }
    }

    /// A frame that cannot be drawn exactly (a layer failed): not rendered.
    pub(crate) fn skip(&self, n: i64) {
        let mut state = self.state.lock().unwrap();
        state.queued.remove(&n);
        state.done += 1;
        bump(&mut state);
    }

    fn writer(&self) -> std::sync::mpsc::SyncSender<Write> {
        let mut writes = self.writes.lock().unwrap();
        if let Some(tx) = writes.as_ref() {
            return tx.clone();
        }
        let (tx, rx) = std::sync::mpsc::sync_channel::<Write>(2);
        let this = self.clone();
        std::thread::Builder::new()
            .name("pm-render-cache-io".into())
            .spawn(move || {
                for (key, n, _, bytes) in rx {
                    let cache = this.state.lock().unwrap().cache.clone();
                    let written = cache.is_some_and(|cache| cache.put(&key, &bytes).is_ok());
                    let mut state = this.state.lock().unwrap();
                    state.done += 1;
                    if written
                        && let Some(frames) = state.frames.as_mut()
                        && frames.key(n).as_ref() == Some(&key)
                    {
                        frames.present.insert(n);
                    }
                    bump(&mut state);
                }
            })
            .ok();
        *writes = Some(tx.clone());
        tx
    }
}

/// Which of `keys`' frames are on disk, and which of those are kept.
fn index(cache: &RenderCache, keys: &[(i64, RenderKey)]) -> (HashSet<i64>, HashSet<i64>) {
    let (mut present, mut kept) = (HashSet::new(), HashSet::new());
    for (n, key) in keys {
        if let Ok(Some(keep)) = cache.presence(key) {
            present.insert(*n);
            if keep {
                kept.insert(*n);
            }
        }
    }
    (present, kept)
}

/// Frame indices of `[from, to)` seconds, clamped to the composition.
fn frame_range(frames: &Frames, from: f64, to: f64) -> std::ops::Range<i64> {
    let tick = |s: f64| (s * pm_protocol::common::TICKS_PER_SECOND as f64).round() as i64;
    let first = frames.rate.frame_of(tick(from.max(0.)));
    let end = frames.rate.frame_of(tick(to.max(0.)));
    let end = if frames.ticks(end).is_some_and(|t| t.get() < tick(to)) {
        end + 1
    } else {
        end
    };
    first.clamp(0, frames.count)..end.clamp(0, frames.count)
}

/// The composition's size, frame rate and frame count.
fn composition(scene: &Scene) -> Option<(Size, Rate, i64)> {
    let comp = pm_eval::Comp::resolve(&scene.doc, &scene.comp).ok()?;
    let number = |key: &str| {
        let v = pm_eval::node::to_number(comp.member(key));
        (v.is_finite() && v > 0.).then_some(v)
    };
    let (w, h) = (number("w")?, number("h")?);
    let fps = number("fps").unwrap_or(30.);
    let duration = number("dur")?;
    let rate = Rate::new((fps * 1000.).round() as i64, 1000).ok()?;
    let count = (duration * fps).round().max(1.) as i64;
    Some((Size::new(w.round() as u32, h.round() as u32), rate, count))
}

/// The whole composition's fingerprint: the document, the contributed
/// definitions, each media file (path, size, modification time) and the
/// renderer; with raster overrides, this session's wrappers too.
fn fingerprint(
    doc: &pm_model::Document,
    comp: &str,
    media: &pm_export::render::MediaPaths,
    definitions: &[u8],
    overrides: Option<u64>,
) -> [u8; 32] {
    let mut hash = blake3::Hasher::new_derive_key("powermove render cache v1");
    if let Some(root) = doc.root() {
        hash.update(&pm_model::hash::canonical_digest_parallel(root));
    }
    hash.update(comp.as_bytes());
    hash.update(definitions);
    for (asset, path) in &media.paths {
        hash.update(asset.as_bytes());
        hash.update(path.as_os_str().as_encoded_bytes());
        if let Ok(meta) = std::fs::metadata(path) {
            hash.update(&meta.len().to_le_bytes());
            let modified = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos());
            hash.update(&modified.to_le_bytes());
        }
    }
    hash.update(RENDERER.as_bytes());
    if let Some(epoch) = overrides {
        hash.update(&std::process::id().to_le_bytes());
        hash.update(&epoch.to_le_bytes());
    }
    *hash.finalize().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene() -> Scene {
        let project = pm_model::json::parse(include_str!("../tests/fixtures/shapes.json")).unwrap();
        let doc = pm_model::Document::new(Some(pm_model::canonical::canonical_project(project)), 1);
        Scene {
            doc: Arc::new(doc),
            comp: "Pshapes-primitives-root".into(),
            media: Arc::default(),
            content: 1,
        }
    }
    fn settle(disk: &DiskFrames, until: impl Fn(&RenderBars) -> bool) -> RenderBars {
        let start = std::time::Instant::now();
        loop {
            let bars = disk.bars();
            if until(&bars) {
                return bars;
            }
            assert!(
                start.elapsed() < std::time::Duration::from_secs(10),
                "{bars:?}"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// Render Selection queues the range's frames; a rendered frame is written
    /// off the engine thread, shows in the render bar, and is read back for
    /// the same composition state only.
    #[test]
    fn rendered_frames_are_kept_shown_and_read_for_their_state_only() {
        let dir = tempfile::tempdir().unwrap();
        let disk = DiskFrames::default();
        disk.set_storage(Some((dir.path().join("frames"), u64::MAX)))
            .unwrap();
        let scene = scene();
        assert!(disk.observe(&scene, &[], None));
        assert!(!disk.observe(&scene, &[], None));
        // The fingerprint is taken off this thread; until then nothing renders.
        settle(&disk, |b| b.composition.is_some());
        assert_eq!(disk.render_range(1., 1.5), 15);
        let start = std::time::Instant::now();
        while disk.next(&scene).is_none() {
            assert!(start.elapsed() < std::time::Duration::from_secs(10));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let (n, time, size) = disk.next(&scene).expect("queued");
        assert_eq!((n, size), (30, Size::new(640, 360)));
        assert_eq!(
            time,
            Ticks::new(pm_time::Rate::FPS_30.frame_start(30).unwrap()).unwrap()
        );
        let bars = disk.bars();
        assert_eq!(bars.bars, vec![(1., 1.5, BarState::Rendering)]);
        let pixels = vec![0.25f32; 640 * 360 * 4];
        disk.rendered(&scene, n, &pixels);
        let bars = settle(&disk, |b| {
            b.bars.first().is_some_and(|b| b.2 == BarState::Rendered)
        });
        assert_eq!(bars.bars[0].0, 1.);
        let (read_size, bytes) = disk.read(&scene, time).expect("read back");
        assert_eq!(read_size, size);
        assert_eq!(&bytes[..2], &half::f16::from_f32(0.25).to_le_bytes());
        // Another frame of the range is not on disk; another state reads nothing.
        assert!(
            disk.read(&scene, Ticks::new(time.get() + 1).unwrap())
                .is_none()
        );
        let mut edited = scene.clone();
        edited.content = 2;
        assert!(disk.read(&edited, time).is_none());
        // Keep Rendered pins it; Delete Rendered Frames removes kept ones too.
        assert_eq!(disk.keep_range(0., 3., true).unwrap(), 1);
        assert_eq!(disk.bars().bars[0].2, BarState::Kept);
        assert_eq!(disk.delete_all().unwrap(), 1);
        assert!(disk.read(&scene, time).is_none());
        assert!(disk.bars().bars.is_empty());
    }
}
