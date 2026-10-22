//! Bounded, tiled memory cache for large still previews. The key includes
//! file identity and requested representation; no tile can be served as an
//! original. Tiles are 256 square and retain native sample precision.

use super::{StillImage, read_reduced};
use crate::error::{MediaError, Result};
use crate::frame::{DecodedFrame, Layout, Pixels, PoolFrame, PoolPlane};
use crate::pool::{FramePool, ROW_ALIGN};
use std::collections::{BTreeMap, HashMap};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

pub const TILE_EDGE: usize = 256;

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
}

struct Entry {
    image: StillImage,
    stamp: u64,
    bytes: usize,
}

#[derive(Default)]
struct State {
    entries: HashMap<Key, Entry>,
    order: BTreeMap<u64, Key>,
    clock: u64,
    bytes: usize,
}

pub struct TiledMipCache {
    budget: usize,
    state: Mutex<State>,
}

impl TiledMipCache {
    pub fn new(budget_bytes: usize) -> Self {
        Self {
            budget: budget_bytes,
            state: Mutex::new(State::default()),
        }
    }

    /// Returns tile coordinates in reduced-image pixels. An uncached miss
    /// decodes the requested resolution once, populating all fitting tiles.
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
        };
        if let Some(hit) = self.get(&key) {
            return Ok(hit);
        }
        let image = read_reduced(path, pool, max_dimension)?;
        let w = image.frame.format.width;
        let h = image.frame.format.height;
        if x as usize * TILE_EDGE >= w || y as usize * TILE_EDGE >= h {
            return Err(MediaError::invalid("still tile outside reduced image"));
        }
        let mut requested = None;
        for ty in 0..h.div_ceil(TILE_EDGE) {
            for tx in 0..w.div_ceil(TILE_EDGE) {
                let tile = split_tile(&image, pool, tx, ty)?;
                if tx == x as usize && ty == y as usize {
                    requested = Some(tile.clone());
                }
                let mut tile_key = key.clone();
                tile_key.x = tx as u32;
                tile_key.y = ty as u32;
                self.insert(tile_key, tile);
            }
        }
        requested.ok_or_else(|| MediaError::invalid("still tile missing"))
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
        if bytes > self.budget {
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
        while s.bytes > self.budget {
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
}

fn split_tile(image: &StillImage, pool: &FramePool, tx: usize, ty: usize) -> Result<StillImage> {
    let src = &image.frame;
    if src.format.layout != Layout::Rgba || src.format.planes.len() != 1 {
        return Err(MediaError::unsupported(
            "tiled mip cache requires packed RGBA",
        ));
    }
    let px = src.format.planes[0].format.bytes();
    let x = tx * TILE_EDGE;
    let y = ty * TILE_EDGE;
    let w = (src.format.width - x).min(TILE_EDGE);
    let h = (src.format.height - y).min(TILE_EDGE);
    let stride = (w * px).div_ceil(ROW_ALIGN) * ROW_ALIGN;
    let slot = pool.acquire(stride * h)?;
    src.with_planes(|planes| {
        for row in 0..h {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    planes[0].ptr.add((y + row) * planes[0].stride + x * px),
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
    Ok(out)
}
