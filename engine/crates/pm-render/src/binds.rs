//! Bind groups kept across frames (docs/engine/perfbench.md).
//!
//! A composition pass bound its quads' uniform buffer and every textured
//! draw's view through a bind group made for that frame: about a hundred
//! bind group creations a frame on the 500-layer synthetic project, where
//! today's WebGL viewer binds the same textures and buffers again without
//! creating anything. The buffers come from the frame-to-frame uniform pool
//! and the views from long-lived targets and caches (the target pool, the
//! media and vector caches), so the same groups are made frame after frame.
//!
//! A group is a pure function of its layout and resources, and wgpu views
//! and buffers compare by identity, so a group made for a view or buffer is
//! the group any later frame would make for it. A group keeps its resources
//! alive, so entries a frame did not use are dropped when it ends, and
//! [`BindCache::clear`] drops all of them (the renderer's trim).

use std::collections::HashMap;

#[derive(Default)]
pub(crate) struct BindCache {
    textures: HashMap<wgpu::TextureView, (wgpu::BindGroup, u64)>,
    quads: HashMap<(wgpu::Buffer, u64), (wgpu::BindGroup, u64)>,
    resamples: HashMap<(wgpu::TextureView, wgpu::Buffer), (wgpu::BindGroup, u64)>,
    frame: u64,
}

impl BindCache {
    /// A frame starts.
    pub fn begin_frame(&mut self) {
        self.frame += 1;
    }

    /// A frame ended: groups it did not use go (and with them the
    /// resources only they kept alive).
    pub fn end_frame(&mut self) {
        let frame = self.frame;
        self.textures.retain(|_, (_, used)| *used == frame);
        self.quads.retain(|_, (_, used)| *used == frame);
        self.resamples.retain(|_, (_, used)| *used == frame);
    }

    pub fn clear(&mut self) {
        self.textures.clear();
        self.quads.clear();
        self.resamples.clear();
    }

    /// The texture group (`binding 0`: `view`, `binding 1`: `sampler`) of
    /// `layout`. The renderer binds one layout and one sampler here.
    pub fn texture(
        &mut self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        view: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        pm_perf::cache(
            pm_perf::Counter::BindCacheHits,
            pm_perf::Counter::BindCacheMisses,
            self.textures.contains_key(view),
        );
        let frame = self.frame;
        let entry = self.textures.entry(view.clone()).or_insert_with(|| {
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("pm texture"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                ],
            });
            (group, frame)
        });
        entry.1 = frame;
        entry.0.clone()
    }

    /// The quads' group (`binding 0`: `size` bytes of `buffer` at a dynamic
    /// offset) of `layout`.
    pub fn quads(
        &mut self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        buffer: &wgpu::Buffer,
        size: u64,
    ) -> wgpu::BindGroup {
        let key = (buffer.clone(), size);
        pm_perf::cache(
            pm_perf::Counter::BindCacheHits,
            pm_perf::Counter::BindCacheMisses,
            self.quads.contains_key(&key),
        );
        let frame = self.frame;
        let entry = self.quads.entry(key).or_insert_with(|| {
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("pm quads"),
                layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer,
                        offset: 0,
                        size: wgpu::BufferSize::new(size),
                    }),
                }],
            });
            (group, frame)
        });
        entry.1 = frame;
        entry.0.clone()
    }
    pub fn resample(
        &mut self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        view: &wgpu::TextureView,
        buffer: &wgpu::Buffer,
        size: u64,
    ) -> wgpu::BindGroup {
        let key = (view.clone(), buffer.clone());
        let frame = self.frame;
        let entry = self.resamples.entry(key).or_insert_with(|| {
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("pm sampled source"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer,
                            offset: 0,
                            size: wgpu::BufferSize::new(size),
                        }),
                    },
                ],
            });
            (group, frame)
        });
        entry.1 = frame;
        entry.0.clone()
    }
}
