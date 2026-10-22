//! Bounded, tiled memory cache for large still previews. The key includes
//! file identity and requested representation; no tile can be served as an
//! original. Tiles are 256 square and retain native sample precision.

use super::{StillImage, read, read_reduced};
use crate::error::{MediaError, Result};
use crate::frame::{DecodedFrame, Layout, Pixels, PoolFrame, PoolPlane};
use crate::pool::{FramePool, ROW_ALIGN};
use std::collections::{BTreeMap, HashMap};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::SystemTime;

pub const TILE_EDGE: usize = 256;
/// Covers bilinear's adjacent texel and the paused viewer's Lanczos-3 taps.
pub const TILE_GUTTER: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Key {
    path: PathBuf,
    len: u64,
    device: u64,
    inode: u64,
    changed: (i64, i64),
    modified: Option<SystemTime>,
    max_dimension: u32,
    x: u32,
    y: u32,
    gutter: usize,
}

/// One uploaded tile, including replicated neighbor pixels around its unique
/// interior. `origin` and `content_size` use the reduced image's pixel grid.
#[derive(Clone)]
pub struct VisibleTile {
    pub image: StillImage,
    pub full_size: (usize, usize),
    pub origin: (usize, usize),
    pub content_size: (usize, usize),
    pub content_offset: (usize, usize),
}

struct Entry {
    image: StillImage,
    stamp: u64,
    bytes: usize,
}

#[derive(Default)]
struct State {
    entries: HashMap<Key, Entry>,
    sizes: HashMap<Key, (usize, usize)>,
    order: BTreeMap<u64, Key>,
    clock: u64,
    bytes: usize,
}

pub struct TiledMipCache {
    budget: AtomicUsize,
    state: Mutex<State>,
}

impl TiledMipCache {
    pub fn new(budget_bytes: usize) -> Self {
        Self {
            budget: AtomicUsize::new(budget_bytes),
            state: Mutex::new(State::default()),
        }
    }

    /// Returns one tile in reduced-image pixels. An uncached miss decodes the
    /// requested representation and retains only this tile.
    pub fn tile(
        &self,
        path: &Path,
        pool: &FramePool,
        max_dimension: u32,
        x: u32,
        y: u32,
    ) -> Result<StillImage> {
        let meta = std::fs::metadata(path)?;
        let key = Key {
            path: path.to_path_buf(),
            len: meta.len(),
            device: meta.dev(),
            inode: meta.ino(),
            changed: (meta.ctime(), meta.ctime_nsec()),
            modified: meta.modified().ok(),
            max_dimension,
            x,
            y,
            gutter: 0,
        };
        if let Some(hit) = self.get(&key) {
            return Ok(hit);
        }
        let image = representation(path, pool, max_dimension)?;
        let w = image.frame.format.width;
        let h = image.frame.format.height;
        if x as usize * TILE_EDGE >= w || y as usize * TILE_EDGE >= h {
            return Err(MediaError::invalid("still tile outside reduced image"));
        }
        let tile = split_tile(&image, pool, x as usize, y as usize, 0)?.image;
        self.insert(key, tile.clone());
        Ok(tile)
    }

    /// The tiles whose interiors intersect a normalized source-UV rectangle.
    /// A cache hit avoids even the reduced decode. Misses share one decode and
    /// populate only the tiles this view can draw; nothing is written to disk.
    pub fn visible_tiles(
        &self,
        path: &Path,
        pool: &FramePool,
        max_dimension: u32,
        uv_bounds: [f64; 4],
    ) -> Result<Vec<VisibleTile>> {
        if !uv_bounds.iter().all(|v| v.is_finite()) {
            return Err(MediaError::invalid("non-finite still tile bounds"));
        }
        let meta = std::fs::metadata(path)?;
        let base = Key {
            path: path.to_path_buf(), len: meta.len(), device: meta.dev(), inode: meta.ino(),
            changed: (meta.ctime(), meta.ctime_nsec()), modified: meta.modified().ok(),
            max_dimension, x: 0, y: 0, gutter: TILE_GUTTER,
        };
        let mut decoded = None;
        let cached_size = { self.state.lock().unwrap().sizes.get(&base).copied() };
        let size = match cached_size {
            Some(size) => size,
            None => {
                let image = representation(path, pool, max_dimension)?;
                let size = (image.frame.format.width, image.frame.format.height);
                let mut state = self.state.lock().unwrap();
                // Dimensions are cheap to re-read and must not become an
                // unbounded side cache as files or viewer sizes change.
                if state.sizes.len() >= 128 { state.sizes.clear(); }
                state.sizes.insert(base.clone(), size);
                decoded = Some(image);
                size
            }
        };
        let x0 = uv_bounds[0].clamp(0.0, 1.0);
        let y0 = uv_bounds[1].clamp(0.0, 1.0);
        let x1 = uv_bounds[2].clamp(0.0, 1.0);
        let y1 = uv_bounds[3].clamp(0.0, 1.0);
        if x1 <= x0 || y1 <= y0 { return Ok(Vec::new()); }
        let from_x = (x0 * size.0 as f64).floor() as usize / TILE_EDGE;
        let from_y = (y0 * size.1 as f64).floor() as usize / TILE_EDGE;
        let to_x = ((x1 * size.0 as f64).ceil() as usize).saturating_sub(1) / TILE_EDGE;
        let to_y = ((y1 * size.1 as f64).ceil() as usize).saturating_sub(1) / TILE_EDGE;
        let mut out = Vec::new();
        for ty in from_y..=to_y {
            for tx in from_x..=to_x {
                let mut key = base.clone(); key.x = tx as u32; key.y = ty as u32;
                let image = match self.get(&key) {
                    Some(hit) => hit,
                    None => {
                        if decoded.is_none() { decoded = Some(representation(path, pool, max_dimension)?); }
                        let tile = split_tile(decoded.as_ref().unwrap(), pool, tx, ty, TILE_GUTTER)?;
                        self.insert(key, tile.image.clone());
                        tile.image
                    }
                };
                let origin = (tx * TILE_EDGE, ty * TILE_EDGE);
                let content_size = ((size.0 - origin.0).min(TILE_EDGE), (size.1 - origin.1).min(TILE_EDGE));
                let content_offset = (origin.0.min(TILE_GUTTER), origin.1.min(TILE_GUTTER));
                out.push(VisibleTile { image, full_size: size, origin, content_size, content_offset });
            }
        }
        Ok(out)
    }

    fn get(&self, key: &Key) -> Option<StillImage> {
        let mut s = self.state.lock().unwrap();
        s.clock += 1;
        let stamp = s.clock;
        let entry = s.entries.get_mut(key)?;
        let old = std::mem::replace(&mut entry.stamp, stamp);
        let image = entry.image.clone();
        s.order.remove(&old);
        s.order.insert(stamp, key.clone());
        Some(image)
    }

    fn insert(&self, key: Key, image: StillImage) {
        let bytes = image.frame.bytes();
        let budget = self.budget.load(Ordering::Relaxed);
        if bytes > budget {
            return;
        }
        let mut s = self.state.lock().unwrap();
        s.clock += 1;
        let stamp = s.clock;
        if let Some(old) = s.entries.remove(&key) {
            s.order.remove(&old.stamp);
            s.bytes -= old.bytes;
        }
        s.entries.insert(
            key.clone(),
            Entry {
                image,
                stamp,
                bytes,
            },
        );
        s.order.insert(stamp, key);
        s.bytes += bytes;
        while s.bytes > budget {
            let Some((_, oldest)) = s.order.pop_first() else {
                break;
            };
            if let Some(old) = s.entries.remove(&oldest) {
                s.bytes -= old.bytes;
            }
        }
    }

    pub fn bytes(&self) -> usize {
        self.state.lock().unwrap().bytes
    }

    pub fn set_budget(&self, budget: usize) {
        self.budget.store(budget, Ordering::Relaxed);
        let mut s = self.state.lock().unwrap();
        while s.bytes > budget {
            let Some((_, oldest)) = s.order.pop_first() else { break };
            if let Some(old) = s.entries.remove(&oldest) { s.bytes -= old.bytes; }
        }
    }
}

fn representation(path: &Path, pool: &FramePool, max_dimension: u32) -> Result<StillImage> {
    let (width, height) = super::dimensions(path)?;
    if width.max(height) <= max_dimension { read(path, pool) }
    else { read_reduced(path, pool, max_dimension) }
}

fn split_tile(image: &StillImage, pool: &FramePool, tx: usize, ty: usize, gutter: usize) -> Result<VisibleTile> {
    let src = &image.frame;
    if src.format.layout != Layout::Rgba || src.format.planes.len() != 1 {
        return Err(MediaError::unsupported(
            "tiled mip cache requires packed RGBA",
        ));
    }
    let px = src.format.planes[0].format.bytes();
    let x = tx * TILE_EDGE;
    let y = ty * TILE_EDGE;
    let inner_w = (src.format.width - x).min(TILE_EDGE);
    let inner_h = (src.format.height - y).min(TILE_EDGE);
    let left = x.saturating_sub(gutter);
    let top = y.saturating_sub(gutter);
    let w = (x + inner_w + gutter).min(src.format.width) - left;
    let h = (y + inner_h + gutter).min(src.format.height) - top;
    let stride = (w * px).div_ceil(ROW_ALIGN) * ROW_ALIGN;
    let slot = pool.acquire(stride * h)?;
    src.with_planes(|planes| {
        for row in 0..h {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    planes[0].ptr.add((top + row) * planes[0].stride + left * px),
                    slot.ptr().add(row * stride),
                    w * px,
                );
            }
        }
    })?;
    let mut format = (*src.format).clone();
    format.width = w;
    format.height = h;
    format.planes[0].width = w;
    format.planes[0].height = h;
    let mut out = image.clone();
    out.frame = DecodedFrame {
        pts: src.pts,
        format: std::sync::Arc::new(format),
        pixels: Pixels::Pool(std::sync::Arc::new(PoolFrame {
            planes: vec![PoolPlane {
                decoder_ptr: slot.ptr() as usize,
                slot,
                offset: 0,
                stride,
            }],
            owner: None,
            copied: true,
        })),
        color: src.color,
    };
    Ok(VisibleTile {
        image: out,
        full_size: (src.format.width, src.format.height),
        origin: (x, y),
        content_size: (inner_w, inner_h),
        content_offset: (x - left, y - top),
    })
}
