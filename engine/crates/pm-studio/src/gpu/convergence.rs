//! A four-byte global convergence probe; pixel stopping stays in the tracer.
use super::{Renderer,storage,uniform};
pub(super) struct Probe {
    layout:wgpu::BindGroupLayout,
    pipeline:wgpu::ComputePipeline,
    dims:wgpu::Buffer,
    result:wgpu::Buffer,
}
impl Renderer {
    /// Idle calibration uses the identical stopping probe between sample chunks.
    pub fn current_max_samples(&mut self)->u32 { self.max_sample_count() }
    pub(super) fn max_sample_count(&mut self)->u32 {
        let probe=self.convergence_probe.get_or_insert_with(|| {
            let layout=self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor{label:Some("Studio sample maximum"),entries:&[uniform(0),storage(1,true),storage(2,false)]});
            let module=self.device.create_shader_module(wgpu::ShaderModuleDescriptor{label:Some("sample-max.wgsl"),source:wgpu::ShaderSource::Wgsl(include_str!("../shaders/sample-max.wgsl").into())});
            let pipeline=self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor{label:Some("Studio sample maximum"),layout:Some(&self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor{label:None,bind_group_layouts:&[Some(&layout)],immediate_size:0})),module:&module,entry_point:Some("main"),compilation_options:Default::default(),cache:None});
            let dims=self.device.create_buffer(&wgpu::BufferDescriptor{label:None,size:16,usage:wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
            let result=self.device.create_buffer(&wgpu::BufferDescriptor{label:None,size:4,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
            Probe{layout,pipeline,dims,result}
        });
        self.queue.write_buffer(&probe.dims,0,bytemuck::cast_slice(&[self.width,self.height,0,0]));
        let group=self.device.create_bind_group(&wgpu::BindGroupDescriptor{label:None,layout:&probe.layout,entries:&[
            wgpu::BindGroupEntry{binding:0,resource:probe.dims.as_entire_binding()},
            wgpu::BindGroupEntry{binding:1,resource:self.accum.as_entire_binding()},
            wgpu::BindGroupEntry{binding:2,resource:probe.result.as_entire_binding()},
        ]});
        let mut enc=self.device.create_command_encoder(&Default::default());enc.clear_buffer(&probe.result,0,None);
        {let mut pass=enc.begin_compute_pass(&Default::default());pass.set_pipeline(&probe.pipeline);pass.set_bind_group(0,&group,&[]);pass.dispatch_workgroups(self.width.div_ceil(16),self.height.div_ceil(16),1);}
        self.queue.submit([enc.finish()]);let result=probe.result.clone();
        self.read_f32(&result,4)[0].to_bits()
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn reduction_matches_actual_film_counts_after_resize_and_refinement() {
        let scene=crate::scene::preset("hero","starlight",0.).unwrap();
        let mut r=super::Renderer::new(33,17).unwrap();r.set_scene(&scene,&scene.prepare());
        for (w,h,spp,reset) in [(33,17,4,true),(33,17,8,false),(19,31,16,true)] {
            r.resize(w,h).unwrap();r.trace(spp,4,reset,64);
            let expected=r.sample_counts().into_iter().fold(0.,f32::max) as u32;
            assert_eq!(r.max_sample_count(),expected);
        }
    }
}
