//! First-hit queries share the same ray traversal and shading as rendering.
use super::{Renderer, rt_source, storage, trace_source, uniform};

pub struct SurfacePick {
    pub layer: u32,
    pub instance: u32,
    pub point: [f32; 3],
    pub normal: [f32; 3],
}
pub(super) struct Query {
    ray_queries: bool,
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    empty: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    output: wgpu::Buffer,
}
impl Query {
    fn new(r: &Renderer) -> Self {
        let ray_queries = r.uses_hardware_rt();
        let device = &r.device;
        let empty = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[],
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Studio pick"),
            entries: &[storage(0, false), uniform(1)],
        });
        let source = if ray_queries {
            rt_source()
        } else {
            trace_source()
        };
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Studio pick"),
            source: wgpu::ShaderSource::Wgsl(
                format!("{source}\n{}", include_str!("../shaders/query.wgsl")).into(),
            ),
        });
        let ray = if ray_queries {
            &r.rt.as_ref().unwrap().layout
        } else {
            &empty
        };
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Studio pick"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: None,
                    bind_group_layouts: &[Some(&r.trace_layout), Some(ray), Some(&layout)],
                    immediate_size: 0,
                }),
            ),
            module: &module,
            entry_point: Some("pick_surface"),
            compilation_options: Default::default(),
            cache: None,
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pick pixel"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pick result"),
            size: 48,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        Self {
            ray_queries,
            pipeline,
            layout,
            empty,
            params,
            output,
        }
    }
}
impl Renderer {
    /// Normalized top-left coordinates. Uses a pinhole even when rendering DOF.
    pub fn pick_surface(&mut self, x: f32, y: f32) -> Option<SurfacePick> {
        if self
            .query
            .as_ref()
            .is_none_or(|q| q.ray_queries != self.uses_hardware_rt())
        {
            self.query = Some(Query::new(self));
        }
        let group = self.trace_group().clone();
        let query = self.query.as_ref().unwrap();
        let mut p = self.base;
        p.cam_pos[3] = 0.;
        p.dims = [self.width, self.height, 0, 1];
        p.opts[0] = 1;
        p.opts[2] = self.instance_count;
        self.queue
            .write_buffer(&self.params, 0, bytemuck::bytes_of(&p));
        self.queue
            .write_buffer(&query.params, 0, bytemuck::cast_slice(&[x, y, 0., 0.]));
        let out = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pick result"),
            layout: &query.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: query.output.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: query.params.as_entire_binding(),
                },
            ],
        });
        let empty = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &query.empty,
            entries: &[],
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_pipeline(&query.pipeline);
            pass.set_bind_group(0, &group, &[]);
            if query.ray_queries {
                pass.set_bind_group(1, self.rt.as_ref().unwrap().group.as_ref(), &[]);
            } else {
                pass.set_bind_group(1, &empty, &[]);
            }
            pass.set_bind_group(2, &out, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        self.queue.submit([enc.finish()]);
        let values = self.read_f32(&query.output, 48);
        if values[0] < 0. || values.iter().any(|v| !v.is_finite()) {
            return None;
        }
        Some(SurfacePick {
            layer: values[0] as u32,
            instance: values[1] as u32,
            point: [values[4], values[5], values[6]],
            normal: [values[8], values[9], values[10]],
        })
    }
}
