use crate::{
    Command, DrawCommand, ImageData, Result,
    coverage::{self, Mask},
};
use kurbo::{Affine, BezPath, Point, Rect, Shape};
use pm_vector::render::scene::{CoverageItem, ItemStyle};
use std::sync::Arc;

const TILE: u32 = 16;
type Tile = Box<[[f32; 4]; 256]>;
pub struct CpuRasterizer {
    width: u32,
    height: u32,
    columns: u32,
    tiles: Vec<Option<Tile>>,
    base: Option<Arc<ImageData>>,
}
impl CpuRasterizer {
    pub fn new(width: u32, height: u32) -> Result<Self> {
        crate::image::byte_len(width, height)?;
        let columns = width.div_ceil(TILE);
        let rows = height.div_ceil(TILE);
        Ok(Self {
            width,
            height,
            columns,
            tiles: vec![None; (columns * rows) as usize],
            base: None,
        })
    }
    pub fn image_data(&self) -> Result<ImageData> {
        let mut data = ImageData::new(self.width, self.height)?;
        self.read_into(0, 0, &mut data);
        Ok(data)
    }
    pub fn replay(&mut self, commands: &[Command]) -> Result<()> {
        for command in commands {
            self.execute(command)?;
        }
        Ok(())
    }
    fn get(&self, x: u32, y: u32) -> [f32; 4] {
        let index = ((y / TILE) * self.columns + x / TILE) as usize;
        if let Some(tile) = &self.tiles[index] {
            tile[((y % TILE) * TILE + x % TILE) as usize]
        } else {
            self.base
                .as_ref()
                .map_or([0.; 4], |base| base.pixel(x as i64, y as i64))
        }
    }
    fn set(&mut self, x: u32, y: u32, value: [f32; 4]) {
        let index = ((y / TILE) * self.columns + x / TILE) as usize;
        if self.tiles[index].is_none() {
            let mut tile = Box::new([[0.; 4]; 256]);
            if let Some(base) = &self.base {
                let (ox, oy) = (x / TILE * TILE, y / TILE * TILE);
                for ty in 0..TILE {
                    for tx in 0..TILE {
                        tile[(ty * TILE + tx) as usize] =
                            base.pixel((ox + tx) as i64, (oy + ty) as i64);
                    }
                }
            }
            self.tiles[index] = Some(tile);
        }
        self.tiles[index].as_mut().expect("tile initialized")
            [((y % TILE) * TILE + x % TILE) as usize] = value;
    }
    pub fn execute(&mut self, command: &Command) -> Result<()> {
        match command {
            Command::Draw(d) => self.draw(d),
            Command::Put {
                image,
                dx,
                dy,
                dirty,
            } => {
                let [x, y, w, h] = dirty.map(i64::from);
                let (sx, sy) = (x + w.min(0), y + h.min(0));
                let (ex, ey) = (sx + w.abs(), sy + h.abs());
                for y in sy.max(0)..ey.min(image.height() as i64) {
                    for x in sx.max(0)..ex.min(image.width() as i64) {
                        let (tx, ty) = (x + i64::from(*dx), y + i64::from(*dy));
                        if tx >= 0 && ty >= 0 && tx < self.width as i64 && ty < self.height as i64 {
                            self.set(tx as u32, ty as u32, image.pixel(x, y));
                        }
                    }
                }
            }
        }
        Ok(())
    }
    fn draw(&mut self, d: &DrawCommand) {
        if self.width == 0 || self.height == 0 {
            return;
        }
        if !d.clear
            && d.alpha == 1.
            && d.composite == crate::Composite::Copy
            && d.clips.is_empty()
            && let crate::paint::FrozenPaint::Image {
                source,
                to_source,
                extend: true,
                crop,
                ..
            } = &d.paint
        {
            let full = Rect::new(0., 0., self.width as f64, self.height as f64);
            if *to_source == Affine::IDENTITY
                && d.transform == Affine::IDENTITY
                && *crop == full
                && source.pixels.width() == self.width
                && source.pixels.height() == self.height
                && d.path == full.to_path(1e-4)
                && matches!(d.style, ItemStyle::Fill(_))
            {
                self.base = Some(source.pixels.clone());
                self.tiles.fill(None);
                return;
            }
        }
        let item = CoverageItem {
            path: d.path.clone(),
            transform: d.transform,
            style: d.style.clone(),
            color: [1.; 4],
        };
        let mut shape = item.device_outline();
        if let Some(support) = d.paint.support(self.width, self.height) {
            shape = coverage::intersect(&shape, &support);
        }
        let mut clip: Option<BezPath> = None;
        for c in d.clips.iter() {
            let outline = CoverageItem {
                path: c.path.clone(),
                transform: Affine::IDENTITY,
                style: ItemStyle::Fill(c.rule),
                color: [1.; 4],
            }
            .device_outline();
            clip = Some(match clip {
                None => outline,
                Some(p) => coverage::intersect(&p, &outline),
            });
        }
        if let Some(c) = &clip {
            shape = coverage::intersect(&shape, c);
        }
        let shape = Mask::new(&shape, self.width, self.height);
        let full = d.composite.unbounded() && !d.clear;
        let clipmask = if full {
            Some(Mask::new(
                &clip.unwrap_or_else(|| {
                    Rect::new(0., 0., self.width as f64, self.height as f64).to_path(1e-4)
                }),
                self.width,
                self.height,
            ))
        } else {
            None
        };
        let bounds = clipmask.as_ref().unwrap_or(&shape);
        for y in bounds.y..bounds.y + bounds.h {
            for x in bounds.x..bounds.x + bounds.w {
                let s = shape.get(x, y);
                let c = clipmask.as_ref().map_or(s, |m| m.get(x, y));
                if c == 0. {
                    continue;
                }
                let dest = self.get(x, y);
                if d.clear {
                    self.set(x, y, dest.map(|v| v * (1. - s)));
                    continue;
                }
                let paint = d
                    .paint
                    .sample(Point::new(x as f64 + 0.5, y as f64 + 0.5))
                    .map(|v| v * d.alpha);
                let covered = d.composite.apply(paint, dest);
                let uncovered = if full {
                    d.composite.apply([0.; 4], dest)
                } else {
                    dest
                };
                self.set(
                    x,
                    y,
                    std::array::from_fn(|j| {
                        (dest[j] * (1. - c) + covered[j] * s + uncovered[j] * (c - s).max(0.))
                            .clamp(0., 1.)
                    }),
                );
            }
        }
    }
    /// Read only the requested region into an existing buffer. Every output
    /// pixel is overwritten, including transparent out-of-bounds pixels.
    /// Coordinates are signed and may span the entire i64 range.
    pub fn read_into(&self, x: i64, y: i64, data: &mut ImageData) {
        if x == 0 && y == 0 && data.width() == self.width && data.height() == self.height {
            if let Some(base) = &self.base {
                data.bytes_mut().copy_from_slice(base.bytes());
                if !base.canonical {
                    for p in data.bytes_mut().chunks_exact_mut(4) {
                        if p[3] == 0 {
                            p[..3].fill(0);
                        }
                    }
                }
            } else {
                data.bytes_mut().fill(0);
            }
            let width = self.width;
            let height = self.height;
            let output = data.bytes_mut();
            for (i, tile) in self.tiles.iter().enumerate() {
                let Some(tile) = tile else {
                    continue;
                };
                let ox = i as u32 % self.columns * TILE;
                let oy = i as u32 / self.columns * TILE;
                for ty in 0..TILE.min(height - oy) {
                    for tx in 0..TILE.min(width - ox) {
                        let pixel = tile[(ty * TILE + tx) as usize];
                        let offset = (((oy + ty) * width + ox + tx) * 4) as usize;
                        encode(pixel, &mut output[offset..offset + 4]);
                    }
                }
            }
            data.canonical = true;
            return;
        }
        let w = data.width();
        for (i, rgba) in data.bytes_mut().chunks_exact_mut(4).enumerate() {
            let sx = x.saturating_add((i % w as usize) as i64);
            let sy = y.saturating_add((i / w as usize) as i64);
            if sx < 0 || sy < 0 || sx >= self.width as i64 || sy >= self.height as i64 {
                rgba.fill(0);
                continue;
            }
            encode(self.get(sx as u32, sy as u32), rgba);
        }
        data.canonical = true;
    }
}
fn encode(p: [f32; 4], rgba: &mut [u8]) {
    let a = p[3];
    for j in 0..3 {
        rgba[j] = if a > 0. { quantize(p[j] / a) } else { 0 };
    }
    rgba[3] = quantize(a);
}
fn quantize(v: f32) -> u8 {
    (v.clamp(0., 1.) * 255.).round_ties_even() as u8
}
