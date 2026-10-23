//! CPU canvas pixels keyed only by the inputs to the exact rasterizer.
//!
//! GPU textures may be evicted between seeks. Keep their expensive CPU
//! source within a separate bound, also shared with speculative workers
//! and the layer-raster API used by extension wrappers. No time, document
//! identity or output precision is needed: those do not change these bytes.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::VectorSource;

pub(super) type Pixels = (u32, u32, Arc<Vec<u8>>);
const MAX_BYTES: usize = 128 << 20;
const MAX_ENTRIES: usize = 4096;

#[derive(Default)]
pub(super) struct Rasters {
    state: Mutex<State>,
}

struct State {
    entries: HashMap<[u8; 32], (Pixels, u64)>,
    bytes: usize,
    tick: u64,
    limit: usize,
}

impl Default for State {
    fn default() -> Self {
        Self { entries: HashMap::new(), bytes: 0, tick: 0, limit: MAX_BYTES }
    }
}

impl State {
    fn trim(&mut self, budget: usize) {
        while self.bytes > budget || self.entries.len() > MAX_ENTRIES {
            let Some(key) = self
                .entries
                .iter()
                .min_by_key(|(_, (_, used))| used)
                .map(|(k, _)| *k)
            else {
                break;
            };
            if let Some(((.., data), _)) = self.entries.remove(&key) {
                self.bytes -= data.len();
            }
        }
    }
}

impl Rasters {
    pub fn get(&self, source: &VectorSource) -> Result<Pixels, String> {
        let key = source.key();
        {
            let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            st.tick += 1;
            let tick = st.tick;
            if let Some((pixels, used)) = st.entries.get_mut(&key) {
                *used = tick;
                pm_perf::bump(pm_perf::Counter::CanvasCacheHits);
                return Ok(pixels.clone());
            }
        }
        pm_perf::bump(pm_perf::Counter::CanvasCacheMisses);
        let bitmap = match source {
            VectorSource::Shape {
                content,
                colors,
                scale,
                crop,
            } => pm_vector::canvas2d::shape::raster_shape_window(content, colors, *scale, *crop),
            VectorSource::Paths {
                nodes,
                colors,
                scale,
                crop,
            } => {
                let layer = pm_vector::path::PathLayer::new(nodes);
                pm_vector::canvas2d::shape::raster_paths_window(&layer, colors, *scale, *crop)
                    .map(|(b, _)| b)
            }
            _ => return Err("not a level 1 raster".into()),
        }
        .map_err(|e| e.to_string())?;
        let pixels = (bitmap.width, bitmap.height, Arc::new(bitmap.rgba));
        {
            let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if pixels.2.len() > st.limit { return Ok(pixels); }
            let tick = st.tick;
            if let Some((old, _)) = st.entries.insert(key, (pixels.clone(), tick)) {
                st.bytes -= old.2.len();
            }
            st.bytes += pixels.2.len();
            let limit = st.limit;
            st.trim(limit);
        }
        Ok(pixels)
    }

    pub fn bytes(&self) -> usize {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).bytes
    }

    pub fn trim(&self, budget: usize) {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.limit = budget.min(MAX_BYTES);
        let limit = st.limit;
        st.trim(limit);
    }

    pub fn begin_frame(&self) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).limit = MAX_BYTES;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_vector::canvas2d::shape::ShapeColors;
    use pm_vector::shape::{ShapeContent, ShapeKind};

    fn source(scale: f64, color: &str) -> VectorSource {
        VectorSource::Shape {
            content: ShapeContent {
                shape: ShapeKind::Rect,
                w: 31.0,
                h: 23.0,
                radius: 4.0,
                ..Default::default()
            },
            colors: ShapeColors {
                color: color.into(),
                stroke_color: "#000000".into(),
            },
            scale,
            crop: None,
        }
    }

    #[test]
    fn seeks_and_wrappers_share_exact_pixels_but_changed_inputs_miss() {
        let rasters = Rasters::default();
        let a = rasters.get(&source(1.0, "#FF8800")).unwrap();
        let b = rasters.get(&source(1.0, "#FF8800")).unwrap();
        assert!(Arc::ptr_eq(&a.2, &b.2));
        for s in [source(1.25, "#FF8800"), source(1.0, "#1188FF")] {
            let c = rasters.get(&s).unwrap();
            assert!(!Arc::ptr_eq(&a.2, &c.2));
            let uncached = Rasters::default().get(&s).unwrap();
            assert_eq!(*c.2, *uncached.2);
        }
        assert!(rasters.bytes() > a.2.len());
        rasters.trim(0);
        assert_eq!(rasters.bytes(), 0);
        let c = rasters.get(&source(1.0, "#FF8800")).unwrap();
        assert!(!Arc::ptr_eq(&a.2, &c.2));
        assert_eq!(*a.2, *c.2);
    }
}
