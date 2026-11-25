//! Offscreen Metal renderer. No window, layer, display capture, or UI thread required.
use crate::{
    Front, Point, Rect, ValidationError, WaveStyle,
    style::oklab,
    timeline::{Particle, rim_lumps},
};
use image::RgbaImage;
use metal::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Debug)]
struct TextureAllocation {
    texture: Texture,
    count: Arc<AtomicUsize>,
}
impl Drop for TextureAllocation {
    fn drop(&mut self) {
        self.count.fetch_sub(1, Ordering::Relaxed);
    }
}
/// A rendered picture. Retain it until every GPU consumer has finished sampling.
#[derive(Debug)]
pub struct WaveFrame {
    allocation: Arc<TextureAllocation>,
    pub event: Event,
    pub value: u64,
}
impl WaveFrame {
    pub fn texture(&self) -> &TextureRef {
        &self.allocation.texture
    }
    /// Test/export readback; never used by the live element's frame loop.
    pub fn read_image(&self) -> Result<RgbaImage, String> {
        // A separate command queue waits on the producer, so no display or AppKit is involved.
        let queue = self.texture().device().new_command_queue();
        let cmd = queue.new_command_buffer();
        cmd.encode_wait_for_event(&self.event, self.value);
        if !self.texture().device().has_unified_memory() {
            let blit = cmd.new_blit_command_encoder();
            blit.synchronize_resource(self.texture());
            blit.end_encoding();
        }
        cmd.commit();
        cmd.wait_until_completed();
        let w = self.texture().width() as u32;
        let h = self.texture().height() as u32;
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        self.texture().get_bytes(
            pixels.as_mut_ptr().cast(),
            u64::from(w) * 4,
            MTLRegion::new_2d(0, 0, w.into(), h.into()),
            0,
        );
        for p in pixels.chunks_exact_mut(4) {
            p.swap(0, 2);
        }
        RgbaImage::from_raw(w, h, pixels).ok_or_else(|| "invalid frame size".into())
    }
}
struct Snapshots {
    screens: [Arc<TextureAllocation>; 6],
    particles: Vec<Particle>,
    width: u32,
    height: u32,
    style: WaveStyle,
    same: bool,
}
/// Prepared immutable screenshots, cached blur levels and a small output pool.
/// All textures are display-encoded sRGB in BGRA8Unorm, GPUI's `NativeTexture` contract.
/// GPUI's sRGB layer lets ColorSync convert to the display color space; do not label
/// these bytes as Display P3 or use an sRGB Metal format (which would decode twice).
/// A picture on the GPU, ready once `event` reaches `value` (a GPUI
/// snapshot taken without reading it back).
pub struct GpuPicture<'a> {
    pub texture: &'a TextureRef,
    pub event: &'a EventRef,
    pub value: u64,
}
pub struct WaveRenderer {
    device: Device,
    queue: CommandQueue,
    event: Event,
    value: u64,
    main: RenderPipelineState,
    copy: RenderPipelineState,
    blur: RenderPipelineState,
    dots: RenderPipelineState,
    snapshots: Option<Snapshots>,
    outputs: Vec<Arc<TextureAllocation>>,
    count: Arc<AtomicUsize>,
}
impl WaveRenderer {
    pub fn new(device: &DeviceRef) -> Result<Self, String> {
        let options = CompileOptions::new();
        options.set_fast_math_enabled(false);
        let library = device.new_library_with_source(include_str!("wave.metal"), &options)?;
        let pipeline =
            |vertex: &str, fragment: &str, blend: bool| -> Result<RenderPipelineState, String> {
                let d = RenderPipelineDescriptor::new();
                let vertex = library.get_function(vertex, None)?;
                let fragment = library.get_function(fragment, None)?;
                d.set_vertex_function(Some(&vertex));
                d.set_fragment_function(Some(&fragment));
                let c = d.color_attachments().object_at(0).unwrap();
                c.set_pixel_format(MTLPixelFormat::BGRA8Unorm);
                if blend {
                    c.set_blending_enabled(true);
                    c.set_source_rgb_blend_factor(MTLBlendFactor::One);
                    c.set_source_alpha_blend_factor(MTLBlendFactor::One);
                    c.set_destination_rgb_blend_factor(MTLBlendFactor::OneMinusSourceAlpha);
                    c.set_destination_alpha_blend_factor(MTLBlendFactor::OneMinusSourceAlpha);
                }
                device.new_render_pipeline_state(&d)
            };
        Ok(Self {
            device: device.to_owned(),
            queue: device.new_command_queue(),
            event: device.new_event(),
            value: 0,
            main: pipeline("wave_vertex", "wave_fragment", false)?,
            copy: pipeline("wave_vertex", "wave_copy", false)?,
            blur: pipeline("wave_vertex", "wave_blur", false)?,
            dots: pipeline("wave_dot_vertex", "wave_dot_fragment", true)?,
            snapshots: None,
            outputs: Vec::with_capacity(3),
            count: Arc::default(),
        })
    }
    pub fn texture_count(&self) -> usize {
        self.count.load(Ordering::Relaxed)
    }
    /// Releases screenshots and reusable outputs. In-flight frames keep their leases.
    pub fn release(&mut self) {
        self.snapshots = None;
        self.outputs.clear();
    }
    pub fn is_prepared(&self) -> bool {
        self.snapshots.is_some()
    }
    fn texture(&self, w: u32, h: u32, mips: bool) -> Arc<TextureAllocation> {
        let d = TextureDescriptor::new();
        d.set_width(w.into());
        d.set_height(h.into());
        d.set_pixel_format(MTLPixelFormat::BGRA8Unorm);
        d.set_usage(MTLTextureUsage::ShaderRead | MTLTextureUsage::RenderTarget);
        d.set_mipmap_level_count(if mips {
            u64::from(w.max(h).ilog2() + 1)
        } else {
            1
        });
        d.set_storage_mode(if self.device.has_unified_memory() {
            MTLStorageMode::Shared
        } else {
            MTLStorageMode::Managed
        });
        self.count.fetch_add(1, Ordering::Relaxed);
        Arc::new(TextureAllocation {
            texture: self.device.new_texture(&d),
            count: self.count.clone(),
        })
    }
    fn encoder<'a>(cmd: &'a CommandBufferRef, target: &TextureRef) -> &'a RenderCommandEncoderRef {
        let pass = RenderPassDescriptor::new();
        let c = pass.color_attachments().object_at(0).unwrap();
        c.set_texture(Some(target));
        c.set_load_action(MTLLoadAction::DontCare);
        c.set_store_action(MTLStoreAction::Store);
        cmd.new_render_command_encoder(pass)
    }
    fn filter(
        &self,
        cmd: &CommandBufferRef,
        input: &TextureRef,
        target: &TextureRef,
        pipeline: &RenderPipelineStateRef,
        params: [f32; 4],
    ) {
        let e = Self::encoder(cmd, target);
        e.set_render_pipeline_state(pipeline);
        e.set_fragment_texture(0, Some(input));
        e.set_fragment_bytes(0, 16, params.as_ptr().cast());
        e.draw_primitives(MTLPrimitiveType::TriangleStrip, 0, 4);
        e.end_encoding();
    }
    fn blurred(
        &self,
        cmd: &CommandBufferRef,
        sharp: &TextureRef,
        width_pt: f64,
        sigma: f64,
    ) -> Arc<TextureAllocation> {
        let w = sharp.width() as u32;
        let h = sharp.height() as u32;
        let sigma_px = sigma * f64::from(w) / width_pt;
        let factor = (sigma_px / 5.).round().max(1.);
        let dw = (f64::from(w) / factor).round().max(1.) as u32;
        let dh = (f64::from(h) / factor).round().max(1.) as u32;
        let small = self.texture(dw, dh, false);
        let tmp = self.texture(dw, dh, false);
        let size = [dw as f32, dh as f32];
        self.filter(
            cmd,
            sharp,
            &small.texture,
            &self.copy,
            [size[0], size[1], factor.log2() as f32, 0.],
        );
        self.filter(
            cmd,
            &small.texture,
            &tmp.texture,
            &self.blur,
            [size[0], size[1], (sigma_px / factor).max(0.01) as f32, 0.],
        );
        self.filter(
            cmd,
            &tmp.texture,
            &small.texture,
            &self.blur,
            [size[0], size[1], (sigma_px / factor).max(0.01) as f32, 1.],
        );
        small
    }
    /// Upload pictures once. The live frame loop never captures or reads pixels back.
    pub fn prepare(
        &mut self,
        from: &RgbaImage,
        to: Option<&RgbaImage>,
        width_pt: f64,
        style: &WaveStyle,
    ) -> Result<(), String> {
        style.validate().map_err(|e| e.to_string())?;
        if from.width() == 0 || from.height() == 0 || !width_pt.is_finite() || width_pt <= 0. {
            return Err("invalid screenshot size".into());
        }
        if let Some(to) = to
            && to.dimensions() != from.dimensions()
        {
            return Err("screenshots must have equal dimensions".into());
        }
        self.release();
        let cmd = self.queue.new_command_buffer();
        let screen = |image: &RgbaImage| {
            let sharp = self.texture(image.width(), image.height(), true);
            let mut pixels = image.as_raw().clone();
            for p in pixels.chunks_exact_mut(4) {
                p.swap(0, 2);
            }
            sharp.texture.replace_region(
                MTLRegion::new_2d(0, 0, image.width().into(), image.height().into()),
                0,
                pixels.as_ptr().cast(),
                u64::from(image.width()) * 4,
            );
            let b = cmd.new_blit_command_encoder();
            b.generate_mipmaps(&sharp.texture);
            b.end_encoding();
            let near = self.blurred(cmd, &sharp.texture, width_pt, 7.);
            let far = self.blurred(cmd, &sharp.texture, width_pt, style.blur.max(7.));
            [sharp, near, far]
        };
        let [a, b, c] = screen(from);
        let [d, e, f] = if let Some(to) = to {
            screen(to)
        } else {
            [a.clone(), b.clone(), c.clone()]
        };
        cmd.commit();
        cmd.wait_until_completed();
        if cmd.status() == MTLCommandBufferStatus::Error {
            return Err("screenshot preparation failed".into());
        }
        for _ in 0..3 {
            self.outputs
                .push(self.texture(from.width(), from.height(), false));
        }
        self.snapshots = Some(Snapshots {
            screens: [a, b, c, d, e, f],
            particles: (0..style.particle_count.round() as usize)
                .map(|i| Particle::new(i, style))
                .collect(),
            width: from.width(),
            height: from.height(),
            style: style.clone(),
            same: to.is_none(),
        });
        Ok(())
    }
    /// Pictures already on the GPU (`GpuPicture`), prepared without a CPU
    /// wait or upload: the command buffer waits for each picture's event,
    /// copies it into the mipmapped screen, blurs, and is committed. Frames
    /// rendered after it on this queue see the result.
    pub fn prepare_gpu(
        &mut self,
        from: GpuPicture<'_>,
        to: Option<GpuPicture<'_>>,
        width_pt: f64,
        style: &WaveStyle,
    ) -> Result<(), String> {
        style.validate().map_err(|e| e.to_string())?;
        let (w, h) = (from.texture.width() as u32, from.texture.height() as u32);
        if w == 0 || h == 0 || !width_pt.is_finite() || width_pt <= 0. {
            return Err("invalid picture size".into());
        }
        if let Some(to) = &to
            && (to.texture.width() as u32, to.texture.height() as u32) != (w, h)
        {
            return Err("pictures must have equal dimensions".into());
        }
        self.release();
        let cmd = self.queue.new_command_buffer();
        let screen = |picture: &GpuPicture<'_>| {
            cmd.encode_wait_for_event(picture.event, picture.value);
            let sharp = self.texture(w, h, true);
            let b = cmd.new_blit_command_encoder();
            b.copy_from_texture(
                picture.texture,
                0,
                0,
                MTLOrigin { x: 0, y: 0, z: 0 },
                MTLSize {
                    width: w.into(),
                    height: h.into(),
                    depth: 1,
                },
                &sharp.texture,
                0,
                0,
                MTLOrigin { x: 0, y: 0, z: 0 },
            );
            b.generate_mipmaps(&sharp.texture);
            b.end_encoding();
            let near = self.blurred(cmd, &sharp.texture, width_pt, 7.);
            let far = self.blurred(cmd, &sharp.texture, width_pt, style.blur.max(7.));
            [sharp, near, far]
        };
        let [a, b, c] = screen(&from);
        let [d, e, f] = match &to {
            Some(to) => screen(to),
            None => [a.clone(), b.clone(), c.clone()],
        };
        cmd.commit();
        for _ in 0..3 {
            self.outputs.push(self.texture(w, h, false));
        }
        self.snapshots = Some(Snapshots {
            screens: [a, b, c, d, e, f],
            particles: (0..style.particle_count.round() as usize)
                .map(|i| Particle::new(i, style))
                .collect(),
            width: w,
            height: h,
            style: style.clone(),
            same: to.is_none(),
        });
        Ok(())
    }
    /// One full-image pass, followed by the original's small point-sprite pass.
    /// Output leases prevent mutation while GPUI or another GPU consumer retains a frame.
    pub fn render(
        &mut self,
        progress: f64,
        source: Rect,
        width_pt: f64,
        height_pt: f64,
        focus: Option<Point>,
        reduced_motion: bool,
    ) -> Result<WaveFrame, String> {
        source.validate().map_err(|e| e.to_string())?;
        if !progress.is_finite()
            || !width_pt.is_finite()
            || !height_pt.is_finite()
            || width_pt <= 0.
            || height_pt <= 0.
            || focus.is_some_and(|p| !p.x.is_finite() || !p.y.is_finite())
        {
            return Err(ValidationError("invalid frame geometry or progress").to_string());
        }
        let snapshots = self
            .snapshots
            .as_ref()
            .ok_or("prepare screenshots before rendering")?;
        let s = &snapshots.style;
        let p = progress.clamp(0., 1.);
        let output = if let Some(t) = self.outputs.iter().find(|t| Arc::strong_count(t) == 1) {
            t.clone()
        } else {
            return Err("all wave output textures are still in use".into());
        };
        let cmd = self.queue.new_command_buffer();
        let mut u = [0f32; 256];
        u[0] = width_pt as f32;
        u[1] = height_pt as f32;
        u[2] = snapshots.width as f32;
        u[3] = snapshots.height as f32;
        let f = Front::at(p, source, width_pt, height_pt, s, focus);
        u[4..23].copy_from_slice(&f.uniforms());
        u[23] = if f.has_cleared { 0. } else { 1. };
        u[24] = if p >= 1. || f.launched { 1. } else { 0. };
        u[25] = if f.energy > 0. || f.gravity > 0. {
            1.
        } else {
            0.
        };
        u[26] = if p > 0. && p < 1. { 1. } else { 0. };
        u[27] = if snapshots.same { 1. } else { 0. };
        u[28] = 1.;
        u[29] = 1.;
        u[30] = s.amplitude as f32;
        u[31] = s.lens_width as f32;
        u[32] = s.aberration as f32;
        u[33] = if f.energy > 0. {
            s.blur.max(7.) as f32
        } else {
            0.
        };
        u[34] = s.blur_ahead as f32;
        u[35] = s.blur_behind as f32;
        u[36] = 7.;
        let palette = s.palette.colors();
        for (i, c) in palette.colors[..palette.count].iter().enumerate() {
            u[37 + i * 3..40 + i * 3].copy_from_slice(&oklab(*c));
        }
        u[64] = palette.count as f32;
        u[65..68].copy_from_slice(&oklab(palette.ahead));
        u[68..71].copy_from_slice(&oklab(palette.behind));
        u[71..82].copy_from_slice(
            &[
                s.lead,
                s.tail,
                s.strength,
                s.rim_width,
                s.rim_softness,
                s.corner_radius,
                s.rim_unevenness,
                s.rim_pulse,
                s.hairline_width,
                s.hairline_opacity,
                s.hairline_white,
            ]
            .map(|v| v as f32),
        );
        u[82..103].copy_from_slice(&rim_lumps(
            p * s.duration(),
            width_pt,
            height_pt,
            s.corner_radius,
            s.rim_pulse,
        ));
        // Dedicated shader branch gives a pure crossfade and exact endpoints.
        u[103] = if reduced_motion { p as f32 } else { -1. };
        let encoder = Self::encoder(cmd, &output.texture);
        encoder.set_render_pipeline_state(&self.main);
        encoder.set_fragment_bytes(0, std::mem::size_of_val(&u) as u64, u.as_ptr().cast());
        for (i, t) in snapshots.screens.iter().enumerate() {
            encoder.set_fragment_texture(i as u64, Some(&t.texture));
        }
        encoder.draw_primitives(MTLPrimitiveType::TriangleStrip, 0, 4);
        if !reduced_motion && p > 0. && p < 1. {
            let mut dots = [[0f32; 4]; 256];
            let mut n = 0;
            for part in &snapshots.particles {
                let at = part.at(&f, source, s);
                if at[2].fract() > 0.002 {
                    dots[n] = at;
                    n += 1;
                }
            }
            if n > 0 {
                let scale = snapshots.width as f64 / width_pt;
                let params = [
                    width_pt as f32,
                    height_pt as f32,
                    ((s.particle_size + 1.) * scale).ceil() as f32 + 2.,
                    scale as f32,
                ];
                encoder.set_render_pipeline_state(&self.dots);
                encoder.set_vertex_bytes(0, (n * 16) as u64, dots.as_ptr().cast());
                encoder.set_vertex_bytes(1, 16, params.as_ptr().cast());
                encoder.draw_primitives(MTLPrimitiveType::Point, 0, n as u64);
            }
        }
        encoder.end_encoding();
        self.value += 1;
        cmd.encode_signal_event(&self.event, self.value);
        cmd.commit();
        Ok(WaveFrame {
            allocation: output,
            event: self.event.clone(),
            value: self.value,
        })
    }
}
