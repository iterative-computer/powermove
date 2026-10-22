//! The float paint pass on the CPU: the reference the GPU pass is tested
//! against, the CPU fallback's paint, and the fitting tool's renderer.
//!
//! Items composite in order with premultiplied source-over into a float
//! RGBA target (E-encoded, premultiplied). A coverage item paints
//! `color × alpha × curve(coverage)`; an image item paints its bilinear
//! sample (transparent outside the image) times the color's alpha. A
//! blurred item is first painted alone, then blurred with a separable
//! Gaussian of radius 3σ, then composited.

use kurbo::Point;

use super::atlas::Atlas;
use super::profile::curve_at;
use super::scene::{Item, ItemContent, TextScene, blur_radius};

/// A premultiplied float RGBA image.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub data: Vec<f32>,
}

impl Image {
    pub fn new(width: u32, height: u32) -> Image {
        Image {
            width,
            height,
            data: vec![0.0; width as usize * height as usize * 4],
        }
    }

    /// Rounded to RGBA8 (premultiplied).
    pub fn to_rgba8(&self) -> Vec<u8> {
        self.data
            .iter()
            .map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8)
            .collect()
    }
}

/// Gaussian weights for a blur of `sigma` px, radius `blur_radius(sigma)`.
pub fn gaussian(sigma: f64) -> Vec<f32> {
    let r = blur_radius(sigma) as i64;
    let w: Vec<f64> = (-r..=r)
        .map(|i| (-(i * i) as f64 / (2.0 * sigma * sigma)).exp())
        .collect();
    let sum: f64 = w.iter().sum();
    w.iter().map(|v| (v / sum) as f32).collect()
}

fn bilinear(img: &super::bitmap::Rgba8, u: f64, v: f64) -> [f32; 4] {
    let x = u - 0.5;
    let y = v - 0.5;
    let x0 = x.floor();
    let y0 = y.floor();
    let fx = (x - x0) as f32;
    let fy = (y - y0) as f32;
    let at = |xi: i64, yi: i64| -> [f32; 4] {
        if xi < 0 || yi < 0 || xi >= i64::from(img.width) || yi >= i64::from(img.height) {
            return [0.0; 4];
        }
        let o = (yi as usize * img.width as usize + xi as usize) * 4;
        std::array::from_fn(|c| f32::from(img.data[o + c]) / 255.0)
    };
    let (xi, yi) = (x0 as i64, y0 as i64);
    let a = at(xi, yi);
    let b = at(xi + 1, yi);
    let c = at(xi, yi + 1);
    let d = at(xi + 1, yi + 1);
    std::array::from_fn(|k| {
        let top = a[k] + (b[k] - a[k]) * fx;
        let bot = c[k] + (d[k] - c[k]) * fx;
        top + (bot - top) * fy
    })
}

/// Paints `item` alone into `layer` (premultiplied), over the device rect
/// `[x0, y0, x1, y1)` of a target `width` px wide.
fn paint_item(item: &Item, coverage: &dyn Fn(i64, i64) -> f32, rect: [i64; 4], layer: &mut Image) {
    let [x0, y0, x1, y1] = rect;
    let a = (item.color[3] * item.opacity) as f32;
    match &item.content {
        ItemContent::Coverage { curve, .. } => {
            let rgb = [
                item.color[0] as f32,
                item.color[1] as f32,
                item.color[2] as f32,
            ];
            for y in y0.max(0)..y1.min(i64::from(layer.height)) {
                for x in x0.max(0)..x1.min(i64::from(layer.width)) {
                    let c = coverage(x, y);
                    if c <= 0.0 {
                        continue;
                    }
                    let k = curve_at(curve, f64::from(c)) as f32 * a;
                    let o = (y as usize * layer.width as usize + x as usize) * 4;
                    let s = [rgb[0] * k, rgb[1] * k, rgb[2] * k, k];
                    for (d, v) in layer.data[o..o + 4].iter_mut().zip(s) {
                        *d = v + *d * (1.0 - k);
                    }
                }
            }
        }
        ItemContent::Image { image, transform }
        | ItemContent::GlyphMask {
            image, transform, ..
        }
        | ItemContent::Graphite {
            image, transform, ..
        } => {
            let inv = transform.inverse();
            for y in y0.max(0)..y1.min(i64::from(layer.height)) {
                for x in x0.max(0)..x1.min(i64::from(layer.width)) {
                    let p = inv * Point::new(x as f64 + 0.5, y as f64 + 0.5);
                    let mut s = bilinear(image, p.x, p.y);
                    if matches!(item.content, ItemContent::GlyphMask { .. }) {
                        s = [
                            item.color[0] as f32 * s[3],
                            item.color[1] as f32 * s[3],
                            item.color[2] as f32 * s[3],
                            s[3],
                        ];
                    }
                    if s[3] <= 0.0 {
                        continue;
                    }
                    let o = (y as usize * layer.width as usize + x as usize) * 4;
                    let sa = s[3] * a;
                    for (d, v) in layer.data[o..o + 4].iter_mut().zip(s) {
                        *d = v * a + *d * (1.0 - sa);
                    }
                }
            }
        }
    }
}

fn blur(img: &mut Image, rect: [i64; 4], sigma: f64) {
    let k = gaussian(sigma);
    let r = (k.len() / 2) as i64;
    let (w, h) = (i64::from(img.width), i64::from(img.height));
    let [x0, y0, x1, y1] = [
        rect[0].max(0),
        rect[1].max(0),
        rect[2].min(w),
        rect[3].min(h),
    ];
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let get = |d: &[f32], x: i64, y: i64, c: usize| -> f32 {
        if x < 0 || y < 0 || x >= w || y >= h {
            0.0
        } else {
            d[(y as usize * w as usize + x as usize) * 4 + c]
        }
    };
    let src = img.data.clone();
    let mut tmp = vec![0.0f32; img.data.len()];
    for y in y0..y1 {
        for x in x0..x1 {
            for c in 0..4 {
                let mut acc = 0.0;
                for (i, kv) in k.iter().enumerate() {
                    acc += kv * get(&src, x + i as i64 - r, y, c);
                }
                tmp[(y as usize * w as usize + x as usize) * 4 + c] = acc;
            }
        }
    }
    for y in y0..y1 {
        for x in x0..x1 {
            for c in 0..4 {
                let mut acc = 0.0;
                for (i, kv) in k.iter().enumerate() {
                    acc += kv * get(&tmp, x, y + i as i64 - r, c);
                }
                img.data[(y as usize * w as usize + x as usize) * 4 + c] = acc;
            }
        }
    }
}

/// The host's image filter, with an N32 save-layer source origin and the
/// full canvas dimensions. The result is clipped to that canvas.
pub type ImageFilter<'a> = dyn Fn(&Image, [i32; 2], (u32, u32), f64) -> Image + 'a;

/// Paints `scene` with the coverage pages of `atlas` (RGBA8 rows, alpha is
/// coverage) into a new float target.
pub fn paint(scene: &TextScene, atlas: &Atlas, pages: &[(u32, u32, Vec<u8>)]) -> Image {
    paint_with_filter(scene, atlas, pages, None)
}

/// The host can provide its Canvas2D image filter without creating a
/// dependency from pm-text back to the vector renderer.
pub fn paint_with_filter(
    scene: &TextScene,
    atlas: &Atlas,
    pages: &[(u32, u32, Vec<u8>)],
    filter: Option<&ImageFilter<'_>>,
) -> Image {
    paint_with_format(scene, atlas, pages, filter, false)
}

fn paint_with_format(
    scene: &TextScene,
    atlas: &Atlas,
    pages: &[(u32, u32, Vec<u8>)],
    filter: Option<&ImageFilter<'_>>,
    n32: bool,
) -> Image {
    let quantize = |target: &mut Image| {
        if n32 {
            for v in &mut target.data {
                *v = (*v * 255.0).round().clamp(0.0, 255.0) / 255.0;
            }
        }
    };
    let mut target = Image::new(scene.width, scene.height);
    let mut cells: std::collections::HashMap<usize, Vec<&super::atlas::Cell>> =
        std::collections::HashMap::new();
    for c in &atlas.cells {
        cells.entry(c.item).or_default().push(c);
    }
    for (i, item) in scene.items.iter().enumerate() {
        let tiles = cells.get(&i).map(Vec::as_slice).unwrap_or(&[]);
        let coverage = |x: i64, y: i64| -> f32 {
            for c in tiles {
                let (px, py) = (x - c.device_x, y - c.device_y);
                if px < 0 || py < 0 || px >= i64::from(c.width) || py >= i64::from(c.height) {
                    continue;
                }
                let (pw, _, data) = &pages[c.page];
                let o =
                    ((c.y as usize + py as usize) * *pw as usize + c.x as usize + px as usize) * 4;
                return f32::from(data[o + 3]) / 255.0;
            }
            0.0
        };
        let rect = match &item.content {
            ItemContent::Coverage { .. } if tiles.is_empty() => continue,
            ItemContent::Coverage { .. } => {
                tiles
                    .iter()
                    .fold([i64::MAX, i64::MAX, i64::MIN, i64::MIN], |r, c| {
                        [
                            r[0].min(c.device_x),
                            r[1].min(c.device_y),
                            r[2].max(c.device_x + i64::from(c.width)),
                            r[3].max(c.device_y + i64::from(c.height)),
                        ]
                    })
            }
            ItemContent::Image { .. }
            | ItemContent::GlyphMask { .. }
            | ItemContent::Graphite { .. } => {
                let b = item.bounds.inflate(1.0, 1.0);
                [
                    b.x0.floor() as i64,
                    b.y0.floor() as i64,
                    b.x1.ceil() as i64,
                    b.y1.ceil() as i64,
                ]
            }
        };
        if item.blur > 0.0
            && let Some(filter) = filter
        {
            // A filter may read beyond the canvas. Paint the save layer
            // first, then clip its filtered result to the canvas.
            let r = i64::from(blur_radius(item.blur));
            let origin = [(rect[0] - r) as i32, (rect[1] - r) as i32];
            let mut layer = Image::new(
                (rect[2] - rect[0] + 2 * r) as u32,
                (rect[3] - rect[1] + 2 * r) as u32,
            );
            let mut shifted = item.clone();
            if let ItemContent::Image { transform, .. }
            | ItemContent::GlyphMask { transform, .. }
            | ItemContent::Graphite { transform, .. } = &mut shifted.content
            {
                *transform = kurbo::Affine::translate((-(origin[0] as f64), -(origin[1] as f64)))
                    * *transform;
            }
            let coverage = |x, y| coverage(x + i64::from(origin[0]), y + i64::from(origin[1]));
            let moved = [
                rect[0] - i64::from(origin[0]),
                rect[1] - i64::from(origin[1]),
                rect[2] - i64::from(origin[0]),
                rect[3] - i64::from(origin[1]),
            ];
            paint_item(&shifted, &coverage, moved, &mut layer);
            let filtered = filter(&layer, origin, (scene.width, scene.height), item.blur);
            for (d, s) in target
                .data
                .chunks_exact_mut(4)
                .zip(filtered.data.chunks_exact(4))
            {
                for c in 0..4 {
                    d[c] = d[c].mul_add(1.0 - s[3], s[c]);
                }
            }
            quantize(&mut target);
            continue;
        }
        if item.blur > 0.0 {
            let mut layer = Image::new(scene.width, scene.height);
            paint_item(item, &coverage, rect, &mut layer);
            let r = i64::from(blur_radius(item.blur));
            let reach = [rect[0] - r, rect[1] - r, rect[2] + r, rect[3] + r];
            blur(&mut layer, reach, item.blur);
            let (w, h) = (i64::from(scene.width), i64::from(scene.height));
            for y in reach[1].max(0)..reach[3].min(h) {
                for x in reach[0].max(0)..reach[2].min(w) {
                    let o = (y as usize * w as usize + x as usize) * 4;
                    let sa = layer.data[o + 3];
                    for ch in 0..4 {
                        target.data[o + ch] = layer.data[o + ch] + target.data[o + ch] * (1.0 - sa);
                    }
                }
            }
        } else {
            paint_item(item, &coverage, rect, &mut target);
        }
        quantize(&mut target);
    }
    target
}

/// Renders `scene` entirely on the CPU (`vello_cpu` coverage, float paint).
pub fn render_cpu(scene: &TextScene) -> Image {
    render_cpu_with_filter(scene, None)
}

pub fn render_cpu_with_filter(scene: &TextScene, filter: Option<&ImageFilter<'_>>) -> Image {
    render_cpu_with_format(scene, filter, false)
}

/// CPU reference for an N32 canvas, rounded after each canvas draw, or a
/// floating-point level 2 target.
pub fn render_cpu_with_format(
    scene: &TextScene,
    filter: Option<&ImageFilter<'_>>,
    n32: bool,
) -> Image {
    let atlas = super::atlas::pack(scene, 4096);
    let pages: Vec<_> = (0..atlas.pages.len())
        .map(|p| super::cpu::render_page(scene, &atlas, p))
        .collect();
    paint_with_format(scene, &atlas, &pages, filter, n32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::scene::{fill_path, rounded_rect};
    use kurbo::Affine;

    #[test]
    fn gaussian_weights_sum_to_one() {
        for s in [0.5, 1.0, 3.7] {
            let k = gaussian(s);
            assert_eq!(k.len(), 2 * blur_radius(s) as usize + 1);
            assert!((k.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn a_filled_rect_paints_its_color() {
        let scene = TextScene {
            width: 20,
            height: 20,
            items: vec![
                fill_path(
                    rounded_rect(4.0, 4.0, 10.0, 10.0, 0.0),
                    Affine::IDENTITY,
                    [1.0, 0.5, 0.0, 1.0],
                    0.5,
                )
                .unwrap(),
            ],
        };
        let img = render_cpu(&scene);
        let at = |x: usize, y: usize| &img.data[(y * 20 + x) * 4..(y * 20 + x) * 4 + 4];
        assert_eq!(at(8, 8), &[0.5, 0.25, 0.0, 0.5]);
        assert_eq!(at(1, 1), &[0.0; 4]);
    }

    #[test]
    fn blur_spreads_and_keeps_mass() {
        let mut item = fill_path(
            rounded_rect(8.0, 8.0, 4.0, 4.0, 0.0),
            Affine::IDENTITY,
            [1.0; 4],
            1.0,
        )
        .unwrap();
        item.blur = 1.5;
        let scene = TextScene {
            width: 24,
            height: 24,
            items: vec![item],
        };
        let img = render_cpu(&scene);
        let mass: f32 = img.data.chunks_exact(4).map(|p| p[3]).sum();
        assert!((mass - 16.0).abs() < 0.05, "{mass}");
        assert!(img.data[(5 * 24 + 10) * 4 + 3] > 0.0);
    }
}
