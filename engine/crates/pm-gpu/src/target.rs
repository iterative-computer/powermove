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
        self.free.push((key, target));
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
