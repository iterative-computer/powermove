//! Uploading a Blender surface: its crop rectangle's premultiplied E values
//! as `rgba32float` at 32-bit precision, `rgba16float` (rounded to nearest
//! even) otherwise, top row first (an image, not a GL target).

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

/// The texture of `surface`'s crop rectangle.
pub(crate) fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    surface: &DecodedSurface,
    precision: Precision,
) -> wgpu::TextureView {
    let (w, h) = (surface.crop.width, surface.crop.height);
    let (format, bytes, texel): (wgpu::TextureFormat, Vec<u8>, u32) = match precision {
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
