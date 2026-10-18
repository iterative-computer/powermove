//! Compatibility pixels on the presenter's render thread (the Phase 3c
//! render transport; plan section 20, the `services` `raster` override and
//! the compatibility WebGL2 context).
//!
//! A renderer uploads the pixels it drew for one layer at one frame (a
//! `raster` override's result, an extension's offscreen WebGL2
//! compatibility context) in row bands ([`CompatMessage::Pixels`]); the
//! store assembles each raster, keeps it under its [`CompatKey`] (the
//! producer's cache key: it changes whenever anything that decides the
//! pixels does), and bounds the memory the rasters take
//! ([`MAX_BYTES`], least recently used first). `Forget` drops a layer's
//! rasters at once.
//!
//! The renderer draws a layer from these through [`CompatStore::get`]. The
//! job engine gets the same pixels for export frames through host services
//! (`compatFrame`), never from this store.

use std::collections::{BTreeMap, HashMap};

use pm_protocol::common::{CompositionId, LayerId, Revision};
use pm_protocol::snapshot::{CompatFormat, CompatKey, CompatMessage};

use super::render::Key;

/// Most bytes of assembled and partial rasters one engine keeps.
pub const MAX_BYTES: u64 = 256 * 1024 * 1024;
/// Most rasters being assembled at once (bands still missing).
pub const MAX_PARTIAL: usize = 32;

/// One assembled raster.
#[derive(Debug, Clone, PartialEq)]
pub struct CompatRaster {
    pub width: u32,
    pub height: u32,
    pub format: CompatFormat,
    pub origin: [f64; 2],
    pub scale: f64,
    pub revision: Revision,
    /// Rows top-down, tightly packed.
    pub data: Vec<u8>,
}

#[derive(Debug)]
struct Partial {
    raster: CompatRaster,
    rows: Vec<bool>,
    missing: u32,
}

/// What a message did (counters for the engine's status).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    /// A band was stored; the raster still misses rows.
    Band,
    /// The raster is complete and usable.
    Complete,
    /// A band that did not match the raster it continues (another size or
    /// format under the same key): the partial raster starts over.
    Restarted,
    Forgot(usize),
}

/// The rasters of every session on this thread.
#[derive(Debug)]
pub struct CompatStore {
    complete: HashMap<(Key, CompatKey), (CompatRaster, u64)>,
    partial: HashMap<(Key, CompatKey), Partial>,
    clock: u64,
    bytes: u64,
    limit: u64,
}

impl Default for CompatStore {
    fn default() -> Self {
        Self::with_limit(MAX_BYTES)
    }
}

fn size(width: u32, height: u32, format: CompatFormat) -> u64 {
    u64::from(width) * u64::from(height) * format.bytes_per_pixel()
}

impl CompatStore {
    /// A store that keeps at most `limit` bytes.
    pub fn with_limit(limit: u64) -> Self {
        Self {
            complete: HashMap::new(),
            partial: HashMap::new(),
            clock: 0,
            bytes: 0,
            limit,
        }
    }

    /// Applies a renderer's message for session `key`. The message was
    /// validated by the protocol (band bounds and lengths).
    pub fn apply(&mut self, key: Key, message: CompatMessage) -> Applied {
        match message {
            CompatMessage::Forget { composition, layer } => {
                Applied::Forgot(self.forget_layer(key, &composition, &layer))
            }
            CompatMessage::Pixels(band) => {
                let id = (key, band.id.clone());
                let bytes = size(band.width, band.height, band.format);
                let mut restarted = false;
                if let Some(p) = self.partial.get(&id)
                    && (p.raster.width != band.width
                        || p.raster.height != band.height
                        || p.raster.format != band.format)
                {
                    let gone = self.partial.remove(&id).expect("present");
                    self.bytes -= size(gone.raster.width, gone.raster.height, gone.raster.format);
                    restarted = true;
                }
                if !self.partial.contains_key(&id) {
                    while self.partial.len() >= MAX_PARTIAL {
                        // The oldest unfinished raster is abandoned.
                        let oldest = self.partial.keys().next().cloned().expect("non-empty");
                        let gone = self.partial.remove(&oldest).expect("present");
                        self.bytes -=
                            size(gone.raster.width, gone.raster.height, gone.raster.format);
                    }
                    self.make_room(bytes);
                    self.bytes += bytes;
                    self.partial.insert(
                        id.clone(),
                        Partial {
                            raster: CompatRaster {
                                width: band.width,
                                height: band.height,
                                format: band.format,
                                origin: band.origin,
                                scale: band.scale,
                                revision: band.revision,
                                data: vec![0; bytes as usize],
                            },
                            rows: vec![false; band.height as usize],
                            missing: band.height,
                        },
                    );
                }
                let partial = self.partial.get_mut(&id).expect("inserted");
                let row_bytes = (u64::from(band.width) * band.format.bytes_per_pixel()) as usize;
                let start = band.row as usize * row_bytes;
                partial.raster.data[start..start + band.data.0.len()].copy_from_slice(&band.data.0);
                for r in band.row..band.row + band.rows {
                    let seen = &mut partial.rows[r as usize];
                    if !*seen {
                        *seen = true;
                        partial.missing -= 1;
                    }
                }
                if partial.missing > 0 {
                    return if restarted {
                        Applied::Restarted
                    } else {
                        Applied::Band
                    };
                }
                let done = self.partial.remove(&id).expect("present").raster;
                self.clock += 1;
                if let Some((old, _)) = self.complete.insert(id, (done, self.clock)) {
                    self.bytes -= size(old.width, old.height, old.format);
                }
                Applied::Complete
            }
        }
    }

    /// A complete raster, marked used.
    pub fn get(&mut self, key: Key, id: &CompatKey) -> Option<&CompatRaster> {
        self.clock += 1;
        let clock = self.clock;
        let entry = self.complete.get_mut(&(key, id.clone()))?;
        entry.1 = clock;
        Some(&entry.0)
    }

    /// Evicts least recently used complete rasters until `incoming` fits.
    fn make_room(&mut self, incoming: u64) {
        while self.bytes + incoming > self.limit && !self.complete.is_empty() {
            let oldest = self
                .complete
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(k, _)| k.clone())
                .expect("non-empty");
            let (gone, _) = self.complete.remove(&oldest).expect("present");
            self.bytes -= size(gone.width, gone.height, gone.format);
        }
    }

    fn forget_layer(&mut self, key: Key, composition: &CompositionId, layer: &LayerId) -> usize {
        let matches = |k: &(Key, CompatKey)| {
            k.0 == key && &k.1.composition == composition && &k.1.layer == layer
        };
        let complete: Vec<_> = self
            .complete
            .keys()
            .filter(|k| matches(k))
            .cloned()
            .collect();
        let partial: Vec<_> = self
            .partial
            .keys()
            .filter(|k| matches(k))
            .cloned()
            .collect();
        for k in &complete {
            let (gone, _) = self.complete.remove(k).expect("present");
            self.bytes -= size(gone.width, gone.height, gone.format);
        }
        for k in &partial {
            let gone = self.partial.remove(k).expect("present");
            self.bytes -= size(gone.raster.width, gone.raster.height, gone.raster.format);
        }
        complete.len() + partial.len()
    }

    /// Drops everything of a closed session.
    pub fn forget_session(&mut self, key: Key) {
        let complete: Vec<_> = self
            .complete
            .keys()
            .filter(|k| k.0 == key)
            .cloned()
            .collect();
        for k in complete {
            let (gone, _) = self.complete.remove(&k).expect("present");
            self.bytes -= size(gone.width, gone.height, gone.format);
        }
        let partial: Vec<_> = self
            .partial
            .keys()
            .filter(|k| k.0 == key)
            .cloned()
            .collect();
        for k in partial {
            let gone = self.partial.remove(&k).expect("present");
            self.bytes -= size(gone.raster.width, gone.raster.height, gone.raster.format);
        }
    }

    /// Complete rasters per session, and every byte held.
    pub fn usage(&self) -> (BTreeMap<String, usize>, u64) {
        let mut per = BTreeMap::new();
        for (key, _) in self.complete.keys() {
            *per.entry(format!("{}:{}", key.0.0, key.1.get()))
                .or_insert(0) += 1;
        }
        (per, self.bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_protocol::common::{Bytes, SessionId, Ticks};
    use pm_protocol::host::PortId;
    use pm_protocol::snapshot::{CompatPixels, CompatSource};

    fn session(n: u64) -> Key {
        (PortId(7), SessionId::new(n).unwrap())
    }

    fn id(layer: &str, key: &str) -> CompatKey {
        CompatKey {
            composition: "c".into(),
            layer: layer.into(),
            time: Ticks::ZERO,
            source: CompatSource::RasterOverride,
            key: key.into(),
        }
    }

    fn band(id: CompatKey, w: u32, h: u32, row: u32, rows: u32, fill: u8) -> CompatMessage {
        CompatMessage::Pixels(CompatPixels {
            id,
            revision: Revision::new(3).unwrap(),
            width: w,
            height: h,
            format: CompatFormat::Rgba8Premultiplied,
            origin: [-2.0, -3.0],
            scale: 1.0,
            row,
            rows,
            data: Bytes(vec![fill; (w * rows * 4) as usize]),
            size: None,
            uv: None,
        })
    }

    #[test]
    fn bands_assemble_in_any_order_into_one_raster() {
        let mut store = CompatStore::default();
        let k = id("L7", "a");
        assert_eq!(
            store.apply(session(1), band(k.clone(), 2, 4, 2, 2, 9)),
            Applied::Band
        );
        assert!(store.get(session(1), &k).is_none(), "not complete yet");
        assert_eq!(
            store.apply(session(1), band(k.clone(), 2, 4, 0, 2, 5)),
            Applied::Complete
        );
        let raster = store.get(session(1), &k).unwrap();
        assert_eq!(
            (raster.width, raster.height, raster.origin),
            (2, 4, [-2.0, -3.0])
        );
        assert_eq!(&raster.data[..16], &[5; 16]);
        assert_eq!(&raster.data[16..], &[9; 16]);
        // Another session's raster under the same key is another raster.
        assert!(store.get(session(2), &k).is_none());
        assert_eq!(store.usage().1, 32);
    }

    #[test]
    fn a_band_of_another_size_restarts_the_raster() {
        let mut store = CompatStore::default();
        let k = id("L7", "a");
        store.apply(session(1), band(k.clone(), 2, 4, 0, 1, 1));
        assert_eq!(
            store.apply(session(1), band(k.clone(), 4, 2, 0, 1, 1)),
            Applied::Restarted
        );
        assert_eq!(
            store.apply(session(1), band(k.clone(), 4, 2, 1, 1, 1)),
            Applied::Complete
        );
        assert_eq!(store.usage().1, 32);
    }

    #[test]
    fn forget_drops_a_layer_and_a_closed_session_drops_everything() {
        let mut store = CompatStore::default();
        store.apply(session(1), band(id("L7", "a"), 1, 1, 0, 1, 1));
        store.apply(session(1), band(id("L7", "b"), 1, 1, 0, 1, 1));
        store.apply(session(1), band(id("L8", "a"), 1, 1, 0, 1, 1));
        store.apply(session(1), band(id("L8", "c"), 1, 2, 0, 1, 1));
        assert_eq!(
            store.apply(
                session(1),
                CompatMessage::Forget {
                    composition: "c".into(),
                    layer: "L7".into()
                }
            ),
            Applied::Forgot(2)
        );
        assert_eq!(store.usage().0.values().sum::<usize>(), 1);
        store.forget_session(session(1));
        assert_eq!(store.usage(), (BTreeMap::new(), 0));
    }

    #[test]
    fn memory_is_bounded_least_recently_used_first() {
        // 1 KiB rasters (16 × 16 × 4) in a 4 KiB store.
        let mut store = CompatStore::with_limit(4096);
        let big = |name: &str| band(id(name, "k"), 16, 16, 0, 16, 0);
        for n in 0..4 {
            store.apply(session(1), big(&format!("L{n}")));
        }
        assert_eq!(store.usage().1, 4096);
        // L0 used recently: L1 goes first.
        assert!(store.get(session(1), &id("L0", "k")).is_some());
        store.apply(session(1), big("L4"));
        assert!(store.usage().1 <= 4096);
        assert!(store.get(session(1), &id("L1", "k")).is_none());
        assert!(store.get(session(1), &id("L0", "k")).is_some());
        assert!(store.get(session(1), &id("L4", "k")).is_some());
    }
}
