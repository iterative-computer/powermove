//! Level 1 SVG rasters (decision S1; docs/engine/vector.md, "Level 1 SVG
//! images").
//!
//! Today a level 1 SVG image is `rasterSvgAsset`: the asset's decoded
//! `<img>` drawn by `drawImage(img, 0, 0, w, h)` into a cleared canvas at
//! the bucket `svgRasterDimensions` picks, uploaded with
//! `UNPACK_PREMULTIPLY_ALPHA_WEBGL`. Chromium's SVG rasterization is not
//! emulated: the renderer that owns the document runs that same code and
//! uploads the premultiplied RGBA8 bytes, once per (content, bucket). This
//! store keeps them; [`super::cache::VectorCache`] uploads each to a
//! texture the compositor samples exactly as today's.
//!
//! A bucket the store does not have is a miss, recorded for the frame
//! ([`SvgRasterNeed`]); the host asks for it:
//!
//! - **Viewer preview**, exports and snapshots use [`SvgMissPolicy::Wait`]:
//!   the caller asks for the exact bucket and renders the frame when it
//!   arrives. The viewer retains its render-ahead slot in the meantime. A
//!   bucket the window could not draw ([`SvgRasters::fail`]) is drawn as
//!   nothing by a viewer frame instead of holding it
//!   (`Renderer::svg_waiting_viewer`), until its raster arrives.
//!   [`SvgMissPolicy::Substitute`] is available to diagnostic callers only;
//!   the app viewer never publishes a previous bucket as a new frame.
//! - **Exports and snapshots** ([`SvgMissPolicy::Wait`]) never draw a
//!   substitute: the frame fails with `RenderError::SvgRastersPending` and
//!   the caller fetches the rasters and renders it again.
//! - **`pm-cli`** has no Chromium: [`SvgRasterizer::Resvg`] draws the bucket
//!   with resvg, an approximation it reports with a named notice.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use pm_protocol::svg_raster::{SvgRasterKey, SvgRasterRequest};

/// Who draws level 1 SVG rasters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SvgRasterizer {
    /// Chromium, in the renderer that owns the document (the app, the
    /// hidden host's Chromium): rasters arrive through
    /// [`super::super::Renderer::put_svg_raster`].
    #[default]
    Chromium,
    /// resvg (`pm-cli`): not Chromium's rasterization; every frame that
    /// draws one counts it ([`super::super::Renderer::svg_approximated`]).
    Resvg,
}

/// What a frame does about a bucket the store does not have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SvgMissPolicy {
    /// Preview: the content's latest bucket, or nothing, this frame.
    Substitute,
    /// Exports and snapshots: nothing drawn, and the frame fails pending.
    Wait,
}

/// A raster a frame needed and the store did not have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SvgRasterNeed {
    pub request: SvgRasterRequest,
    /// The SVG's file (`pm-cli`, and hosts that rasterize from the file).
    pub path: std::path::PathBuf,
    /// The first layer that drew it, and its composition (the job
    /// engine's host request names them).
    pub composition: String,
    pub layer: String,
}

/// Where a renderer without a document window gets rasters (the job
/// engine: host services ask the window that owns the document,
/// `CompatFrameRequest` with source `svgImage`). Blocking.
pub trait SvgRasterSource: Send + Sync {
    /// The raster's pixels (premultiplied RGBA8, top-down), or why there
    /// are none (shown as the frame's error).
    fn fetch(&self, need: &SvgRasterNeed) -> Result<Vec<u8>, String>;
}

/// Chromium's rasters captured to a directory, one file per key
/// (`<content>-<width>x<height>.rgba`, premultiplied RGBA8, top-down):
/// what headless comparisons against the legacy captures draw with
/// (`pm-cli parity render --svg-rasters`, the corpus test's
/// `PM_SVG_RASTERS`). A missing file is recorded ([`RasterDir::missing`],
/// written as `needs.json` for the capture tool,
/// `engine/crates/pm-render/tools/svg-rasters`) and fails the frame.
pub struct RasterDir {
    dir: std::path::PathBuf,
    missing: std::sync::Mutex<std::collections::BTreeMap<SvgRasterKey, std::path::PathBuf>>,
}

impl RasterDir {
    pub fn new(dir: impl Into<std::path::PathBuf>) -> RasterDir {
        RasterDir {
            dir: dir.into(),
            missing: Default::default(),
        }
    }

    /// The file of `key`.
    pub fn file(&self, key: &SvgRasterKey) -> std::path::PathBuf {
        self.dir.join(format!(
            "{}-{}x{}{}.rgba",
            key.content,
            key.width,
            key.height,
            if key.texture == Some(true) {
                "-texture"
            } else {
                ""
            }
        ))
    }

    /// Rasters asked for that were not captured, and their SVG files.
    pub fn missing(&self) -> Vec<(SvgRasterKey, std::path::PathBuf)> {
        let m = self.missing.lock().unwrap_or_else(|e| e.into_inner());
        m.iter().map(|(k, p)| (k.clone(), p.clone())).collect()
    }

    /// Merges the missing rasters into `<dir>/needs.json` (`[{ content,
    /// width, height, svg }]`) and returns how many it lists.
    pub fn write_needs(&self) -> std::io::Result<usize> {
        let path = self.dir.join("needs.json");
        let mut all: std::collections::BTreeMap<(String, u32, u32, bool), String> =
            Default::default();
        if let Ok(text) = std::fs::read_to_string(&path)
            && let Ok(serde_json::Value::Array(list)) = serde_json::from_str(&text)
        {
            for v in list {
                if let (Some(c), Some(w), Some(h), Some(svg)) = (
                    v["content"].as_str(),
                    v["width"].as_u64(),
                    v["height"].as_u64(),
                    v["svg"].as_str(),
                ) {
                    all.insert(
                        (
                            c.into(),
                            w as u32,
                            h as u32,
                            v["texture"].as_bool().unwrap_or(false),
                        ),
                        svg.into(),
                    );
                }
            }
        }
        for (k, svg) in self.missing() {
            all.insert(
                (k.content, k.width, k.height, k.texture == Some(true)),
                svg.to_string_lossy().into_owned(),
            );
        }
        let list: Vec<serde_json::Value> = all
            .into_iter()
            .map(|((content, width, height, texture), svg)| {
                serde_json::json!({ "content": content, "width": width, "height": height, "texture": texture, "svg": svg })
            })
            .collect();
        std::fs::create_dir_all(&self.dir)?;
        std::fs::write(&path, serde_json::to_string_pretty(&list)?)?;
        Ok(list.len())
    }
}

impl SvgRasterSource for RasterDir {
    fn fetch(&self, need: &SvgRasterNeed) -> Result<Vec<u8>, String> {
        let key = &need.request.key;
        let file = self.file(key);
        match std::fs::read(&file) {
            Ok(data) if data.len() as u64 == key.bytes() => Ok(data),
            Ok(data) => Err(format!(
                "{}: {} bytes for a {}x{} SVG raster",
                file.display(),
                data.len(),
                key.width,
                key.height
            )),
            Err(_) => {
                self.missing
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(key.clone(), need.path.clone());
                Err(format!(
                    "the Chromium SVG raster {}x{} of {} was not captured ({}; capture it with engine/crates/pm-render/tools/svg-rasters)",
                    key.width,
                    key.height,
                    need.path.display(),
                    file.display()
                ))
            }
        }
    }
}

struct Stored {
    data: Arc<Vec<u8>>,
    used: u64,
}

/// Uploaded rasters by key, least recently used out past the budget.
pub struct SvgRasters {
    stored: HashMap<SvgRasterKey, Stored>,
    /// Each content's most recently drawn bucket (preview substitutes).
    latest: HashMap<String, SvgRasterKey>,
    bytes: u64,
    max_bytes: u64,
    tick: u64,
    /// `tick` when the latest frame began: rasters stored or read since are
    /// the working set (the frame being drawn, the rasters fetched for its
    /// next pass, the ones the window just answered) and are never
    /// evicted, as today's raster cache always keeps the raster it just
    /// drew (`cache.size <= 1`). A raster over the budget on its own is
    /// therefore kept until a later frame no longer reads it.
    frame_start: u64,
    misses: Vec<SvgRasterNeed>,
    /// Buckets the document's window could not draw (or did not answer
    /// for in time): a viewer frame draws without them instead of waiting
    /// ([`super::super::Renderer::svg_waiting_viewer`]) until their raster
    /// arrives, as today's compositor draws the frame without a layer
    /// whose `rasterSvgAsset` throws.
    failed: HashSet<SvgRasterKey>,
    pub(crate) rasterizer: SvgRasterizer,
    pub(crate) approximated: u64,
}

/// Today's raster budget (legacy/gl/raster.ts: the `raster` category's
/// 192 MiB): uploaded rasters are today's raster canvases.
pub const DEFAULT_MAX_BYTES: u64 = 192 * 1024 * 1024;

impl Default for SvgRasters {
    fn default() -> Self {
        SvgRasters {
            stored: HashMap::new(),
            latest: HashMap::new(),
            bytes: 0,
            max_bytes: DEFAULT_MAX_BYTES,
            tick: 0,
            frame_start: 0,
            misses: Vec::new(),
            failed: HashSet::new(),
            rasterizer: SvgRasterizer::default(),
            approximated: 0,
        }
    }
}

impl SvgRasters {
    /// Stores a raster's pixels in its key's encoding (canvas bucket or
    /// straight, flipped material-map upload).
    pub fn put(&mut self, key: SvgRasterKey, data: Vec<u8>) -> Result<(), String> {
        pm_perf::bump(pm_perf::Counter::RasterAnswers);
        key.validate().map_err(|v| v.to_string())?;
        if data.len() as u64 != key.bytes() {
            return Err(format!(
                "{} bytes for a {}×{} SVG raster",
                data.len(),
                key.width,
                key.height
            ));
        }
        self.tick += 1;
        self.failed.remove(&key);
        let bytes = data.len() as u64;
        if let Some(old) = self.stored.insert(
            key,
            Stored {
                data: Arc::new(data),
                used: self.tick,
            },
        ) {
            self.bytes -= old.data.len() as u64;
        }
        self.bytes += bytes;
        self.evict(self.max_bytes);
        Ok(())
    }

    /// The pixels of `key`, marked used.
    pub fn get(&mut self, key: &SvgRasterKey) -> Option<Arc<Vec<u8>>> {
        self.tick += 1;
        let tick = self.tick;
        self.stored.get_mut(key).map(|s| {
            s.used = tick;
            s.data.clone()
        })
    }

    pub fn contains(&self, key: &SvgRasterKey) -> bool {
        self.stored.contains_key(key)
    }

    /// Records that `key` was drawn (the content's preview substitute).
    pub fn drawn(&mut self, key: &SvgRasterKey) {
        self.latest.insert(key.content.clone(), key.clone());
    }

    /// The content's most recently drawn bucket other than `key`.
    pub fn latest(&self, key: &SvgRasterKey) -> Option<&SvgRasterKey> {
        self.latest.get(&key.content).filter(|k| *k != key)
    }

    /// Records a miss (once per key per frame).
    pub fn miss(&mut self, need: SvgRasterNeed) {
        if !self
            .misses
            .iter()
            .any(|m| m.request.key == need.request.key)
        {
            self.misses.push(need);
        }
    }

    /// Whether the frame being drawn missed anything.
    pub fn missed(&self) -> bool {
        !self.misses.is_empty()
    }

    /// Whether the frame being drawn missed a raster it waits for: any
    /// miss, or with `skip_failed` (the viewer) any the window did not fail.
    pub fn missed_waiting(&self, skip_failed: bool) -> bool {
        self.misses
            .iter()
            .any(|m| !(skip_failed && self.failed.contains(&m.request.key)))
    }

    /// The window could not draw `key`, or did not answer for it in time.
    pub fn fail(&mut self, key: SvgRasterKey) {
        self.failed.insert(key);
    }

    /// Whether `key` failed and has not arrived since.
    pub fn is_failed(&self, key: &SvgRasterKey) -> bool {
        self.failed.contains(key)
    }

    /// The frame's misses.
    pub fn take_misses(&mut self) -> Vec<SvgRasterNeed> {
        std::mem::take(&mut self.misses)
    }

    /// Drops the rasters of `content` (every content when `None`).
    pub fn forget(&mut self, content: Option<&str>) {
        let keep = |k: &SvgRasterKey| content.is_some_and(|c| k.content != c);
        let mut freed = 0;
        self.stored.retain(|k, s| {
            let keep = keep(k);
            if !keep {
                freed += s.data.len() as u64;
            }
            keep
        });
        self.bytes -= freed;
        self.latest.retain(|_, k| keep(k));
        self.failed.retain(keep);
    }

    /// Bytes stored.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    pub fn len(&self) -> usize {
        self.stored.len()
    }

    pub fn is_empty(&self) -> bool {
        self.stored.is_empty()
    }

    pub fn set_budget(&mut self, max_bytes: u64) {
        self.max_bytes = max_bytes;
        self.evict(max_bytes);
    }

    /// Starts a frame: what the previous frames stored or read may be
    /// evicted from now on (`frame_start`).
    pub fn begin_frame(&mut self) {
        self.frame_start = self.tick;
    }

    /// Evicts least recently used rasters down to `budget` bytes, never the
    /// working set (stored or read since the latest frame began).
    pub fn evict(&mut self, budget: u64) {
        if self.bytes <= budget {
            return;
        }
        let mut by_age: Vec<(u64, SvgRasterKey)> = self
            .stored
            .iter()
            .filter(|(_, s)| s.used <= self.frame_start)
            .map(|(k, s)| (s.used, k.clone()))
            .collect();
        by_age.sort_unstable();
        for (_, k) in by_age {
            if self.bytes <= budget {
                break;
            }
            if let Some(s) = self.stored.remove(&k) {
                self.bytes -= s.data.len() as u64;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(content: char, w: u32, h: u32) -> SvgRasterKey {
        SvgRasterKey {
            texture: None,
            source_fit: None,
            content: content.to_string().repeat(64),
            width: w,
            height: h,
        }
    }

    #[test]
    fn rasters_are_kept_by_content_and_bucket_within_a_budget() {
        let mut s = SvgRasters::default();
        assert!(s.put(key('a', 2, 2), vec![0; 15]).is_err(), "wrong size");
        assert!(
            s.put(key('A', 2, 2), vec![0; 16]).is_err(),
            "not lowercase hex"
        );
        s.put(key('a', 2, 2), vec![1; 16]).unwrap();
        s.put(key('a', 4, 2), vec![2; 32]).unwrap();
        s.put(key('b', 2, 2), vec![3; 16]).unwrap();
        assert_eq!((s.len(), s.bytes()), (3, 64));
        assert_eq!(s.get(&key('a', 4, 2)).unwrap()[0], 2);
        // Least recently used first: b, then a 2×2 (a 4×2 was just read),
        // once a later frame began.
        s.get(&key('b', 2, 2));
        s.set_budget(48);
        assert_eq!(s.len(), 3, "the working set stays");
        s.begin_frame();
        s.set_budget(48);
        assert!(!s.contains(&key('a', 2, 2)));
        assert!(s.contains(&key('a', 4, 2)) && s.contains(&key('b', 2, 2)));
        s.forget(Some(&"a".repeat(64)));
        assert_eq!((s.len(), s.bytes()), (1, 16));
        s.forget(None);
        assert!(s.is_empty() && s.bytes() == 0);
    }

    #[test]
    fn a_frames_rasters_stay_even_over_the_budget() {
        let mut s = SvgRasters::default();
        s.set_budget(40);
        // One raster over the budget on its own is kept (today keeps the
        // raster it just drew), as are several a frame needs together.
        s.put(key('a', 4, 4), vec![1; 64]).unwrap();
        assert!(s.contains(&key('a', 4, 4)));
        s.put(key('b', 4, 4), vec![2; 64]).unwrap();
        assert!(s.contains(&key('a', 4, 4)) && s.contains(&key('b', 4, 4)));
        // The next frame draws only b: a goes when room is needed.
        s.begin_frame();
        assert!(s.get(&key('b', 4, 4)).is_some());
        s.put(key('c', 2, 2), vec![3; 16]).unwrap();
        assert!(!s.contains(&key('a', 4, 4)));
        assert!(s.contains(&key('b', 4, 4)) && s.contains(&key('c', 2, 2)));
        assert_eq!(s.bytes(), 80);
        // A frame that reads neither lets both go.
        s.begin_frame();
        s.begin_frame();
        s.evict(40);
        assert!(s.bytes() <= 40, "{}", s.bytes());
    }

    #[test]
    fn the_latest_bucket_of_a_content_substitutes_for_another() {
        let mut s = SvgRasters::default();
        assert!(s.latest(&key('a', 2, 2)).is_none());
        s.drawn(&key('a', 2, 2));
        assert_eq!(s.latest(&key('a', 4, 4)), Some(&key('a', 2, 2)));
        assert!(s.latest(&key('a', 2, 2)).is_none(), "never itself");
        assert!(s.latest(&key('b', 2, 2)).is_none(), "never another content");
        s.forget(Some(&"a".repeat(64)));
        assert!(s.latest(&key('a', 4, 4)).is_none());
    }

    #[test]
    fn raster_directories_read_captures_and_list_what_is_missing() {
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                std::fs::remove_dir_all(&self.0).unwrap();
            }
        }
        let dir = std::env::temp_dir().join(format!("pm-svg-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let _cleanup = Cleanup(dir.clone());
        let rasters = RasterDir::new(&dir);
        let need = |k: SvgRasterKey| SvgRasterNeed {
            request: SvgRasterRequest {
                key: k,
                asset: "a".into(),
            },
            path: "/art.svg".into(),
            composition: "c".into(),
            layer: "l".into(),
        };
        std::fs::write(rasters.file(&key('a', 2, 1)), [7u8; 8]).unwrap();
        assert_eq!(rasters.fetch(&need(key('a', 2, 1))).unwrap(), vec![7; 8]);
        std::fs::write(rasters.file(&key('a', 3, 1)), [7u8; 8]).unwrap();
        assert!(
            rasters
                .fetch(&need(key('a', 3, 1)))
                .unwrap_err()
                .contains("8 bytes")
        );
        let e = rasters.fetch(&need(key('b', 4, 4))).unwrap_err();
        assert!(e.contains("was not captured"), "{e}");
        assert_eq!(rasters.write_needs().unwrap(), 1);
        // Merged with what an earlier run listed.
        let again = RasterDir::new(&dir);
        let _ = again.fetch(&need(key('c', 1, 1)));
        assert_eq!(again.write_needs().unwrap(), 2);
        let text = std::fs::read_to_string(dir.join("needs.json")).unwrap();
        assert!(text.contains("/art.svg") && text.contains(&"b".repeat(64)));
    }

    #[test]
    fn misses_are_listed_once_per_frame() {
        let mut s = SvgRasters::default();
        let need = |k: SvgRasterKey| SvgRasterNeed {
            request: SvgRasterRequest {
                key: k,
                asset: "a1".into(),
            },
            path: "/x.svg".into(),
            composition: "c".into(),
            layer: "l".into(),
        };
        s.miss(need(key('a', 2, 2)));
        s.miss(need(key('a', 2, 2)));
        s.miss(need(key('a', 3, 2)));
        assert!(s.missed());
        assert_eq!(s.take_misses().len(), 2);
        assert!(!s.missed());
    }

    #[test]
    fn a_failed_bucket_is_waited_for_only_when_failures_are_not_skipped() {
        let mut s = SvgRasters::default();
        let need = |k: SvgRasterKey| SvgRasterNeed {
            request: SvgRasterRequest {
                key: k,
                asset: "a1".into(),
            },
            path: "/x.svg".into(),
            composition: "c".into(),
            layer: "l".into(),
        };
        s.fail(key('a', 2, 2));
        s.miss(need(key('a', 2, 2)));
        assert!(s.missed_waiting(false), "snapshots and exports wait");
        assert!(!s.missed_waiting(true), "the viewer draws without it");
        s.miss(need(key('a', 3, 2)));
        assert!(s.missed_waiting(true), "another bucket is still waited for");
        s.take_misses();
        // Its raster arriving ends the failure; forgetting its content too.
        s.put(key('a', 2, 2), vec![0; 16]).unwrap();
        assert!(!s.is_failed(&key('a', 2, 2)));
        s.fail(key('a', 3, 2));
        s.fail(key('b', 3, 2));
        s.forget(Some(&"a".repeat(64)));
        assert!(!s.is_failed(&key('a', 3, 2)) && s.is_failed(&key('b', 3, 2)));
        s.forget(None);
        assert!(!s.is_failed(&key('b', 3, 2)));
    }
}
