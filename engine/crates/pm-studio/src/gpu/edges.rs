//! Conservative feature classification after the first completed sample batch.
use super::{Renderer, storage, uniform};
pub(super) struct Classifier {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    params: wgpu::Buffer,
}
impl Renderer {
    pub(super) fn classify_edges(&mut self) {
        let device = &self.device;
        let classifier = self.edge_classifier.get_or_insert_with(|| {
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Studio adaptive edges"),
                entries: &[
                    uniform(0),
                    storage(1, true),
                    storage(2, true),
                    storage(3, false),
                ],
            });
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("adaptive-edges.wgsl"),
                source: wgpu::ShaderSource::Wgsl(
                    include_str!("../shaders/adaptive-edges.wgsl").into(),
                ),
            });
            let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("Studio adaptive edges"),
                layout: Some(
                    &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: None,
                        bind_group_layouts: &[Some(&layout)],
                        immediate_size: 0,
                    }),
                ),
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });
            Classifier {
                layout,
                pipeline,
                params: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("edge dimensions"),
                    size: 16,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
            }
        });
        self.queue.write_buffer(
            &classifier.params,
            0,
            bytemuck::cast_slice(&[self.width, self.height, 0, 0]),
        );
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &classifier.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: classifier.params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.gbuf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.accum.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.ebuf.as_entire_binding(),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&classifier.pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(self.width.div_ceil(8), self.height.div_ceil(8), 1);
        }
        self.queue.submit([encoder.finish()]);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn stochastic_first_batch_variation_is_not_a_deterministic_detail_edge() {
        let (w,h)=(9u32,7u32);let pixels=(w*h) as usize;
        let mut r=super::Renderer::new(w,h).unwrap();
        let guide=vec![[0f32,1.,1.,1.];pixels];r.queue.write_buffer(&r.gbuf,0,bytemuck::cast_slice(&guide));
        for noisy in [true,false] {
            let mut a=vec![[0f32;4];pixels];let mut e=vec![[0f32;4];pixels*2];
            for i in 0..pixels {
                let mean=if noisy {if i%2==0 {0.2}else{1.}}else{if (i%w as usize)<4 {0.2}else{1.}};
                a[i]=[4.*mean,4.*mean,4.*mean,4.];
                e[pixels+i]=[4.*(mean*mean+if noisy{1.}else{0.});4];e[pixels+i][3]=0.;
            }
            r.queue.write_buffer(&r.accum,0,bytemuck::cast_slice(&a));r.queue.write_buffer(&r.ebuf,0,bytemuck::cast_slice(&e));
            r.classify_edges();let flags=r.read_f32(&r.ebuf,(pixels*32) as u64);
            let count=flags[pixels*4..].chunks_exact(4).filter(|p|p[3]>0.).count();
            if noisy {assert_eq!(count,0,"noise must not force the fourfold edge floor");}else{assert!(count>0,"deterministic detail must keep extra samples");}
        }
    }
}
