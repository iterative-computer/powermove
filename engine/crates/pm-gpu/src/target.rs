//! Render targets and their pool (plan section 8, "Working encoding and
//! precision").
//!
//! Working storage is `rgba16float` by default and `rgba32float` at 32-bit
//! precision. Targets are pooled by size, format, and usage: a frame's
//! intermediates are taken and returned as the graph walks, so a
//! steady-state frame allocates nothing. The pool counts its bytes for the
//! GPU lease (contract 8).

use pm_protocol::color::Precision;

/// The working format for a precision.
pub const fn working_format(precision: Precision) -> wgpu::TextureFormat {
    match precision {
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

    /// Bytes of texel storage.
    pub fn bytes(&self) -> u64 {
        let bpp = u64::from(self.format().block_copy_size(None).unwrap_or(16));
        u64::from(self.width()) * u64::from(self.height()) * bpp
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
    /// Idle targets, with the frame they were last given back in.
    free: Vec<(Key, Target, u64)>,
    /// Bytes held by targets in the pool and taken from it.
    live_bytes: u64,
    created: u64,
    /// Frames ended so far ([`TargetPool::end_frame`]).
    frame: u64,
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
        if let Some(i) = self.free.iter().position(|(k, _, _)| *k == key) {
            return self.free.swap_remove(i).1;
        }
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
        self.free.push((key, target, self.frame));
    }

    /// Frees idle targets until the pool holds at most `bytes`.
    pub fn trim(&mut self, bytes: u64) {
        while self.live_bytes > bytes {
            let Some((_, t, _)) = self.free.pop() else { break };
            self.live_bytes = self.live_bytes.saturating_sub(t.bytes());
            t.texture.destroy();
        }
    }

    /// Ends a frame: idle targets no frame since an earlier one used are
    /// freed, oldest first, until at most `keep_idle` bytes of them stay.
    /// The frame's own targets always stay (the next frame of the same
    /// layers takes them again); sizes that went by (a layer whose scale
    /// animates sizes its target anew every frame) do not pile up.
    pub fn end_frame(&mut self, keep_idle: u64) {
        let frame = self.frame;
        self.frame += 1;
        let mut idle: u64 = self
            .free
            .iter()
            .filter(|(_, _, used)| *used < frame)
            .map(|(_, t, _)| t.bytes())
            .sum();
        if idle <= keep_idle {
            return;
        }
        self.free.sort_by_key(|(_, _, used)| *used);
        while idle > keep_idle && !self.free.is_empty() {
            if self.free[0].2 >= frame {
                break;
            }
            let (_, t, _) = self.free.remove(0);
            idle -= t.bytes();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_keeps_its_own_targets_and_sizes_that_went_by_do_not_pile_up() {
        let Ok(gpu) = crate::Gpu::new("target pool test") else {
            eprintln!("no GPU: skipped");
            return;
        };
        let usage = working_usage();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut pool = TargetPool::new();
        // A layer whose scale animates sizes its target anew every frame.
        for frame in 0..50u32 {
            let t = pool.take(&gpu.device, 100 + frame, 100, format, usage);
            let steady = pool.take(&gpu.device, 64, 64, format, usage);
            pool.give(t);
            pool.give(steady);
            pool.end_frame(200 * 1024);
        }
        // About 40 KB per animated target: no more than the idle budget
        // stays, plus the last frame's own two.
        assert!(pool.bytes() <= 200 * 1024 + 2 * 150 * 400, "{} bytes", pool.bytes());
        // The steady size is reused, never made again.
        let before = pool.created();
        let t = pool.take(&gpu.device, 64, 64, format, usage);
        assert_eq!(pool.created(), before);
        pool.give(t);
        // A frame's own targets stay even over the budget.
        let big = pool.take(&gpu.device, 512, 512, format, usage);
        pool.give(big);
        pool.end_frame(0);
        assert!(pool.idle() >= 1);
        let again = pool.take(&gpu.device, 512, 512, format, usage);
        assert_eq!(pool.created(), before + 1, "kept for the next frame");
        pool.give(again);
    }
}
