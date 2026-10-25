//! The exact separable Gaussian blur for path mask feathers
//! ([`super::path_masks`]), on the GPU (`gaussian.wgsl`), with a CPU model
//! of the same plan.
//!
//! Pixels are squares, transparent outside the source. The blur of
//! standard deviation σ px weighs the pixel `k` away by
//! `w(k) = Φ((k + ½)/σ) − Φ((k − ½)/σ)` (the Gaussian of the box
//! reconstruction; `pm_reference::gaussian`), per axis.
//!
//! **Plan.** Up to σ ≈ 5.6 the passes sum each source row, then each
//! column, over `|k| ≤ ⌈4.5σ⌉` (the tails beyond hold
//! `2Φ(−4.5) < 6.8·10⁻⁶`, and the kernel is normalized). Beyond, the exact
//! blur is evaluated at nodes `s = ⌊0.36σ⌋` px apart, in two stages, and
//! interpolated between them:
//!
//! 1. The box-integrated Gaussian of σₐ = s at the nodes (exact).
//! 2. The Gaussian of σ_b = √(σ² − σₐ²) on the node lattice (a
//!    trapezoid sum of a function smooth at σₐ: its error is below
//!    `e^{−2π²·0.87} < 10⁻⁷`), so the nodes hold the blur of σ.
//! 3. 6-point Lagrange interpolation along rows, then columns. Per axis
//!    its error is at most `max|ω₆|/6! · s⁶ · |∂⁶f|`, `|∂⁶f| ≤ ∫(G⁽⁶⁾)₊ =
//!    6.908/σ⁶` for any source in `[0, 1]`, and the second axis carries the
//!    first's through the Lebesgue constant 1.391: at most
//!    `0.0806 · (s/σ)⁶ < 1.76·10⁻⁴`.
//!
//! With the kernels' tails (5.3·10⁻⁵ through the interpolation) the plan is
//! within 2⁻¹² of the exact blur, half an `r16float` step below 1. Every
//! pixel costs `O(1)` taps at any σ.

use pm_vector::render::scene::PixelRect;
use wgpu::util::DeviceExt;

use crate::pipelines::{LazyComputePipeline, LazyRenderPipeline, bytes_of};

/// Kernel reach in σ.
const TAIL: f64 = 4.5;
/// Node spacing per σ (module docs).
const STEP_PER_SIGMA: f64 = 0.36;
/// The longest kernel stored: twice the largest target side.
const MAX_RADIUS: i32 = 1 << 15;

/// One axis of the blur.
#[derive(Debug, Clone, PartialEq)]
pub struct Kernel {
    /// Node spacing, px.
    pub step: i32,
    /// Stage 1, the box-integrated Gaussian of σ (or σₐ = `step`):
    /// `radius` px each way, normalized.
    pub radius: i32,
    pub weights: Vec<f32>,
    /// Stage 2, the Gaussian of σ_b on the node lattice: `coarse_radius`
    /// nodes each way, normalized (the identity when `step` is 1).
    pub coarse_radius: i32,
    pub coarse: Vec<f32>,
    /// Where the blur of a pixel is kept, px: ⌈4.5σ⌉ (beyond, less than
    /// `Φ(−4.5) < 3.4·10⁻⁶` of it).
    pub extent: i32,
}

/// Where stage 1 evaluates the blur: `size` nodes `step` px apart from
/// `org` (target px).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grid {
    pub org: [i32; 2],
    pub size: [u32; 2],
    pub step: i32,
}

/// Standard normal CDF.
fn phi(x: f64) -> f64 {
    0.5 * libm::erfc(-x / core::f64::consts::SQRT_2)
}

/// The box-integrated Gaussian of `sigma` px: its radius and weights.
fn box_gaussian(sigma: f64) -> (i32, Vec<f32>) {
    let full = (TAIL * sigma).ceil();
    let radius = full.min(f64::from(MAX_RADIUS)) as i32;
    // The smaller tail of each difference keeps precision far out.
    let w = |k: f64| {
        let k = k.abs();
        phi(-(k - 0.5) / sigma) - phi(-(k + 0.5) / sigma)
    };
    let total = 1.0 - 2.0 * phi(-(full + 0.5) / sigma);
    let weights = (-radius..=radius)
        .map(|k| (w(f64::from(k)) / total) as f32)
        .collect();
    (radius, weights)
}

/// Lagrange weights of the nodes at −2 … 3 for `t` in [0, 1).
fn lagrange6(t: f64) -> [f64; 6] {
    let (a, b, d, e, f) = (t + 2.0, t + 1.0, t - 1.0, t - 2.0, t - 3.0);
    [
        -(b * t * d * e * f) / 120.0,
        (a * t * d * e * f) / 24.0,
        -(a * b * d * e * f) / 12.0,
        (a * b * t * e * f) / 12.0,
        -(a * b * t * d * f) / 24.0,
        (a * b * t * d * e) / 120.0,
    ]
}

impl Kernel {
    /// The blur of `sigma` px, or `None` when it is the identity (σ not
    /// positive, or not finite: CSS rejects `blur(NaN px)`).
    pub fn new(sigma: f64) -> Option<Kernel> {
        if !(sigma > 0.0 && sigma.is_finite()) {
            return None;
        }
        let step = (STEP_PER_SIGMA * sigma)
            .floor()
            .clamp(1.0, f64::from(MAX_RADIUS)) as i32;
        if step == 1 {
            let (radius, weights) = box_gaussian(sigma);
            return Some(Kernel {
                step,
                radius,
                weights,
                coarse_radius: 0,
                coarse: vec![1.0],
                extent: radius,
            });
        }
        let s = f64::from(step);
        let (radius, weights) = box_gaussian(s);
        let coarse_sigma = (sigma * sigma - s * s).sqrt() / s;
        let coarse_radius = (TAIL * coarse_sigma).ceil() as i32;
        let g: Vec<f64> = (-coarse_radius..=coarse_radius)
            .map(|d| (-f64::from(d * d) / (2.0 * coarse_sigma * coarse_sigma)).exp())
            .collect();
        let total: f64 = g.iter().sum();
        Some(Kernel {
            step,
            radius,
            weights,
            coarse_radius,
            coarse: g.iter().map(|v| (v / total) as f32).collect(),
            extent: (TAIL * sigma).ceil().min(f64::from(MAX_RADIUS)) as i32,
        })
    }

    /// The identity.
    pub fn identity() -> Kernel {
        Kernel {
            step: 1,
            radius: 0,
            weights: vec![1.0],
            coarse_radius: 0,
            coarse: vec![1.0],
            extent: 0,
        }
    }

    /// Stage 1's nodes for `out`: its pixels, or with spacing the final
    /// nodes (two before and three after each output pixel's for the
    /// interpolation) and stage 2's margin.
    pub fn grid(&self, out: PixelRect) -> Grid {
        let s = self.step;
        if s == 1 {
            return Grid {
                org: [out.x, out.y],
                size: [out.width, out.height],
                step: 1,
            };
        }
        let margin = 2 + self.coarse_radius;
        let n = |len: u32| ((len.max(1) - 1) / s as u32 + 6) + 2 * self.coarse_radius as u32;
        Grid {
            org: [out.x - margin * s, out.y - margin * s],
            size: [n(out.width), n(out.height)],
            step: s,
        }
    }

    /// The CPU model of the GPU passes: `src` (`channels` values per pixel
    /// over `rect`, transparent outside) blurred, over `out`.
    pub fn blur_cpu(
        &self,
        src: &[f32],
        rect: PixelRect,
        channels: usize,
        out: PixelRect,
    ) -> Vec<f32> {
        let g = self.grid(out);
        let (gw, gh) = (g.size[0] as usize, g.size[1] as usize);
        let (sw, sh) = (rect.width as i32, rect.height as i32);
        let (r, ch) = (self.radius, channels);
        let w = |d: i32| f64::from(self.weights[(d + r) as usize]);
        // Stage 1.
        let mut h = vec![0.0f64; gw * sh as usize * ch];
        for j in 0..sh {
            for k in 0..gw {
                let x = g.org[0] + k as i32 * g.step - rect.x;
                let o = (j as usize * gw + k) * ch;
                for d in (-r).max(-x)..=r.min(sw - 1 - x) {
                    let s = (j * sw + x + d) as usize * ch;
                    for c in 0..ch {
                        h[o + c] += w(d) * f64::from(src[s + c]);
                    }
                }
            }
        }
        let mut u = vec![0.0f64; gw * gh * ch];
        for m in 0..gh {
            let y = g.org[1] + m as i32 * g.step - rect.y;
            for k in 0..gw {
                let o = (m * gw + k) * ch;
                for d in (-r).max(-y)..=r.min(sh - 1 - y) {
                    let s = ((y + d) as usize * gw + k) * ch;
                    for c in 0..ch {
                        u[o + c] += w(d) * h[s + c];
                    }
                }
            }
        }
        let (ow, oh) = (out.width as usize, out.height as usize);
        if g.step == 1 {
            return u.iter().map(|v| *v as f32).collect();
        }
        // Stage 2.
        let cr = self.coarse_radius;
        let cw = |d: i32| f64::from(self.coarse[(d + cr) as usize]);
        let (nw, nh) = (gw - 2 * cr as usize, gh - 2 * cr as usize);
        let mut h2 = vec![0.0f64; nw * gh * ch];
        for m in 0..gh {
            for k in 0..nw {
                for d in -cr..=cr {
                    let s = (m * gw + (k as i32 + cr + d) as usize) * ch;
                    for c in 0..ch {
                        h2[(m * nw + k) * ch + c] += cw(d) * u[s + c];
                    }
                }
            }
        }
        let mut v = vec![0.0f64; nw * nh * ch];
        for m in 0..nh {
            for k in 0..nw {
                for d in -cr..=cr {
                    let s = ((m as i32 + cr + d) as usize * nw + k) * ch;
                    for c in 0..ch {
                        v[(m * nw + k) * ch + c] += cw(d) * h2[s + c];
                    }
                }
            }
        }
        // Interpolation: rows, then columns.
        let origin = [g.org[0] + cr * g.step, g.org[1] + cr * g.step];
        let at = |p: i32, axis: usize| {
            let f = p - origin[axis];
            let k = f / g.step;
            (
                k - 2,
                lagrange6(f64::from(f - k * g.step) / f64::from(g.step)),
            )
        };
        let mut ix = vec![0.0f64; ow * nh * ch];
        for m in 0..nh {
            for x in 0..ow {
                let (k, l) = at(out.x + x as i32, 0);
                for (a, la) in l.iter().enumerate() {
                    let s = (m * nw + (k + a as i32) as usize) * ch;
                    for c in 0..ch {
                        ix[(m * ow + x) * ch + c] += la * v[s + c];
                    }
                }
            }
        }
        let mut result = vec![0.0f32; ow * oh * ch];
        for y in 0..oh {
            let (k, l) = at(out.y + y as i32, 1);
            for x in 0..ow {
                for c in 0..ch {
                    let sum: f64 = l
                        .iter()
                        .enumerate()
                        .map(|(b, lb)| lb * ix[((k + b as i32) as usize * ow + x) * ch + c])
                        .sum();
                    result[(y * ow + x) * ch + c] = sum as f32;
                }
            }
        }
        result
    }
}

/// `r` grown by `d` px each way.
pub(crate) fn inflate(r: PixelRect, d: i32) -> PixelRect {
    PixelRect::new(
        r.x - d,
        r.y - d,
        r.width + 2 * d as u32,
        r.height + 2 * d as u32,
    )
}

/// `P` in `gaussian.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Params {
    pub src: [i32; 4],
    pub grid: [i32; 4],
    pub out: [i32; 4],
    pub dst: [i32; 4],
    pub step: i32,
    pub radius: i32,
    pub mode: i32,
    pub wbase: i32,
    pub pad: [i32; 2],
    pub cradius: i32,
    pub cwbase: i32,
}

pub(crate) fn rect4(r: PixelRect) -> [i32; 4] {
    [r.x, r.y, r.width as i32, r.height as i32]
}

impl Params {
    /// The blur of the source `src` by `kernel` over `out`, its weights
    /// at `wbase` of the weights buffer.
    pub(crate) fn blur(kernel: &Kernel, src: PixelRect, out: PixelRect, wbase: i32) -> Params {
        let g = kernel.grid(out);
        Params {
            src: rect4(src),
            grid: [g.org[0], g.org[1], g.size[0] as i32, g.size[1] as i32],
            out: rect4(out),
            step: g.step,
            radius: kernel.radius,
            wbase,
            cradius: kernel.coarse_radius,
            cwbase: wbase + kernel.weights.len() as i32,
            ..Default::default()
        }
    }

    pub(crate) fn uniform(&self, device: &wgpu::Device) -> wgpu::Buffer {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pm gaussian params"),
            contents: bytes_of(self),
            usage: wgpu::BufferUsages::UNIFORM,
        })
    }
}

/// A storage buffer of `bytes` (at least 4) the GPU fills.
pub(crate) fn scratch(device: &wgpu::Device, label: &str, bytes: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: bytes.max(16),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

pub(crate) fn storage(device: &wgpu::Device, label: &str, contents: &[u8]) -> wgpu::Buffer {
    let mut padded;
    let contents = if contents.len() < 16 {
        padded = contents.to_vec();
        padded.resize(16, 0);
        &padded[..]
    } else {
        contents
    };
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents,
        usage: wgpu::BufferUsages::STORAGE,
    })
}

pub(crate) fn f32_bytes(v: &[f32]) -> &[u8] {
    // SAFETY: f32 has no padding and every byte pattern is a valid u8.
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), size_of_val(v)) }
}

pub(crate) fn compute(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    entry: &'static str,
) -> LazyComputePipeline {
    let (device, module) = (device.clone(), module.clone());
    LazyComputePipeline::new(move || {
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(entry),
            layout: None,
            module: &module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            cache: None,
        })
    })
}

/// A full-target draw of `entry` into `format`, no blending.
pub(crate) fn render(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    entry: &'static str,
    format: wgpu::TextureFormat,
) -> LazyRenderPipeline {
    let (device, module) = (device.clone(), module.clone());
    LazyRenderPipeline::new(move || {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(entry),
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    })
}

/// A bind group for group 0 of `layout_of`'s auto layout.
pub(crate) fn bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    entries: &[(u32, wgpu::BindingResource<'_>)],
) -> wgpu::BindGroup {
    let entries: Vec<wgpu::BindGroupEntry<'_>> = entries
        .iter()
        .map(|(binding, resource)| wgpu::BindGroupEntry {
            binding: *binding,
            resource: resource.clone(),
        })
        .collect();
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("pm gaussian"),
        layout,
        entries: &entries,
    })
}

pub(crate) fn groups(n: i32, size: u32) -> u32 {
    (n.max(0) as u32).div_ceil(size)
}

/// The blur's passes, compiled from a module that includes
/// `gaussian.wgsl`.
pub(crate) struct Separable {
    h: LazyComputePipeline,
    v: LazyComputePipeline,
    ch: LazyComputePipeline,
    cv: LazyComputePipeline,
    ix: LazyComputePipeline,
}

/// Buffers of one blur.
pub(crate) struct Passes<'a> {
    pub uniform: &'a wgpu::Buffer,
    /// Stage 1's weights at `wbase`, stage 2's right after.
    pub weights: &'a wgpu::Buffer,
    /// Binding 2: the source.
    pub src: wgpu::BindingResource<'a>,
    /// Stage 1 rows, stage 1 nodes, stage 2 rows, final nodes,
    /// interpolated rows ([`sizes`]).
    pub scratch: [&'a wgpu::Buffer; 5],
}

impl Separable {
    pub(crate) fn new(device: &wgpu::Device, module: &wgpu::ShaderModule) -> Separable {
        Separable {
            h: compute(device, module, "h_pass"),
            v: compute(device, module, "v_pass"),
            ch: compute(device, module, "c_h"),
            cv: compute(device, module, "c_v"),
            ix: compute(device, module, "i_x"),
        }
    }

    /// The blur at the nodes of `p`, interpolated along rows when they are
    /// spaced: what `grid_at` reads.
    pub(crate) fn encode(
        &self,
        device: &wgpu::Device,
        pass: &mut wgpu::ComputePass<'_>,
        b: &Passes<'_>,
        p: &Params,
    ) {
        let [h, u, h2, v, ix] = b.scratch.map(|s| s.as_entire_binding());
        let uniform = || (0, b.uniform.as_entire_binding());
        let weights = || (1, b.weights.as_entire_binding());
        let nodes = [p.grid[2] - 2 * p.cradius, p.grid[3] - 2 * p.cradius];
        let mut run = |pipeline: &LazyComputePipeline,
                       entries: &[(u32, wgpu::BindingResource<'_>)],
                       size: [i32; 2]| {
            let pipeline = pipeline.get();
            let group = bind(device, &pipeline.get_bind_group_layout(0), entries);
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(groups(size[0], 64), groups(size[1], 4), 1);
        };
        run(
            &self.h,
            &[uniform(), weights(), (2, b.src.clone()), (3, h.clone())],
            [p.grid[2], p.src[3]],
        );
        run(
            &self.v,
            &[uniform(), weights(), (3, h), (8, u.clone())],
            [p.grid[2], p.grid[3]],
        );
        if p.step == 1 {
            return;
        }
        run(
            &self.ch,
            &[uniform(), weights(), (8, u), (9, h2.clone())],
            [nodes[0], p.grid[3]],
        );
        run(
            &self.cv,
            &[uniform(), weights(), (9, h2), (4, v.clone())],
            nodes,
        );
        run(
            &self.ix,
            &[uniform(), (4, v), (10, ix)],
            [p.out[2], nodes[1]],
        );
    }

    #[cfg(test)]
    pub(crate) fn pipelines(&self) -> [(&'static str, &LazyComputePipeline); 5] {
        [
            ("h", &self.h),
            ("v", &self.v),
            ("coarse h", &self.ch),
            ("coarse v", &self.cv),
            ("interpolate", &self.ix),
        ]
    }
}

/// Bytes of a blur's scratch buffers ([`Passes::scratch`]), `elem` bytes a
/// value.
pub(crate) fn sizes(p: &Params, elem: u64) -> [u64; 5] {
    let n = |v: i32| v.max(0) as u64;
    let (gw, gh) = (n(p.grid[2]), n(p.grid[3]));
    let (nw, nh) = (n(p.grid[2] - 2 * p.cradius), n(p.grid[3] - 2 * p.cradius));
    if p.step == 1 {
        return [gw * n(p.src[3]) * elem, gw * gh * elem, 0, 0, 0];
    }
    [
        gw * n(p.src[3]) * elem,
        gw * gh * elem,
        nw * gh * elem,
        nw * nh * elem,
        n(p.out[2]) * nh * elem,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_reference::gaussian::{Edge, blur};

    /// Sources that reach the bound: a step, a band two σ wide, an
    /// impulse, and a checkerboard (the largest high frequencies).
    fn sources(w: usize, h: usize, sigma: f64) -> Vec<(&'static str, Vec<f64>)> {
        let (cx, cy) = (w / 2, h / 2);
        let band = (2.0 * sigma).round().max(1.0) as usize;
        vec![
            (
                "step",
                (0..w * h)
                    .map(|i| f64::from(u8::from(i % w < cx)))
                    .collect(),
            ),
            (
                "band",
                (0..w * h)
                    .map(|i| {
                        f64::from(u8::from(
                            (i % w).abs_diff(cx) * 2 < band && (i / w).abs_diff(cy) * 2 < band,
                        ))
                    })
                    .collect(),
            ),
            (
                "impulse",
                (0..w * h)
                    .map(|i| f64::from(u8::from(i == cy * w + cx)))
                    .collect(),
            ),
            (
                "checker",
                (0..w * h)
                    .map(|i| {
                        f64::from(u8::from(
                            (i % w + i / w).is_multiple_of(2) && (i % w).abs_diff(cx) < 40,
                        ))
                    })
                    .collect(),
            ),
        ]
    }

    #[test]
    fn the_plan_is_within_its_bound_of_the_exact_gaussian() {
        // The plan's bound (module docs) and f32 weights.
        let bound = 2f64.powi(-12) + 1e-6;
        // Both sides of the first spacings (5.56: s = 2, 8.34: s = 3).
        for sigma in [
            0.02, 0.3, 1.0, 2.5, 5.5, 5.56, 8.3, 8.34, 10.0, 17.3, 33.0, 60.0,
        ] {
            let k = Kernel::new(sigma).unwrap();
            let n = (6.0 * sigma).ceil() as usize + 24;
            let (w, h) = (n.max(64), n.max(64));
            for (name, src) in sources(w, h, sigma) {
                let exact = blur(&src, w, h, 1, (sigma, sigma), Edge::Transparent);
                let rect = PixelRect::new(0, 0, w as u32, h as u32);
                let f: Vec<f32> = src.iter().map(|v| *v as f32).collect();
                let ours = k.blur_cpu(&f, rect, 1, rect);
                let max = ours
                    .iter()
                    .zip(&exact)
                    .map(|(a, b)| (f64::from(*a) - b).abs())
                    .fold(0.0, f64::max);
                assert!(max <= bound, "σ {sigma} {name}: {max:e} (step {})", k.step);
            }
        }
    }

    #[test]
    fn kernels() {
        assert!(Kernel::new(0.0).is_none());
        assert!(Kernel::new(f64::NAN).is_none());
        assert!(Kernel::new(f64::INFINITY).is_none());
        let k = Kernel::new(10.0).unwrap();
        assert_eq!((k.step, k.radius, k.coarse_radius), (3, 14, 15));
        for w in [&k.weights, &k.coarse] {
            let sum: f64 = w.iter().map(|w| f64::from(*w)).sum();
            assert!((sum - 1.0).abs() < 1e-6);
        }
        assert_eq!(Kernel::new(5.5).unwrap().step, 1);
        assert_eq!(Kernel::new(5.56).unwrap().step, 2);
        // σ is the authored one: no cap (Skia clamps at 532).
        let huge = Kernel::new(2000.0).unwrap();
        assert_eq!(
            (huge.step, huge.radius, huge.coarse_radius),
            (720, 3240, 12)
        );
    }
}
