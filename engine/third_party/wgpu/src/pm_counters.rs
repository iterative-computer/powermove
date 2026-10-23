//! Powermove: deterministic work counters (`pm-perf`; engine/third_party/wgpu/POWERMOVE.md).
//!
//! The only change to upstream wgpu 30.0.1: the front API's resource
//! creations, pass commands, uploads and readbacks add to the calling
//! thread's `pm_perf` counters. Nothing else changes.

pub(crate) use pm_perf::{add, bump, Counter};

/// Bytes of a texture: every mip level and array layer (or depth slice of
/// level 0 for 3D, halved per level), times the sample count.
pub(crate) fn texture_bytes(desc: &crate::TextureDescriptor<'_>) -> u64 {
    let format = desc.format;
    let (bw, bh) = format.block_dimensions();
    let block = format
        .block_copy_size(None)
        .map(u64::from)
        .unwrap_or_else(|| {
            // Combined depth and stencil: both aspects.
            let depth = format
                .block_copy_size(Some(wgt::TextureAspect::DepthOnly))
                .unwrap_or(4);
            let stencil = format
                .block_copy_size(Some(wgt::TextureAspect::StencilOnly))
                .unwrap_or(0);
            u64::from(depth + stencil)
        });
    let mut total = 0u64;
    for level in 0..desc.mip_level_count.max(1) {
        let w = (desc.size.width >> level).max(1);
        let h = (desc.size.height >> level).max(1);
        let layers = match desc.dimension {
            wgt::TextureDimension::D3 => (desc.size.depth_or_array_layers >> level).max(1),
            _ => desc.size.depth_or_array_layers.max(1),
        };
        let blocks = u64::from(w.div_ceil(bw)) * u64::from(h.div_ceil(bh));
        total = total.saturating_add(blocks * block * u64::from(layers));
    }
    total.saturating_mul(u64::from(desc.sample_count.max(1)))
}
