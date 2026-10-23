//! Blender surface uploads. Level 1 reproduces today's straight RGBA8
//! ImageBitmap upload; level 2 uses premultiplied E as `rgba32float` or
//! `rgba16float` (rounded to nearest even). Both are top row first.

use pm_blender::DecodedSurface;
use pm_protocol::color::Precision;

/// IEEE 754 binary16 bits of `v`, rounded to nearest even (overflow to
/// infinity, subnormals kept).
pub fn f16_bits(v: f32) -> u16 {
    let x = v.to_bits();
    let sign = ((x >> 16) & 0x8000) as u16;
    let exp = ((x >> 23) & 0xff) as i32;
    let man = x & 0x7f_ffff;
    if exp == 0xff {
        // Inf or NaN (keep NaN quiet).
        return sign | 0x7c00 | if man != 0 { 0x200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 0x1f {
        return sign | 0x7c00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        // Subnormal: shift the implicit one in, round to nearest even.
        let m = man | 0x80_0000;
        let shift = (14 - e) as u32;
        let half = 1u32 << (shift - 1);
        let rest = m & ((1u32 << shift) - 1);
        let mut out = m >> shift;
        if rest > half || (rest == half && out & 1 == 1) {
            out += 1;
        }
        return sign | out as u16;
    }
    let mut out = ((e as u32) << 10) | (man >> 13);
    let rest = man & 0x1fff;
    if rest > 0x1000 || (rest == 0x1000 && out & 1 == 1) {
        out += 1;
    }
    sign | out as u16
}

/// Today's crop is measured after Canvas2D's 8-bit alpha conversion. The
/// cropped ImageBitmap keeps straight RGB even where that alpha is zero.
/// Uploads use normalized RGBA8 and premultiplied blending, exactly as the
/// old compositor does. Do not apply this to compat level 2.
pub(crate) fn crop(
    surface: &DecodedSurface,
    compatibility: bool,
) -> pm_protocol::common::PixelRect {
    if !compatibility {
        return surface.crop;
    }
    let c = surface.crop;
    let (mut x0, mut y0, mut x1, mut y1) = (c.width, c.height, 0, 0);
    let mut any = false;
    for y in 0..c.height {
        for x in 0..c.width {
            let i = ((y * c.width + x) * 4) as usize;
            if code8(surface.straight[i + 3]) != 0 {
                any = true;
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    if any {
        let x = c.x + x0.saturating_sub(2);
        let y = c.y + y0.saturating_sub(2);
        pm_protocol::common::PixelRect {
            x,
            y,
            width: (c.x + x1 + 2).min(c.x + c.width - 1) - x + 1,
            height: (c.y + y1 + 2).min(c.y + c.height - 1) - y + 1,
        }
    } else {
        pm_protocol::common::PixelRect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        }
    }
}

fn compatibility_bytes(surface: &DecodedSurface, crop: pm_protocol::common::PixelRect) -> Vec<u8> {
    let c = surface.crop;
    let mut pixels = Vec::with_capacity((crop.width * crop.height * 4) as usize);
    for y in crop.y..crop.y + crop.height {
        for x in crop.x..crop.x + crop.width {
            if x >= c.x && y >= c.y && x < c.x + c.width && y < c.y + c.height {
                let i = (((y - c.y) * c.width + x - c.x) * 4) as usize;
                pixels.extend(surface.straight[i..i + 4].iter().map(|v| code8(*v)));
            } else {
                pixels.extend([0; 4]);
            }
        }
    }
    pixels
}

fn code8(v: f32) -> u8 {
    (v * 255.0).round_ties_even().clamp(0.0, 255.0) as u8
}

/// The texture of `surface`'s crop rectangle.
pub(crate) fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    surface: &DecodedSurface,
    precision: Precision,
    compatibility: bool,
) -> wgpu::TextureView {
    let crop = crop(surface, compatibility);
    let (w, h) = (crop.width, crop.height);
    let (format, bytes, texel): (wgpu::TextureFormat, Vec<u8>, u32) = if compatibility {
        (
            wgpu::TextureFormat::Rgba8Unorm,
            compatibility_bytes(surface, crop),
            4,
        )
    } else {
        match precision {
            Precision::Unorm8 => (
                wgpu::TextureFormat::Rgba8Unorm,
                surface.pixels.iter().map(|v| code8(*v)).collect(),
                4,
            ),
            Precision::Float32 => (
                wgpu::TextureFormat::Rgba32Float,
                surface
                    .pixels
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect(),
                16,
            ),
            _ => (
                wgpu::TextureFormat::Rgba16Float,
                surface
                    .pixels
                    .iter()
                    .flat_map(|v| f16_bits(*v).to_le_bytes())
                    .collect(),
                8,
            ),
        }
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pm blender surface"),
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
    use super::f16_bits;

    #[test]
    fn half_conversion_rounds_to_nearest_even() {
        assert_eq!(f16_bits(0.0), 0);
        assert_eq!(f16_bits(1.0), 0x3c00);
        assert_eq!(f16_bits(-2.0), 0xc000);
        assert_eq!(f16_bits(65504.0), 0x7bff);
        assert_eq!(f16_bits(1e6), 0x7c00);
        assert_eq!(f16_bits(f32::INFINITY), 0x7c00);
        // 1 + 2^-11 is halfway between 1 and the next half: even wins.
        assert_eq!(f16_bits(1.0 + 2f32.powi(-11)), 0x3c00);
        assert_eq!(f16_bits(1.0 + 3.0 * 2f32.powi(-11)), 0x3c02);
        // Smallest subnormal and below.
        assert_eq!(f16_bits(2f32.powi(-24)), 1);
        assert_eq!(f16_bits(2f32.powi(-26)), 0);
        // Every 16-bit code survives as the nearest half of code / 65535.
        for code in (0..=65535u32).step_by(97) {
            let v = code as f32 / 65535.0;
            let h = f16_bits(v);
            let back = half_to_f32(h);
            let next = half_to_f32(h + 1);
            let prev = if h > 0 { half_to_f32(h - 1) } else { back };
            assert!(
                (back - v).abs() <= (next - v).abs() && (back - v).abs() <= (prev - v).abs(),
                "{code}"
            );
        }
    }

    fn half_to_f32(h: u16) -> f32 {
        let e = (h >> 10) & 0x1f;
        let m = f32::from(h & 0x3ff);
        let s = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
        s * if e == 0 {
            m * 2f32.powi(-24)
        } else {
            (1.0 + m / 1024.0) * 2f32.powi(i32::from(e) - 15)
        }
    }
}
