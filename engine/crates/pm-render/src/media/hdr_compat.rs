//! Chromium 152 / pinned Skia default HDR-to-SDR Canvas color transform.
//!
//! The input is the *stored* RGBA8 video upload, not unquantized YUV RGB.
//! Skia crosses that source store before its PQ/HLG color-space transform and
//! default RWTMO filter. This module is separate from the native BT.2446A
//! source transform; callers must select it only for unoverridden level-1
//! Canvas video without adaptive tone-map or mastering metadata.

use crate::pipelines::bytes_of;
use pm_gpu::target::{Target, working_usage};
use pm_protocol::color::Primaries;
use wgpu::util::DeviceExt;

const HDR_WHITE: f32 = 203.0;
const DEFAULT_PEAK: f32 = 1000.0;
const CONTROL_POINTS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Signal {
    Pq,
    Hlg,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Params {
    source_to_srgb: [[f32; 4]; 3],
    srgb_to_2020: [[f32; 4]; 3],
    to_709: [[f32; 4]; 3],
    curve: [[f32; 4]; CONTROL_POINTS],
    hlg_y: [f32; 4],
    signal: [u32; 4],
}

impl Params {
    fn new(signal: Signal, primaries: Primaries) -> Option<Self> {
        if !matches!(primaries, Primaries::Bt2020 | Primaries::DisplayP3) {
            return None;
        }
        // Chromium first converts the HDR image to the destination sRGB
        // working surface. SkWorkingFormatColorFilter then decodes that sRGB
        // signal and converts it to linear Rec.2020 for AGTM. Keeping the two
        // transforms distinct matches Skia's binary32 stage order.
        // Coefficients are exact bits from pinned skcms D50 inversion/concat:
        // source gamut -> named sRGB, named sRGB -> dynamically generated
        // Rec.2020 gain-application gamut, and back to named sRGB.
        let col =
            |x: u32, y: u32, z: u32| [f32::from_bits(x), f32::from_bits(y), f32::from_bits(z), 0.0];
        let source_to_srgb = match (signal, primaries) {
            (Signal::Pq, Primaries::Bt2020) => [
                col(0x42a39480, 0xc0c44eb4, 0xbf64a6b4),
                col(0xc1e7a4b8, 0x425f3fed, 0xc09e8ef0),
                col(0xc065a1e4, 0xbed258cd, 0x425c77df),
            ],
            (Signal::Hlg, Primaries::Bt2020) => [
                col(0x4102dd33, 0xbf1d0bc3, 0xbdb6ebc2),
                col(0xc0395093, 0x40b2998a, 0xbefdb17f),
                col(0xbeb7b4b6, 0xbd28470a, 0x40b05fe5),
            ],
            (Signal::Pq, Primaries::DisplayP3) => [
                col(0x427157d9, 0xc00488d6, 0xbf7777ed),
                col(0xc1316e2e, 0x424d5967, 0xc077ea58),
                col(0x394fd1f9, 0x3a121352, 0x42586ff4),
            ],
            _ => return None,
        };
        let srgb_to_2020 = [
            col(0x3f20a183, 0x3d8d7de3, 0x3c862d2c),
            col(0x3ea8a380, 0x3f6b624f, 0x3db43782),
            col(0x3d315cd0, 0x3c39fa7c, 0x3f653dea),
        ];
        let to_709 = [
            col(0x3fd485eb, 0xbdff0a90, 0xbc948848),
            col(0xbf1679c9, 0x3f9105e6, 0xbdcdffc4),
            col(0xbd952af0, 0xbc08a556, 0x3f8f3757),
        ];
        let hlg_y = [f32::from_bits(0x3e86809c), 0.678, 0.0593, 0.0];
        Some(Self {
            source_to_srgb,
            srgb_to_2020,
            to_709,
            curve: rwtmo_sdr_curve(),
            hlg_y,
            signal: [u32::from(signal == Signal::Hlg), 0, 0, 0],
        })
    }
}

/// Skia `PopulateUsingRwtmo`: default no-metadata 1000-nit baseline and
/// target headroom 0. The gain is a Hermite interpolation of log2(Y/X) at
/// eight points sampled from the RWTMO cubic, not direct cubic evaluation.
fn rwtmo_sdr_curve() -> [[f32; 4]; CONTROL_POINTS] {
    let headroom = (DEFAULT_PEAK / HDR_WHITE).log2();
    let white = 1.0 - 0.5 * (headroom / (1000.0f32 / 203.0).log2()).min(1.0);
    let x_max = headroom.exp2();
    let y_max = 1.0;
    let mid_x = 0.35 + 0.65 * y_max / white;
    let mid_y = 0.35 * white + 0.65 * y_max;
    let xa = 1.0 - 2.0 * mid_x + x_max;
    let ya = white - 2.0 * mid_y + y_max;
    let xb = 2.0 * mid_x - 2.0;
    let yb = 2.0 * mid_y - 2.0 * white;
    std::array::from_fn(|index| {
        let t = index as f32 / (CONTROL_POINTS - 1) as f32;
        let x = 1.0 + t * (xb + t * xa);
        let y = white + t * (yb + t * ya);
        let tangent = (2.0 * ya * t + yb) / (2.0 * xa * t + xb);
        let gain = (y / x).log2();
        let slope = (x * tangent - y) / (std::f32::consts::LN_2 * x * y);
        // MakeGainCurveXYMImage stores the CPU points in RGBAF16. The shader
        // reads those rounded values through a nearest-filtered texture.
        let half = |value| pm_gpu::readback::half_to_f32(pm_gpu::readback::f32_to_half(value));
        [half(x), half(gain), half(slope), 0.0]
    })
}

pub(super) struct ToneMap {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
}

impl ToneMap {
    pub fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Chromium HDR Canvas tone map"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Chromium HDR Canvas tone map"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../shaders/hdr_video_upload.wgsl").into(),
            ),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Chromium HDR Canvas tone map"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Chromium HDR Canvas tone map"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        Self { layout, pipeline }
    }

    /// Returns `None` outside the measured default HDR/P3 color classes.
    pub fn run(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &Target,
        signal: Signal,
        primaries: Primaries,
        reuse: Option<Target>,
    ) -> Option<Target> {
        if source.format() != wgpu::TextureFormat::Rgba8Unorm {
            return None;
        }
        let params = Params::new(signal, primaries)?;
        let target = reuse.unwrap_or_else(|| {
            Target::new(
                device,
                "Chromium HDR Canvas tone map",
                source.width(),
                source.height(),
                wgpu::TextureFormat::Rgba8Unorm,
                working_usage(),
            )
        });
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Chromium HDR Canvas tone map parameters"),
            contents: bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Chromium HDR Canvas tone map"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&source.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&target.view),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Chromium HDR Canvas tone map"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(source.width().div_ceil(8), source.height().div_ceil(8), 1);
        drop(pass);
        Some(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skia_default_curve_anchors() {
        let cp = rwtmo_sdr_curve();
        assert_eq!(cp[0][0], 1.0);
        assert_eq!(cp[0][1], -1.0); // RWTMO white is one-half SDR white.
        assert!((cp[7][0] - 1000.0 / 203.0).abs() < 0.002);
        assert!((cp[7][1] + cp[7][0].log2()).abs() < 0.002);
    }
}
