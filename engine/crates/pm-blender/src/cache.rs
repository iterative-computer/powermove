//! The Blender surface cache (section 14.7: "Blender surfaces", GPU memory
//! and disk; contract 8's `blenderSurfaces` lease category; contract 11
//! invariant 6).
//!
//! Decoded surfaces live in memory under a byte budget (the lease), least
//! recently used first out; their PNGs live on disk under the key's hex
//! name, so a surface evicted from memory is decoded again, never rendered
//! again. The disk area has its own bound, oldest files first out.
//!
//! The cache also remembers which device Blender resolved for a (version,
//! engine, requested device): `auto` is machine-dependent (appendix A
//! item 33), and the device is part of every key, so the first job on a
//! machine learns it and later jobs key their lookups with it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pm_protocol::blender::{BlenderSettings, ResolvedDevice};
use pm_protocol::common::ContentHash;

use crate::key::hex;
use crate::surface::{DecodedSurface, SurfaceError, decode_surface};

/// Default disk bound.
pub const DEFAULT_DISK_BYTES: u64 = 8 << 30;

struct Entry {
    surface: Arc<DecodedSurface>,
    used: u64,
}

/// Hit and miss counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub memory_hits: u64,
    pub disk_hits: u64,
    pub misses: u64,
    pub inserted: u64,
    pub evicted: u64,
}

pub struct SurfaceCache {
    memory: HashMap<[u8; 32], Entry>,
    bytes: u64,
    budget: u64,
    tick: u64,
    disk: Option<PathBuf>,
    disk_budget: u64,
    devices: HashMap<String, ResolvedDevice>,
    pub stats: CacheStats,
}

fn device_slot(version: &str, settings: &BlenderSettings) -> String {
    format!("{version}|{:?}|{:?}", settings.engine, settings.device)
}

impl SurfaceCache {
    /// A cache holding at most `budget` bytes of decoded surfaces, with PNGs
    /// in `disk` when given.
    pub fn new(budget: u64, disk: Option<PathBuf>) -> SurfaceCache {
        let mut c = SurfaceCache {
            memory: HashMap::new(),
            bytes: 0,
            budget,
            tick: 0,
            disk,
            disk_budget: DEFAULT_DISK_BYTES,
            devices: HashMap::new(),
            stats: CacheStats::default(),
        };
        if let Some(d) = &c.disk {
            let _ = crate::supervisor::create_private_dir(d);
            if let Ok(text) = std::fs::read(d.join("devices.json"))
                && let Ok(map) = serde_json::from_slice::<HashMap<String, ResolvedDevice>>(&text)
            {
                c.devices = map;
            }
        }
        c
    }

    pub fn set_disk_budget(&mut self, bytes: u64) {
        self.disk_budget = bytes;
    }

    /// Bytes of decoded surfaces in memory.
    pub fn usage(&self) -> u64 {
        self.bytes
    }

    pub fn len(&self) -> usize {
        self.memory.len()
    }

    pub fn is_empty(&self) -> bool {
        self.memory.is_empty()
    }

    /// Changes the memory budget (a lease grant), evicting as needed.
    pub fn set_budget(&mut self, bytes: u64) {
        self.budget = bytes;
        self.evict(None);
    }

    fn evict(&mut self, keep: Option<[u8; 32]>) {
        while self.bytes > self.budget && self.memory.len() > usize::from(keep.is_some()) {
            let Some(oldest) = self
                .memory
                .iter()
                .filter(|(k, _)| Some(**k) != keep)
                .min_by_key(|(_, e)| e.used)
                .map(|(k, _)| *k)
            else {
                break;
            };
            if let Some(e) = self.memory.remove(&oldest) {
                self.bytes -= e.surface.bytes();
                self.stats.evicted += 1;
            }
        }
    }

    fn disk_path(&self, key: &ContentHash) -> Option<PathBuf> {
        self.disk
            .as_ref()
            .map(|d| d.join(format!("{}.png", hex(key))))
    }

    fn remember(&mut self, key: &ContentHash, surface: Arc<DecodedSurface>) {
        self.tick += 1;
        if let Some(old) = self.memory.insert(
            key.0,
            Entry {
                surface: surface.clone(),
                used: self.tick,
            },
        ) {
            self.bytes -= old.surface.bytes();
        }
        self.bytes += surface.bytes();
        self.evict(Some(key.0));
    }

    /// The surface under `key`, from memory or decoded from disk.
    pub fn get(&mut self, key: &ContentHash) -> Option<Arc<DecodedSurface>> {
        self.tick += 1;
        if let Some(e) = self.memory.get_mut(&key.0) {
            e.used = self.tick;
            self.stats.memory_hits += 1;
            return Some(e.surface.clone());
        }
        if let Some(path) = self.disk_path(key)
            && let Ok(bytes) = std::fs::read(&path)
            && let Ok(s) = decode_surface(&bytes)
        {
            self.stats.disk_hits += 1;
            let s = Arc::new(s);
            self.remember(key, s.clone());
            return Some(s);
        }
        self.stats.misses += 1;
        None
    }

    /// Whether `key` is cached (memory or disk) without decoding it.
    pub fn contains(&self, key: &ContentHash) -> bool {
        self.memory.contains_key(&key.0) || self.disk_path(key).is_some_and(|p| p.exists())
    }

    /// Adds a surface rendered into `png` (moved into the disk area when the
    /// cache has one), decoded once.
    pub fn insert_file(
        &mut self,
        key: &ContentHash,
        png: &Path,
    ) -> Result<Arc<DecodedSurface>, SurfaceError> {
        let bytes = std::fs::read(png).map_err(|e| SurfaceError::Decode(e.to_string()))?;
        let surface = Arc::new(decode_surface(&bytes)?);
        if let Some(target) = self.disk_path(key) {
            if std::fs::rename(png, &target).is_err() {
                let _ = std::fs::write(&target, &bytes);
            }
            self.prune_disk();
        }
        self.stats.inserted += 1;
        self.remember(key, surface.clone());
        Ok(surface)
    }

    /// Adds a decoded surface with its PNG bytes.
    pub fn insert_bytes(
        &mut self,
        key: &ContentHash,
        png: &[u8],
    ) -> Result<Arc<DecodedSurface>, SurfaceError> {
        let surface = Arc::new(decode_surface(png)?);
        if let Some(target) = self.disk_path(key) {
            let tmp = target.with_extension(format!("{}.part", std::process::id()));
            if std::fs::write(&tmp, png).is_ok() {
                let _ = std::fs::rename(&tmp, &target);
            }
            self.prune_disk();
        }
        self.stats.inserted += 1;
        self.remember(key, surface.clone());
        Ok(surface)
    }

    /// Removes the oldest PNGs while the disk area exceeds its bound.
    fn prune_disk(&mut self) {
        let Some(dir) = &self.disk else { return };
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = entries
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "png"))
            .filter_map(|e| {
                let m = e.metadata().ok()?;
                Some((m.modified().ok()?, m.len(), e.path()))
            })
            .collect();
        let mut total: u64 = files.iter().map(|f| f.1).sum();
        if total <= self.disk_budget {
            return;
        }
        files.sort();
        for (_, len, path) in files {
            if total <= self.disk_budget {
                break;
            }
            if std::fs::remove_file(&path).is_ok() {
                total -= len;
            }
        }
    }

    /// The device Blender resolved for these settings on this machine, if
    /// a job has recorded it.
    pub fn resolved_device(
        &self,
        version: &str,
        settings: &BlenderSettings,
    ) -> Option<ResolvedDevice> {
        crate::key::predicted_device(settings)
            .or_else(|| self.devices.get(&device_slot(version, settings)).cloned())
    }

    /// Records the device a job resolved (kept on disk with the surfaces).
    pub fn record_device(
        &mut self,
        version: &str,
        settings: &BlenderSettings,
        device: &ResolvedDevice,
    ) {
        let slot = device_slot(version, settings);
        if self.devices.get(&slot) == Some(device) {
            return;
        }
        self.devices.insert(slot, device.clone());
        if let Some(d) = &self.disk
            && let Ok(text) = serde_json::to_vec(&self.devices)
        {
            let tmp = d.join(format!("devices.json.{}.part", std::process::id()));
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(tmp, d.join("devices.json"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::encode_rgba16;
    use pm_protocol::blender::{BlenderEngine, RequestedDevice};

    fn png(alpha: u16) -> Vec<u8> {
        encode_rgba16(4, 4, &[alpha; 64], &[])
    }

    #[test]
    fn memory_is_bounded_and_disk_keeps_evicted_surfaces() {
        let dir = std::env::temp_dir().join(format!(
            "pm-blender-cache-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        // Each 4 × 4 surface decodes to 256 bytes; budget for two.
        let mut c = SurfaceCache::new(512, Some(dir.clone()));
        let keys: Vec<ContentHash> = (1..=3u8).map(|n| ContentHash([n; 32])).collect();
        for (i, k) in keys.iter().enumerate() {
            c.insert_bytes(k, &png(1000 * (i as u16 + 1))).unwrap();
        }
        assert_eq!(c.len(), 2);
        assert!(c.usage() <= 512);
        // The first was evicted from memory but comes back from disk.
        let s = c.get(&keys[0]).unwrap();
        assert_eq!(s.pixel(0, 0)[3], 1000.0 / 65535.0);
        assert_eq!(c.stats.disk_hits, 1);
        assert!(c.get(&ContentHash([9; 32])).is_none());
        assert_eq!(c.stats.misses, 1);
        c.set_budget(0);
        assert!(c.len() <= 1);
        // Disk bound.
        c.set_disk_budget(0);
        c.insert_bytes(&ContentHash([4; 32]), &png(5)).unwrap();
        let pngs = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "png"))
            .count();
        assert_eq!(pngs, 0);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn resolved_devices_are_remembered_on_disk() {
        let dir = std::env::temp_dir().join(format!("pm-blender-devices-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let settings = BlenderSettings {
            engine: BlenderEngine::Cycles,
            samples: 16,
            preview: false,
            preview_scale: 0.5,
            denoise: true,
            device: RequestedDevice::Auto,
        };
        let metal = ResolvedDevice::Metal {
            name: "Apple M4 Max (GPU - 40 cores)".into(),
        };
        {
            let mut c = SurfaceCache::new(1 << 20, Some(dir.clone()));
            assert_eq!(c.resolved_device("4.5.14", &settings), None);
            c.record_device("4.5.14", &settings, &metal);
        }
        let c = SurfaceCache::new(1 << 20, Some(dir.clone()));
        assert_eq!(c.resolved_device("4.5.14", &settings), Some(metal));
        assert_eq!(c.resolved_device("4.5.15", &settings), None);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
