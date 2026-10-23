//! Compatibility rasters (plan section 20; owner amendments 23 and 24):
//! layer pixels only the document's renderer can draw.
//!
//! An extension at apiVersion 1 to 3 may wrap the `raster` service: for the
//! layers it changes, today's compositor draws the wrapper's canvas instead
//! of its own raster, on the quad the surface describes (`w`, `h`,
//! `anchorX`, `anchorY`, `uv`). The engine does the same: when the session
//! has a producer ([`CompatRasters::producing`]), every layer today's
//! compositor rasterizes through `PM.raster` (text, captions, shapes) asks
//! it ([`CompatRasters::answer`]) at the layer's own time and raster density
//! (`continuousRasterScale`). The producer either drew the layer
//! ([`CompatAnswer::Raster`]), passed it through ([`CompatAnswer::Native`]:
//! the engine draws it as usual), or has not answered yet
//! ([`CompatAnswer::Missing`]): the frame then fails with
//! [`crate::RenderError::CompatPending`] naming what it needs, and the
//! caller asks the renderer and renders it again (the decision 19 fallback
//! runner's pattern). Rasters are drawn exactly like a text raster: an
//! E-encoded premultiplied image (a canvas's backing store), sampled through
//! the layer's quad, so effects, masks, mattes, transforms, and blending
//! apply to them as to the layer's own raster.

use std::sync::Arc;

pub use pm_protocol::snapshot::{CompatFormat, CompatNeed, CompatSource};

/// One raster a producer drew for a layer at one frame.
pub struct CompatRaster {
    pub width: u32,
    pub height: u32,
    pub format: CompatFormat,
    /// The quad's anchor in layer pixels (today's `anchorX`, `anchorY`).
    pub anchor: [f64; 2],
    /// The quad's size in layer pixels (today's `w`, `h`).
    pub size: [f64; 2],
    /// The part of the raster the quad samples (`[ox, oy, sx, sy]`).
    pub uv: [f64; 4],
    /// Rows top-down, tightly packed in `format`.
    pub data: Arc<Vec<u8>>,
}

impl std::fmt::Debug for CompatRaster {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "CompatRaster({}×{} {:?} anchor {:?} size {:?})",
            self.width, self.height, self.format, self.anchor, self.size
        )
    }
}

/// A producer's answer for one layer at one frame.
#[derive(Debug, Clone)]
pub enum CompatAnswer {
    Raster(Arc<CompatRaster>),
    /// Passed through: the engine draws the layer itself.
    Native,
    /// Not answered yet.
    Missing,
}

/// The compatibility rasters of one session (the interactive engine's
/// store, the job engine's host answers).
pub trait CompatRasters: Send + Sync {
    /// Whether the session has a producer for `source`.
    fn producing(&self, source: CompatSource) -> bool;
    /// The answer for `layer` of `composition` at composition time `time`
    /// (seconds), drawn at `scale` output pixels per layer pixel, for the
    /// window `crop` of its raster (today's preview crop,
    /// `crate::compat_crop`; `None`: the whole raster).
    fn answer(
        &self,
        composition: &str,
        layer: &str,
        time: f64,
        scale: f64,
        source: CompatSource,
        crop: Option<pm_protocol::common::PixelRect>,
    ) -> CompatAnswer;
}

/// The protocol's time of a composition time in seconds (1/705,600,000 s;
/// the producer reads it back as `ticks / 705_600_000`).
pub fn ticks_of(seconds: f64) -> pm_protocol::common::Ticks {
    let ticks = (seconds * pm_protocol::common::TICKS_PER_SECOND as f64).round();
    pm_protocol::common::Ticks::saturating_from_i128(if ticks.is_finite() {
        ticks as i128
    } else {
        0
    })
}

/// The raster's pixels as premultiplied E values in `f32` (top row first).
pub(crate) fn premultiplied_f32(raster: &CompatRaster) -> Vec<f32> {
    let n = (raster.width as usize) * (raster.height as usize);
    let mut out = Vec::with_capacity(n * 4);
    match raster.format {
        CompatFormat::Rgba8Premultiplied => {
            out.extend(
                raster
                    .data
                    .iter()
                    .take(n * 4)
                    .map(|&v| f32::from(v) / 255.0),
            );
        }
        CompatFormat::Rgba8Straight => {
            for px in raster.data.chunks_exact(4).take(n) {
                let a = f32::from(px[3]) / 255.0;
                out.extend([
                    f32::from(px[0]) / 255.0 * a,
                    f32::from(px[1]) / 255.0 * a,
                    f32::from(px[2]) / 255.0 * a,
                    a,
                ]);
            }
        }
        CompatFormat::Rgba16fPremultiplied => {
            for half in raster.data.chunks_exact(2).take(n * 4) {
                out.push(f16_to_f32(u16::from_le_bytes([half[0], half[1]])));
            }
        }
        // Encoded Blender maps never become layer rasters: the protocol
        // refuses PNG pixel bands and the job engine refuses PNG layer
        // answers, so nothing reaches here (transparent if it did).
        CompatFormat::Png => {}
    }
    out.resize(n * 4, 0.0);
    out
}

fn f16_to_f32(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = i32::from((bits >> 10) & 0x1f);
    let man = f32::from(bits & 0x3ff);
    match exp {
        0 => sign * man * 2f32.powi(-24),
        0x1f => {
            if man == 0.0 {
                sign * f32::INFINITY
            } else {
                f32::NAN
            }
        }
        e => sign * (1.0 + man / 1024.0) * 2f32.powi(e - 15),
    }
}

/// Convert each possible 8-bit channel once. The usual canvas answer is
/// already premultiplied; expanding it through an f32 allocation and then
/// converting every channel to half on every edit does unnecessary work.
/// Keep exactly the same float rounding, including at Float16 precision.
fn upload_bytes(
    raster: &CompatRaster,
    precision: pm_protocol::color::Precision,
) -> (wgpu::TextureFormat, Vec<u8>, u32) {
    use pm_protocol::color::Precision;
    use std::sync::OnceLock;
    if raster.format == CompatFormat::Rgba8Premultiplied {
        let channels = raster.width as usize * raster.height as usize * 4;
        if precision == Precision::Float32 {
            static VALUES: OnceLock<[[u8; 4]; 256]> = OnceLock::new();
            let values =
                VALUES.get_or_init(|| std::array::from_fn(|v| (v as f32 / 255.0).to_le_bytes()));
            let mut bytes = Vec::with_capacity(channels * 4);
            for &value in raster.data.iter().take(channels) {
                bytes.extend_from_slice(&values[value as usize]);
            }
            bytes.resize(channels * 4, 0);
            return (wgpu::TextureFormat::Rgba32Float, bytes, 16);
        }
        static VALUES: OnceLock<[[u8; 2]; 256]> = OnceLock::new();
        let values = VALUES.get_or_init(|| {
            std::array::from_fn(|v| crate::blender::gpu::f16_bits(v as f32 / 255.0).to_le_bytes())
        });
        let mut bytes = Vec::with_capacity(channels * 2);
        for &value in raster.data.iter().take(channels) {
            bytes.extend_from_slice(&values[value as usize]);
        }
        bytes.resize(channels * 2, 0);
        return (wgpu::TextureFormat::Rgba16Float, bytes, 8);
    }
    let values = premultiplied_f32(raster);
    match precision {
        Precision::Float32 => (
            wgpu::TextureFormat::Rgba32Float,
            values.iter().flat_map(|v| v.to_le_bytes()).collect(),
            16,
        ),
        _ => (
            wgpu::TextureFormat::Rgba16Float,
            values
                .iter()
                .flat_map(|v| crate::blender::gpu::f16_bits(*v).to_le_bytes())
                .collect(),
            8,
        ),
    }
}

/// The texture of a raster: premultiplied E values as `rgba32float` at
/// 32-bit precision, `rgba16float` otherwise (a Blender surface's upload).
pub(crate) fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    raster: &CompatRaster,
    precision: pm_protocol::color::Precision,
) -> wgpu::TextureView {
    let (format, bytes, texel) = upload_bytes(raster, precision);
    let (w, h) = (raster.width.max(1), raster.height.max(1));
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pm compatibility raster"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(w * texel),
            rows_per_image: Some(h),
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_lookup_is_bit_identical_for_every_channel_and_precision() {
        use pm_protocol::color::Precision;
        let r = CompatRaster {
            width: 64,
            ..raster(CompatFormat::Rgba8Premultiplied, (0..=255).collect())
        };
        let values = premultiplied_f32(&r);
        let (_, bytes, _) = upload_bytes(&r, Precision::Float32);
        let expected: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        assert_eq!(bytes, expected);
        let (_, bytes, _) = upload_bytes(&r, Precision::Float16);
        let expected: Vec<u8> = values
            .iter()
            .flat_map(|v| crate::blender::gpu::f16_bits(*v).to_le_bytes())
            .collect();
        assert_eq!(bytes, expected);
    }

    #[test]
    fn upload_lookup_keeps_short_answers_transparent() {
        use pm_protocol::color::Precision;
        let r = CompatRaster {
            width: 2,
            ..raster(CompatFormat::Rgba8Premultiplied, vec![255; 4])
        };
        assert_eq!(&upload_bytes(&r, Precision::Float16).1[8..], &[0; 8]);
        assert_eq!(&upload_bytes(&r, Precision::Float32).1[16..], &[0; 16]);
    }

    fn raster(format: CompatFormat, data: Vec<u8>) -> CompatRaster {
        CompatRaster {
            width: 1,
            height: 1,
            format,
            anchor: [0.0, 0.0],
            size: [1.0, 1.0],
            uv: [0.0, 0.0, 1.0, 1.0],
            data: Arc::new(data),
        }
    }

    #[test]
    fn eight_bit_premultiplied_rasters_are_today_s_texture_values() {
        let r = raster(CompatFormat::Rgba8Premultiplied, vec![128, 64, 0, 128]);
        assert_eq!(
            premultiplied_f32(&r),
            vec![128.0 / 255.0, 64.0 / 255.0, 0.0, 128.0 / 255.0]
        );
    }

    #[test]
    fn straight_rasters_are_premultiplied_in_e() {
        let r = raster(CompatFormat::Rgba8Straight, vec![255, 128, 0, 51]);
        let v = premultiplied_f32(&r);
        assert!(
            (v[0] - 0.2).abs() < 1e-6
                && (v[1] - 128.0 / 255.0 * 0.2).abs() < 1e-6
                && v[2] == 0.0
                && (v[3] - 0.2).abs() < 1e-6
        );
    }

    #[test]
    fn half_float_rasters_decode() {
        let one = 0x3c00u16.to_le_bytes();
        let half = 0x3800u16.to_le_bytes();
        let r = raster(
            CompatFormat::Rgba16fPremultiplied,
            [half, half, [0, 0], one].concat(),
        );
        assert_eq!(premultiplied_f32(&r), vec![0.5, 0.5, 0.0, 1.0]);
    }

    #[test]
    fn short_data_is_transparent_not_a_panic() {
        let r = CompatRaster {
            width: 2,
            ..raster(CompatFormat::Rgba8Premultiplied, vec![255; 4])
        };
        assert_eq!(premultiplied_f32(&r)[4..], [0.0; 4]);
    }

    #[test]
    fn times_name_rasters_stably() {
        assert_eq!(ticks_of(1.0 / 30.0).get(), 23_520_000);
        assert_eq!(ticks_of(0.0).get(), 0);
        assert_eq!(ticks_of(f64::NAN).get(), 0);
    }
}
