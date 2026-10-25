//! Render targets and their pool (plan section 8, "Working encoding and
//! precision").
//!
//! Working storage is `rgba8unorm` at 8-bit, `rgba16float` by default,
//! and `rgba32float` at 32-bit
//! precision. Targets are pooled by size, format, and usage: a frame's
//! intermediates are taken and returned as the graph walks, so a
//! steady-state frame allocates nothing. The pool counts its bytes for the
//! GPU lease (contract 8).

use pm_protocol::color::Precision;

/// The working format for a precision.
pub const fn working_format(precision: Precision) -> wgpu::TextureFormat {
    match precision {
        Precision::Unorm8 => wgpu::TextureFormat::Rgba8Unorm,
        Precision::Float16 => wgpu::TextureFormat::Rgba16Float,
        Precision::Float32 => wgpu::TextureFormat::Rgba32Float,
    }
}

/// Usages every working target gets: drawn into, sampled, copied out
/// (readback, presentation), and written by compute passes.
pub fn working_usage() -> wgpu::TextureUsages {
    wgpu::TextureUsages::RENDER_ATTACHMENT
        | wgpu::TextureUsages::TEXTURE_BINDING
        | wgpu::TextureUsages::COPY_SRC
        | wgpu::TextureUsages::COPY_DST
        | wgpu::TextureUsages::STORAGE_BINDING
}

/// A texture with its default view.
#[derive(Debug, Clone)]
pub struct Target {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
}

impl Target {
    pub fn new(
        device: &wgpu::Device,
        label: &str,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        usage: wgpu::TextureUsages,
    ) -> Target {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Target { texture, view }
    }

    /// A target with its full mip chain in the same texture; `view` reads
    /// and writes level 0 only (the levels below are filled separately).
    pub fn with_mips(
        device: &wgpu::Device,
        label: &str,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        usage: wgpu::TextureUsages,
    ) -> Target {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: width.max(height).max(1).ilog2() + 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            base_mip_level: 0,
            mip_level_count: Some(1),
            ..Default::default()
        });
        Target { texture, view }
    }

    /// A target over an existing texture (the presenter's working texture,
    /// an export buffer), with its default view.
    pub fn wrap(texture: wgpu::Texture) -> Target {
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Target { texture, view }
    }

    pub fn width(&self) -> u32 {
        self.texture.width()
    }

    pub fn height(&self) -> u32 {
        self.texture.height()
    }

    pub fn format(&self) -> wgpu::TextureFormat {
        self.texture.format()
    }

    /// Bytes of texel storage, every mip level included.
    pub fn bytes(&self) -> u64 {
        let bpp = u64::from(self.format().block_copy_size(None).unwrap_or(16));
        (0..self.texture.mip_level_count())
            .map(|l| {
                u64::from((self.width() >> l).max(1)) * u64::from((self.height() >> l).max(1))
            })
            .sum::<u64>()
            * bpp
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct Key {
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
}

/// Reusable targets.
#[derive(Default)]
pub struct TargetPool {
    free: Vec<(Key, Target)>,
    /// Bytes held by targets in the pool and taken from it.
    live_bytes: u64,
    created: u64,
}

impl TargetPool {
    pub fn new() -> TargetPool {
        TargetPool::default()
    }

    /// A target of this size and format: a free one when the pool has it.
    pub fn take(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        usage: wgpu::TextureUsages,
    ) -> Target {
        let key = Key {
            width,
            height,
            format,
            usage,
        };
        if let Some(i) = self.free.iter().position(|(k, _)| *k == key) {
            pm_perf::bump(pm_perf::Counter::TargetPoolHits);
            return self.free.swap_remove(i).1;
        }
        pm_perf::bump(pm_perf::Counter::TargetPoolMisses);
        let target = Target::new(device, "pm working target", width, height, format, usage);
        self.live_bytes += target.bytes();
        self.created += 1;
        target
    }

    /// A working target at `precision`.
    pub fn take_working(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        precision: Precision,
    ) -> Target {
        self.take(
            device,
            width,
            height,
            working_format(precision),
            working_usage(),
        )
    }

    /// Returns a target for reuse. The caller must not use it afterwards
    /// except through commands already recorded (GPU order keeps those
    /// correct).
    pub fn give(&mut self, target: Target) {
        let key = Key {
            width: target.width(),
            height: target.height(),
            format: target.format(),
            usage: target.texture.usage(),
        };
        self.free.push((key, target));
    }

    /// Discards a target owned by this pool without calling Texture::destroy.
    /// Recorded command buffers or retained views may still refer to it;
    /// dropping this handle lets wgpu release its storage when those uses end.
    pub fn release(&mut self, target: Target) {
        self.live_bytes = self
            .live_bytes
            .checked_sub(target.bytes())
            .expect("released target was not accounted for by this pool");
        drop(target);
    }

    /// Frees idle targets until the pool holds at most `bytes`.
    pub fn trim(&mut self, bytes: u64) {
        while self.live_bytes > bytes {
            let Some((_, t)) = self.free.pop() else { break };
            self.live_bytes = self.live_bytes.saturating_sub(t.bytes());
            t.texture.destroy();
        }
    }

    /// Bytes of every target this pool created and has not freed.
    pub fn bytes(&self) -> u64 {
        self.live_bytes
    }

    /// Idle targets.
    pub fn idle(&self) -> usize {
        self.free.len()
    }

    /// Targets created so far (a steady state stops creating).
    pub fn created(&self) -> u64 {
        self.created
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn release_accounts_for_a_target_still_used_by_recorded_commands() {
        let gpu = crate::device::Gpu::new("target pool release test").unwrap();
        let mut pool = TargetPool::new();
        let target = pool.take(
            &gpu.device,
            2,
            2,
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        assert_eq!(pool.bytes(), 16);
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("released target readback"),
            size: 512,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 1.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }
        encoder.copy_texture_to_buffer(
            target.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(2),
                },
            },
            wgpu::Extent3d {
                width: 2,
                height: 2,
                depth_or_array_layers: 1,
            },
        );
        pool.release(target);
        assert_eq!(pool.bytes(), 0);
        let submission = gpu.queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        gpu.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .unwrap();
        rx.recv().unwrap().unwrap();
        let data = slice.get_mapped_range().unwrap();
        assert_eq!(&data[..8], &[0, 255, 0, 255, 0, 255, 0, 255]);
        drop(data);
        buffer.unmap();
    }
}
