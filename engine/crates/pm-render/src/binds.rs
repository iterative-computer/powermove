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
    /// Compatibility rasters' textures on this renderer's device, kept while
    /// frames draw them ([`crate::compat::Uploads`]).
    pub compat: crate::compat::Uploads,
    frame: u64,
    budget: u64,
    /// [`Self::retained_bytes`] as last measured, while no binding changed
    /// since (`None` once one did). Measuring builds two sets over every
    /// group; the engine reads it once a loop pass and every frame's end
    /// trims by it.
    retained: Option<u64>,
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
            retained: Some(0),
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
        let before = self.len();
        self.textures.retain(|_, (_, used)| *used == frame);
        self.quads.retain(|_, (_, used)| *used == frame);
        self.resamples.retain(|_, (_, used)| *used == frame);
        if self.len() != before {
            self.retained = None;
        }
        self.compat.end_frame(frame);
        self.trim();
    }

    /// The texture of a compatibility raster this frame draws.
    pub fn compat_view(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        raster: &std::sync::Arc<crate::compat::CompatRaster>,
        precision: pm_protocol::color::Precision,
    ) -> wgpu::TextureView {
        self.compat
            .view(device, queue, raster, precision, self.frame)
    }

    pub fn clear(&mut self) {
        self.clear_bindings();
        self.compat.clear();
    }

    fn clear_bindings(&mut self) {
        self.textures.clear();
        self.quads.clear();
        self.resamples.clear();
        self.retained = Some(0);
    }

    /// Groups held, in every map.
    fn len(&self) -> usize {
        self.textures.len() + self.quads.len() + self.resamples.len()
    }

    pub fn set_budget(&mut self, bytes: u64) {
        self.budget = bytes;
        self.trim();
    }

    fn trim(&mut self) {
        // The last frame can contain hundreds of transient textures. Keeping
        // its bindings used to bypass every source cache's eviction budget.
        // Submitted commands retain their own references until completion.
        let retained = self.retained_bytes();
        self.retained = Some(retained);
        if retained > self.budget {
            self.clear_bindings();
        }
    }

    /// Unique texture and buffer backings referenced by these bindings.
    /// Some are also owned by other caches; this is not additive accounting.
    /// Measured again only after a binding was added or dropped.
    pub fn retained_bytes(&self) -> u64 {
        self.retained.unwrap_or_else(|| self.measure_retained())
    }

    /// [`Self::retained_bytes`], measured now.
    fn measure_retained(&self) -> u64 {
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
        let retained = &mut self.retained;
        let entry = self.textures.entry(view.clone()).or_insert_with(|| {
            *retained = None;
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
        let retained = &mut self.retained;
        let entry = self.quads.entry(key).or_insert_with(|| {
            *retained = None;
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
            let before = self.resamples.len();
            self.resamples.retain(|(old_source, old_tail), _| {
                old_tail.texture() != tail.texture() || old_source == view
            });
            if self.resamples.len() != before {
                self.retained = None;
            }
        }
        let key = (view.clone(), tail.clone());
        let frame = self.frame;
        let retained = &mut self.retained;
        let entry = self.resamples.entry(key).or_insert_with(|| {
            *retained = None;
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

    #[test]
    fn cached_retained_bytes_equal_a_fresh_measure() {
        let gpu = pm_gpu::Gpu::new("binding-retained-cache").unwrap();
        let device = &gpu.device;
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sampler_entry = wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[texture_entry(0), sampler_entry],
        });
        let resample_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[texture_entry(0), sampler_entry, texture_entry(2)],
        });
        let quads_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let sampler = device.create_sampler(&Default::default());
        let targets: Vec<_> = [(64, 64), (128, 32), (256, 256), (17, 9)]
            .into_iter()
            .map(|(w, h)| {
                pm_gpu::Target::new(
                    device,
                    "retained bytes",
                    w,
                    h,
                    wgpu::TextureFormat::Rgba16Float,
                    wgpu::TextureUsages::TEXTURE_BINDING,
                )
            })
            .collect();
        let buffers: Vec<_> = [256u64, 4096]
            .into_iter()
            .map(|size| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size,
                    usage: wgpu::BufferUsages::UNIFORM,
                    mapped_at_creation: false,
                })
            })
            .collect();
        let mut cache = BindCache::default();
        let check = |cache: &BindCache, step: &str| {
            assert_eq!(
                cache.retained_bytes(),
                cache.measure_retained(),
                "after {step}"
            );
        };
        check(&cache, "nothing");
        // A deterministic mix of inserts, repeats and drops across frames.
        let mut seed = 0x9e37_79b9_u32;
        let mut next = |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as usize % n
        };
        for frame in 0..5 {
            cache.begin_frame();
            for step in 0..6 {
                match next(3) {
                    0 => {
                        cache.texture(device, &texture_layout, &sampler, &targets[next(4)].view);
                    }
                    1 => {
                        let buffer = &buffers[next(2)];
                        cache.quads(device, &quads_layout, buffer, 256);
                    }
                    _ => {
                        let (a, b) = (next(4), next(4));
                        cache.resample(
                            device,
                            &resample_layout,
                            &sampler,
                            &targets[a].view,
                            &targets[b].view,
                        );
                    }
                }
                check(&cache, &format!("frame {frame} step {step}"));
            }
            cache.end_frame();
            check(&cache, &format!("the end of frame {frame}"));
        }
        // A frame that uses nothing drops every group.
        cache.begin_frame();
        cache.end_frame();
        check(&cache, "an empty frame");
        assert_eq!(cache.retained_bytes(), 0);
        cache.begin_frame();
        cache.texture(device, &texture_layout, &sampler, &targets[2].view);
        cache.quads(device, &quads_layout, &buffers[1], 256);
        cache.end_frame();
        check(&cache, "a refill");
        assert!(cache.retained_bytes() > 0);
        // A budget below what is held drops it all; one above keeps it.
        cache.set_budget(u64::MAX);
        check(&cache, "a larger budget");
        assert!(cache.retained_bytes() > 0);
        cache.set_budget(1);
        check(&cache, "a smaller budget");
        assert_eq!(cache.retained_bytes(), 0);
        cache.set_budget(32 << 20);
        cache.begin_frame();
        cache.texture(device, &texture_layout, &sampler, &targets[0].view);
        check(&cache, "an insert in a frame");
        cache.clear();
        check(&cache, "a clear");
        assert_eq!(cache.retained_bytes(), 0);
    }
}
