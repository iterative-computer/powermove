//! GPU resources of glTF draws: textures of the file's images (with their
//! samplers and generated mipmaps), float data textures (`boneTexture`,
//! `morphTargetsTexture`), vertex attributes beyond position, normal, and
//! `uv` (with WebGL's constant (0, 0, 0, 1) for attributes a geometry
//! lacks), instance buffers, and the transmission render target
//! (`renderTransmissionPass`: RGBA half float, 4× MSAA, resolved into a
//! mipmapped texture sampled trilinearly and clamped).

use std::collections::HashMap;
use std::sync::Arc;

use pm_geom3d::gltf::{Filter, Sampler, Wrap};

use super::{GltfMap, MeshExt, VertexExtra};

/// The transmission target's MSAA sample count (`samples: 4`).
pub const TRANSMISSION_SAMPLES: u32 = 4;

pub(crate) struct Transmission {
    pub width: u32,
    pub height: u32,
    pub msaa: wgpu::TextureView,
    pub depth: wgpu::TextureView,
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
}

/// A vertex buffer with its format and stride.
#[derive(Clone)]
pub(crate) struct VertexBuffer {
    pub buffer: wgpu::Buffer,
}

type ImageEntry = (Arc<wgpu::Texture>, wgpu::TextureView, u64);

#[derive(Default)]
pub(crate) struct GltfGpu {
    /// By image key, color space, and mipmapping: texture, view, last use.
    images: HashMap<(u64, bool, bool), ImageEntry>,
    samplers: HashMap<Sampler, wgpu::Sampler>,
    data: HashMap<u64, (wgpu::TextureView, u64)>,
    /// Per geometry key and attribute name.
    vertex: HashMap<(u64, &'static str), (VertexBuffer, u64)>,
    instances: HashMap<(u64, &'static str), (VertexBuffer, u64)>,
    pub transmission: Option<Transmission>,
    mip16: Option<(wgpu::RenderPipeline, wgpu::BindGroupLayout, wgpu::Sampler)>,
    nearest: Option<wgpu::Sampler>,
    transmission_sampler: Option<wgpu::Sampler>,
    generation: u64,
}

fn buffer(
    device: &wgpu::Device,
    label: &str,
    mut bytes: Vec<u8>,
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    // Padding as the scene meshes' buffers (S7 finding 6).
    bytes.resize(bytes.len() + 256, 0);
    wgpu::util::DeviceExt::create_buffer_init(
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: &bytes,
            usage,
        },
    )
}

fn f32_bytes(v: impl Iterator<Item = f32>) -> Vec<u8> {
    v.flat_map(f32::to_le_bytes).collect()
}

fn address(w: Wrap) -> wgpu::AddressMode {
    match w {
        Wrap::Repeat => wgpu::AddressMode::Repeat,
        Wrap::Clamp => wgpu::AddressMode::ClampToEdge,
        Wrap::Mirror => wgpu::AddressMode::MirrorRepeat,
    }
}

/// The vertex attributes a glTF program reads, with their item size and
/// step (instance attributes step per instance).
pub(crate) fn attribute_format(
    name: &str,
    extra: Option<&VertexExtra>,
) -> Option<(wgpu::VertexFormat, u64, bool)> {
    Some(match name {
        "position" | "normal" => (wgpu::VertexFormat::Float32x3, 12, false),
        "uv" | "uv1" | "uv2" | "uv3" => (wgpu::VertexFormat::Float32x2, 8, false),
        "tangent" | "skinIndex" | "skinWeight" => (wgpu::VertexFormat::Float32x4, 16, false),
        "color" => match extra.and_then(|x| x.color.as_ref()).map(|c| c.1) {
            Some(4) => (wgpu::VertexFormat::Float32x4, 16, false),
            _ => (wgpu::VertexFormat::Float32x3, 12, false),
        },
        "instanceMatrix_pmc0"
        | "instanceMatrix_pmc1"
        | "instanceMatrix_pmc2"
        | "instanceMatrix_pmc3" => (wgpu::VertexFormat::Float32x4, 64, true),
        "instanceColor" => (wgpu::VertexFormat::Float32x3, 12, true),
        _ => return None,
    })
}

impl GltfGpu {
    pub fn evict(&mut self, generation: u64, keep_from: u64) {
        self.generation = generation;
        self.images.retain(|_, e| e.2 >= keep_from);
        self.data.retain(|_, e| e.1 >= keep_from);
        self.vertex.retain(|_, e| e.1 >= keep_from);
        self.instances.retain(|_, e| e.1 >= keep_from);
    }

    pub fn resident(&self) -> (usize, u64) {
        let bytes = self
            .vertex
            .values()
            .chain(self.instances.values())
            .map(|(b, _)| b.buffer.size())
            .sum();
        (
            self.vertex.len() + self.instances.len() + self.images.len() + self.data.len(),
            bytes,
        )
    }

    fn mip_pipeline(&mut self, device: &wgpu::Device) {
        if self.mip16.is_some() {
            return;
        }
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("3d transmission mips"),
            source: wgpu::ShaderSource::Wgsl(
                r#"
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;
struct Out { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vs(@builtin(vertex_index) i: u32) -> Out {
    let a = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: Out;
    o.pos = vec4<f32>(a * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2<f32>(a.x, 1.0 - a.y);
    return o;
}
@fragment fn fs(in: Out) -> @location(0) vec4<f32> { return textureSampleLevel(src, smp, in.uv, 0.0); }
"#
                .into(),
            ),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("3d transmission mips"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("3d transmission mips"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("3d transmission mips"),
            layout: Some(&pl),
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
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba16Float,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("3d transmission mip"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        self.mip16 = Some((pipeline, layout, sampler));
    }

    /// `generateMipmap` of an RGBA half-float texture (each level a box
    /// filter of the one above).
    pub fn generate_mips(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
    ) {
        self.mip_pipeline(device);
        let (pipeline, layout, sampler) = self.mip16.as_ref().expect("mip pipeline");
        for level in 1..texture.mip_level_count() {
            let view = |l: u32| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    base_mip_level: l,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            };
            let (src, dst) = (view(level - 1), view(level));
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("3d transmission mip"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&src),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                ],
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("3d transmission mip"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &dst,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    /// The transmission target for a `width × height` canvas.
    pub fn transmission(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> &Transmission {
        if !matches!(&self.transmission, Some(t) if t.width == width && t.height == height) {
            let make = |format, samples, levels, usage, label| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: levels,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
            };
            let msaa = make(
                wgpu::TextureFormat::Rgba16Float,
                TRANSMISSION_SAMPLES,
                1,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
                "3d transmission msaa",
            );
            let depth = make(
                wgpu::TextureFormat::Depth32Float,
                TRANSMISSION_SAMPLES,
                1,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
                "3d transmission depth",
            );
            let texture = make(
                wgpu::TextureFormat::Rgba16Float,
                1,
                super::super::textures::mip_levels(width, height),
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                "3d transmission",
            );
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.transmission = Some(Transmission {
                width,
                height,
                msaa: msaa.create_view(&Default::default()),
                depth: depth.create_view(&Default::default()),
                texture,
                view,
            });
        }
        self.transmission.as_ref().expect("transmission target")
    }

    /// The level-0 view of the transmission texture (the resolve target).
    pub fn transmission_level0(&self) -> Option<wgpu::TextureView> {
        self.transmission.as_ref().map(|t| {
            t.texture.create_view(&wgpu::TextureViewDescriptor {
                base_mip_level: 0,
                mip_level_count: Some(1),
                ..Default::default()
            })
        })
    }

    /// `transmissionSamplerMap`: the mipmapped texture, trilinear, clamped.
    pub fn transmission_binding(
        &mut self,
        device: &wgpu::Device,
    ) -> Option<(wgpu::TextureView, wgpu::Sampler)> {
        let s = self
            .transmission_sampler
            .get_or_insert_with(|| {
                device.create_sampler(&wgpu::SamplerDescriptor {
                    label: Some("3d transmission"),
                    address_mode_u: wgpu::AddressMode::ClampToEdge,
                    address_mode_v: wgpu::AddressMode::ClampToEdge,
                    address_mode_w: wgpu::AddressMode::ClampToEdge,
                    mag_filter: wgpu::FilterMode::Linear,
                    min_filter: wgpu::FilterMode::Linear,
                    mipmap_filter: wgpu::MipmapFilterMode::Linear,
                    ..Default::default()
                })
            })
            .clone();
        self.transmission.as_ref().map(|t| (t.view.clone(), s))
    }

    /// The sampler of a glTF texture (`magFilter`, `minFilter`, wraps).
    fn sampler(&mut self, device: &wgpu::Device, s: Sampler) -> wgpu::Sampler {
        self.samplers
            .entry(s)
            .or_insert_with(|| {
                let (min, mip) = match s.min {
                    Filter::Nearest | Filter::NearestMipmapNearest => {
                        (wgpu::FilterMode::Nearest, wgpu::MipmapFilterMode::Nearest)
                    }
                    Filter::Linear | Filter::LinearMipmapNearest => {
                        (wgpu::FilterMode::Linear, wgpu::MipmapFilterMode::Nearest)
                    }
                    Filter::NearestMipmapLinear => {
                        (wgpu::FilterMode::Nearest, wgpu::MipmapFilterMode::Linear)
                    }
                    Filter::LinearMipmapLinear => {
                        (wgpu::FilterMode::Linear, wgpu::MipmapFilterMode::Linear)
                    }
                };
                device.create_sampler(&wgpu::SamplerDescriptor {
                    label: Some("3d gltf map"),
                    address_mode_u: address(s.wrap_s),
                    address_mode_v: address(s.wrap_t),
                    address_mode_w: wgpu::AddressMode::Repeat,
                    mag_filter: if s.mag == Filter::Nearest {
                        wgpu::FilterMode::Nearest
                    } else {
                        wgpu::FilterMode::Linear
                    },
                    min_filter: min,
                    mipmap_filter: mip,
                    ..Default::default()
                })
            })
            .clone()
    }

    /// The texture of a glTF map (uploaded once per image, color space, and
    /// mipmapping).
    pub fn map(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        textures: &mut super::super::textures::Textures,
        m: &GltfMap,
    ) -> (wgpu::TextureView, wgpu::Sampler) {
        let mips = m.sampler.min.mipmapped();
        let key = (m.image.key, m.srgb, mips);
        let generation = self.generation;
        if let Some(e) = self.images.get_mut(&key) {
            e.2 = generation;
            let v = e.1.clone();
            return (v, self.sampler(device, m.sampler));
        }
        let img = &m.image;
        let t = textures.upload_rgba(
            device, queue, encoder, img.width, img.height, &img.rgba, m.srgb, mips,
        );
        let view = t.create_view(&wgpu::TextureViewDescriptor::default());
        self.images
            .insert(key, (Arc::new(t), view.clone(), generation));
        (view, self.sampler(device, m.sampler))
    }

    /// A float data texture (nearest, clamped).
    #[allow(clippy::too_many_arguments)]
    pub fn data(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: u64,
        width: u32,
        height: u32,
        layers: u32,
        array: bool,
        data: &[f32],
    ) -> (wgpu::TextureView, wgpu::Sampler) {
        let generation = self.generation;
        let nearest = self
            .nearest
            .get_or_insert_with(|| {
                device.create_sampler(&wgpu::SamplerDescriptor {
                    label: Some("3d data texture"),
                    ..Default::default()
                })
            })
            .clone();
        if let Some(e) = self.data.get_mut(&key) {
            e.1 = generation;
            return (e.0.clone(), nearest);
        }
        let t = wgpu::util::DeviceExt::create_texture_with_data(
            device,
            queue,
            &wgpu::TextureDescriptor {
                label: Some("3d data texture"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: layers,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba32Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &f32_bytes(data.iter().copied()),
        );
        let view = t.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(if array || layers > 1 {
                wgpu::TextureViewDimension::D2Array
            } else {
                wgpu::TextureViewDimension::D2
            }),
            ..Default::default()
        });
        self.data.insert(key, (view.clone(), generation));
        (view, nearest)
    }

    /// A vertex attribute of a glTF geometry (WebGL's constant (0, 0, 0, 1)
    /// for attributes the geometry lacks).
    pub fn vertex(
        &mut self,
        device: &wgpu::Device,
        geometry: &super::super::model::Geometry3d,
        name: &'static str,
    ) -> Option<wgpu::Buffer> {
        let generation = self.generation;
        let key = (geometry.key, name);
        if let Some(e) = self.vertex.get_mut(&key) {
            e.1 = generation;
            return Some(e.0.buffer.clone());
        }
        let x = geometry.extra.as_deref();
        let n = geometry.geometry.vertex_count();
        let (_, stride, _) = attribute_format(name, x)?;
        let comps = (stride / 4) as usize;
        let data: Option<&Vec<f32>> = match name {
            "tangent" => x.and_then(|x| x.tangent.as_ref()),
            "color" => x.and_then(|x| x.color.as_ref()).map(|c| &c.0),
            "uv1" => x.and_then(|x| x.uv1.as_ref()),
            "uv2" => x.and_then(|x| x.uv2.as_ref()),
            "uv3" => x.and_then(|x| x.uv3.as_ref()),
            "skinIndex" => x.and_then(|x| x.skin_index.as_ref()),
            "skinWeight" => x.and_then(|x| x.skin_weight.as_ref()),
            _ => None,
        };
        let bytes = match data {
            Some(d) if d.len() >= n * comps => f32_bytes(d[..n * comps].iter().copied()),
            _ => {
                f32_bytes((0..n * comps).map(|i| if comps == 4 && i % 4 == 3 { 1.0 } else { 0.0 }))
            }
        };
        let b = buffer(
            device,
            "3d gltf attribute",
            bytes,
            wgpu::BufferUsages::VERTEX,
        );
        self.vertex
            .insert(key, (VertexBuffer { buffer: b.clone() }, generation));
        Some(b)
    }

    /// An instance attribute (`instanceMatrix` columns, `instanceColor`).
    pub fn instance(
        &mut self,
        device: &wgpu::Device,
        e: &MeshExt,
        name: &'static str,
    ) -> Option<wgpu::Buffer> {
        let (key, inst) = e.instances.as_ref()?;
        let generation = self.generation;
        let column = match name {
            "instanceMatrix_pmc0" => Some(0),
            "instanceMatrix_pmc1" => Some(1),
            "instanceMatrix_pmc2" => Some(2),
            "instanceMatrix_pmc3" => Some(3),
            _ => None,
        };
        let k = (
            *key,
            if column.is_some() {
                "instanceMatrix"
            } else {
                name
            },
        );
        if let Some(e) = self.instances.get_mut(&k) {
            e.1 = generation;
            return Some(e.0.buffer.clone());
        }
        let bytes = match column {
            // One buffer of whole matrices; each column attribute reads it
            // at its offset with a 64-byte stride.
            Some(_) => f32_bytes(inst.matrices.iter().flat_map(|m| m.iter().copied())),
            None => {
                let c = inst.colors.as_ref()?;
                f32_bytes((0..inst.matrices.len()).flat_map(|i| {
                    (0..3).map(move |k| c.data.get(i * c.size + k).copied().unwrap_or(0.0))
                }))
            }
        };
        let b = buffer(
            device,
            "3d gltf instances",
            bytes,
            wgpu::BufferUsages::VERTEX,
        );
        self.instances
            .insert(k, (VertexBuffer { buffer: b.clone() }, generation));
        Some(b)
    }
}
