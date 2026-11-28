//! The Powermove mark as a lit object over the dimmed desktop, drawn by one
//! Metal pass (`light.metal`) into GPUI textures: a warm gradient body with a
//! bright rim under an exposure that starts white-hot, a bloom that breathes
//! and the soft light it spills under it, all premultiplied over a clear
//! ground (the stage's dim is a plain veil under it). The mark is a signed
//! distance field made once from its outline, on the worker that compiles the
//! shader; nothing is made or read back per frame.
use gpui::{NativeTexture, TextureLease, Window};
use metal::*;
use std::sync::{Arc, mpsc};

/// What one frame of the mark shows (`light.metal`'s `Light`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Light {
    /// Output size, device pixels.
    pub size: [f32; 2],
    /// The mark's origin, device pixels.
    pub centre: [f32; 2],
    /// Device pixels per mark unit (the outline is 112 units high).
    pub scale: f32,
    pub exposure: f32,
    pub bloom: f32,
    pub pool: f32,
    pub mark: f32,
    pub time: f32,
    pub density: f32,
    /// The mark's resting height, device pixels: the size of its light.
    pub reach: f32,
    /// The outline's softness, device pixels (the light leading the shape).
    pub soft: f32,
    /// The body's slow colour drift (0–1).
    pub iris: f32,
    /// Clockwise rotation of the mark in radians (`0` = upright playhead;
    /// `~2.58` = agent cursor pointing up-left).
    pub angle: f32,
    pub pad: f32,
    pub domain: [f32; 4],
}

/// The field's rectangle in mark units (the outline spans about
/// −69…68 × −55…57) and its resolution.
const DOMAIN: [f32; 4] = [-96., -80., 96., 82.];
const FIELD: (usize, usize) = (1024, 864);

#[derive(Debug)]
struct Lease(#[allow(dead_code)] Arc<OutputHandle>);
#[derive(Debug)]
struct OutputHandle;
impl TextureLease for Lease {
    fn gpu_completed(&self, _: bool) {}
}

pub struct LightRenderer {
    queue: CommandQueue,
    event: Event,
    value: u64,
    pipeline: RenderPipelineState,
    field: Texture,
    /// Outputs by size, each free again once
    /// GPUI lets go of it.
    outputs: Vec<((u32, u32), Texture, Arc<OutputHandle>)>,
}
impl LightRenderer {
    /// Compiles the shader and makes the mark's field (a worker's job).
    pub fn new(device: &DeviceRef) -> Result<Self, String> {
        let library =
            device.new_library_with_source(include_str!("light.metal"), &CompileOptions::new())?;
        let d = RenderPipelineDescriptor::new();
        let vertex = library.get_function("light_vertex", None)?;
        let fragment = library.get_function("light_fragment", None)?;
        d.set_vertex_function(Some(&vertex));
        d.set_fragment_function(Some(&fragment));
        d.color_attachments()
            .object_at(0)
            .unwrap()
            .set_pixel_format(MTLPixelFormat::BGRA8Unorm);
        let pipeline = device.new_render_pipeline_state(&d)?;
        let values = mark_field(FIELD.0, FIELD.1);
        let t = TextureDescriptor::new();
        t.set_width(FIELD.0 as u64);
        t.set_height(FIELD.1 as u64);
        t.set_pixel_format(MTLPixelFormat::R32Float);
        t.set_usage(MTLTextureUsage::ShaderRead);
        t.set_storage_mode(if device.has_unified_memory() {
            MTLStorageMode::Shared
        } else {
            MTLStorageMode::Managed
        });
        let field = device.new_texture(&t);
        field.replace_region(
            MTLRegion::new_2d(0, 0, FIELD.0 as u64, FIELD.1 as u64),
            0,
            values.as_ptr().cast(),
            (FIELD.0 * 4) as u64,
        );
        Ok(Self {
            queue: device.new_command_queue(),
            event: device.new_event(),
            value: 0,
            pipeline,
            field,
            outputs: Vec::new(),
        })
    }
    /// The mark for `light` at its size; the texture is free again once
    /// GPUI lets go of it. `None` while every output is still in use.
    pub fn render(&mut self, mut light: Light) -> Option<Arc<NativeTexture>> {
        let size = (light.size[0].round() as u32, light.size[1].round() as u32);
        if size.0 == 0 || size.1 == 0 {
            return None;
        }
        let free = self
            .outputs
            .iter()
            .position(|(at, _, handle)| *at == size && Arc::strong_count(handle) == 1);
        let index = match free {
            Some(i) => i,
            None if self.outputs.iter().filter(|(at, ..)| *at == size).count() < 3 => {
                // Outputs of a size no longer drawn go first.
                if self.outputs.len() >= 6
                    && let Some(i) = self
                        .outputs
                        .iter()
                        .position(|(at, _, handle)| *at != size && Arc::strong_count(handle) == 1)
                {
                    self.outputs.remove(i);
                }
                let device = self.queue.device();
                let d = TextureDescriptor::new();
                d.set_width(u64::from(size.0));
                d.set_height(u64::from(size.1));
                d.set_pixel_format(MTLPixelFormat::BGRA8Unorm);
                d.set_usage(MTLTextureUsage::ShaderRead | MTLTextureUsage::RenderTarget);
                d.set_storage_mode(MTLStorageMode::Private);
                self.outputs
                    .push((size, device.new_texture(&d), Arc::new(OutputHandle)));
                self.outputs.len() - 1
            }
            None => return None,
        };
        light.domain = DOMAIN;
        let (_, output, handle) = &self.outputs[index];
        let cmd = self.queue.new_command_buffer();
        let pass = RenderPassDescriptor::new();
        let c = pass.color_attachments().object_at(0).unwrap();
        c.set_texture(Some(output));
        c.set_load_action(MTLLoadAction::DontCare);
        c.set_store_action(MTLStoreAction::Store);
        let encoder = cmd.new_render_command_encoder(pass);
        encoder.set_render_pipeline_state(&self.pipeline);
        encoder.set_fragment_bytes(
            0,
            std::mem::size_of::<Light>() as u64,
            (&raw const light).cast(),
        );
        encoder.set_fragment_texture(0, Some(&self.field));
        encoder.draw_primitives(MTLPrimitiveType::TriangleStrip, 0, 4);
        encoder.end_encoding();
        self.value += 1;
        cmd.encode_signal_event(&self.event, self.value);
        cmd.commit();
        NativeTexture::new(
            output.clone(),
            self.event.clone(),
            self.value,
            Arc::new(Lease(handle.clone())),
        )
        .ok()
    }
}

/// The renderer, made on a worker from the stage's first frame.
#[derive(Default)]
pub struct LightState {
    renderer: Option<LightRenderer>,
    warming: Option<mpsc::Receiver<Result<LightRenderer, String>>>,
    failed: bool,
    /// The last frames: a held picture is drawn once.
    last: Vec<(Light, Arc<NativeTexture>)>,
}
impl LightState {
    pub fn warm(&mut self, window: &Window) {
        if self.renderer.is_some() || self.warming.is_some() || self.failed {
            return;
        }
        let Some(device) = crate::wave::device(window) else {
            self.failed = true;
            return;
        };
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("stage-light".into())
            .spawn(move || {
                let _ = tx.send(LightRenderer::new(&device));
            });
        match spawned {
            Ok(_) => self.warming = Some(rx),
            Err(_) => self.failed = true,
        }
    }
    /// The mark at `light`; `wait` blocks for a renderer still being made
    /// (captures); a live frame paints nothing until it is ready.
    pub fn picture(&mut self, light: Light, wait: bool) -> Option<Arc<NativeTexture>> {
        if self.renderer.is_none() {
            let rx = self.warming.as_ref()?;
            let made = if wait {
                rx.recv().ok()
            } else {
                rx.try_recv().ok()
            };
            match made {
                Some(Ok(renderer)) => self.renderer = Some(renderer),
                Some(Err(error)) => {
                    eprintln!("The stage draws no light: {error}");
                    self.failed = true;
                }
                None => return None,
            }
            self.warming = None;
        }
        if let Some((_, texture)) = self.last.iter().find(|(at, _)| *at == light) {
            return Some(texture.clone());
        }
        let texture = self.renderer.as_mut()?.render(light)?;
        if self.last.len() >= 2 {
            self.last.remove(0);
        }
        self.last.push((light, texture.clone()));
        Some(texture)
    }
}

/// The mark's signed distance (mark units, negative inside) on a grid over
/// `DOMAIN`: exact near the outline, a distance transform further out.
pub(crate) fn mark_field(w: usize, h: usize) -> Vec<f32> {
    let [x0, y0, x1, y1] = DOMAIN.map(f64::from);
    let unit = (x1 - x0) / w as f64;
    let to_grid = |x: f64, y: f64| {
        (
            (x - x0) / unit - 0.5,
            (y - y0) / ((y1 - y0) / h as f64) - 0.5,
        )
    };
    // The sculpted Powermove playhead head from `assets/brand/wordmark.svg`,
    // centred on (495, 334) and normalised to 112 mark units high so the Metal
    // glow, rim light and zoom-out hug the exact top of the playhead.
    const SCALE: f64 = 112.0 / 368.0;
    let pt = |x: f64, y: f64| to_grid((x - 495.0) * SCALE, (y - 334.0) * SCALE);
    let mut poly = Vec::new();
    let line_to = |poly: &mut Vec<(f64, f64)>, a: (f64, f64), b: (f64, f64)| {
        const SEG: usize = 24;
        for k in 0..SEG {
            let t = k as f64 / SEG as f64;
            poly.push(pt(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
        }
    };
    let cubic_to = |poly: &mut Vec<(f64, f64)>,
                        p0: (f64, f64),
                        p1: (f64, f64),
                        p2: (f64, f64),
                        p3: (f64, f64)| {
        const SEG: usize = 48;
        for k in 0..SEG {
            let t = k as f64 / SEG as f64;
            let n = 1. - t;
            let x = n * n * n * p0.0
                + 3. * n * n * t * p1.0
                + 3. * n * t * t * p2.0
                + t * t * t * p3.0;
            let y = n * n * n * p0.1
                + 3. * n * n * t * p1.1
                + 3. * n * t * t * p2.1
                + t * t * t * p3.1;
            poly.push(pt(x, y));
        }
    };
    line_to(&mut poly, (346., 150.), (654., 150.));
    cubic_to(&mut poly, (654., 150.), (690., 150.), (708., 176.), (708., 206.));
    cubic_to(&mut poly, (708., 206.), (708., 226.), (702., 238.), (692., 251.));
    line_to(&mut poly, (692., 251.), (628., 327.));
    cubic_to(&mut poly, (628., 327.), (612., 346.), (606., 364.), (603., 386.));
    line_to(&mut poly, (603., 386.), (592., 465.));
    cubic_to(&mut poly, (592., 465.), (587., 498.), (560., 518.), (527., 518.));
    cubic_to(&mut poly, (527., 518.), (501., 518.), (485., 504.), (472., 486.));
    line_to(&mut poly, (472., 486.), (297., 248.));
    cubic_to(&mut poly, (297., 248.), (286., 233.), (282., 220.), (282., 206.));
    cubic_to(&mut poly, (282., 206.), (282., 174.), (306., 150.), (346., 150.));
    let n = poly.len();
    // Inside: non-zero winding along each row's centre.
    let mut inside = vec![false; w * h];
    for row in 0..h {
        let y = row as f64;
        let mut crossings: Vec<(f64, i32)> = Vec::new();
        for i in 0..n {
            let (a, b) = (poly[i], poly[(i + 1) % n]);
            if (a.1 <= y) != (b.1 <= y) {
                let x = a.0 + (y - a.1) / (b.1 - a.1) * (b.0 - a.0);
                crossings.push((x, if b.1 > a.1 { 1 } else { -1 }));
            }
        }
        crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut wind = 0;
        for pair in crossings.windows(2) {
            wind += pair[0].1;
            if wind != 0 {
                let from = pair[0].0.ceil().max(0.) as isize;
                let to = (pair[1].0.floor() as isize).min(w as isize - 1);
                for col in from..=to {
                    inside[row * w + col as usize] = true;
                }
            }
        }
    }
    // Far: Euclidean distance transforms of each side (texels).
    let outside_d = edt(&inside, w, h, true);
    let inside_d = edt(&inside, w, h, false);
    let mut field: Vec<f32> = (0..w * h)
        .map(|i| {
            let d = if inside[i] {
                -(inside_d[i].sqrt() - 0.5)
            } else {
                outside_d[i].sqrt() - 0.5
            };
            (d * unit) as f32
        })
        .collect();
    // Near: the exact distance to the outline within a few texels.
    const BAND: f64 = 3.;
    let mut near = vec![f64::INFINITY; w * h];
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        let (lx, hx) = (a.0.min(b.0) - BAND, a.0.max(b.0) + BAND);
        let (ly, hy) = (a.1.min(b.1) - BAND, a.1.max(b.1) + BAND);
        for row in ly.ceil().max(0.) as usize..=(hy.floor().max(0.) as usize).min(h - 1) {
            for col in lx.ceil().max(0.) as usize..=(hx.floor().max(0.) as usize).min(w - 1) {
                let (px, py) = (col as f64, row as f64);
                let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                let len = dx * dx + dy * dy;
                let t = if len > 0. {
                    (((px - a.0) * dx + (py - a.1) * dy) / len).clamp(0., 1.)
                } else {
                    0.
                };
                let (ex, ey) = (a.0 + t * dx - px, a.1 + t * dy - py);
                let d = (ex * ex + ey * ey).sqrt();
                let at = row * w + col;
                if d < near[at] {
                    near[at] = d;
                }
            }
        }
    }
    for (i, d) in near.iter().enumerate() {
        if *d <= BAND {
            field[i] = ((if inside[i] { -d } else { *d }) * unit) as f32;
        }
    }
    field
}

/// Squared distance (texels) from each texel to the nearest texel where
/// `mask == target` (Felzenszwalb and Huttenlocher).
fn edt(mask: &[bool], w: usize, h: usize, target: bool) -> Vec<f64> {
    const INF: f64 = 1e20;
    let mut f: Vec<f64> = mask
        .iter()
        .map(|&m| if m == target { 0. } else { INF })
        .collect();
    let mut line = vec![0.; w.max(h)];
    let mut out = vec![0.; w.max(h)];
    let mut v = vec![0usize; w.max(h)];
    let mut z = vec![0.; w.max(h) + 1];
    let mut pass =
        |f: &mut Vec<f64>, count: usize, len: usize, at: &dyn Fn(usize, usize) -> usize| {
            for k in 0..count {
                for i in 0..len {
                    line[i] = f[at(k, i)];
                }
                dt1(&line[..len], &mut out[..len], &mut v, &mut z);
                for i in 0..len {
                    f[at(k, i)] = out[i];
                }
            }
        };
    pass(&mut f, h, w, &|row, col| row * w + col);
    pass(&mut f, w, h, &|col, row| row * w + col);
    f
}
fn dt1(f: &[f64], d: &mut [f64], v: &mut [usize], z: &mut [f64]) {
    let n = f.len();
    let mut k = 0;
    v[0] = 0;
    z[0] = f64::NEG_INFINITY;
    z[1] = f64::INFINITY;
    for q in 1..n {
        // `z[0]` is −∞, so `k` never passes below zero.
        let mut s;
        loop {
            let p = v[k];
            s = ((f[q] + (q * q) as f64) - (f[p] + (p * p) as f64))
                / (2. * q as f64 - 2. * p as f64);
            if s <= z[k] {
                k -= 1;
            } else {
                break;
            }
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = f64::INFINITY;
    }
    k = 0;
    for (q, out) in d.iter_mut().enumerate().take(n) {
        while z[k + 1] < q as f64 {
            k += 1;
        }
        let p = v[k];
        *out = (q as f64 - p as f64).powi(2) + f[p];
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_uniforms_match_the_shaders_layout() {
        // `light.metal`'s `Light`: 16 floats (two of them padding), then a
        // float4 at 64.
        assert_eq!(std::mem::size_of::<super::Light>(), 80);
        assert_eq!(std::mem::offset_of!(super::Light, domain), 64);
    }
    #[test]
    fn the_field_is_negative_inside_the_mark_and_grows_outside() {
        let (w, h) = (256, 216);
        let field = super::mark_field(w, h);
        let at = |x: f32, y: f32| {
            let [x0, y0, x1, y1] = super::DOMAIN;
            let col = ((x - x0) / (x1 - x0) * w as f32) as usize;
            let row = ((y - y0) / (y1 - y0) * h as f32) as usize;
            field[row * w + col]
        };
        // The stem's middle, inside; the corners of the field, far outside.
        assert!(at(-3., -20.) < 0., "{}", at(-3., -20.));
        assert!(at(-90., -75.) > 30.);
        assert!(at(0., 75.) > 5., "below the mark's notch");
        // A distance field: neighbours differ by at most a texel's width.
        let unit = 192. / w as f32;
        for row in 1..h {
            for col in 1..w {
                let d = (field[row * w + col] - field[row * w + col - 1]).abs();
                assert!(d <= unit * 1.6, "{row},{col}: {d}");
            }
        }
    }
}
#[cfg(test)]
mod gpu_cost {
    #[test]
    #[ignore = "measures the GPU; run by hand"]
    fn the_light_pass_costs_little_gpu_time() {
        let device = metal::Device::system_default().unwrap();
        let mut r = super::LightRenderer::new(&device).unwrap();
        let s = super::super::cue::CueSheet::default();
        let q = device.new_command_queue();
        let mut times = Vec::new();
        for i in 0..60 {
            let l =
                super::super::intro::light_at(2.0 + i as f64 / 60., &s, true, (3024., 1964.), 2.);
            let tex = r.render(l).unwrap();
            let started = std::time::Instant::now();
            let cmd = q.new_command_buffer();
            cmd.encode_wait_for_event(&tex.event, tex.value);
            cmd.commit();
            cmd.wait_until_completed();
            times.push(started.elapsed().as_secs_f64() * 1e3);
        }
        times.sort_by(f64::total_cmp);
        eprintln!(
            "light pass 3024x1964: p50 {:.2} ms p95 {:.2} ms max {:.2} ms",
            times[30], times[57], times[59]
        );
    }
}
