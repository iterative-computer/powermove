//! Ordered float source-over paint for text scenes. Blurs use the exact
//! pixel-integrated Gaussian at authored sigma times world scale.

use kurbo::{Affine, Point};
use pm_color::contract::WorkingEncoding;
use pm_color::transfer::display_decode_f32;

use super::atlas::Atlas;
use super::scene::{ImagePixels, Item, ItemContent, TextScene, blur_radius};

#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// Premultiplied float RGBA in the renderer's working encoding.
    pub data: Vec<f32>,
}

impl Image {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            data: vec![0.0; width as usize * height as usize * 4],
        }
    }

    pub fn to_rgba8(&self) -> Vec<u8> {
        self.data
            .iter()
            .map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8)
            .collect()
    }
}

fn phi(x: f64) -> f64 {
    0.5 * libm::erfc(-x / std::f64::consts::SQRT_2)
}

/// Gaussian weights integrate the continuous kernel over each pixel square.
/// The tails stop at 8 sigma and are never renormalized.
pub fn gaussian(sigma: f64) -> Vec<f64> {
    assert!(sigma >= 0.0 && sigma.is_finite());
    if sigma == 0.0 {
        return vec![1.0];
    }
    let r = blur_radius(sigma) as i64;
    (-r..=r)
        .map(|k| {
            let k = k as f64;
            if k >= 0.0 {
                phi(-(k - 0.5) / sigma) - phi(-(k + 0.5) / sigma)
            } else {
                phi((k + 0.5) / sigma) - phi((k - 0.5) / sigma)
            }
        })
        .collect()
}

fn to_working(mut pixel: [f32; 4], working: WorkingEncoding) -> [f32; 4] {
    if working == WorkingEncoding::LinearLight {
        let alpha = pixel[3];
        if alpha > 0.0 {
            for channel in &mut pixel[..3] {
                *channel = display_decode_f32(*channel / alpha) * alpha;
            }
        } else {
            pixel[..3].fill(0.0);
        }
    }
    pixel
}

fn inverse_major(inv: Affine) -> f64 {
    let [a, b, c, d, _, _] = inv.as_coeffs();
    let xx = a * a + c * c;
    let xy = a * b + c * d;
    let yy = b * b + d * d;
    ((xx + yy + ((xx - yy).powi(2) + 4.0 * xy * xy).sqrt()) * 0.5).sqrt()
}

fn polygon_area(poly: &[(f64, f64)]) -> f64 {
    if poly.len() < 3 {
        return 0.0;
    }
    let mut sum = 0.0;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        sum += a.0 * b.1 - b.0 * a.1;
    }
    sum.abs() * 0.5
}

/// Clip a convex inverse pixel quad to a source texel without allocations.
fn clipped_area(quad: &[(f64, f64); 4], sx: i64, sy: i64) -> f64 {
    let mut current = [(0.0, 0.0); 8];
    let mut next = [(0.0, 0.0); 8];
    current[..4].copy_from_slice(quad);
    let mut len = 4;
    for (axis, bound, keep_above) in [
        (0, sx as f64, true),
        (0, sx as f64 + 1.0, false),
        (1, sy as f64, true),
        (1, sy as f64 + 1.0, false),
    ] {
        let inside = |p: (f64, f64)| {
            let value = if axis == 0 { p.0 } else { p.1 };
            if keep_above {
                value >= bound
            } else {
                value <= bound
            }
        };
        let mut count = 0;
        for i in 0..len {
            let (a, b) = (current[i], current[(i + 1) % len]);
            let (av, bv) = if axis == 0 { (a.0, b.0) } else { (a.1, b.1) };
            if inside(a) != inside(b) {
                let t = (bound - av) / (bv - av);
                next[count] = (a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1));
                count += 1;
            }
            if inside(b) {
                next[count] = b;
                count += 1;
            }
        }
        if count < 3 {
            return 0.0;
        }
        std::mem::swap(&mut current, &mut next);
        len = count;
    }
    polygon_area(&current[..len])
}

struct ImageSampler {
    image: Image,
    inv: Affine,
    minify: bool,
}

impl ImageSampler {
    fn new(image: &ImagePixels, transform: Affine, working: WorkingEncoding) -> Self {
        let base = match image {
            ImagePixels::Rgba8(i) => {
                let mut base = Image::new(i.width, i.height);
                for (source, pixel) in i.data.chunks_exact(4).zip(base.data.chunks_exact_mut(4)) {
                    pixel.copy_from_slice(&to_working(
                        std::array::from_fn(|c| f32::from(source[c]) / 255.0),
                        working,
                    ));
                }
                base
            }
            ImagePixels::WorkingF32(i) => Image {
                width: i.width,
                height: i.height,
                data: i.data.clone(),
            },
            ImagePixels::Colr(program) => {
                let pixels = program.rasterize(working);
                Image {
                    width: pixels.width,
                    height: pixels.height,
                    data: pixels.data,
                }
            }
        };
        let inv = transform.inverse();
        Self {
            image: base,
            inv,
            minify: inverse_major(inv) > 1.0,
        }
    }

    fn texel(&self, x: i64, y: i64) -> [f32; 4] {
        if x < 0 || y < 0 || x >= i64::from(self.image.width) || y >= i64::from(self.image.height) {
            return [0.0; 4];
        }
        let o = (y as usize * self.image.width as usize + x as usize) * 4;
        self.image.data[o..o + 4].try_into().expect("RGBA pixel")
    }

    fn bilinear(&self, p: Point) -> [f32; 4] {
        let x = p.x - 0.5;
        let y = p.y - 0.5;
        let (ix, iy) = (x.floor() as i64, y.floor() as i64);
        let (fx, fy) = (x - x.floor(), y - y.floor());
        let a = self.texel(ix, iy);
        let b = self.texel(ix + 1, iy);
        let c = self.texel(ix, iy + 1);
        let d = self.texel(ix + 1, iy + 1);
        std::array::from_fn(|k| {
            let top = f64::from(a[k]) * (1.0 - fx) + f64::from(b[k]) * fx;
            let bottom = f64::from(c[k]) * (1.0 - fx) + f64::from(d[k]) * fx;
            (top * (1.0 - fy) + bottom * fy) as f32
        })
    }

    fn sample(&self, x: i64, y: i64) -> [f32; 4] {
        if !self.minify {
            return self.bilinear(self.inv * Point::new(x as f64 + 0.5, y as f64 + 0.5));
        }
        let (x, y) = (x as f64, y as f64);
        let corners = [
            Point::new(x, y),
            Point::new(x + 1.0, y),
            Point::new(x + 1.0, y + 1.0),
            Point::new(x, y + 1.0),
        ];
        let quad = corners.map(|point| {
            let p = self.inv * point;
            (p.x, p.y)
        });
        let whole = polygon_area(&quad);
        if whole <= 0.0 {
            return [0.0; 4];
        }
        let x0 = quad
            .iter()
            .map(|p| p.0)
            .fold(f64::INFINITY, f64::min)
            .floor() as i64;
        let x1 = quad
            .iter()
            .map(|p| p.0)
            .fold(f64::NEG_INFINITY, f64::max)
            .ceil() as i64;
        let y0 = quad
            .iter()
            .map(|p| p.1)
            .fold(f64::INFINITY, f64::min)
            .floor() as i64;
        let y1 = quad
            .iter()
            .map(|p| p.1)
            .fold(f64::NEG_INFINITY, f64::max)
            .ceil() as i64;
        let mut out = [0.0f64; 4];
        for sy in y0.max(0)..y1.min(i64::from(self.image.height)) {
            for sx in x0.max(0)..x1.min(i64::from(self.image.width)) {
                let weight = clipped_area(&quad, sx, sy) / whole;
                let texel = self.texel(sx, sy);
                for c in 0..4 {
                    out[c] += f64::from(texel[c]) * weight;
                }
            }
        }
        out.map(|v| v as f32)
    }
}

fn over(dst: &mut [f32], src: [f32; 4]) {
    let keep = 1.0 - src[3];
    for c in 0..4 {
        dst[c] = src[c] + dst[c] * keep;
    }
}

fn paint_item(
    item: &Item,
    coverage: &dyn Fn(i64, i64) -> f32,
    rect: [i64; 4],
    layer: &mut Image,
    working: WorkingEncoding,
) {
    let alpha = (item.color[3] * item.opacity) as f32;
    let rgb: [f32; 3] = if working == WorkingEncoding::LinearLight {
        std::array::from_fn(|c| display_decode_f32(item.color[c] as f32))
    } else {
        std::array::from_fn(|c| item.color[c] as f32)
    };
    let sampler = match &item.content {
        ItemContent::Image { image, transform } => {
            Some(ImageSampler::new(image, *transform, working))
        }
        ItemContent::Coverage { .. } => None,
    };
    for y in rect[1].max(0)..rect[3].min(i64::from(layer.height)) {
        for x in rect[0].max(0)..rect[2].min(i64::from(layer.width)) {
            let mut src = match &item.content {
                ItemContent::Coverage { .. } => {
                    let a = coverage(x, y) * alpha;
                    [rgb[0] * a, rgb[1] * a, rgb[2] * a, a]
                }
                ItemContent::Image { .. } => {
                    let mut s = sampler.as_ref().expect("image sampler").sample(x, y);
                    for v in &mut s {
                        *v *= alpha;
                    }
                    s
                }
            };
            if src[3] <= 0.0 {
                continue;
            }
            src[3] = src[3].clamp(0.0, 1.0);
            let o = (y as usize * layer.width as usize + x as usize) * 4;
            over(&mut layer.data[o..o + 4], src);
        }
    }
}

fn blur(image: &mut Image, sigma: f64) {
    let kernel = gaussian(sigma);
    let radius = (kernel.len() / 2) as i64;
    let (w, h) = (image.width as i64, image.height as i64);
    let mut intermediate = vec![0.0f32; image.data.len()];
    for y in 0..h {
        for x in 0..w {
            let o = (y * w + x) as usize * 4;
            for (j, weight) in kernel.iter().enumerate() {
                let sx = x + j as i64 - radius;
                if sx < 0 || sx >= w {
                    continue;
                }
                let source = (y * w + sx) as usize * 4;
                for c in 0..4 {
                    intermediate[o + c] += (image.data[source + c] as f64 * weight) as f32;
                }
            }
        }
    }
    image.data.fill(0.0);
    for y in 0..h {
        for x in 0..w {
            let o = (y * w + x) as usize * 4;
            for (j, weight) in kernel.iter().enumerate() {
                let sy = y + j as i64 - radius;
                if sy < 0 || sy >= h {
                    continue;
                }
                let source = (sy * w + x) as usize * 4;
                for c in 0..4 {
                    image.data[o + c] += (intermediate[source + c] as f64 * weight) as f32;
                }
            }
        }
    }
}

pub fn paint(scene: &TextScene, atlas: &Atlas, pages: &[(u32, u32, Vec<u8>)]) -> Image {
    paint_in_working(scene, atlas, pages, WorkingEncoding::Standard)
}

pub fn paint_in_working(
    scene: &TextScene,
    atlas: &Atlas,
    pages: &[(u32, u32, Vec<u8>)],
    working: WorkingEncoding,
) -> Image {
    let mut target = Image::new(scene.width, scene.height);
    let mut cells: std::collections::HashMap<usize, Vec<&super::atlas::Cell>> =
        std::collections::HashMap::new();
    for cell in &atlas.cells {
        cells.entry(cell.item).or_default().push(cell);
    }
    for (index, item) in scene.items.iter().enumerate() {
        let tiles = cells.get(&index).map(Vec::as_slice).unwrap_or(&[]);
        if matches!(item.content, ItemContent::Coverage { .. }) && tiles.is_empty() {
            continue;
        }
        let coverage = |x: i64, y: i64| -> f32 {
            for cell in tiles {
                let (px, py) = (x - cell.device_x, y - cell.device_y);
                if px < 0 || py < 0 || px >= cell.width as i64 || py >= cell.height as i64 {
                    continue;
                }
                let (pw, _, data) = &pages[cell.page];
                let o = ((cell.y as usize + py as usize) * *pw as usize
                    + cell.x as usize
                    + px as usize)
                    * 4;
                return f32::from(data[o + 3]) / 255.0;
            }
            0.0
        };
        let b = item.bounds.inflate(1.0, 1.0);
        let rect = [
            b.x0.floor() as i64,
            b.y0.floor() as i64,
            b.x1.ceil() as i64,
            b.y1.ceil() as i64,
        ];
        if item.blur > 0.0 {
            // Preserve source pixels outside the canvas until after the
            // convolution. Source farther than the kernel radius from the
            // canvas cannot affect a visible pixel.
            let r = blur_radius(item.blur) as i64;
            let rect = [
                rect[0].max(-r),
                rect[1].max(-r),
                rect[2].min(i64::from(scene.width) + r),
                rect[3].min(i64::from(scene.height) + r),
            ];
            if rect[2] <= rect[0] || rect[3] <= rect[1] {
                continue;
            }
            let origin = (rect[0] - r, rect[1] - r);
            let width = (rect[2] - rect[0] + 2 * r).max(0) as u32;
            let height = (rect[3] - rect[1] + 2 * r).max(0) as u32;
            let mut layer = Image::new(width, height);
            let mut shifted = item.clone();
            if let ItemContent::Image { transform, .. } = &mut shifted.content {
                *transform =
                    kurbo::Affine::translate((-(origin.0 as f64), -(origin.1 as f64))) * *transform;
            }
            let local_coverage = |x, y| coverage(x + origin.0, y + origin.1);
            paint_item(
                &shifted,
                &local_coverage,
                [r, r, r + rect[2] - rect[0], r + rect[3] - rect[1]],
                &mut layer,
                working,
            );
            blur(&mut layer, item.blur);
            for y in origin.1.max(0)..(origin.1 + i64::from(height)).min(scene.height as i64) {
                for x in origin.0.max(0)..(origin.0 + i64::from(width)).min(scene.width as i64) {
                    let source =
                        ((y - origin.1) as usize * width as usize + (x - origin.0) as usize) * 4;
                    let target_offset = (y as usize * scene.width as usize + x as usize) * 4;
                    over(
                        &mut target.data[target_offset..target_offset + 4],
                        layer.data[source..source + 4].try_into().unwrap(),
                    );
                }
            }
        } else {
            paint_item(item, &coverage, rect, &mut target, working);
        }
    }
    target
}

pub fn render_cpu(scene: &TextScene) -> Image {
    render_cpu_in_working(scene, WorkingEncoding::Standard)
}

pub fn render_cpu_in_working(scene: &TextScene, working: WorkingEncoding) -> Image {
    let atlas = super::atlas::pack(scene, 4096);
    let pages: Vec<_> = (0..atlas.pages.len())
        .map(|page| super::cpu::render_page(scene, &atlas, page))
        .collect();
    paint_in_working(scene, &atlas, &pages, working)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emoji_area_is_integrated_after_working_decode() {
        use crate::render::bitmap::Rgba8;
        use std::sync::Arc;
        let source = ImagePixels::Rgba8(Arc::new(Rgba8 {
            width: 2,
            height: 2,
            data: [128, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255, 128, 0, 0, 255].to_vec(),
        }));
        let transform = Affine::scale(0.5);
        let standard =
            ImageSampler::new(&source, transform, WorkingEncoding::Standard).sample(0, 0);
        let linear =
            ImageSampler::new(&source, transform, WorkingEncoding::LinearLight).sample(0, 0);
        assert!((standard[0] - 128.0 / 510.0).abs() < 1e-6);
        assert!((linear[0] - display_decode_f32(128.0 / 255.0) * 0.5).abs() < 1e-6);
        assert!(linear[0] < standard[0]);
        assert_eq!(standard[3], 1.0);
    }

    #[test]
    fn inverse_pixel_quad_clips_to_source_texels() {
        let diamond = [(0.0, 1.0), (1.0, 0.0), (2.0, 1.0), (1.0, 2.0)];
        assert_eq!(polygon_area(&diamond), 2.0);
        for (x, y) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            assert!((clipped_area(&diamond, x, y) - 0.5).abs() < 1e-12);
        }
        assert_eq!(clipped_area(&diamond, 2, 2), 0.0);
        assert!(inverse_major(Affine::scale_non_uniform(8.0, 2.0)) > 1.0);
    }

    #[test]
    fn rotated_emoji_minification_integrates_original_texel_area() {
        use crate::render::bitmap::Rgba8;
        use std::sync::Arc;
        let source = ImagePixels::Rgba8(Arc::new(Rgba8 {
            width: 2,
            height: 2,
            data: [255, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255].to_vec(),
        }));
        // The inverse output pixel is a diamond with area 2: each of the
        // four original texels contributes area 1/2 to the output pixel.
        let inverse = Affine::new([1.0, -1.0, 1.0, 1.0, 0.0, 1.0]);
        let pixel =
            ImageSampler::new(&source, inverse.inverse(), WorkingEncoding::Standard).sample(0, 0);
        assert!((pixel[0] - 0.25).abs() < 1e-7);
        assert_eq!(pixel[3], 1.0);
    }

    #[test]
    fn offcanvas_ink_blurs_into_visible_pixels() {
        use crate::render::scene::{fill_path, rounded_rect};
        let mut item = fill_path(
            rounded_rect(-3.0, 2.0, 2.0, 2.0, 0.0),
            kurbo::Affine::IDENTITY,
            [1.0; 4],
            1.0,
        )
        .unwrap();
        item.blur = 2.0;
        let scene = TextScene {
            width: 8,
            height: 8,
            items: vec![item],
        };
        let image = render_cpu(&scene);
        assert!(image.data[(2 * 8) * 4 + 3] > 0.0);
    }

    #[test]
    fn translucent_overlapping_glyphs_build_alpha_in_order() {
        use crate::render::scene::{fill_path, rounded_rect};
        let path = rounded_rect(1.0, 1.0, 3.0, 3.0, 0.0);
        let glyph = fill_path(path, kurbo::Affine::IDENTITY, [0.8, 0.2, 0.0, 0.5], 1.0).unwrap();
        let scene = TextScene {
            width: 6,
            height: 6,
            items: vec![glyph.clone(), glyph],
        };
        let image = render_cpu(&scene);
        let o = (2 * 6 + 2) * 4;
        assert!((image.data[o + 3] - 0.75).abs() < 0.01);
        assert!((image.data[o] - 0.6).abs() < 0.01);
    }

    #[test]
    fn integrated_gaussian_preserves_mass_without_normalization() {
        for sigma in [0.3, 1.0, 2.5, 17.0] {
            let weights = gaussian(sigma);
            assert_eq!(weights.len(), 2 * blur_radius(sigma) as usize + 1);
            assert!((weights.iter().sum::<f64>() - 1.0).abs() < 1e-14);
        }
    }
}
