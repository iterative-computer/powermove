//! The viewer surface pool (contract 3 `SurfacePool`, plan section 18 item 2).

use pm_protocol::common::{Seq, Size};
use pm_protocol::interop::{
    self, MAX_POOL_SURFACES, MIN_POOL_SURFACES, PlatformSurface, SurfaceDescriptor, SurfaceFormat,
    SurfaceRequest,
};

use super::BridgeOutError;
use super::surface::ExportedSurface;

/// Surfaces a pool needs: the render ring (frames being rendered or waiting
/// for their command buffer), the jitter buffer depth (frames the viewer
/// holds ahead of their presentation time), and two more (the frame on
/// screen and the one being drawn), clamped to contract 3's 4 to 32.
pub fn pool_size(ring: u32, jitter: u32) -> u32 {
    (ring + jitter + 2).clamp(MIN_POOL_SURFACES as u32, 32)
}

/// Amendment 4: `rgbaf16` on EDR displays (values above 1.0 reach the
/// screen), `bgra` with blue-noise dither elsewhere and whenever the memory
/// for half floats is not available.
pub fn choose_format(edr: bool, f16_fits: bool) -> SurfaceFormat {
    if edr && f16_fits {
        SurfaceFormat::Rgba16Float
    } else {
        SurfaceFormat::Bgra8Unorm
    }
}

/// Bytes a pool of `count` surfaces of `size` and `format` occupies, before
/// IOSurface's row and allocation alignment (an estimate for leases).
pub fn estimated_bytes(size: Size, format: SurfaceFormat, count: u32) -> u64 {
    u64::from(size.width)
        * u64::from(size.height)
        * u64::from(format.bytes_per_pixel())
        * u64::from(count)
}

/// One pool generation: same-size, same-format exported surfaces, each sent
/// to main once and imported once.
pub struct SurfacePool {
    pub generation: u32,
    pub size: Size,
    pub format: SurfaceFormat,
    pub surfaces: Vec<ExportedSurface>,
}

impl SurfacePool {
    pub fn new(
        device: &wgpu::Device,
        generation: u32,
        size: Size,
        format: SurfaceFormat,
        count: u32,
    ) -> Result<Self, BridgeOutError> {
        if !(MIN_POOL_SURFACES as u32..=MAX_POOL_SURFACES as u32).contains(&count) {
            return Err(BridgeOutError::PoolSize(count));
        }
        let request = SurfaceRequest { size, format };
        let surfaces = (0..count)
            .map(|_| ExportedSurface::create(device, &request))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            generation,
            size,
            format,
            surfaces,
        })
    }

    pub fn len(&self) -> usize {
        self.surfaces.len()
    }

    pub fn is_empty(&self) -> bool {
        self.surfaces.is_empty()
    }

    /// Bytes the IOSurfaces allocated.
    pub fn bytes(&self) -> u64 {
        self.surfaces.iter().map(|s| s.iosurface.alloc_size()).sum()
    }

    /// The pool as contract 3 describes it. `bridge_index` is the pool index:
    /// the bridge keys surfaces by (generation, pool index).
    pub fn descriptor(&self) -> interop::SurfacePool {
        interop::SurfacePool {
            generation: Seq::new(u64::from(self.generation)).unwrap_or(Seq::MAX),
            surfaces: (0..self.surfaces.len())
                .map(|i| SurfaceDescriptor {
                    pool_index: i as u32,
                    size: self.size,
                    format: self.format,
                    platform: PlatformSurface::IoSurface {
                        bridge_index: i as u32,
                    },
                })
                .collect(),
            role: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_size_is_ring_plus_jitter_plus_two_within_contract_bounds() {
        assert_eq!(pool_size(2, 4), 8);
        assert_eq!(pool_size(0, 0), 4);
        assert_eq!(pool_size(10, 30), 32);
    }

    #[test]
    fn half_floats_only_on_edr_displays_with_memory() {
        assert_eq!(choose_format(true, true), SurfaceFormat::Rgba16Float);
        assert_eq!(choose_format(true, false), SurfaceFormat::Bgra8Unorm);
        assert_eq!(choose_format(false, true), SurfaceFormat::Bgra8Unorm);
    }

    #[test]
    fn estimates_match_the_s1_pools() {
        // S1: a 4K pool of 14 is 443 MiB in bgra and 886 MiB in rgbaf16.
        let k4 = Size::new(3840, 2160);
        assert_eq!(
            estimated_bytes(k4, SurfaceFormat::Bgra8Unorm, 14) >> 20,
            442
        );
        assert_eq!(
            estimated_bytes(k4, SurfaceFormat::Rgba16Float, 14) >> 20,
            885
        );
    }
}
