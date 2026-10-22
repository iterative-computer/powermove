//! Texture readback (tests, agent captures, the CPU encoder-input
//! reference, `pm-cli parity`).
//!
//! Reads a 2D texture into tightly packed rows, top-down, converting to f32
//! RGBA. Supported formats: `rgba16float`, `rgba32float`, `rgba8unorm`,
//! `rgba16unorm`, `r32float`, `rg32float`.

#[derive(Debug, thiserror::Error)]
pub enum ReadbackError {
    #[error("readback of {0:?} is not supported")]
    Format(wgpu::TextureFormat),
    #[error("map failed: {0}")]
    Map(String),
}

/// IEEE 754 binary16 to f32 (exact).
pub fn half_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0f32 } else { 1.0 };
    let exp = (h >> 10) & 0x1f;
    let frac = f32::from(h & 0x3ff);
    match exp {
        0 => sign * frac * (2.0f32).powi(-24),
        31 => {
            if frac == 0.0 {
                sign * f32::INFINITY
            } else {
                f32::NAN
            }
        }
        e => sign * (1.0 + frac / 1024.0) * (2.0f32).powi(i32::from(e) - 15),
    }
}

/// f32 to binary16, rounding to nearest even (for uploads and tests).
pub fn f32_to_half(v: f32) -> u16 {
    let bits = v.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32;
    let mant = bits & 0x7f_ffff;
    if exp == 0xff {
        return sign | 0x7c00 | if mant != 0 { 0x200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 31 {
        return sign | 0x7c00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        let m = mant | 0x80_0000;
        let shift = (14 - e) as u32;
        let half = 1u32 << (shift - 1);
        let rest = m & ((1 << shift) - 1);
        let mut out = m >> shift;
        if rest > half || (rest == half && out & 1 == 1) {
            out += 1;
        }
        return sign | out as u16;
    }
    let mut out = ((e as u32) << 10) | (mant >> 13);
    let rest = mant & 0x1fff;
    if rest > 0x1000 || (rest == 0x1000 && out & 1 == 1) {
        out += 1;
    }
    sign | out as u16
}

/// Channels per texel and a decoder for one channel of a format.
fn layout(format: wgpu::TextureFormat) -> Result<(usize, usize), ReadbackError> {
    use wgpu::TextureFormat as F;
    Ok(match format {
        F::Rgba16Float => (4, 2),
        F::Rgba32Float => (4, 4),
        F::Rgba8Unorm | F::Bgra8Unorm => (4, 1),
        F::Rgba16Unorm => (4, 2),
        F::R32Float => (1, 4),
        F::Rg32Float => (2, 4),
        other => return Err(ReadbackError::Format(other)),
    })
}

/// Reads `texture` (mip 0) as f32 channels, tightly packed, top-down.
/// Blocks until the copy completes.
pub fn read_f32(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
) -> Result<Vec<f32>, ReadbackError> {
    let format = texture.format();
    let (channels, bytes_per_channel) = layout(format)?;
    let raw = read_raw(device, queue, texture)?;
    let (w, h) = (texture.width() as usize, texture.height() as usize);
    let mut out = Vec::with_capacity(w * h * channels);
    for chunk in raw.chunks_exact(bytes_per_channel) {
        let v = match (format, bytes_per_channel) {
            (wgpu::TextureFormat::Rgba16Float, 2) => {
                half_to_f32(u16::from_le_bytes([chunk[0], chunk[1]]))
            }
            (wgpu::TextureFormat::Rgba16Unorm, 2) => {
                f32::from(u16::from_le_bytes([chunk[0], chunk[1]])) / 65535.0
            }
            (_, 1) => f32::from(chunk[0]) / 255.0,
            (_, 4) => f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]),
            _ => unreachable!(),
        };
        out.push(v);
    }
    if format == wgpu::TextureFormat::Bgra8Unorm {
        for pixel in out.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
    }
    Ok(out)
}

/// Reads `texture` (mip 0) as raw texel bytes, tightly packed rows.
pub fn read_raw(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
) -> Result<Vec<u8>, ReadbackError> {
    let format = texture.format();
    let bpp = format
        .block_copy_size(None)
        .ok_or(ReadbackError::Format(format))?;
    let (w, h) = (texture.width(), texture.height());
    let row = w * bpp;
    let padded =
        row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("pm readback"),
        size: u64::from(padded) * u64::from(h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("pm readback"),
    });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv()
        .map_err(|e| ReadbackError::Map(e.to_string()))?
        .map_err(|e| ReadbackError::Map(e.to_string()))?;
    let data = slice
        .get_mapped_range()
        .map_err(|e| ReadbackError::Map(format!("{e:?}")))?;
    let mut out = Vec::with_capacity((row * h) as usize);
    for y in 0..h as usize {
        let start = y * padded as usize;
        out.extend_from_slice(&data[start..start + row as usize]);
    }
    drop(data);
    buffer.unmap();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_round_trips() {
        for h in 0..=u16::MAX {
            let f = half_to_f32(h);
            if f.is_nan() {
                continue;
            }
            assert_eq!(f32_to_half(f), h, "{h:#06x} {f}");
        }
        assert_eq!(f32_to_half(1.0), 0x3c00);
        assert_eq!(f32_to_half(65520.0), 0x7c00);
        // 1 + 2^-11 is halfway between 1 and the next half: ties to even (1).
        assert_eq!(f32_to_half(1.0 + 1.0 / 2048.0), 0x3c00);
        assert_eq!(f32_to_half(1.0 + 3.0 / 2048.0), 0x3c02);
    }
}
