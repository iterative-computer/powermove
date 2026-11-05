//! The source's coverage/variance-aware Final filter. All passes remain on GPU.
use super::*;

pub(super) struct Guided {
    layout: wgpu::BindGroupLayout,
    demod: wgpu::ComputePipeline,
    step: wgpu::ComputePipeline,
    finish: wgpu::ComputePipeline,
    side: [wgpu::Buffer; 2],
    params: Vec<wgpu::Buffer>,
    pub ones: wgpu::Buffer,
    pub zero: wgpu::Buffer,
}

impl Guided {
    pub fn new(device: &wgpu::Device, w: u32, h: u32) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("studio guided denoise"),
            entries: &[
                uniform(0),
                storage(2, true),
                storage(3, true),
                storage(4, false),
                storage(5, true),
                storage(6, false),
                storage(7, true),
                storage(8, true),
                storage(9, true),
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("studio guided denoise"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/denoise.wgsl").into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("studio guided denoise"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let make = |name| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(name),
                layout: Some(&pl),
                module: &module,
                entry_point: Some(name),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let buf = |label| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: u64::from(w) * u64::from(h) * 16,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            })
        };
        let mut ones = vec![0f32; w as usize * h as usize * 4];
        for p in ones.chunks_exact_mut(4) {
            p[3] = 1.;
        }
        Self {
            demod: make("demod"),
            step: make("atrous_step"),
            finish: make("finish"),
            layout,
            side: [buf("studio denoise side a"), buf("studio denoise side b")],
            params: (0..7)
                .map(|_| {
                    device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("studio denoise params"),
                        size: 32,
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    })
                })
                .collect(),
            ones: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("studio denoise ones"),
                contents: bytemuck::cast_slice(&ones),
                usage: wgpu::BufferUsages::STORAGE,
            }),
            zero: buf("studio denoise zero emission"),
        }
    }

    /// Returns (display-buffer index, filtered coverage). Parameters are retained
    /// separately for each dispatch; write_buffer never overwrites an earlier pass.
    pub fn encode<'a>(
        &'a self,
        r: &Renderer,
        enc: &mut wgpu::CommandEncoder,
    ) -> (usize, &'a wgpu::Buffer) {
        let mut slot = 0;
        let mut run = |pipe: &wgpu::ComputePipeline,
                       src: usize,
                       dst: usize,
                       vin: usize,
                       vout: usize,
                       step: u32| {
            let mut bytes = Vec::from(bytemuck::cast_slice(&[r.width, r.height, step, 0u32]));
            bytes.extend_from_slice(bytemuck::cast_slice(&[4f32, r.base.cam_pos[3], 0., 0.]));
            r.queue.write_buffer(&self.params[slot], 0, &bytes);
            let group = r.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("studio guided denoise"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.params[slot].as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: r.aov.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: r.display[src].as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: r.display[dst].as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: self.side[vin].as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: self.side[vout].as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 7,
                        resource: r.accum.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 8,
                        resource: r.bgsum.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 9,
                        resource: r.ebuf.as_entire_binding(),
                    },
                ],
            });
            slot += 1;
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(r.width.div_ceil(8), r.height.div_ceil(8), 1);
        };
        run(&self.demod, 0, 1, 1, 0, 1);
        let (mut cur, mut v) = (1, 0);
        for i in 0..5 {
            run(&self.step, cur, 1 - cur, v, 1 - v, 1 << i);
            cur = 1 - cur;
            v = 1 - v;
        }
        run(&self.finish, cur, 1 - cur, v, 1 - v, 1);
        (1 - cur, &self.side[1 - v])
    }
}
