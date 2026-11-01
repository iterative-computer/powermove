use crate::{Error, Result};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// Host-assigned source identity. Content under a key is immutable for its entire lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceKey {
    pub id: u64,
    pub version: u64,
}

/// Straight-alpha sRGB/unorm8, row-major, top-left origin. No platform handles.
#[derive(Debug, Clone)]
pub struct ImageData {
    width: u32,
    height: u32,
    bytes: Vec<u8>,
    pub(crate) canonical: bool,
}
impl PartialEq for ImageData {
    fn eq(&self, other: &Self) -> bool {
        self.width == other.width && self.height == other.height && self.bytes == other.bytes
    }
}
impl Eq for ImageData {}
impl ImageData {
    pub fn new(width: u32, height: u32) -> Result<Self> {
        if width == 0 || height == 0 {
            return Err(Error::IndexSize);
        }
        Ok(Self {
            width,
            height,
            bytes: vec![0; byte_len(width, height)?],
            canonical: true,
        })
    }
    pub fn from_rgba(width: u32, height: u32, bytes: Vec<u8>) -> Result<Self> {
        if width == 0 || height == 0 || bytes.len() != byte_len(width, height)? {
            return Err(Error::IndexSize);
        }
        let canonical = bytes
            .chunks_exact(4)
            .all(|p| p[3] != 0 || p[..3] == [0, 0, 0]);
        Ok(Self {
            width,
            height,
            bytes,
            canonical,
        })
    }
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        self.canonical = false;
        &mut self.bytes
    }
    pub(crate) fn pixel(&self, x: i64, y: i64) -> [f32; 4] {
        if x < 0 || y < 0 || x >= self.width as i64 || y >= self.height as i64 {
            return [0.; 4];
        }
        let i = (y as usize * self.width as usize + x as usize) * 4;
        let a = self.bytes[i + 3] as f32 / 255.;
        [
            self.bytes[i] as f32 / 255. * a,
            self.bytes[i + 1] as f32 / 255. * a,
            self.bytes[i + 2] as f32 / 255. * a,
            a,
        ]
    }
}
pub(crate) fn byte_len(w: u32, h: u32) -> Result<usize> {
    let n = u64::from(w) * u64::from(h);
    // Admission limit, not a silent resolution reduction. Host can choose a tiled backend later.
    if n > 16_777_216 {
        return Err(Error::Capacity);
    }
    usize::try_from(n * 4).map_err(|_| Error::Capacity)
}

#[derive(Debug, Clone)]
pub struct Source {
    pub key: SourceKey,
    pub pixels: Arc<ImageData>,
}
#[derive(Debug, Default)]
pub struct Sources {
    versions: BTreeMap<SourceKey, Arc<ImageData>>,
    retired: BTreeSet<SourceKey>,
}
impl Sources {
    pub fn insert(&mut self, key: SourceKey, pixels: ImageData) -> Result<Source> {
        if self.retired.contains(&key) {
            return Err(Error::VersionConflict);
        }
        if let Some(old) = self.versions.get(&key) {
            if **old != pixels {
                return Err(Error::VersionConflict);
            }
            return Ok(Source {
                key,
                pixels: old.clone(),
            });
        }
        let pixels = Arc::new(pixels);
        self.versions.insert(key, pixels.clone());
        Ok(Source { key, pixels })
    }
    pub fn get(&self, key: SourceKey) -> Result<Source> {
        Ok(Source {
            key,
            pixels: self.versions.get(&key).ok_or(Error::MissingSource)?.clone(),
        })
    }
    /// Retiring the registry entry never invalidates a recorded command or held pattern.
    pub fn retire(&mut self, key: SourceKey) {
        self.versions.remove(&key);
        self.retired.insert(key);
    }
}
