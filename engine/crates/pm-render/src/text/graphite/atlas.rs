//! Skia m152 RectanizerSkyline and A8 glyph-atlas placement. Atlas texel
//! coordinates are kept until the vertex shader divides by the atlas size.
//! Skia, Copyright Google LLC, BSD-3-Clause; see pm-skia-path/LICENSE.
use pm_text::render::sdf::SdfGlyph;
use std::sync::Arc;

const DIM: u32 = 2048;
const PLOT: u32 = 512;

#[derive(Clone)]
struct Segment {
    x: u32,
    y: u32,
    width: u32,
}
struct Skyline {
    segments: Vec<Segment>,
}
impl Skyline {
    fn new() -> Self {
        Self {
            segments: vec![Segment {
                x: 0,
                y: 0,
                width: PLOT,
            }],
        }
    }
    fn add(&mut self, width: u32, height: u32) -> Option<[u32; 2]> {
        if width > PLOT || height > PLOT {
            return None;
        }
        let mut best = None;
        let (mut best_width, mut best_y) = (PLOT + 1, PLOT + 1);
        for (i, segment) in self.segments.iter().enumerate() {
            if segment.x + width > PLOT {
                continue;
            }
            let (mut remaining, mut y, mut j) = (width, segment.y, i);
            while remaining > 0 {
                y = y.max(self.segments[j].y);
                if y + height > PLOT {
                    break;
                }
                remaining = remaining.saturating_sub(self.segments[j].width);
                j += 1;
            }
            if remaining == 0 && (y < best_y || (y == best_y && segment.width < best_width)) {
                best = Some(i);
                best_width = segment.width;
                best_y = y;
            }
        }
        let i = best?;
        let x = self.segments[i].x;
        self.segments.insert(
            i,
            Segment {
                x,
                y: best_y + height,
                width,
            },
        );
        let j = i + 1;
        while j < self.segments.len() {
            let right = self.segments[j - 1].x + self.segments[j - 1].width;
            if self.segments[j].x >= right {
                break;
            }
            let shrink = right - self.segments[j].x;
            if shrink >= self.segments[j].width {
                self.segments.remove(j);
            } else {
                self.segments[j].x += shrink;
                self.segments[j].width -= shrink;
                break;
            }
        }
        let mut j = 0;
        while j + 1 < self.segments.len() {
            if self.segments[j].y == self.segments[j + 1].y {
                let next = self.segments.remove(j + 1);
                self.segments[j].width += next.width;
            } else {
                j += 1;
            }
        }
        Some([x, best_y])
    }
}

struct Page {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    plots: Vec<Skyline>,
    order: Vec<usize>,
}
impl Page {
    fn new(device: &wgpu::Device) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Graphite A8 glyph atlas"),
            size: wgpu::Extent3d {
                width: DIM,
                height: DIM,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Self {
            texture,
            view,
            plots: (0..16).map(|_| Skyline::new()).collect(),
            order: (0..16).collect(),
        }
    }
    fn add(&mut self, width: u32, height: u32) -> Option<[u32; 2]> {
        for i in 0..self.order.len() {
            let plot = self.order[i];
            if let Some(mut pos) = self.plots[plot].add(width, height) {
                self.order.remove(i);
                self.order.insert(0, plot);
                pos[0] += (plot as u32 % 4) * PLOT;
                pos[1] += (plot as u32 / 4) * PLOT;
                return Some(pos);
            }
        }
        None
    }
}
struct Entry {
    field: Arc<SdfGlyph>,
    font_size: u32,
    page: usize,
    pos: [u32; 2],
}
struct MaskEntry {
    mask: Arc<pm_text::render::scene::GlyphMask>,
    paint: u64,
    pos: [u32; 2],
    page: usize,
}
pub(in crate::text) struct Atlas {
    pages: Vec<Page>,
    entries: Vec<Entry>,
    masks: Vec<MaskEntry>,
    touched: std::collections::BTreeSet<(usize, u32)>,
}
impl Atlas {
    pub fn new() -> Self {
        Self {
            pages: Vec::new(),
            entries: Vec::new(),
            masks: Vec::new(),
            touched: Default::default(),
        }
    }
    pub fn locate(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        field: &Arc<SdfGlyph>,
        font_size: u32,
    ) -> ([u32; 2], usize) {
        if let Some(e) = self
            .entries
            .iter()
            .find(|e| Arc::ptr_eq(&e.field, field) && e.font_size == font_size)
        {
            let location = (e.pos, e.page);
            self.touch(location.1, location.0);
            return location;
        }
        let (pos, page) = self.allocate(device, field.width, field.height);
        self.touch(page, pos);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.pages[page].texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: pos[0],
                    y: pos[1],
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            &field.field,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(field.width),
                rows_per_image: Some(field.height),
            },
            wgpu::Extent3d {
                width: field.width,
                height: field.height,
                depth_or_array_layers: 1,
            },
        );
        self.entries.push(Entry {
            field: field.clone(),
            font_size,
            page,
            pos,
        });
        (pos, page)
    }
    pub fn reserve_mask(
        &mut self,
        device: &wgpu::Device,
        mask: &Arc<pm_text::render::scene::GlyphMask>,
        paint: u64,
    ) {
        if let Some(e) = self
            .masks
            .iter()
            .find(|e| Arc::ptr_eq(&e.mask, mask) && e.paint == paint)
        {
            let (pos, page) = (e.pos, e.page);
            self.touch(page, pos);
            return;
        }
        let (pos, page) = self.allocate(device, mask.width, mask.height);
        self.touch(page, pos);
        self.masks.push(MaskEntry {
            mask: mask.clone(),
            paint,
            pos,
            page,
        });
    }
    pub fn begin_run(&mut self) {
        self.touched.clear();
    }
    fn touch(&mut self, page: usize, pos: [u32; 2]) {
        let plot = (pos[1] / PLOT) * 4 + pos[0] / PLOT;
        if self.touched.insert((page, plot)) {
            let order = &mut self.pages[page].order;
            let i = order.iter().position(|&p| p == plot as usize).unwrap();
            order.remove(i);
            order.insert(0, plot as usize);
        }
    }
    fn allocate(&mut self, device: &wgpu::Device, width: u32, height: u32) -> ([u32; 2], usize) {
        let mut found = None;
        for (i, page) in self.pages.iter_mut().enumerate() {
            if let Some(pos) = page.add(width, height) {
                found = Some((pos, i));
                break;
            }
        }
        found.unwrap_or_else(|| {
            if self.pages.len() == 4 {
                // A fresh generation preserves pending command buffers: an
                // upload never overwrites texels an encoded draw still reads.
                self.pages.clear();
                self.entries.clear();
                self.masks.clear();
                self.touched.clear();
            }
            let i = self.pages.len();
            let mut page = Page::new(device);
            let pos = page
                .add(width, height)
                .expect("SDF glyph fits a Skia A8 plot");
            self.pages.push(page);
            (pos, i)
        })
    }
    pub fn view(&self, index: usize) -> &wgpu::TextureView {
        &self.pages[if index < self.pages.len() { index } else { 0 }].view
    }
    pub const fn inverse_size() -> f32 {
        1. / DIM as f32
    }
}

#[cfg(test)]
mod tests {
    use super::Skyline;
    #[test]
    fn synthetic_allocations_match_pinned_rectanizer_skyline() {
        // Independently emitted by tools/graphite-oracle/atlas.cpp.
        let sizes = [
            [47, 61],
            [35, 96],
            [203, 97],
            [131, 171],
            [256, 256],
            [9, 190],
            [300, 128],
            [1, 512],
            [513, 1],
            [71, 37],
        ];
        let expected = [
            Some([0, 0]),
            Some([47, 0]),
            Some([82, 0]),
            Some([285, 0]),
            Some([0, 97]),
            Some([416, 0]),
            Some([0, 353]),
            Some([425, 0]),
            None,
            Some([426, 0]),
            Some([426, 37]),
            Some([473, 37]),
            None,
            None,
            None,
            Some([426, 98]),
            None,
            Some([508, 0]),
            None,
            Some([435, 133]),
            Some([435, 170]),
            Some([300, 171]),
            None,
            None,
            None,
            Some([482, 170]),
            None,
            Some([509, 0]),
            None,
            Some([335, 171]),
        ];
        let mut skyline = Skyline::new();
        for (i, position) in expected.into_iter().enumerate() {
            let [w, h] = sizes[i % sizes.len()];
            assert_eq!(skyline.add(w, h), position, "allocation {i}");
        }
    }
}
