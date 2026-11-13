//! Optional Live direct-light reservoirs. Separate from the shared portable
//! tracer; Refine/Final keep its MIS estimator and never reuse reservoirs.
use super::{Renderer, storage, uniform};
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    dims: [u32; 4],
    camera: [[f32; 4]; 4],
}

pub(super) struct Resampling {
    pub layout: wgpu::BindGroupLayout,
    pub params: wgpu::Buffer,
    pub buffers: [wgpu::Buffer; 2],
    pub index: usize,
}
impl Resampling {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let _ = (width, height);
        let rw = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
        let make = |name| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(name),
                size: 64,
                usage: rw,
                mapped_at_creation: false,
            })
        };
        Self {
            layout: device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("studio reservoirs"),
                entries: &[uniform(0), storage(1, true), storage(2, false)],
            }),
            params: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("reservoir params"),
                size: std::mem::size_of::<Params>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            buffers: [make("reservoir a"), make("reservoir b")],
            index: 0,
        }
    }
    pub fn ensure(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let size = u64::from(width) * u64::from(height) * 64;
        if self.buffers[0].size() >= size {
            return;
        }
        self.buffers = std::array::from_fn(|_| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("live reservoirs"),
                size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
        self.index = 0;
    }
    pub fn group(&self, device: &wgpu::Device) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("reservoirs"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.buffers[self.index].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.buffers[1 - self.index].as_entire_binding(),
                },
            ],
        })
    }
}
pub(super) fn source(base: &str) -> String {
    let base = base.replacen("fn sample_lights(", "fn sample_lights_uniform(", 1);
    let base=base.replace("  seed = pcg(idx ^ pcg(P.dims.z * 9781u + 1u));", "  seed = pcg(idx ^ pcg(P.dims.z * 9781u + 1u));\n  reservoir_pixel=idx;reservoir_used=false;reservoirs_out[idx]=Reservoir(vec4u(0u),vec4f(0.),vec4f(0.),vec4f(0.));");
    format!("{base}\n{}", include_str!("../shaders/resampled.wgsl"))
}
impl Renderer {
    pub(super) fn reservoir_group(&mut self) -> wgpu::BindGroup {
        let r = self.resampling.as_mut().unwrap();
        r.ensure(&self.device, self.width, self.height);
        let p = Params {
            dims: [
                self.width,
                self.height,
                u32::from(self.history_valid),
                self.base.opts[3] & 1,
            ],
            camera: self.previous_camera,
        };
        self.queue
            .write_buffer(&r.params, 0, bytemuck::bytes_of(&p));
        r.group(&self.device)
    }
}
