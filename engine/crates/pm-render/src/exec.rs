//! Executes a plan on the GPU: sources first, nested targets depth first,
//! then one render pass per target with every draw blended in stack order.

use pm_gpu::{Target, TargetPool};
use pm_protocol::color::{Precision, WorkingEncoding};

use crate::capability::Unsupported;
use crate::error::LayerFailure;
use crate::media::{MediaCache, MediaFailure};
use crate::pipelines::{FillUniform, Pipelines, QuadUniform, UNIFORM_STRIDE, bytes_of};
use crate::plan::{Background, CompPlan, Content, Draw, Source};
use crate::scene3d::Scene3dRenderer;

/// A region of a target in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// A draw with its source resolved to textures.
pub(crate) struct Resolved {
    pub matrix: [f64; 6],
    pub view: Option<wgpu::TextureView>,
    /// Linear view depth of a 3D layer (`r32float`).
    pub depth: Option<wgpu::TextureView>,
    pub alpha: f64,
    /// Solid premultiplied color.
    pub solid: Option<[f64; 4]>,
    /// The texture is a target in GL row order (`u_fromFbo`).
    pub from_fbo: bool,
    pub uv: [f64; 4],
    pub layer: String,
}

pub(crate) struct Exec<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub pipelines: &'a Pipelines,
    pub media: &'a mut MediaCache,
    pub pool: &'a mut TargetPool,
    pub scene3d: &'a mut Scene3dRenderer,
    pub precision: Precision,
    pub working: WorkingEncoding,
    pub export: bool,
    /// The frame also renders the depth and id surface (3D layers then
    /// produce their linear view depth).
    pub depth_id: bool,
    /// Pool targets to give back after the frame is submitted.
    pub taken: Vec<Target>,
    pub failures: Vec<LayerFailure>,
    pub unsupported: Vec<Unsupported>,
}

/// `m3(M)`: a 2×3 matrix as the column-major mat3 uniform, in f32.
fn m3(m: &[f64; 6]) -> [[f32; 4]; 3] {
    [
        [m[0] as f32, m[1] as f32, 0.0, 0.0],
        [m[2] as f32, m[3] as f32, 0.0, 0.0],
        [m[4] as f32, m[5] as f32, 1.0, 0.0],
    ]
}

/// A deterministic session instance per layer (FNV-1a of its id).
fn instance_of(layer: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in layer.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

impl Exec<'_> {
    fn fail(&mut self, plan: &CompPlan, draw: &Draw, error: MediaFailure) {
        match error {
            MediaFailure::Unsupported(feature) => self.unsupported.push(Unsupported {
                composition: Some(plan.composition.clone()),
                layer: Some(draw.layer.clone()),
                feature,
            }),
            MediaFailure::Failed(message) => self.failures.push(LayerFailure {
                composition: plan.composition.clone(),
                layer: draw.layer.clone(),
                message,
            }),
        }
    }

    /// Resolves every draw's source, recording any passes it needs.
    fn resolve(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        plan: &CompPlan,
        region: Option<Region>,
    ) -> Vec<Resolved> {
        let mut out = Vec::with_capacity(plan.draws.len());
        for draw in &plan.draws {
            let mut r = Resolved {
                matrix: draw.matrix,
                view: None,
                depth: None,
                alpha: 1.0,
                solid: None,
                from_fbo: false,
                uv: [0.0, 0.0, 1.0, 1.0],
                layer: draw.layer.clone(),
            };
            match &draw.content {
                Content::Solid(c) => r.solid = Some(*c),
                Content::Texture { source, uv, alpha } => {
                    r.uv = *uv;
                    r.alpha = *alpha;
                    match source {
                        Source::Comp(child) => {
                            // A group boundary shares its parent's pixel
                            // space: it renders only the region, drawn 1:1.
                            let group = child.group;
                            let child_region = if group { region } else { None };
                            let (w, h) = match child_region {
                                Some(g) => (g.width, g.height),
                                None => (child.width, child.height),
                            };
                            let target = self.pool.take_working(self.device, w, h, self.precision);
                            self.render_comp(encoder, child, child_region, &target);
                            r.view = Some(target.view.clone());
                            r.from_fbo = true;
                            if let (true, Some(g)) = (group, region) {
                                r.matrix = [
                                    f64::from(g.width),
                                    0.0,
                                    0.0,
                                    f64::from(g.height),
                                    f64::from(g.x),
                                    f64::from(g.y),
                                ];
                            }
                            self.taken.push(target);
                        }
                        Source::Image { path, .. } => {
                            match self.media.image(
                                self.device,
                                self.queue,
                                self.pipelines,
                                encoder,
                                path,
                                self.working,
                                self.precision,
                            ) {
                                Ok(t) => r.view = Some(t.view.clone()),
                                Err(e) => self.fail(plan, draw, e),
                            }
                        }
                        Source::Video { path, time, .. } => {
                            match self.media.video(
                                self.device,
                                self.queue,
                                self.pipelines,
                                encoder,
                                path,
                                instance_of(&draw.layer),
                                *time,
                                self.export,
                                self.working,
                                self.precision,
                            ) {
                                Ok(Some(t)) => r.view = Some(t.view.clone()),
                                Ok(None) => {}
                                Err(e) => self.fail(plan, draw, e),
                            }
                        }
                        Source::Object3d(object) => {
                            match self.scene3d.render(
                                self.device,
                                self.queue,
                                encoder,
                                self.pool,
                                object,
                                self.depth_id,
                            ) {
                                Ok(out) => {
                                    // The pass renders in GL row order.
                                    r.view = Some(out.color.view.clone());
                                    r.from_fbo = true;
                                    r.depth = out.depth.as_ref().map(|d| d.view.clone());
                                    self.taken.push(out.color);
                                    if let Some(d) = out.depth {
                                        self.taken.push(d);
                                    }
                                }
                                Err(message) => {
                                    self.fail(plan, draw, MediaFailure::Failed(message))
                                }
                            }
                        }
                    }
                }
            }
            out.push(r);
        }
        out
    }

    /// Renders `plan` into `target` (the region's size when `region` is set).
    pub fn render_comp(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        plan: &CompPlan,
        region: Option<Region>,
        target: &Target,
    ) -> Vec<Resolved> {
        let resolved = self.resolve(encoder, plan, region);
        let (ox, oy) = region.map_or((0.0, 0.0), |g| (f64::from(g.x), f64::from(g.y)));
        let (tw, th) = (target.width(), target.height());
        let format = target.format();
        // Uniforms: one slot per draw plus the background quad.
        let slots = resolved.len() + 1;
        let mut data = vec![0u8; slots * UNIFORM_STRIDE as usize];
        let quad = |m: [f64; 6], uv: [f64; 4], color: [f64; 4]| QuadUniform {
            m: m3(&[m[0], m[1], m[2], m[3], m[4] - ox, m[5] - oy]),
            res: [tw as f32, th as f32, 0.0, 0.0],
            uv: uv.map(|v| v as f32),
            color: color.map(|v| v as f32),
        };
        let background = quad(
            [f64::from(tw), 0.0, 0.0, f64::from(th), ox, oy],
            [0.0, 0.0, 1.0, 1.0],
            [0.0; 4],
        );
        data[..size_of::<QuadUniform>()].copy_from_slice(bytes_of(&background));
        for (i, r) in resolved.iter().enumerate() {
            let color = r
                .solid
                .unwrap_or([r.alpha, if r.from_fbo { 1.0 } else { 0.0 }, 0.0, 0.0]);
            let u = quad(r.matrix, r.uv, color);
            let at = (i + 1) * UNIFORM_STRIDE as usize;
            data[at..at + size_of::<QuadUniform>()].copy_from_slice(bytes_of(&u));
        }
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pm quads"),
            size: data.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&buffer, 0, &data);
        let quad_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pm quads"),
            layout: &self.pipelines.quad_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(size_of::<QuadUniform>() as u64),
                }),
            }],
        });
        let fill_bind = match &plan.background {
            Background::Gradient(g) => {
                let mut u = FillUniform::default();
                for (i, (rgb, pos)) in g.stops.iter().take(8).enumerate() {
                    u.stops[i] = [rgb[0] as f32, rgb[1] as f32, rgb[2] as f32, *pos as f32];
                }
                for i in g.stops.len().min(8)..8 {
                    u.stops[i] = [0.0, 0.0, 0.0, 1.0];
                }
                u.params = [
                    g.stops.len().min(8) as f32,
                    if g.radial { 2.0 } else { 1.0 },
                    g.angle_degrees as f32,
                    th as f32,
                ];
                u.view = [ox as f32, oy as f32, plan.width as f32, plan.height as f32];
                let b = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("pm fill"),
                    size: size_of::<FillUniform>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                self.queue.write_buffer(&b, 0, bytes_of(&u));
                Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("pm fill"),
                    layout: &self.pipelines.fill_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 2,
                        resource: b.as_entire_binding(),
                    }],
                }))
            }
            _ => None,
        };
        let texture_binds: Vec<Option<wgpu::BindGroup>> = resolved
            .iter()
            .map(|r| {
                r.view.as_ref().map(|view| {
                    self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("pm texture"),
                        layout: &self.pipelines.texture_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::Sampler(&self.pipelines.linear),
                            },
                        ],
                    })
                })
            })
            .collect();
        let clear = match &plan.background {
            Background::Transparent => wgpu::Color::TRANSPARENT,
            Background::Solid(c) => wgpu::Color {
                r: c[0],
                g: c[1],
                b: c[2],
                a: c[3],
            },
            Background::Gradient(_) => wgpu::Color::BLACK,
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("pm composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if let Some(fill) = &fill_bind {
            pass.set_pipeline(self.pipelines.background(format));
            pass.set_bind_group(0, &quad_bind, &[0]);
            pass.set_bind_group(1, fill, &[]);
            pass.draw(0..4, 0..1);
        }
        for (i, r) in resolved.iter().enumerate() {
            let offset = ((i + 1) as u64 * UNIFORM_STRIDE) as u32;
            if r.solid.is_some() {
                pass.set_pipeline(self.pipelines.solid(format));
                pass.set_bind_group(0, &quad_bind, &[offset]);
                pass.draw(0..4, 0..1);
            } else if let Some(bind) = &texture_binds[i] {
                pass.set_pipeline(self.pipelines.draw(format));
                pass.set_bind_group(0, &quad_bind, &[offset]);
                pass.set_bind_group(1, bind, &[]);
                pass.draw(0..4, 0..1);
            }
        }
        drop(pass);
        resolved
    }

    /// Records the depth and id surface of the top-level draws.
    pub fn render_ids(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        resolved: &[Resolved],
        region: Option<Region>,
        target: &Target,
    ) {
        let (ox, oy) = region.map_or((0.0, 0.0), |g| (f64::from(g.x), f64::from(g.y)));
        let (tw, th) = (target.width(), target.height());
        let mut data = vec![0u8; resolved.len().max(1) * UNIFORM_STRIDE as usize];
        for (i, r) in resolved.iter().enumerate() {
            let id = i as u32;
            let alpha = r.solid.map_or(r.alpha, |c| c[3]);
            let m = r.matrix;
            let u = QuadUniform {
                m: m3(&[m[0], m[1], m[2], m[3], m[4] - ox, m[5] - oy]),
                res: [tw as f32, th as f32, 0.0, 0.0],
                uv: r.uv.map(|v| v as f32),
                color: [
                    alpha as f32,
                    (id & 0x7ff) as f32,
                    (id >> 11) as f32,
                    if r.from_fbo { 1.0 } else { 0.0 },
                ],
            };
            let at = i * UNIFORM_STRIDE as usize;
            data[at..at + size_of::<QuadUniform>()].copy_from_slice(bytes_of(&u));
        }
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pm id quads"),
            size: data.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&buffer, 0, &data);
        let quad_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pm id quads"),
            layout: &self.pipelines.quad_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(size_of::<QuadUniform>() as u64),
                }),
            }],
        });
        let binds: Vec<(Option<wgpu::BindGroup>, Option<wgpu::BindGroup>)> = resolved
            .iter()
            .map(|r| {
                let tex = r.view.as_ref().map(|view| {
                    self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("pm id texture"),
                        layout: &self.pipelines.texture_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::Sampler(&self.pipelines.linear),
                            },
                        ],
                    })
                });
                let depth = r.depth.as_ref().map(|view| {
                    self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("pm id depth"),
                        layout: &self.pipelines.depth_layout,
                        entries: &[wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(view),
                        }],
                    })
                });
                (tex, depth)
            })
            .collect();
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("pm depth and id"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: f64::INFINITY,
                        g: 0.0,
                        b: 0.0,
                        a: 0.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        for (i, r) in resolved.iter().enumerate() {
            let offset = (i as u64 * UNIFORM_STRIDE) as u32;
            match (&binds[i], r.solid) {
                (_, Some(_)) => {
                    pass.set_pipeline(&self.pipelines.id_solid);
                    pass.set_bind_group(0, &quad_bind, &[offset]);
                    pass.draw(0..4, 0..1);
                }
                ((Some(tex), Some(depth)), None) => {
                    pass.set_pipeline(&self.pipelines.id_3d);
                    pass.set_bind_group(0, &quad_bind, &[offset]);
                    pass.set_bind_group(1, tex, &[]);
                    pass.set_bind_group(2, depth, &[]);
                    pass.draw(0..4, 0..1);
                }
                ((Some(tex), None), None) => {
                    pass.set_pipeline(&self.pipelines.id_draw);
                    pass.set_bind_group(0, &quad_bind, &[offset]);
                    pass.set_bind_group(1, tex, &[]);
                    pass.draw(0..4, 0..1);
                }
                _ => {}
            }
        }
    }

    /// Copies `source` (GL row order) into `target` top-down, exactly.
    pub fn flip(&self, encoder: &mut wgpu::CommandEncoder, source: &Target, target: &Target) {
        let (w, h) = (target.width(), target.height());
        let u = QuadUniform {
            m: m3(&[f64::from(w), 0.0, 0.0, f64::from(h), 0.0, 0.0]),
            res: [w as f32, h as f32, 0.0, 0.0],
            uv: [0.0, 0.0, 1.0, 1.0],
            color: [0.0; 4],
        };
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pm flip"),
            size: UNIFORM_STRIDE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&buffer, 0, bytes_of(&u));
        let quad = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pm flip"),
            layout: &self.pipelines.quad_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(size_of::<QuadUniform>() as u64),
                }),
            }],
        });
        let tex = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pm flip source"),
            layout: &self.pipelines.texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&source.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.pipelines.linear),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("pm flip"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.view,
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
        pass.set_pipeline(self.pipelines.flip(target.format()));
        pass.set_bind_group(0, &quad, &[0]);
        pass.set_bind_group(1, &tex, &[]);
        pass.draw(0..4, 0..1);
    }
}
