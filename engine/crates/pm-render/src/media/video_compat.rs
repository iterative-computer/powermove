//! Chromium's source reading is distinct from Interpret Footage's defaults.
//! Apply only to unoverridden level 1 SDR video, never to level 2 or a
//! sequence's separately preserved persisted-rendition reading.
use pm_media::probe::VideoProbe;
use pm_protocol::color::{ColorDescription, MatrixCoefficients, Range};

pub(super) fn bt2020_upload(video: &VideoProbe) -> bool {
    let tags = &video.color.tagged;
    matches!(
        (video.codec.codec.as_str(), video.codec.bit_depth),
        ("h264", 8) | ("hevc", 10)
    ) && tags.primaries == Some(9)
        && tags.transfer == Some(1)
        && tags.matrix == Some(9)
        && (tags.full_range == Some(false) || video.codec.codec == "hevc")
}

pub(super) fn color(video: &VideoProbe, mut desc: ColorDescription) -> ColorDescription {
    let tags = &video.color.tagged;
    // An incomplete SD H.264 color description is invalid to Chromium's
    // hardware frame upload. Its Skia fallback is limited Rec.601, even
    // when a lone matrix tag says Rec.709. Keep fully specified color tags.
    if video.codec.codec == "h264"
        && video.height <= 576
        && desc.bit_depth == 8
        && desc.range == Range::Limited
        && tags.primaries.is_none()
        && tags.transfer.is_none()
    {
        desc.matrix = MatrixCoefficients::Bt601;
    }
    // Opaque VP9's default Chromium hardware reading is Rec.709 at SD as
    // well as HD. The native source interpretation intentionally defaults
    // untagged SD to Rec.601; that remains the level 2 / Interpret reading.
    if video.codec.codec == "vp9" && tags.matrix.is_none() {
        desc.matrix = MatrixCoefficients::Bt709;
    }
    desc
}

use crate::pipelines::bytes_of;
use pm_gpu::decode_in::ImportedFrame;
use pm_gpu::target::Target;
use pm_media::frame::Layout;
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy)]
struct Params {
    rows: [[f32; 4]; 3],
    primaries: [[f32; 4]; 3],
    flags: [u32; 4],
}

/// Chromium/Skia samples the decoder's unorm planes before its affine
/// matrix, without section 13's range-normalization intermediate.
pub(super) struct Upload {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    gamut_pipeline: wgpu::ComputePipeline,
    sampler: wgpu::Sampler,
    decode_lut: wgpu::Buffer,
}

impl Upload {
    pub fn supports(frame: &ImportedFrame, matrix: MatrixCoefficients) -> bool {
        !frame.format.has_alpha()
            && matches!(frame.format.layout, Layout::Biplanar | Layout::Planar)
            && (frame.format.bits == 8 || matches!(frame.format.label.as_str(), "x420" | "xf20"))
            && matches!(
                matrix,
                MatrixCoefficients::Bt601
                    | MatrixCoefficients::Bt709
                    | MatrixCoefficients::Bt2020Ncl
            )
    }

    pub fn new(device: &wgpu::Device) -> Self {
        let textures = (0..3).map(|binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Skia video upload"),
            entries: &textures
                .chain([
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 5,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::WriteOnly,
                            format: wgpu::TextureFormat::Rgba8Unorm,
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 6,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ])
                .collect::<Vec<_>>(),
        });
        let source = format!(
            "{}\n{}",
            pm_color::wgsl::COLOR,
            include_str!("../shaders/video_upload.wgsl")
        );
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Skia video upload"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Skia video upload"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Skia video upload"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let gamut_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Skia Canvas gamut"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("gamut"),
            compilation_options: Default::default(),
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Skia video upload"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let table = pm_media::image::compat::srgb_decode_table();
        let decode_lut = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Skia sRGB decode"),
            contents: bytes_of(&table),
            usage: wgpu::BufferUsages::STORAGE,
        });
        Self {
            layout,
            pipeline,
            gamut_pipeline,
            sampler,
            decode_lut,
        }
    }

    pub fn run(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        frame: &ImportedFrame,
        matrix: MatrixCoefficients,
        gamut: bool,
        reuse: Option<Target>,
    ) -> Target {
        let (w, h) = (frame.format.width as u32, frame.format.height as u32);
        let out = reuse.unwrap_or_else(|| {
            Target::new(
                device,
                "Skia video upload",
                w,
                h,
                wgpu::TextureFormat::Rgba8Unorm,
                pm_gpu::target::working_usage(),
            )
        });
        // SkYUVMath.cpp's default six-decimal tables (chrome/m152).
        // Offsets are independently rounded; deriving them from Kr/Kb moves
        // half-byte boundaries even on flat regions of an otherwise exact decode.
        let rows = match (matrix, frame.format.range) {
            (MatrixCoefficients::Bt601, Range::Limited) => [
                [1.164384, 0.0, 1.596027, -0.874202],
                [1.164384, -0.391762, -0.812968, 0.531668],
                [1.164384, 2.017232, 0.0, -1.085631],
            ],
            (MatrixCoefficients::Bt709, Range::Limited) => [
                [1.164384, 0.0, 1.792741, -0.972945],
                [1.164384, -0.213249, -0.532909, 0.301483],
                [1.164384, 2.112402, 0.0, -1.133402],
            ],
            (MatrixCoefficients::Bt2020Ncl, Range::Limited) => [
                [1.164384, 0.0, 1.678674, -0.915688],
                [1.164384, -0.187326, -0.650424, 0.347458],
                [1.164384, 2.141772, 0.0, -1.148145],
            ],
            (MatrixCoefficients::Bt601, Range::Full) => [
                [1.0, 0.0, 1.402, -0.703749],
                [1.0, -0.344136, -0.714136, 0.531211],
                [1.0, 1.772, 0.0, -0.889475],
            ],
            (MatrixCoefficients::Bt709, Range::Full) => [
                [1.0, 0.0, 1.5748, -0.790488],
                [1.0, -0.187324, -0.468124, 0.32901],
                [1.0, 1.8556, 0.0, -0.931439],
            ],
            _ => [
                [1.0, 0.0, 1.4746, -0.740191],
                [1.0, -0.164553, -0.571353, 0.369396],
                [1.0, 1.8814, 0.0, -0.944389],
            ],
        };
        // Skia's named D50 gamuts, concatenated in f32 by pinned skcms.
        let p = [
            [1.6603378f32, -0.5877952, -0.07283652],
            [-0.12453252, 1.1329923, -0.008339925],
            [-0.018131323, -0.100585446, 1.1188763],
        ];
        let rows = if matrix == MatrixCoefficients::Bt2020Ncl && frame.format.bits > 8 {
            [
                [1.167808, 0.0, 1.683611, -0.915688],
                [1.167808, -0.187877, -0.652337, 0.347458],
                [1.167808, 2.148072, 0.0, -1.148145],
            ]
        } else {
            rows
        };
        let params = Params {
            rows,
            primaries: std::array::from_fn(|j| [p[0][j], p[1][j], p[2][j], 0.0]),
            flags: [u32::from(frame.format.layout == Layout::Planar), 0, 0, 0],
        };
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Skia video upload"),
            contents: bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let source = if gamut {
            Target::new(
                device,
                "Skia video source bitmap",
                w,
                h,
                wgpu::TextureFormat::Rgba8Unorm,
                pm_gpu::target::working_usage(),
            )
        } else {
            out.clone()
        };
        let views: Vec<_> = frame
            .textures
            .iter()
            .map(|t| t.create_view(&Default::default()))
            .collect();
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Skia video upload"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&views[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&views[1]),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(views.get(2).unwrap_or(&views[1])),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&source.view),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: self.decode_lut.as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Skia video upload"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        drop(pass);
        if gamut {
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Skia Canvas gamut"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&source.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&source.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&source.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::TextureView(&out.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: self.decode_lut.as_entire_binding(),
                    },
                ],
            });
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Skia Canvas gamut"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.gamut_pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        out
    }
}
