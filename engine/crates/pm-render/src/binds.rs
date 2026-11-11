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

use std::collections::{HashMap, HashSet};

pub(crate) struct BindCache {
    textures: HashMap<wgpu::TextureView, (wgpu::BindGroup, u64)>,
    quads: HashMap<(wgpu::Buffer, u64), (wgpu::BindGroup, u64)>,
    resamples: HashMap<(wgpu::TextureView, wgpu::TextureView), (wgpu::BindGroup, u64)>,
    /// Compatibility rasters' textures on this renderer's device
    /// (`crate::compat::upload_cached`).
    pub compat: crate::compat::Uploads,
    frame: u64,
    budget: u64,
}

impl Default for BindCache {
    fn default() -> Self {
        Self {
            textures: HashMap::new(),
            quads: HashMap::new(),
            resamples: HashMap::new(),
            compat: Default::default(),
            frame: 0,
            budget: 32 << 20,
        }
    }
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
        self.trim();
    }

    pub fn clear(&mut self) {
        self.clear_bindings();
        self.compat.clear();
    }

    fn clear_bindings(&mut self) {
        self.textures.clear();
        self.quads.clear();
        self.resamples.clear();
    }

    pub fn set_budget(&mut self, bytes: u64) {
        self.budget = bytes;
        self.trim();
    }

    fn trim(&mut self) {
        // The last frame can contain hundreds of transient textures. Keeping
        // its bindings used to bypass every source cache's eviction budget.
        // Submitted commands retain their own references until completion.
        if self.retained_bytes() > self.budget {
            self.clear_bindings();
        }
    }

    /// Unique texture and buffer backings referenced by these bindings.
    /// Some are also owned by other caches; this is not additive accounting.
    pub fn retained_bytes(&self) -> u64 {
        let textures: HashSet<_> = self
            .textures
            .keys()
            .chain(self.resamples.keys().flat_map(|(a, b)| [a, b]))
            .map(wgpu::TextureView::texture)
            .collect();
        let buffers: HashSet<_> = self.quads.keys().map(|(buffer, _)| buffer).collect();
        textures
            .into_iter()
            .map(|texture| {
                let size = texture.size();
                let (bw, bh) = texture.format().block_dimensions();
                (0..texture.mip_level_count())
                    .map(|level| {
                        let depth = if texture.dimension() == wgpu::TextureDimension::D3 {
                            (size.depth_or_array_layers >> level).max(1)
                        } else {
                            size.depth_or_array_layers
                        };
                        u64::from((size.width >> level).max(1).div_ceil(bw))
                            * u64::from((size.height >> level).max(1).div_ceil(bh))
                            * u64::from(depth)
                            * u64::from(texture.format().block_copy_size(None).unwrap_or(16))
                    })
                    .sum::<u64>()
            })
            .sum::<u64>()
            + buffers.into_iter().map(wgpu::Buffer::size).sum::<u64>()
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
        tail: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        // A recycled mip tail has a new source. Retire groups that still
        // hold the former source view; the pyramid pool only reassigns a
        // tail after the previous encoder has been submitted.
        if tail != view {
            self.resamples.retain(|(old_source, old_tail), _| {
                old_tail.texture() != tail.texture() || old_source == view
            });
        }
        let key = (view.clone(), tail.clone());
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
                        resource: wgpu::BindingResource::TextureView(tail),
                    },
                ],
            });
            (group, frame)
        });
        entry.1 = frame;
        entry.0.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bindings_cannot_pin_evicted_textures_past_their_byte_grant() {
        let gpu = pm_gpu::Gpu::new("binding-memory-budget").unwrap();
        let device = &gpu.device;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
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
        let sampler = device.create_sampler(&Default::default());
        let target = pm_gpu::Target::new(
            device,
            "evictable texture",
            512,
            512,
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let another_view = target.texture.create_view(&Default::default());
        let mut cache = BindCache::default();
        cache.set_budget(4 << 20);
        cache.begin_frame();
        cache.texture(device, &layout, &sampler, &target.view);
        cache.texture(device, &layout, &sampler, &another_view);
        cache.end_frame();
        assert_eq!(
            cache.retained_bytes(),
            2 << 20,
            "two views share one backing"
        );
        cache.set_budget(1 << 20);
        assert_eq!(
            cache.retained_bytes(),
            0,
            "the evicted texture must be releasable"
        );
        assert!(cache.textures.is_empty());
    }
}
