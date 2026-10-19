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
//! - **Preview** draws the content's most recent bucket in its place (the
//!   quad and UVs do not depend on the bucket), or nothing when it has
//!   none, and draws the frame again when the raster arrives: one frame at
//!   most, as today's asynchronous `<img>` decode shows nothing until it
//!   resolves.
//! - **Exports and snapshots** ([`SvgMissPolicy::Wait`]) never draw a
//!   substitute: the frame fails with `RenderError::SvgRastersPending` and
//!   the caller fetches the rasters and renders it again.
//! - **`pm-cli`** has no Chromium: [`SvgRasterizer::Resvg`] draws the bucket
//!   with resvg, an approximation it reports with a named notice.

use std::collections::HashMap;
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
    misses: Vec<SvgRasterNeed>,
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
            misses: Vec::new(),
            rasterizer: SvgRasterizer::default(),
            approximated: 0,
        }
    }
}

impl SvgRasters {
    /// Stores a raster's pixels (premultiplied RGBA8, top-down).
    pub fn put(&mut self, key: SvgRasterKey, data: Vec<u8>) -> Result<(), String> {
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

    /// Evicts least recently used rasters down to `budget` bytes.
    pub fn evict(&mut self, budget: u64) {
        if self.bytes <= budget {
            return;
        }
        let mut by_age: Vec<(u64, SvgRasterKey)> = self
            .stored
            .iter()
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
        // Least recently used first: b, then a 2×2 (a 4×2 was just read).
        s.get(&key('b', 2, 2));
        s.set_budget(48);
        assert!(!s.contains(&key('a', 2, 2)));
        assert!(s.contains(&key('a', 4, 2)) && s.contains(&key('b', 2, 2)));
        s.forget(Some(&"a".repeat(64)));
        assert_eq!((s.len(), s.bytes()), (1, 16));
        s.forget(None);
        assert!(s.is_empty() && s.bytes() == 0);
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
    fn misses_are_listed_once_per_frame() {
        let mut s = SvgRasters::default();
        let need = |k: SvgRasterKey| SvgRasterNeed {
            request: SvgRasterRequest {
                key: k,
                asset: "a1".into(),
            },
            path: "/x.svg".into(),
        };
        s.miss(need(key('a', 2, 2)));
        s.miss(need(key('a', 2, 2)));
        s.miss(need(key('a', 3, 2)));
        assert!(s.missed());
        assert_eq!(s.take_misses().len(), 2);
        assert!(!s.missed());
    }
}
