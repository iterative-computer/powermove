//! Skia's image-filter evaluation of Blink's `feGaussianBlur` layer
//! (`SkCanvas::internalSaveLayer` sizing, `skif::FilterResult`, the
//! `SkShaderBlurAlgorithm` kernels Graphite runs as runtime shaders in
//! `half` precision, `FilterResult::rescale` for sigmas above 4, and the
//! final `drawSpecial` back into the device).
//!
//! Portions are derived from Skia, Copyright Google LLC, used under the BSD
//! 3-Clause license.

use super::blinksvg::ImageFilter;
use super::gradient::f16_round;
use super::svgrender::{DeviceInfo, LayerImage, Transform};
use crate::render::svg::SvgError;
use std::sync::Arc;

/// `kRoundEpsilon`.
const ROUND_EPSILON: f32 = 1e-3;
/// `SkShaderBlurAlgorithm::kMaxLinearSigma`.
const MAX_LINEAR_SIGMA: f32 = 4.0;
/// `kMaxSamples`.
const MAX_SAMPLES: usize = 28;

/// `skif::RoundOut`: inset by the epsilon, then round out.
fn round_out(r: [f32; 4]) -> [i32; 4] {
    let r = [r[0] + ROUND_EPSILON, r[1] + ROUND_EPSILON, r[2] - ROUND_EPSILON, r[3] - ROUND_EPSILON];
    [libm::floorf(r[0]) as i32, libm::floorf(r[1]) as i32, libm::ceilf(r[2]) as i32, libm::ceilf(r[3]) as i32]
}

fn iintersect(a: [i32; 4], b: [i32; 4]) -> [i32; 4] {
    let r = [a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].min(b[3])];
    if r[2] <= r[0] || r[3] <= r[1] { [0, 0, 0, 0] } else { r }
}

fn iempty(r: [i32; 4]) -> bool {
    r[2] <= r[0] || r[3] <= r[1]
}

fn ioutset(r: [i32; 4], dx: i32, dy: i32) -> [i32; 4] {
    [r[0] - dx, r[1] - dy, r[2] + dx, r[3] + dy]
}

/// `SkBlurEngine::SigmaToRadius`.
fn sigma_to_radius(sigma: f32) -> i32 {
    if sigma <= 0.03 { 0 } else { libm::ceilf(3.0 * sigma) as i32 }
}

/// The blur's layer-space sigma (`mapSigma`: `paramToLayer` of the sigma
/// through the layer matrix, clamped, identity below 0.03).
fn layer_sigma(sigma: [f32; 2], ltd: &Transform) -> [f32; 2] {
    // Mapping::map(SkSize): the lengths of the mapped axis vectors.
    let m = ltd.m;
    let sx = (m.sx * m.sx + m.ky * m.ky).sqrt() * sigma[0];
    let sy = (m.kx * m.kx + m.sy * m.sy).sqrt() * sigma[1];
    let clamp = |v: f32| if !v.is_finite() || v <= 0.03 { 0.0 } else { v.min(532.0) };
    [clamp(sx), clamp(sy)]
}

/// The crop rect in layer space (`SkCropImageFilter::cropRect`, decal:
/// `RoundOut`).
fn layer_crop(crop: [f32; 4], ltd: &Transform) -> [i32; 4] {
    round_out(ltd.map_rect(crop))
}

/// `get_layer_mapping_and_bounds`: the input bounds the filter needs for
/// the clip bounds (the layer texture before padding).
pub fn input_layer_bounds(f: &ImageFilter, ltd: &Transform, clip_bounds: [i32; 4]) -> [i32; 4] {
    match f {
        ImageFilter::GaussianBlur { sigma, crop, .. } => {
            let sigma = layer_sigma(*sigma, ltd);
            // Crop.requiredInput: the crop within the desired output.
            let crop = layer_crop(*crop, ltd);
            let required = iintersect(crop, clip_bounds);
            if iempty(required) {
                return [0, 0, 0, 0];
            }
            // Blur.kernelBounds: outset by ceil(3 sigma).
            let rx = libm::ceilf(3.0 * sigma[0]) as i32;
            let ry = libm::ceilf(3.0 * sigma[1]) as i32;
            ioutset(required, rx, ry)
        }
    }
}

/// An intermediate image: premultiplied RGBA8 with its layer-space origin,
/// one pixel of transparent padding around the subset (`PixelBoundary::kTransparent`).
#[derive(Debug, Clone)]
struct Img {
    /// Including padding.
    w: usize,
    h: usize,
    rgba: Vec<u8>,
    /// Layer-space origin of pixel (1, 1) (the subset's top-left).
    origin: [i32; 2],
}

impl Img {
    fn texel(&self, x: i32, y: i32) -> [f32; 4] {
        // Clamp sampling (the HW sampler on the padded image).
        let xi = (x + 1).clamp(0, self.w as i32 - 1) as usize;
        let yi = (y + 1).clamp(0, self.h as i32 - 1) as usize;
        let i = (yi * self.w + xi) * 4;
        let s = &self.rgba[i..i + 4];
        [f16_round(f32::from(s[0]) / 255.0), f16_round(f32::from(s[1]) / 255.0), f16_round(f32::from(s[2]) / 255.0), f16_round(f32::from(s[3]) / 255.0)]
    }

    /// Bilinear sample at a subset-relative position (pixel centers at
    /// integer + 0.5), with the sampler's 1/256 weight quantization.
    fn sample_linear(&self, x: f32, y: f32) -> [f32; 4] {
        let fx = x - 0.5;
        let fy = y - 0.5;
        let x0 = libm::floorf(fx);
        let y0 = libm::floorf(fy);
        let wx = libm::roundf((fx - x0) * 256.0) / 256.0;
        let wy = libm::roundf((fy - y0) * 256.0) / 256.0;
        let (x0, y0) = (x0 as i32, y0 as i32);
        let c00 = self.texel(x0, y0);
        let c10 = self.texel(x0 + 1, y0);
        let c01 = self.texel(x0, y0 + 1);
        let c11 = self.texel(x0 + 1, y0 + 1);
        let mut out = [0.0f32; 4];
        for c in 0..4 {
            let top = c00[c] + (c10[c] - c00[c]) * wx;
            let bottom = c01[c] + (c11[c] - c01[c]) * wx;
            out[c] = f16_round(top + (bottom - top) * wy);
        }
        out
    }

    fn subset_w(&self) -> usize {
        self.w - 2
    }

    fn subset_h(&self) -> usize {
        self.h - 2
    }
}

fn new_img(w: usize, h: usize, origin: [i32; 2]) -> Img {
    Img { w: w + 2, h: h + 2, rgba: vec![0; (w + 2) * (h + 2) * 4], origin }
}

fn put(img: &mut Img, x: usize, y: usize, c: [f32; 4]) {
    let i = ((y + 1) * img.w + x + 1) * 4;
    for k in 0..4 {
        img.rgba[i + k] = super::msaa::unorm8(c[k]);
    }
}

/// `SkShaderBlurAlgorithm::Compute1DBlurKernel`.
fn kernel_1d(sigma: f32, radius: i32) -> Vec<f32> {
    let width = (2 * radius + 1) as usize;
    let two_sigma_sq = 2.0 * sigma * sigma;
    let denom = if radius > 0 { 1.0 / two_sigma_sq } else { 1.0 };
    let mut k = vec![0.0f32; width];
    let mut sum = 0.0f32;
    for (x, v) in k.iter_mut().enumerate() {
        let term = (x as i32 - radius) as f32;
        let e = libm::expf(-(term * term * denom));
        *v = e;
        sum += e;
    }
    let scale = 1.0 / sum;
    for v in &mut k {
        *v *= scale;
    }
    k
}

/// `Compute1DBlurLinearKernel`: (offset, weight) pairs, as the `half4`
/// uniforms hold them.
fn linear_kernel(sigma: f32, radius: i32) -> Vec<(f32, f32)> {
    let full = kernel_1d(sigma, radius);
    // LinearKernelWidth(radius) = radius + 1.
    let half_size = radius as usize + 1;
    let half_radius = half_size / 2;
    let mut kernel = vec![0.0f32; half_size];
    let mut offsets = vec![0.0f32; half_size];
    let mut low_index = half_radius as i64 - 1;
    let r = radius as usize;
    let mut index = r;
    let new_weight = |wi: f32, wj: f32| -> (f32, f32) { (wi + wj, wj / (wi + wj)) };
    if radius % 2 == 1 {
        let (w, o) = new_weight(full[index] * 0.5, full[index + 1]);
        kernel[half_radius] = w;
        offsets[half_radius] = o;
        if low_index >= 0 {
            kernel[low_index as usize] = w;
            offsets[low_index as usize] = -o;
        }
        index += 1;
        low_index -= 1;
    } else {
        kernel[half_radius] = full[index];
        offsets[half_radius] = 0.0;
    }
    index += 1;
    let mut i = half_radius + 1;
    while i < half_size {
        let (w, mut o) = new_weight(full[index], full[index + 1]);
        o += (index - r) as f32;
        kernel[i] = w;
        offsets[i] = o;
        if low_index >= 0 {
            kernel[low_index as usize] = w;
            offsets[low_index as usize] = -o;
        }
        index += 2;
        i += 1;
        low_index -= 1;
    }
    // Uploaded as half4: both rounded to half.
    offsets.iter().zip(&kernel).map(|(o, k)| (f16_round(*o), f16_round(*k))).collect()
}

/// One 1D linear-sampled blur pass over `src`, producing `dst_rect`
/// (layer/pixel space) sampled from the source's clamped padded image.
fn blur_1d(src: &Img, sigma: f32, radius: i32, dir: [f32; 2], dst_rect: [i32; 4]) -> Img {
    let taps = linear_kernel(sigma, radius);
    let (w, h) = ((dst_rect[2] - dst_rect[0]) as usize, (dst_rect[3] - dst_rect[1]) as usize);
    let mut out = new_img(w, h, [dst_rect[0], dst_rect[1]]);
    for y in 0..h {
        for x in 0..w {
            // The fragment's coordinate in the source subset's pixel space.
            let cx = (dst_rect[0] + x as i32 - src.origin[0]) as f32 + 0.5;
            let cy = (dst_rect[1] + y as i32 - src.origin[1]) as f32 + 0.5;
            let mut sum = [0.0f32; 4];
            for (o, k) in &taps {
                // coord + s.x * dir (half products), child.eval (linear).
                let ox = f16_round(*o * dir[0]);
                let oy = f16_round(*o * dir[1]);
                let s = src.sample_linear(cx + ox, cy + oy);
                for c in 0..4 {
                    sum[c] = f16_round(f64_fma(*k, s[c], sum[c]));
                }
            }
            put(&mut out, x, y, sum);
        }
    }
    out
}

#[inline]
fn f64_fma(a: f32, b: f32, c: f32) -> f32 {
    (f64::from(a) * f64::from(b) + f64::from(c)) as f32
}

/// `Compute2DBlurKernel` and the single-pass 2D blur.
fn blur_2d(src: &Img, sigma: [f32; 2], radius: [i32; 2], dst_rect: [i32; 4]) -> Img {
    let width = (2 * radius[0] + 1) as usize;
    let height = (2 * radius[1] + 1) as usize;
    let denom_x = if radius[0] > 0 { 1.0 / (2.0 * sigma[0] * sigma[0]) } else { 1.0 };
    let denom_y = if radius[1] > 0 { 1.0 / (2.0 * sigma[1] * sigma[1]) } else { 1.0 };
    let mut kernel = vec![0.0f32; width * height];
    let mut sum = 0.0f32;
    for x in 0..width {
        let xt = (x as i32 - radius[0]) as f32;
        let xt = xt * xt * denom_x;
        for y in 0..height {
            let yt = (y as i32 - radius[1]) as f32;
            let v = libm::expf(-(xt + yt * yt * denom_y));
            kernel[y * width + x] = v;
            sum += v;
        }
    }
    let scale = 1.0 / sum;
    for v in &mut kernel {
        *v = f16_round(*v * scale);
    }
    let (w, h) = ((dst_rect[2] - dst_rect[0]) as usize, (dst_rect[3] - dst_rect[1]) as usize);
    let mut out = new_img(w, h, [dst_rect[0], dst_rect[1]]);
    for y in 0..h {
        for x in 0..w {
            let cx = dst_rect[0] + x as i32 - src.origin[0];
            let cy = dst_rect[1] + y as i32 - src.origin[1];
            let mut acc = [0.0f32; 4];
            let mut i = 0;
            for oy in -radius[1]..=radius[1] {
                for ox in -radius[0]..=radius[0] {
                    let k = kernel[i];
                    i += 1;
                    let s = src.texel(cx + ox, cy + oy);
                    for c in 0..4 {
                        acc[c] = f16_round(f64_fma(k, s[c], acc[c]));
                    }
                }
            }
            put(&mut out, x, y, acc);
        }
    }
    out
}

/// `SkShaderBlurAlgorithm::blur`: a 2D pass when the kernel fits, else an
/// X pass then a Y pass.
fn shader_blur(src: &Img, sigma: [f32; 2], dst_rect: [i32; 4]) -> Img {
    let rx = sigma_to_radius(sigma[0]);
    let ry = sigma_to_radius(sigma[1]);
    let area = (2 * rx + 1) * (2 * ry + 1);
    if area as usize <= MAX_SAMPLES && rx > 0 && ry > 0 {
        return blur_2d(src, sigma, [rx, ry], dst_rect);
    }
    let src_rect = [src.origin[0], src.origin[1], src.origin[0] + src.subset_w() as i32, src.origin[1] + src.subset_h() as i32];
    let mut cur = src.clone();
    if rx > 0 {
        let mut intermediate = dst_rect;
        if ry > 0 {
            intermediate = ioutset(intermediate, 0, ry);
            intermediate[1] = intermediate[1].max(src_rect[1]);
            intermediate[3] = intermediate[3].min(src_rect[3]);
        }
        cur = blur_1d(&cur, sigma[0], rx, [1.0, 0.0], intermediate);
    }
    if ry > 0 {
        cur = blur_1d(&cur, sigma[1], ry, [0.0, 1.0], dst_rect);
    }
    cur
}

/// `sRGB` ↔ linear on an unpremultiplied channel (`SkColorSpaceXformSteps`).
fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 { v / 12.92 } else { libm::powf((v + 0.055) / 1.055, 2.4) }
}

fn linear_to_srgb(v: f32) -> f32 {
    if v <= 0.0031308 { v * 12.92 } else { 1.055 * libm::powf(v, 1.0 / 2.4) - 0.055 }
}

/// A color filter applied to premultiplied RGBA (unpremul, transform, premul).
fn color_filter(c: [f32; 4], to_linear: bool) -> [f32; 4] {
    let a = c[3];
    if a == 0.0 {
        return [0.0; 4];
    }
    let f = if to_linear { srgb_to_linear } else { linear_to_srgb };
    let mut out = [0.0f32; 4];
    for i in 0..3 {
        out[i] = f((c[i] / a).clamp(0.0, 1.0)) * a;
    }
    out[3] = a;
    out
}

/// `downscale_step_count`.
fn downscale_step_count(net_scale: f32) -> i32 {
    let mut steps = next_log2(libm::ceilf(1.0 / net_scale) as i32);
    if steps > 0 {
        let final_step_scale = net_scale * (1 << (steps - 1)) as f32;
        let limit = if steps == 1 { 1.0 - ROUND_EPSILON } else { 0.9 };
        if final_step_scale >= limit {
            steps -= 1;
        }
    }
    steps
}

fn next_log2(v: i32) -> i32 {
    if v <= 1 {
        return 0;
    }
    32 - (v - 1).leading_zeros() as i32
}

/// `scale_about_center`.
fn scale_about_center(src: [f32; 4], sx: f32, sy: f32) -> [f32; 4] {
    let cx = if sx == 1.0 { 0.0 } else { 0.5 * src[0] + 0.5 * src[2] };
    let cy = if sy == 1.0 { 0.0 } else { 0.5 * src[1] + 0.5 * src[3] };
    [(src[0] - cx) * sx, (src[1] - cy) * sy, (src[2] - cx) * sx, (src[3] - cy) * sy]
}

/// `FilterResult::rescale` for a decal-tiled, pixel-aligned image with a
/// pending color filter: each step draws the previous image with bilinear
/// sampling into a surface of the rounded-out scaled bounds.
fn rescale(src: &Img, src_rect: [i32; 4], scale: [f32; 2], linear: bool) -> (Img, [f32; 4]) {
    let mut x_steps = downscale_step_count(scale[0]);
    let mut y_steps = downscale_step_count(scale[1]);
    let src_rect_f = [src_rect[0] as f32, src_rect[1] as f32, src_rect[2] as f32, src_rect[3] as f32];
    let mut step_bounds = src_rect_f;
    let final_scale_x = if x_steps > 0 { 1.0 / (1 << x_steps) as f32 } else { 1.0 };
    let final_scale_y = if y_steps > 0 { 1.0 / (1 << y_steps) as f32 } else { 1.0 };
    let mut image = src.clone();
    let mut first = true;
    loop {
        let mut sx = 1.0f32;
        if x_steps > 0 {
            sx = if x_steps > 1 { 0.5 } else { (src_rect_f[2] - src_rect_f[0]) * final_scale_x / (step_bounds[2] - step_bounds[0]) };
            x_steps -= 1;
        }
        let mut sy = 1.0f32;
        if y_steps > 0 {
            sy = if y_steps > 1 { 0.5 } else { (src_rect_f[3] - src_rect_f[1]) * final_scale_y / (step_bounds[3] - step_bounds[1]) };
            y_steps -= 1;
        }
        let dst_bounds = scale_about_center(step_bounds, sx, sy);
        let dst_pixels = [libm::floorf(dst_bounds[0]) as i32, libm::floorf(dst_bounds[1]) as i32, libm::ceilf(dst_bounds[2]) as i32, libm::ceilf(dst_bounds[3]) as i32];
        let (w, h) = ((dst_pixels[2] - dst_pixels[0]) as usize, (dst_pixels[3] - dst_pixels[1]) as usize);
        let mut out = new_img(w, h, [dst_pixels[0], dst_pixels[1]]);
        // RectToRect(stepBounds, dstBounds): dst pixel centers map back.
        let kx = (step_bounds[2] - step_bounds[0]) / (dst_bounds[2] - dst_bounds[0]);
        let ky = (step_bounds[3] - step_bounds[1]) / (dst_bounds[3] - dst_bounds[1]);
        for y in 0..h {
            for x in 0..w {
                let dx = dst_pixels[0] as f32 + x as f32 + 0.5;
                let dy = dst_pixels[1] as f32 + y as f32 + 0.5;
                let lx = step_bounds[0] + (dx - dst_bounds[0]) * kx;
                let ly = step_bounds[1] + (dy - dst_bounds[1]) * ky;
                // Sample the previous image (subset-relative coordinates).
                let px = lx - image.origin[0] as f32;
                let py = ly - image.origin[1] as f32;
                let mut c = if first && sx == 1.0 && sy == 1.0 {
                    image.texel(libm::floorf(px) as i32, libm::floorf(py) as i32)
                } else {
                    image.sample_linear(px, py)
                };
                if first && linear {
                    c = color_filter(c, true);
                }
                put(&mut out, x, y, c);
            }
        }
        first = false;
        image = out;
        step_bounds = dst_bounds;
        if x_steps <= 0 && y_steps <= 0 {
            break;
        }
    }
    (image, step_bounds)
}

/// Evaluates the blur filter on the rendered layer and returns the draws
/// (device rect, geometry) that composite its result into the parent.
pub fn filter_layer(
    f: &ImageFilter,
    ltd: &Transform,
    layer_bounds: &[i32; 4],
    width: u32,
    height: u32,
    image: &[u8],
    parent: &DeviceInfo,
) -> Result<Vec<([f32; 4], Arc<LayerImage>)>, SvgError> {
    let ImageFilter::GaussianBlur { sigma, crop, linear } = f;
    let sigma = layer_sigma(*sigma, ltd);
    // The source: the layer without its padding pixel.
    let src_rect = [layer_bounds[0] + 1, layer_bounds[1] + 1, layer_bounds[2] - 1, layer_bounds[3] - 1];
    let (sw, sh) = ((src_rect[2] - src_rect[0]) as usize, (src_rect[3] - src_rect[1]) as usize);
    if sw == 0 || sh == 0 || width as usize != sw + 2 || height as usize != sh + 2 {
        return Ok(Vec::new());
    }
    let src = Img { w: width as usize, h: height as usize, rgba: image.to_vec(), origin: [src_rect[0], src_rect[1]] };
    let desired = parent.clip_bounds;
    let crop_layer = layer_crop(*crop, ltd);
    let crop_desired = iintersect(crop_layer, desired);
    if iempty(crop_desired) {
        return Ok(Vec::new());
    }
    let radii = [libm::ceilf(3.0 * sigma[0]) as i32, libm::ceilf(3.0 * sigma[1]) as i32];
    let max_output = ioutset(src_rect, radii[0], radii[1]);
    let output_bounds = iintersect(crop_desired, max_output);
    if iempty(output_bounds) {
        return Ok(Vec::new());
    }
    let sample_bounds = ioutset(output_bounds, radii[0], radii[1]);
    let sx = if sigma[0] > MAX_LINEAR_SIGMA { MAX_LINEAR_SIGMA / sigma[0] } else { 1.0 };
    let sy = if sigma[1] > MAX_LINEAR_SIGMA { MAX_LINEAR_SIGMA / sigma[1] } else { 1.0 };
    // rescale: visible layer bounds = src ∩ sample bounds.
    let visible = iintersect(src_rect, sample_bounds);
    if iempty(visible) {
        return Ok(Vec::new());
    }
    let (low, low_bounds_f) = rescale(&src, visible, [sx, sy], *linear);
    let inv_scale_x = (low_bounds_f[2] - low_bounds_f[0]) / (visible[2] - visible[0]) as f32;
    let inv_scale_y = (low_bounds_f[3] - low_bounds_f[1]) / (visible[3] - visible[1]) as f32;
    let scale_x = 1.0 / inv_scale_x;
    let scale_y = 1.0 / inv_scale_y;
    let low_sigma = [(sigma[0] * inv_scale_x).min(MAX_LINEAR_SIGMA), (sigma[1] * inv_scale_y).min(MAX_LINEAR_SIGMA)];
    // The output in the low-res pixel space: outputBounds mapped by the
    // inverse transform (RoundOut), intersected with the low image outset by
    // the low-res radii, then one pixel more.
    let low_rect = [low.origin[0], low.origin[1], low.origin[0] + low.subset_w() as i32, low.origin[1] + low.subset_h() as i32];
    let map_inv = |r: [i32; 4]| -> [f32; 4] {
        [
            low_bounds_f[0] + (r[0] as f32 - visible[0] as f32) * inv_scale_x,
            low_bounds_f[1] + (r[1] as f32 - visible[1] as f32) * inv_scale_y,
            low_bounds_f[0] + (r[2] as f32 - visible[0] as f32) * inv_scale_x,
            low_bounds_f[1] + (r[3] as f32 - visible[1] as f32) * inv_scale_y,
        ]
    };
    let mut src_relative = round_out(map_inv(output_bounds));
    let low_max = ioutset(low_rect, libm::ceilf(3.0 * low_sigma[0]) as i32, libm::ceilf(3.0 * low_sigma[1]) as i32);
    src_relative = iintersect(low_max, src_relative);
    if iempty(src_relative) {
        return Ok(Vec::new());
    }
    src_relative = ioutset(src_relative, 1, 1);
    let blurred = shader_blur(&low, low_sigma, src_relative);
    // insetByPixel, then the transform back to layer space.
    let inset = [src_relative[0] + 1, src_relative[1] + 1, src_relative[2] - 1, src_relative[3] - 1];
    // The result image maps low-res pixel p to layer x = visible.x + (p - low_bounds.x) * scale.
    let to_layer = |px: f32, py: f32| -> [f32; 2] {
        [visible[0] as f32 + (px - low_bounds_f[0]) * scale_x, visible[1] as f32 + (py - low_bounds_f[1]) * scale_y]
    };
    // Final draw: the image with the (upscaling) transform, bilinear unless
    // pixel aligned, through the color filter, cropped to the layer bounds
    // (the decal crop) with an AA clip when it extends past them.
    let final_bounds = iintersect(crop_desired, round_out({
        let a = to_layer(inset[0] as f32, inset[1] as f32);
        let b = to_layer(inset[2] as f32, inset[3] as f32);
        [a[0], a[1], b[0], b[1]]
    }));
    if iempty(final_bounds) {
        return Ok(Vec::new());
    }
    let pixel_aligned = (scale_x - 1.0).abs() < 1e-6 && (scale_y - 1.0).abs() < 1e-6;
    let (w, h) = ((final_bounds[2] - final_bounds[0]) as usize, (final_bounds[3] - final_bounds[1]) as usize);
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let lx = final_bounds[0] as f32 + x as f32 + 0.5;
            let ly = final_bounds[1] as f32 + y as f32 + 0.5;
            // Layer → low-res pixel coordinates (subset relative).
            let px = low_bounds_f[0] + (lx - visible[0] as f32) * inv_scale_x - blurred.origin[0] as f32;
            let py = low_bounds_f[1] + (ly - visible[1] as f32) * inv_scale_y - blurred.origin[1] as f32;
            let mut c = if pixel_aligned {
                blurred.texel(libm::floorf(px) as i32, libm::floorf(py) as i32)
            } else {
                blurred.sample_linear(px, py)
            };
            if *linear {
                c = color_filter(c, false);
            }
            let i = (y * w + x) * 4;
            for k in 0..4 {
                rgba[i + k] = super::msaa::unorm8(c[k]);
            }
        }
    }
    let img = Arc::new(LayerImage { origin: [final_bounds[0], final_bounds[1]], width: w as u32, height: h as u32, rgba, alpha: 1.0 });
    let rect = [final_bounds[0] as f32, final_bounds[1] as f32, final_bounds[2] as f32, final_bounds[3] as f32];
    Ok(vec![(rect, img)])
}
