//! In-memory scene-linear mip generation for EXRs without stored levels.
//! EXR channels are already premultiplied, so area averaging preserves
//! transparent edges and values outside display range.

use super::{Rgba, StillImage, pool_rgba};
use crate::error::{MediaError, Result};
use crate::frame::PlaneFormat;
use crate::pool::FramePool;
use half::f16;

pub(super) fn from_original(
    mut image: StillImage,
    pool: &FramePool,
    max_dimension: u32,
) -> Result<StillImage> {
    if max_dimension == 0 {
        return Err(MediaError::invalid("zero EXR mip target"));
    }
    while image.frame.format.width.max(image.frame.format.height) > max_dimension as usize {
        let src = &image.frame;
        let (w, h) = (src.format.width, src.format.height);
        let format = src
            .format
            .planes
            .first()
            .ok_or_else(|| MediaError::invalid("EXR has no plane"))?
            .format;
        if !matches!(format, PlaneFormat::Rgba16Float | PlaneFormat::Rgba32Float) {
            return Err(MediaError::unsupported(
                "EXR mip requires RGBA float samples",
            ));
        }
        let (ow, oh) = (w.div_ceil(2), h.div_ceil(2));
        let mut out = vec![0f32; ow * oh * 4];
        src.with_planes(|planes| {
            let p = &planes[0];
            for oy in 0..oh {
                for ox in 0..ow {
                    let mut sum = [0f64; 4];
                    let mut count = 0f64;
                    for sy in oy * 2..(oy * 2 + 2).min(h) {
                        for sx in ox * 2..(ox * 2 + 2).min(w) {
                            let pixel = unsafe { p.ptr.add(sy * p.stride + sx * format.bytes()) };
                            for (component, value) in sum.iter_mut().enumerate() {
                                let at = unsafe { pixel.add(component * format.bytes() / 4) };
                                *value += if format == PlaneFormat::Rgba16Float {
                                    let bits = unsafe { u16::from_ne_bytes([*at, *at.add(1)]) };
                                    f16::from_bits(bits).to_f32() as f64
                                } else {
                                    let bytes =
                                        unsafe { [*at, *at.add(1), *at.add(2), *at.add(3)] };
                                    f32::from_ne_bytes(bytes) as f64
                                };
                            }
                            count += 1.0;
                        }
                    }
                    for c in 0..4 {
                        out[(oy * ow + ox) * 4 + c] = (sum[c] / count) as f32;
                    }
                }
            }
        })?;
        image.frame = pool_rgba(
            pool,
            ow,
            oh,
            Rgba::F32(&out),
            image.frame.format.rgba_premultiplied,
            32,
            "OpenEXR in-memory mip".into(),
        )?;
        image.bit_depth = 32;
        image.float = true;
    }
    Ok(image)
}
