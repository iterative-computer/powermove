//! bwdif deinterlacing on the GPU (section 13), on normalized planes in
//! Y′CbCr before chroma upsampling. `pm_media::interlace` decides field rate
//! or frame rate and which field a time shows; the session supplies the
//! previous, current, and next frames.

use super::{ImportError, Normalized, Result};
use pm_media::frame::{Layout, PixelFormat};
use pm_protocol::color::SampleAlignment;

/// How normalized values map back to the integer codes FFmpeg's bwdif
/// filters: per channel `code = value · div + off`, clipped to `0..=max`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CodeSpace {
    /// False for float images: the filter runs on f32 values.
    pub integer: bool,
    /// Channels of the luma (or RGBA) texture.
    pub main: [[f32; 4]; 3],
    /// Channels of the chroma texture.
    pub chroma: [[f32; 4]; 3],
}

impl CodeSpace {
    pub fn of(f: &PixelFormat) -> CodeSpace {
        let n = f.norm();
        // MSB-aligned formats filter n-bit codes (c = v / 64 for x420).
        let unit = if f.alignment == SampleAlignment::MsbIn16 {
            (1u32 << (16 - f.bits)) as f64
        } else {
            1.0
        };
        let max = ((1u64 << f.bits) - 1) as f32;
        let amax = n.a_div as f32;
        let rgb = matches!(f.layout, Layout::PlanarRgb | Layout::Rgba);
        let main = if rgb {
            [[0.0; 4], [max, max, max, amax], [max, max, max, amax]]
        } else {
            [
                [(n.y_off / unit) as f32, 0.0, 0.0, 0.0],
                [(n.y_div / unit) as f32, 1.0, 1.0, amax],
                [max, 0.0, 0.0, amax],
            ]
        };
        let (co, cd) = ((n.c_off / unit) as f32, (n.c_div / unit) as f32);
        CodeSpace {
            integer: !f.is_float(),
            main,
            chroma: [[co, co, 0.0, 0.0], [cd, cd, 1.0, 1.0], [max, max, 0.0, 0.0]],
        }
    }
}

/// One output picture: which field of the current frame is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldPass {
    /// The source is top field first.
    pub tff: bool,
    /// Keep the second field in display order (field-rate output's odd
    /// pictures); frame-rate output keeps the first.
    pub second: bool,
    /// The kept field has no temporal neighbor on one side: the first field
    /// of the first frame, or the second field of the last frame (FFmpeg's
    /// `YADIF_FIELD_END`): interpolate spatially.
    pub intra: bool,
}

impl FieldPass {
    /// For frame-rate or field-rate output of a frame with the given
    /// neighbors (`first`/`last` from `pm_media`'s `FieldFrames`).
    pub fn new(tff: bool, second: bool, first: bool, last: bool) -> FieldPass {
        FieldPass {
            tff,
            second,
            intra: (first && !second) || (last && second),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Params {
    parity: u32,
    prev2: u32,
    intra: u32,
    h: u32,
    integer: u32,
    _pad: [u32; 3],
    off: [f32; 4],
    div: [f32; 4],
    cmax: [f32; 4],
}

struct Variant {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
}

pub struct Deinterlacer {
    device: wgpu::Device,
    rgba: Variant,
    rg: Variant,
}

impl Deinterlacer {
    pub fn new(device: &wgpu::Device) -> Deinterlacer {
        let variant = |format: wgpu::TextureFormat, name: &str| {
            let src = include_str!("bwdif.wgsl").replace("FORMAT", name);
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("pm-gpu bwdif"),
                source: wgpu::ShaderSource::Wgsl(src.into()),
            });
            let tex = |b| wgpu::BindGroupLayoutEntry {
                binding: b,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            };
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("pm-gpu bwdif"),
                entries: &[
                    tex(0),
                    tex(1),
                    tex(2),
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::WriteOnly,
                            format,
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                ],
            });
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("pm-gpu bwdif"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
            let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("pm-gpu bwdif"),
                layout: Some(&pl),
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });
            Variant {
                pipeline,
                layout,
                format,
            }
        };
        Deinterlacer {
            device: device.clone(),
            rgba: variant(wgpu::TextureFormat::Rgba32Float, "rgba32float"),
            rg: variant(wgpu::TextureFormat::Rg32Float, "rg32float"),
        }
    }

    fn plane(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        planes: [&wgpu::Texture; 3],
        pass: FieldPass,
        integer: bool,
        codes: [[f32; 4]; 3],
    ) -> Result<wgpu::Texture> {
        let format = planes[1].format();
        let v = if format == self.rgba.format {
            &self.rgba
        } else if format == self.rg.format {
            &self.rg
        } else {
            return Err(ImportError::Unsupported(format!("bwdif input {format:?}")));
        };
        let (w, h) = (planes[1].width(), planes[1].height());
        if planes.iter().any(|t| t.width() != w || t.height() != h) {
            return Err(ImportError::Unsupported(
                "bwdif frames differ in size".into(),
            ));
        }
        // FFmpeg: parity = tff ^ !is_second; prev2/next2 select on parity ^ tff.
        let parity = (pass.tff ^ !pass.second) as u32;
        let params = Params {
            parity,
            prev2: parity ^ pass.tff as u32,
            intra: pass.intra as u32,
            h,
            integer: integer as u32,
            _pad: [0; 3],
            off: codes[0],
            div: codes[1],
            cmax: codes[2],
        };
        let ubuf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bwdif params"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&ubuf, 0, unsafe {
            std::slice::from_raw_parts(
                &params as *const Params as *const u8,
                std::mem::size_of::<Params>(),
            )
        });
        let out = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("deinterlaced"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let views: Vec<wgpu::TextureView> = planes
            .iter()
            .map(|t| t.create_view(&Default::default()))
            .collect();
        let ov = out.create_view(&Default::default());
        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bwdif"),
            layout: &v.layout,
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
                    resource: wgpu::BindingResource::TextureView(&views[2]),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: ubuf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&ov),
                },
            ],
        });
        let mut cp = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("bwdif"),
            timestamp_writes: None,
        });
        cp.set_pipeline(&v.pipeline);
        cp.set_bind_group(0, &bg, &[]);
        cp.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        drop(cp);
        Ok(out)
    }

    /// Deinterlaces the current frame, keeping the field `pass` names.
    /// `codes` comes from the frames' pixel format ([`CodeSpace::of`]).
    #[allow(clippy::too_many_arguments)]
    pub fn run(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        prev: &Normalized,
        cur: &Normalized,
        next: &Normalized,
        pass: FieldPass,
        codes: &CodeSpace,
    ) -> Result<Normalized> {
        match (prev, cur, next) {
            (
                Normalized::YCbCr {
                    luma: pl,
                    chroma: pc,
                },
                Normalized::YCbCr {
                    luma: cl,
                    chroma: cc,
                },
                Normalized::YCbCr {
                    luma: nl,
                    chroma: nc,
                },
            ) => Ok(Normalized::YCbCr {
                luma: self.plane(
                    queue,
                    encoder,
                    [pl, cl, nl],
                    pass,
                    codes.integer,
                    codes.main,
                )?,
                chroma: self.plane(
                    queue,
                    encoder,
                    [pc, cc, nc],
                    pass,
                    codes.integer,
                    codes.chroma,
                )?,
            }),
            (
                Normalized::Rgb { rgba: p },
                Normalized::Rgb { rgba: c },
                Normalized::Rgb { rgba: n },
            ) => Ok(Normalized::Rgb {
                rgba: self.plane(queue, encoder, [p, c, n], pass, codes.integer, codes.main)?,
            }),
            _ => Err(ImportError::Unsupported(
                "bwdif frames differ in layout".into(),
            )),
        }
    }
}
