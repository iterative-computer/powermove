//! Canvas2D's `ctx.filter = blur(σ px)` as Chromium 152's Skia (Graphite)
//! runs it on the GPU, planned pass for pass (docs/engine/composite.md
//! "Path masks"). Every number here is computed in `f32` in Skia's order;
//! pm-render executes the passes.
//!
//! Skia (`b6d106297ff9ef2ff8094033695d045e87775581`): the draw goes into a
//! save layer whose image filter is `SkBlurImageFilter` with decal tiling.
//! On restore, `FilterResult::Builder::blur`:
//!
//! - σ ≤ 0.03 (`IsEffectivelyIdentity`) on both axes: no blur.
//! - σ above `kMaxLinearSigma` (4): `FilterResult::rescale` downscales the
//!   layer image about its center (`downscale_step_count`: ½ steps, then a
//!   final step between ½ and 1), each step a bilinear draw into an 8-bit
//!   image with a transparent 1-pixel border.
//! - `SkShaderBlurAlgorithm::blur` at that resolution: one 2D pass with
//!   nearest sampling when `(2rx + 1)(2ry + 1) ≤ 28`, else a horizontal and
//!   a vertical 1D pass whose taps pair adjacent texels into one bilinear
//!   sample (`Compute1DBlurLinearKernel`); sums at full precision (the
//!   effects declare `half`, which Graphite evaluates as `float` on Metal),
//!   8-bit outputs, transparent outside the image.
//! - The result drawn back onto the canvas, bilinear when it was rescaled
//!   (else pixel aligned), under the draw's composite operation.
//!
//! Images are rectangles on an integer pixel grid in their own space,
//! transparent outside. A pass's sample positions are given in the space of
//! its source image.

/// `kMaxLinearSigma`.
pub const MAX_LINEAR_SIGMA: f32 = 4.0;
/// `SkBlurImageFilter`'s `kMaxSigma`.
pub const MAX_SIGMA: f32 = 532.0;
/// `SkShaderBlurAlgorithm::kMaxSamples`.
pub const MAX_SAMPLES: usize = 28;
/// `skif`'s `kRoundEpsilon`.
const ROUND_EPSILON: f32 = 1e-3;

/// An image's pixels: `[x0, x0 + w) × [y0, y0 + h)` in its space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x0: i32,
    pub y0: i32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub fn ltrb(l: i32, t: i32, r: i32, b: i32) -> Rect {
        Rect {
            x0: l,
            y0: t,
            w: (r - l).max(0) as u32,
            h: (b - t).max(0) as u32,
        }
    }
    pub fn r(&self) -> i32 {
        self.x0 + self.w as i32
    }
    pub fn b(&self) -> i32 {
        self.y0 + self.h as i32
    }
    pub fn outset(&self, dx: i32, dy: i32) -> Rect {
        Rect::ltrb(self.x0 - dx, self.y0 - dy, self.r() + dx, self.b() + dy)
    }
    pub fn intersect(&self, o: &Rect) -> Rect {
        Rect::ltrb(
            self.x0.max(o.x0),
            self.y0.max(o.y0),
            self.r().min(o.r()),
            self.b().min(o.b()),
        )
    }
}

/// A float rectangle (`SkRect`).
#[derive(Debug, Clone, Copy, PartialEq)]
struct RectF {
    l: f32,
    t: f32,
    r: f32,
    b: f32,
}

impl RectF {
    fn of(r: Rect) -> RectF {
        RectF {
            l: r.x0 as f32,
            t: r.y0 as f32,
            r: r.r() as f32,
            b: r.b() as f32,
        }
    }
    fn width(&self) -> f32 {
        self.r - self.l
    }
    fn height(&self) -> f32 {
        self.b - self.t
    }
    /// `skif::RoundOut`: inset by `kRoundEpsilon`, then round out.
    fn round_out(&self) -> Rect {
        Rect::ltrb(
            (self.l + ROUND_EPSILON).floor() as i32,
            (self.t + ROUND_EPSILON).floor() as i32,
            (self.r - ROUND_EPSILON).ceil() as i32,
            (self.b - ROUND_EPSILON).ceil() as i32,
        )
    }
}

/// A scale and translate per axis (`SkMatrix` without skew):
/// `x' = x · scale + translate`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Map {
    pub scale: [f32; 2],
    pub translate: [f32; 2],
}

impl Map {
    /// `SkMatrix::RectToRect(src, dst)` (fill).
    fn rect_to_rect(src: RectF, dst: RectF) -> Map {
        let sx = dst.width() / src.width();
        let sy = dst.height() / src.height();
        Map {
            scale: [sx, sy],
            translate: [dst.l - src.l * sx, dst.t - src.t * sy],
        }
    }
    /// `SkMatrix::invert` of a scale and translate.
    pub fn invert(&self) -> Map {
        let ix = 1.0 / self.scale[0];
        let iy = 1.0 / self.scale[1];
        Map {
            scale: [ix, iy],
            translate: [-self.translate[0] * ix, -self.translate[1] * iy],
        }
    }
}

/// One pass of the blur: renders `dst` (in its space) from the previous
/// image (the item for the first pass).
#[derive(Debug, Clone, PartialEq)]
pub enum Pass {
    /// A rescale step: each pixel center `p` of `dst` samples the previous
    /// image bilinearly at `p · sample.scale + sample.translate` (`sample`
    /// maps the new space to the previous one).
    Rescale { dst: Rect, sample: Map },
    /// The single 2D pass: each pixel center `p` sums `weights[i]` times
    /// the previous image's texel (nearest) at `p + offsets[i]`, in order. Same space as the previous image.
    Blur2d {
        dst: Rect,
        weights: Vec<f32>,
        offsets: Vec<[f32; 2]>,
    },
    /// A 1D pass along `dir` (`[1, 0]` or `[0, 1]`): each pixel center
    /// `p` sums `weight` times the bilinear sample at `p + offset · dir`
    /// over `taps`, in order. Same space.
    Blur1d {
        dst: Rect,
        dir: [f32; 2],
        taps: Vec<(f32, f32)>,
    },
}

impl Pass {
    pub fn dst(&self) -> Rect {
        match self {
            Pass::Rescale { dst, .. } | Pass::Blur2d { dst, .. } | Pass::Blur1d { dst, .. } => *dst,
        }
    }
}

/// The blur of a `width` × `height` canvas layer.
#[derive(Debug, Clone, PartialEq)]
pub struct BlurPlan {
    pub passes: Vec<Pass>,
    /// Canvas pixel centers to the last image's space.
    pub draw: Map,
    /// Whether the final draw filters bilinearly (else it is pixel
    /// aligned and samples texels exactly).
    pub linear: bool,
}

/// `SkNextLog2(v)`: the smallest `n` with `2ⁿ ≥ v` (0 for `v ≤ 1`).
fn next_log2(v: i32) -> i32 {
    if v <= 1 { 0 } else { 32 - ((v - 1) as u32).leading_zeros() as i32 }
}

/// `downscale_step_count`.
fn downscale_step_count(net_scale: f32) -> i32 {
    let mut steps = next_log2((1.0 / net_scale).ceil() as i32);
    if steps > 0 {
        let final_step = net_scale * (1 << (steps - 1)) as f32;
        let limit = if steps == 1 { 1.0 - ROUND_EPSILON } else { 0.9 };
        if final_step >= limit {
            steps -= 1;
        }
    }
    steps
}

/// `scale_about_center`.
fn scale_about_center(src: RectF, sx: f32, sy: f32) -> RectF {
    let cx = if sx == 1.0 { 0.0 } else { 0.5 * src.l + 0.5 * src.r };
    let cy = if sy == 1.0 { 0.0 } else { 0.5 * src.t + 0.5 * src.b };
    RectF {
        l: (src.l - cx) * sx,
        t: (src.t - cy) * sy,
        r: (src.r - cx) * sx,
        b: (src.b - cy) * sy,
    }
}

/// `SkBlurEngine::SigmaToRadius`.
pub fn sigma_to_radius(sigma: f32) -> i32 {
    if sigma <= 0.03 { 0 } else { (3.0 * sigma).ceil() as i32 }
}

/// `Compute2DBlurKernel`, row-major, `(2rx + 1)(2ry + 1)` weights.
fn kernel_2d(sigma: [f32; 2], radius: [i32; 2]) -> Vec<f32> {
    let width = (2 * radius[0] + 1) as usize;
    let height = (2 * radius[1] + 1) as usize;
    let two_x = 2.0 * sigma[0] * sigma[0];
    let two_y = 2.0 * sigma[1] * sigma[1];
    let dx = if radius[0] > 0 { 1.0 / two_x } else { 1.0 };
    let dy = if radius[1] > 0 { 1.0 / two_y } else { 1.0 };
    let mut kernel = vec![0.0f32; width * height];
    let mut sum = 0.0f32;
    for x in 0..width {
        let mut xt = (x as i32 - radius[0]) as f32;
        xt = xt * xt * dx;
        for y in 0..height {
            let yt = (y as i32 - radius[1]) as f32;
            let v = libm::expf(-(xt + yt * yt * dy));
            kernel[y * width + x] = v;
            sum += v;
        }
    }
    let scale = 1.0 / sum;
    for k in &mut kernel {
        *k *= scale;
    }
    kernel
}

/// The loop count of the generated effect for `n` taps (`to_stablekey`'s
/// batches: multiples of 4 up to 20, then 28).
fn batched(n: usize) -> usize {
    match n {
        0..=4 => 4,
        5..=8 => 8,
        9..=12 => 12,
        13..=16 => 16,
        17..=20 => 20,
        _ => 28,
    }
}

/// `Compute1DBlurLinearKernel`: (offset, weight) per bilinear tap, lowest
/// offset first, padded with zero weights at the last offset to the
/// effect's loop count.
fn kernel_1d_linear(sigma: f32, radius: i32) -> Vec<(f32, f32)> {
    let full = kernel_2d([sigma, 0.0], [radius, 0]);
    let at = |i: usize| full.get(i).copied().unwrap_or(0.0);
    let half_size = (radius + 1) as usize;
    let half_radius = half_size / 2;
    let mut kernel = [0.0f32; MAX_SAMPLES];
    let mut offsets = [0.0f32; MAX_SAMPLES];
    let mut low = half_radius as isize - 1;
    let mut index = radius as usize;
    let new_weight = |wi: f32, wj: f32| (wi + wj, wj / (wi + wj));
    if radius & 1 == 1 {
        let (w, o) = new_weight(at(index) * 0.5, at(index + 1));
        kernel[half_radius] = w;
        offsets[half_radius] = o;
        kernel[low as usize] = w;
        offsets[low as usize] = -o;
        index += 1;
        low -= 1;
    } else {
        kernel[half_radius] = at(index);
        offsets[half_radius] = 0.0;
    }
    index += 1;
    let mut i = half_radius + 1;
    while i < half_size {
        let (w, o) = new_weight(at(index), at(index + 1));
        kernel[i] = w;
        offsets[i] = o + (index as i32 - radius) as f32;
        kernel[low as usize] = kernel[i];
        offsets[low as usize] = -offsets[i];
        index += 2;
        i += 1;
        low -= 1;
    }
    let taps = batched(half_size);
    (0..taps)
        .map(|i| {
            if i < half_size {
                (offsets[i], kernel[i])
            } else {
                (offsets[half_size - 1], 0.0)
            }
        })
        .collect()
}

/// The blur passes for an image `src` (its space's pixels) at `sigma`,
/// writing `dst` (`SkShaderBlurAlgorithm::blur` with clamp tiling over a
/// transparent border).
fn shader_blur(sigma: [f32; 2], src: Rect, dst: Rect, passes: &mut Vec<Pass>) {
    let rx = sigma_to_radius(sigma[0]);
    let ry = sigma_to_radius(sigma[1]);
    let area = ((2 * rx + 1) * (2 * ry + 1)) as usize;
    if area <= MAX_SAMPLES && rx > 0 && ry > 0 {
        let weights = kernel_2d(sigma, [rx, ry]);
        let mut offsets = Vec::with_capacity(area);
        for y in -ry..=ry {
            for x in -rx..=rx {
                offsets.push([x as f32, y as f32]);
            }
        }
        let n = batched(area);
        let last = *offsets.last().unwrap();
        let mut w = weights;
        w.resize(n, 0.0);
        offsets.resize(n, last);
        passes.push(Pass::Blur2d {
            dst,
            weights: w,
            offsets,
        });
        return;
    }
    // The source with its 1-pixel transparent border (`makePixelOutset`).
    let outset_src = src.outset(1, 1);
    let mut intermediate = dst;
    if rx > 0 {
        if ry > 0 {
            let o = dst.outset(0, ry);
            intermediate = Rect::ltrb(o.x0, o.y0.max(outset_src.y0), o.r(), o.b().min(outset_src.b()));
            if intermediate.h == 0 {
                return;
            }
        }
        passes.push(Pass::Blur1d {
            dst: intermediate,
            dir: [1.0, 0.0],
            taps: kernel_1d_linear(sigma[0], rx),
        });
    }
    if ry > 0 {
        passes.push(Pass::Blur1d {
            dst,
            dir: [0.0, 1.0],
            taps: kernel_1d_linear(sigma[1], ry),
        });
    }
}

impl BlurPlan {
    /// The blur `blur(sigma px)` applies to a layer drawn on a `width` ×
    /// `height` canvas (identity transform), or `None` when it is an
    /// identity and the layer is drawn as it is.
    pub fn new(width: u32, height: u32, sigma: f32) -> Option<BlurPlan> {
        if !sigma.is_finite() {
            return None;
        }
        let sigma = sigma.min(MAX_SIGMA);
        if sigma <= 0.03 {
            return None;
        }
        // The save layer covers the canvas outset by the kernel bounds.
        let r = (3.0 * sigma).ceil() as i32;
        let canvas = Rect::ltrb(0, 0, width as i32, height as i32);
        let src_rect = canvas.outset(r, r);
        let scale = if sigma > MAX_LINEAR_SIGMA { MAX_LINEAR_SIGMA / sigma } else { 1.0 };
        let steps = if scale == 1.0 { 0 } else { downscale_step_count(scale) };
        let mut passes = Vec::new();
        if steps == 0 {
            // The blur at full resolution, output over the canvas and one
            // pixel around it, drawn pixel aligned.
            shader_blur([sigma, sigma], canvas, canvas.outset(1, 1), &mut passes);
            return Some(BlurPlan {
                passes,
                draw: Map {
                    scale: [1.0, 1.0],
                    translate: [0.0, 0.0],
                },
                linear: false,
            });
        }
        let (mut xs, mut ys) = (steps, steps);
        let src_f = RectF::of(src_rect);
        let mut step = src_f;
        let mut pixels = src_rect;
        while xs > 0 || ys > 0 {
            let mut sx = 1.0;
            if xs > 0 {
                sx = if xs > 1 { 0.5 } else { src_f.width() * scale / step.width() };
                xs -= 1;
            }
            let mut sy = 1.0;
            if ys > 0 {
                sy = if ys > 1 { 0.5 } else { src_f.height() * scale / step.height() };
                ys -= 1;
            }
            let dst_f = scale_about_center(step, sx, sy);
            let dst = dst_f.round_out();
            passes.push(Pass::Rescale {
                dst,
                sample: Map::rect_to_rect(step, dst_f).invert(),
            });
            step = dst_f;
            pixels = dst;
        }
        // Low resolution pixel space to the layer.
        let to_layer = Map::rect_to_rect(step, src_f);
        let inv = [1.0 / to_layer.scale[0], 1.0 / to_layer.scale[1]];
        let low_sigma = [
            (sigma * inv[0]).min(MAX_LINEAR_SIGMA),
            (sigma * inv[1]).min(MAX_LINEAR_SIGMA),
        ];
        // The canvas in pixel space (`inverseMapRect` of an IRect, in
        // double with the round epsilon), bounded by the image outset by
        // the blur's reach, plus one pixel for the upscale.
        let (sx, sy) = (f64::from(to_layer.scale[0]), f64::from(to_layer.scale[1]));
        let (tx, ty) = (f64::from(to_layer.translate[0]), f64::from(to_layer.translate[1]));
        let e = f64::from(ROUND_EPSILON);
        let l = (0.0 - tx) / sx;
        let rr = (f64::from(width) - tx) / sx;
        let t = (0.0 - ty) / sy;
        let bb = (f64::from(height) - ty) / sy;
        let relative = Rect::ltrb(
            (l.min(rr) + e).floor() as i32,
            (t.min(bb) + e).floor() as i32,
            (l.max(rr) - e).ceil() as i32,
            (t.max(bb) - e).ceil() as i32,
        );
        let reach = pixels.outset(
            (3.0 * low_sigma[0]).ceil() as i32,
            (3.0 * low_sigma[1]).ceil() as i32,
        );
        let out = reach.intersect(&relative).outset(1, 1);
        shader_blur(low_sigma, pixels, out, &mut passes);
        Some(BlurPlan {
            passes,
            draw: to_layer.invert(),
            linear: true,
        })
    }
}


/// An 8-bit single-channel image on its pixel rectangle, transparent
/// outside.
#[derive(Debug, Clone)]
pub struct Image {
    pub rect: Rect,
    pub px: Vec<u8>,
}

impl Image {
    fn texel(&self, x: i32, y: i32) -> f32 {
        let (lx, ly) = (x - self.rect.x0, y - self.rect.y0);
        if lx < 0 || ly < 0 || lx >= self.rect.w as i32 || ly >= self.rect.h as i32 {
            return 0.0;
        }
        f32::from(self.px[(ly as usize) * self.rect.w as usize + lx as usize]) / 255.0
    }
    /// Bilinear at `(x, y)` in the image's space, including Metal's
    /// 8-bit interpolation weights and four fractional channel bits.
    pub fn bilinear(&self, x: f32, y: f32) -> f32 {
        self.bilinear_ring(x, y, 1)
    }
    fn bilinear_ring(&self, x: f32, y: f32, ring: u32) -> f32 {
        // Match sample_at's normalized coordinates before the sampler's
        // fixed-point interpolation. The initial RGBA layer has no ring;
        // filter intermediates and path-mask inputs have a 1-pixel ring.
        let (w,h) = ((self.rect.w+2*ring) as f32, (self.rect.h+2*ring) as f32);
        let u = ((x-(self.rect.x0-ring as i32) as f32)*(1.0/w))*w - ring as f32 - 0.5;
        let v = ((y-(self.rect.y0-ring as i32) as f32)*(1.0/h))*h - ring as f32 - 0.5;
        let (i, j) = (libm::floorf(u), libm::floorf(v));
        let (fx, fy) = ((256.0 * (u - i)).round()/256.0, (256.0 * (v - j)).round()/256.0);
        let (i, j) = (i as i32 + self.rect.x0, j as i32 + self.rect.y0);
        // Interpolate integer channel values before normalization. The
        // hardware's 8-bit weights and 4 fractional output bits make this
        // exact fixed-point arithmetic, including halfway decisions.
        let at = |x: i32, y: i32| {
            let (x,y) = (x-self.rect.x0, y-self.rect.y0);
            if x < 0 || y < 0 || x >= self.rect.w as i32 || y >= self.rect.h as i32 { 0.0 }
            else { f32::from(self.px[y as usize*self.rect.w as usize+x as usize]) }
        };
        let a = at(i, j) * (1.0 - fx) + at(i + 1, j) * fx;
        let b = at(i, j + 1) * (1.0 - fx) + at(i + 1, j + 1) * fx;
        ((a * (1.0 - fy) + b * fy) * 16.0 + 0.5).floor() / 4080.0
    }
    fn nearest(&self, x: f32, y: f32) -> f32 {
        self.texel(libm::floorf(x) as i32, libm::floorf(y) as i32)
    }
}

fn unorm8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

impl BlurPlan {
    /// Premultiplied RGBA, with the same plan and per-pass N32 storage as
    /// scalar coverage. Each channel follows precisely the same arithmetic.
    pub fn run_cpu_rgba(&self, rgba: &[u8], width: u32, height: u32) -> Vec<f32> {
        self.run_cpu_rgba_region(rgba, Rect::ltrb(0, 0, width as i32, height as i32), width, height)
    }

    /// The input need not cover the canvas: save layers may include
    /// off-canvas ink. Quantize the final filtered draw to N32 too.
    pub fn run_cpu_rgba_region(&self, rgba: &[u8], rect: Rect, width: u32, height: u32) -> Vec<f32> {
        assert_eq!(rgba.len(), (rect.w * rect.h * 4) as usize);
        let mut out = vec![0.0; (width*height*4) as usize];
        for c in 0..4 {
            let img = Image { rect, px: rgba.chunks_exact(4).map(|p| p[c]).collect() };
            for (p, v) in out.chunks_exact_mut(4).zip(self.run_cpu_ring(&img, width, height, 0)) {
                p[c] = f32::from(unorm8(v)) / 255.0;
            }
        }
        out
    }

    /// The plan on the CPU (the measured UNORM sampler, `f32` sums): a model of
    /// the GPU passes for tests and diagnostics. Returns each canvas
    /// pixel's value as the final draw samples it (before the composite).
    pub fn run_cpu(&self, item: &Image, width: u32, height: u32) -> Vec<f32> {
        self.run_cpu_ring(item, width, height, 1)
    }
    fn run_cpu_ring(&self, item: &Image, width: u32, height: u32, mut ring: u32) -> Vec<f32> {
        let mut img = item.clone();
        for pass in &self.passes {
            let dst = pass.dst();
            let mut px = vec![0u8; (dst.w * dst.h) as usize];
            for y in 0..dst.h as i32 {
                for x in 0..dst.w as i32 {
                    let (cx, cy) = ((dst.x0 + x) as f32 + 0.5, (dst.y0 + y) as f32 + 0.5);
                    let v = match pass {
                        Pass::Rescale { sample, .. } => img.bilinear_ring(
                            cx.mul_add(sample.scale[0], sample.translate[0]),
                            cy.mul_add(sample.scale[1], sample.translate[1]), ring,
                        ),
                        Pass::Blur2d { weights, offsets, .. } => {
                            let mut sum = 0.0f32;
                            for (w, o) in weights.iter().zip(offsets) {
                                sum = w.mul_add(img.nearest(cx + o[0], cy + o[1]), sum);
                            }
                            sum
                        }
                        Pass::Blur1d { dir, taps, .. } => {
                            let mut sum = 0.0f32;
                            for (o, w) in taps {
                                sum = w.mul_add(img.bilinear_ring(o.mul_add(dir[0], cx), o.mul_add(dir[1], cy), ring), sum);
                            }
                            sum
                        }
                    };
                    px[(y as u32 * dst.w + x as u32) as usize] = unorm8(v);
                }
            }
            img = Image { rect: dst, px };
            ring = 1;
        }
        let mut out = vec![0.0f32; (width * height) as usize];
        for y in 0..height {
            for x in 0..width {
                let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
                let (sx, sy) = (
                    cx.mul_add(self.draw.scale[0], self.draw.translate[0]),
                    cy.mul_add(self.draw.scale[1], self.draw.translate[1]),
                );
                out[(y * width + x) as usize] = if self.linear { img.bilinear_ring(sx, sy, ring) } else { img.nearest(sx, sy) };
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bilinear_uses_integer_channels_before_normalizing() {
        let img = Image { rect: Rect::ltrb(0, 0, 2, 1), px: vec![0,255] };
        // 9/256 of 255 is 8.96484375; the sampler rounds it to
        // 8.9375 code values before converting to normalized float.
        assert_eq!(img.bilinear(0.5+9.0/256.0, 0.5), 143.0/4080.0);
        assert_eq!(img.bilinear(0.5+8.0/256.0, 0.5), 8.0/255.0);
    }

    #[test]
    fn step_counts_follow_skia() {
        assert_eq!(downscale_step_count(1.0), 0);
        assert_eq!(downscale_step_count(0.9995), 0);
        assert_eq!(downscale_step_count(0.8), 1);
        assert_eq!(downscale_step_count(0.5), 1);
        assert_eq!(downscale_step_count(0.47), 1);
        assert_eq!(downscale_step_count(1.0 / 3.0), 2);
        assert_eq!(downscale_step_count(0.25), 2);
        assert_eq!(downscale_step_count(0.1), 4);
    }

    #[test]
    fn linear_kernel_sums_to_one_and_mirrors() {
        for r in 1..=12 {
            let taps = kernel_1d_linear(r as f32 / 3.0, r);
            let sum: f32 = taps.iter().map(|t| t.1).sum();
            assert!((sum - 1.0).abs() < 1e-5, "r {r}: {sum}");
            let n = r as usize + 1;
            for i in 0..n {
                assert!((taps[i].0 + taps[n - 1 - i].0).abs() < 1e-6, "r {r}");
            }
        }
    }

    #[test]
    fn small_sigmas_take_one_2d_pass() {
        let p = BlurPlan::new(100, 80, 0.5).unwrap();
        assert!(matches!(p.passes.as_slice(), [Pass::Blur2d { .. }]));
        let p = BlurPlan::new(100, 80, 0.7).unwrap();
        assert!(matches!(p.passes.as_slice(), [Pass::Blur1d { .. }, Pass::Blur1d { .. }]));
        assert!(BlurPlan::new(100, 80, 0.03).is_none());
        let p = BlurPlan::new(480, 300, 12.0).unwrap();
        assert_eq!(p.passes.iter().filter(|p| matches!(p, Pass::Rescale { .. })).count(), 2);
        assert!(p.linear);
    }
}
