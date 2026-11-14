//! Studio's inexpensive forward preview. Geometry and texture identities are
//! retained; camera motion updates only a small instance/camera upload.
//! The legacy pm-render raster pass expects file maps and Scene3d geometry;
//! Studio already supplies resolved live map arrays and immutable Mesh Arcs.
//! Keeping this adapter on that boundary avoids re-evaluating the document.
use crate::{
    SceneExt,
    extract::{MeshInstance, SceneInput, SceneRequest, TextureMaps},
    service::{FastRenderer, FrameTimings, LayerTexture, RenderedScene},
};
use std::sync::Arc;
use wgpu::util::DeviceExt;
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
    uv1: [f32; 2],
    color: [f32; 4],
    material: u32,
}
struct Mesh {
    identity: MeshInstance,
    buffer: wgpu::Buffer,
    count: u32,
}
pub struct Raster {
    device: wgpu::Device,
    queue: wgpu::Queue,
    color: wgpu::RenderPipeline,
    depth: wgpu::RenderPipeline,
    background: wgpu::RenderPipeline,
    flip: wgpu::RenderPipeline,
    meshes: Vec<Mesh>,
    maps: Option<TextureMaps>,
    texture: Option<wgpu::Texture>,
    sampler: wgpu::Sampler,
    #[cfg(test)]
    test_depth: Option<wgpu::Texture>,
}
fn buffer(device: &wgpu::Device, bytes: &[u8], usage: wgpu::BufferUsages) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Studio Fast data"),
        contents: if bytes.is_empty() { &[0; 16] } else { bytes },
        usage,
    })
}
impl Raster {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Studio Fast forward"),
            source: wgpu::ShaderSource::Wgsl(include_str!("fast.wgsl").into()),
        });
        let entries = [
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            storage(1, wgpu::ShaderStages::FRAGMENT),
            storage(2, wgpu::ShaderStages::VERTEX),
            storage(3, wgpu::ShaderStages::FRAGMENT),
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 5,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ];
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Studio Fast"),
            entries: &entries,
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Studio Fast"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let attributes = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x2,3=>Float32x2,4=>Float32x4,5=>Uint32];
        let make = |depth| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Studio Fast"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &attributes,
                    })],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(if depth { "depth" } else { "fs" }),
                    compilation_options: Default::default(),
                    targets: if depth {
                        &[]
                    } else {
                        &[Some(wgpu::ColorTargetState {
                            format: wgpu::TextureFormat::Rgba8Unorm,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })]
                    },
                }),
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(depth),
                    depth_compare: Some(if depth {
                        wgpu::CompareFunction::LessEqual
                    } else {
                        wgpu::CompareFunction::Equal
                    }),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let background = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Studio Fast background"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("sky_vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("sky_fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Equal),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let flip_shader=device.create_shader_module(wgpu::ShaderModuleDescriptor { label:Some("Studio Fast row orientation"),source:wgpu::ShaderSource::Wgsl(r#"
@group(0) @binding(0) var source:texture_2d<f32>;
@vertex fn vs(@builtin(vertex_index) i:u32)->@builtin(position) vec4f {
 let p=array<vec2f,3>(vec2f(-1.,-1.),vec2f(3.,-1.),vec2f(-1.,3.));return vec4f(p[i],0.,1.);
}
@fragment fn fs(@builtin(position) p:vec4f)->@location(0) vec4f {
 let size=textureDimensions(source);return textureLoad(source,vec2i(i32(p.x),i32(size.y)-1-i32(p.y)),0);
}"#.into()) });
        let flip = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Studio Fast row orientation"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &flip_shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &flip_shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let color = make(false);
        let depth = make(true);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Studio Fast maps"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            device,
            queue,
            color,
            depth,
            background,
            flip,
            meshes: vec![],
            maps: None,
            texture: None,
            sampler,
            #[cfg(test)]
            test_depth: None,
        }
    }
    fn mesh(&mut self, i: &MeshInstance, materials: usize) -> Result<usize, String> {
        if let Some(index) = self.meshes.iter().position(|m| {
            Arc::ptr_eq(&m.identity.mesh, &i.mesh)
                && m.identity.material == i.material
                && m.identity.material_palette == i.material_palette
                && match (&m.identity.attributes, &i.attributes) {
                    (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                    (None, None) => true,
                    _ => false,
                }
        }) {
            return Ok(index);
        }
        crate::extract::validate_mesh(&i.mesh, i.attributes.as_deref())?;
        let mut vertices = Vec::new();
        let m = &i.mesh;
        for (f, t) in m.indices.iter().enumerate() {
            let slot = m.triangle_materials.get(f).copied();
            let material = match (slot, &i.material_palette) {
                (None, _) => i.material,
                (Some(n), Some(p)) => *p.get(n).ok_or("Fast material palette index")?,
                (Some(n), None) => n,
            };
            if material >= materials {
                return Err("Fast material index".into());
            }
            let p = t.map(|n| m.positions[n as usize]);
            let face = (p[1] - p[0]).cross(p[2] - p[0]).normalize();
            for &n in t {
                let n = n as usize;
                let a = i.attributes.as_deref();
                let normal = if a.is_some_and(|a| a.flat_shading) {
                    face
                } else {
                    m.normals.get(n).copied().unwrap_or(face)
                };
                vertices.push(Vertex {
                    position: m.positions[n].to_array(),
                    normal: normal.to_array(),
                    uv: m.uvs.get(n).copied().unwrap_or([0.; 2]),
                    uv1: a
                        .and_then(|a| a.secondary_uvs.get(n))
                        .copied()
                        .unwrap_or_else(|| m.uvs.get(n).copied().unwrap_or([0.; 2])),
                    color: a
                        .and_then(|a| a.colors.get(n))
                        .copied()
                        .unwrap_or_else(|| m.colors.get(n).map_or([1.; 4], |v| v.to4(1.))),
                    material: material as u32,
                });
            }
        }
        let index = self.meshes.len();
        self.meshes.push(Mesh {
            identity: i.clone(),
            buffer: buffer(
                &self.device,
                bytemuck::cast_slice(&vertices),
                wgpu::BufferUsages::VERTEX,
            ),
            count: vertices.len() as u32,
        });
        Ok(index)
    }
    fn maps(&mut self, input: &SceneInput) {
        if self.texture.is_some() && self.maps == input.maps {
            return;
        }
        let (width, height, layers) = input
            .maps
            .as_ref()
            .map_or((1, 1, 1), |m| (m.width, m.height, m.layers.len() as u32));
        let reuse = self.texture.as_ref().is_some_and(|t| {
            t.width() == width && t.height() == height && t.depth_or_array_layers() == layers
        });
        if !reuse {
            self.texture = Some(self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Studio Fast maps"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: layers,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            }));
        }
        let texture = self.texture.as_ref().unwrap();
        for layer in 0..layers {
            if reuse
                && self
                    .maps
                    .as_ref()
                    .zip(input.maps.as_ref())
                    .is_some_and(|(old, new)| {
                        old.layers[layer as usize] == new.layers[layer as usize]
                    })
            {
                continue;
            }
            let bytes = input
                .maps
                .as_ref()
                .map_or(&[255; 4][..], |m| m.layers[layer as usize].as_slice());
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: layer,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * 4),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }
        self.maps = input.maps.clone();
    }
    pub fn render_oriented(
        &mut self,
        input: &SceneInput,
        request: &SceneRequest,
        flip_y: bool,
    ) -> Result<RenderedScene, String> {
        self.render_outputs(input, request, flip_y, &[])
    }
    fn render_outputs(
        &mut self,
        input: &SceneInput,
        request: &SceneRequest,
        flip_y: bool,
        auxiliary: &[crate::extract::AuxiliaryLayer],
    ) -> Result<RenderedScene, String> {
        let start = std::time::Instant::now();
        let mut materials = input
            .materials
            .clone()
            .unwrap_or_else(|| input.scene.gpu_materials());
        if materials.is_empty() {
            materials.push(crate::MaterialExt::gpu(
                &crate::scene::Material::dielectric(crate::math::Vec3::ZERO, 1.),
            ));
        }
        let mut draws = Vec::new();
        let mut layers = Vec::new();
        let mut instances = Vec::<[[f32; 4]; 8]>::new();
        for i in &input.instances {
            if !i.primary_visible || i.mesh.indices.is_empty() {
                continue;
            }
            i.world.validate()?;
            // Extraction marks vertex-tinted material slots on the Full path;
            // preserve that same policy without building a BVH for Fast.
            if !i.mesh.colors.is_empty()
                || i.attributes.as_ref().is_some_and(|a| !a.colors.is_empty())
            {
                let slots =
                    i.mesh.triangle_materials.iter().copied().map(Some).chain(
                        std::iter::once(None).filter(|_| i.mesh.triangle_materials.is_empty()),
                    );
                for slot in slots {
                    let slot = match (slot, &i.material_palette) {
                        (None, _) => i.material,
                        (Some(n), Some(p)) => *p.get(n).ok_or("Fast material palette index")?,
                        (Some(n), None) => n,
                    };
                    let m = materials.get_mut(slot).ok_or("Fast material index")?;
                    m.policy[1] = (m.policy[1] as u32 | 16) as f32;
                }
            }
            let mesh = self.mesh(i, materials.len())?;
            let world = glam::Mat4::from_cols_array(&i.world.0);
            let normal = world.inverse().transpose();
            let mut matrices = [[0.; 4]; 8];
            matrices[..4].copy_from_slice(&world.to_cols_array_2d());
            matrices[4..].copy_from_slice(&normal.to_cols_array_2d());
            // Three/Full preserve authored facing under reflected transforms.
            // This unused affine-normal element carries orientation to the shader.
            matrices[7][3] = world.determinant().signum();
            let instance = instances.len() as u32;
            instances.push(matrices);
            if !layers.contains(&i.layer) {
                layers.push(i.layer.clone());
            }
            draws.push((i.layer.clone(), mesh, instance));
        }
        for aux in auxiliary {
            if !layers.contains(&aux.id) {
                layers.push(aux.id.clone());
            }
        }
        self.maps(input);
        // Orientation is a final pixel-row transport, never a camera reflection:
        // reflecting projected Y changes the rasterizer's top-left edge rule.
        let camera = camera(input, request, false);
        let mut lights = input.scene.gpu_lights();
        if lights.is_empty() {
            lights.push(bytemuck::Zeroable::zeroed());
        }
        let camera = buffer(
            &self.device,
            bytemuck::cast_slice(&camera),
            wgpu::BufferUsages::UNIFORM,
        );
        let materials = buffer(
            &self.device,
            bytemuck::cast_slice(&materials),
            wgpu::BufferUsages::STORAGE,
        );
        if instances.is_empty() {
            instances.push([[0.; 4]; 8]);
        }
        let instances = buffer(
            &self.device,
            bytemuck::cast_slice(&instances),
            wgpu::BufferUsages::STORAGE,
        );
        let lights = buffer(
            &self.device,
            bytemuck::cast_slice(&lights),
            wgpu::BufferUsages::STORAGE,
        );
        let maps = self
            .texture
            .as_ref()
            .unwrap()
            .create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            });
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Studio Fast"),
            layout: &self.color.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: materials.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: instances.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: lights.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&maps),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let depth_texture = self.target(request, wgpu::TextureFormat::Depth32Float);
        let depth = depth_texture.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Studio Fast shared depth"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&self.depth);
            pass.set_bind_group(0, &group, &[]);
            for (_, mesh, instance) in &draws {
                let m = &self.meshes[*mesh];
                pass.set_vertex_buffer(0, m.buffer.slice(..));
                pass.draw(0..m.count, *instance..*instance + 1);
            }
        }
        let scratch = flip_y.then(|| self.target(request, wgpu::TextureFormat::Rgba8Unorm));
        let mut outputs = Vec::new();
        for layer in layers {
            let texture = self.target(request, wgpu::TextureFormat::Rgba8Unorm);
            let view = scratch
                .as_ref()
                .unwrap_or(&texture)
                .create_view(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Studio Fast layer"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
                if auxiliary.iter().any(|a| a.id == layer && a.include_void) {
                    pass.set_pipeline(&self.background);
                    pass.set_bind_group(0, &group, &[]);
                    pass.draw(0..3, 0..1);
                }
                pass.set_pipeline(&self.color);
                pass.set_bind_group(0, &group, &[]);
                for (id, mesh, instance) in &draws {
                    if id == &layer {
                        let m = &self.meshes[*mesh];
                        pass.set_vertex_buffer(0, m.buffer.slice(..));
                        pass.draw(0..m.count, *instance..*instance + 1);
                    }
                }
            }
            if scratch.is_some() {
                let output = texture.create_view(&Default::default());
                let flip_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Studio Fast row orientation"),
                    layout: &self.flip.get_bind_group_layout(0),
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    }],
                });
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Studio Fast row orientation"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &output,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                pass.set_pipeline(&self.flip);
                pass.set_bind_group(0, &flip_group, &[]);
                pass.draw(0..3, 0..1);
            }
            outputs.push(LayerTexture { layer, texture });
        }
        self.queue.submit([encoder.finish()]);
        #[cfg(test)]
        {
            self.test_depth = Some(depth_texture);
        }
        // Keep only geometry used by the current immutable scene.
        let used: std::collections::HashSet<_> = draws.iter().map(|(_, m, _)| *m).collect();
        let mut index = 0;
        self.meshes.retain(|_| {
            let keep = used.contains(&index);
            index += 1;
            keep
        });
        Ok(RenderedScene {
            request: request.clone(),
            layers: outputs,
            samples: 1,
            adapter: "Studio Fast raster".into(),
            hardware_rt: false,
            timings: FrameTimings {
                total_ms: start.elapsed().as_secs_f64() * 1000.,
                ..Default::default()
            },
            failures: vec![],
        })
    }
    fn target(&self, r: &SceneRequest, format: wgpu::TextureFormat) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Studio Fast target"),
            size: wgpu::Extent3d {
                width: r.width,
                height: r.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }
}
fn storage(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
fn camera(input: &SceneInput, r: &SceneRequest, flip: bool) -> [[f32; 4]; 7] {
    let c = input.scene.camera;
    let (f, right, up) = input
        .view
        .basis
        .map_or_else(|| c.basis(), |b| (b[0], b[1], b[2]));
    let span = input
        .view
        .orthographic_span
        .unwrap_or_else(|| (c.fov.to_radians() * 0.5).tan());
    let [near, far] = input.view.clip_planes.unwrap_or([0.0001, 1e8]);
    [
        c.position.to4(input.scene.exposure),
        f.to4(0.),
        right.to4(span * r.width as f32 / r.height as f32),
        up.to4(span),
        [
            near.max(0.000001),
            far,
            u8::from(input.view.orthographic_span.is_some()) as f32,
            if flip { -1. } else { 1. },
        ],
        (input.view.ambient + (input.scene.environment.0 + input.scene.environment.1) * 0.5)
            .to4(input.scene.gpu_lights().len() as f32),
        input.scene.backdrop.to4(1.),
    ]
}
impl FastRenderer for Raster {
    fn render_outputs(
        &mut self,
        input: &SceneInput,
        request: &SceneRequest,
        flip_y: bool,
        auxiliary: &[crate::extract::AuxiliaryLayer],
    ) -> Result<RenderedScene, String> {
        Raster::render_outputs(self, input, request, flip_y, auxiliary)
    }
    fn render(
        &mut self,
        input: &SceneInput,
        request: &SceneRequest,
    ) -> Result<RenderedScene, String> {
        self.render_oriented(input, request, false)
    }
    fn render_bottom_up(
        &mut self,
        input: &SceneInput,
        request: &SceneRequest,
    ) -> Result<RenderedScene, String> {
        self.render_oriented(input, request, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        extract::{SceneSource, Transform, View},
        math::{Vec3, v3},
        scene::{Camera, Material},
        service::{StudioService, Tier},
    };
    struct Planes {
        mesh: Arc<crate::mesh::Mesh>,
        clip: [f32; 2],
        mirrored: bool,
    }
    impl SceneSource for Planes {
        fn scene(&self, _: &SceneRequest) -> Result<SceneInput, String> {
            let mut scene = crate::swatch::scene(Material::dielectric(Vec3::splat(0.5), 1.));
            scene.camera = Camera::lens(v3(0., 0., 5.), Vec3::ZERO, 32., 0.);
            scene.shadow_catcher = false;
            scene.materials = vec![Material::dielectric(Vec3::splat(0.5), 1.)];
            let instances = [("front", 0.5, 0.), ("rear", 2., -1.)]
                .into_iter()
                .map(|(layer, width, z)| {
                    let mut world = Transform::default();
                    world.0[0] = width * if self.mirrored { -1. } else { 1. };
                    world.0[14] = z;
                    MeshInstance {
                        layer: layer.into(),
                        mesh: self.mesh.clone(),
                        attributes: None,
                        material: 0,
                        material_palette: None,
                        primary_visible: true,
                        world,
                    }
                })
                .collect();
            let mut materials = scene.gpu_materials();
            materials[0].policy[1] = (materials[0].policy[1] as u32 & !4) as f32;
            Ok(SceneInput {
                scene,
                instances,
                materials: Some(materials),
                maps: None,
                view: View {
                    clip_planes: Some(self.clip),
                    ..Default::default()
                },
            })
        }
    }
    fn read_depth(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<f32> {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Fast test depth read"),
            source: wgpu::ShaderSource::Wgsl(
                r#"
@group(0) @binding(0) var depth:texture_depth_2d;
@group(0) @binding(1) var<storage,read_write> result:array<f32>;
@compute @workgroup_size(1) fn main(@builtin(global_invocation_id) id:vec3u) {
 let coords=array<vec2i,2>(vec2i(32,24),vec2i(45,24));
 result[id.x]=textureLoad(depth,coords[id.x],0);
}"#
                .into(),
            ),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Fast test depth read"),
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let result = buffer(
            device,
            bytemuck::cast_slice(&[0f32; 2]),
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        );
        let view = texture.create_view(&Default::default());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: result.as_entire_binding(),
                },
            ],
        });
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 8,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(2, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&result, 0, &read, 0, 8);
        queue.submit([encoder.finish()]);
        let (send, receive) = std::sync::mpsc::channel();
        read.slice(..)
            .map_async(wgpu::MapMode::Read, move |r| send.send(r).unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        receive.recv().unwrap().unwrap();
        let mapped = read.slice(..).get_mapped_range().unwrap();
        mapped
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect()
    }
    #[test]
    fn fast_depth_and_clip_planes_match_full_surface_queries() {
        let gpu = pm_gpu::Gpu::new("Fast depth regression").unwrap();
        let mut full =
            StudioService::with_device(gpu.device.clone(), gpu.queue.clone(), 64, 48).unwrap();
        let mut fast = Raster::new(gpu.device.clone(), gpu.queue.clone());
        let mesh = Arc::new(crate::mesh::Mesh {
            positions: vec![
                v3(-1., -1., 0.),
                v3(1., -1., 0.),
                v3(1., 1., 0.),
                v3(-1., 1., 0.),
            ],
            normals: vec![v3(0., 0., 1.); 4],
            uvs: vec![[0.; 2]; 4],
            indices: vec![[0, 1, 2], [0, 2, 3]],
            ..Default::default()
        });
        let request = SceneRequest {
            composition: "depth".into(),
            revision: 1,
            ticks: 0,
            width: 64,
            height: 48,
        };
        for (clip, mirrored) in [[0.1, 100.], [0.1, 5.5], [5.5, 100.]]
            .into_iter()
            .flat_map(|clip| [false, true].map(|mirrored| (clip, mirrored)))
        {
            let source = Planes {
                mesh: mesh.clone(),
                clip,
                mirrored,
            };
            let input = source.scene(&request).unwrap();
            fast.render(&input, &request).unwrap();
            let depth = read_depth(&gpu.device, &gpu.queue, fast.test_depth.as_ref().unwrap());
            for (index, x) in [32.5, 45.5].into_iter().enumerate() {
                let hit = full
                    .pick_surface(
                        &source,
                        request.clone(),
                        Tier::QualityLive,
                        x / 64.,
                        24.5 / 48.,
                    )
                    .unwrap();
                let expected = hit.as_ref().map_or(1., |h| {
                    let z = 5. - h.point[2];
                    clip[1] * (z - clip[0]) / ((clip[1] - clip[0]) * z)
                });
                assert!(
                    (depth[index] - expected).abs() < 1e-5,
                    "clip={clip:?}, pixel={x}, raster={}, Full={expected}, hit={hit:?}",
                    depth[index]
                );
            }
        }
    }
}
