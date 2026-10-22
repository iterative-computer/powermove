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
    let sign = u32::from(h & 0x8000) << 16;
    let exp = u32::from((h >> 10) & 0x1f);
    let frac = u32::from(h & 0x3ff);
    match exp {
        0 => {
            let magnitude = (frac as f32) * f32::from_bits(103 << 23);
            f32::from_bits(sign | magnitude.to_bits())
        }
        31 => f32::from_bits(sign | 0x7f80_0000 | (frac << 13)),
        e => f32::from_bits(sign | ((e + 112) << 23) | (frac << 13)),
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
        F::Rgba8Unorm => (4, 1),
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
    Ok(out)
}

/// Reads `texture` (mip 0) as raw texel bytes, tightly packed rows.
pub fn read_raw(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
) -> Result<Vec<u8>, ReadbackError> {
    with_mapped_rows(device, queue, texture, |data, row, padded, h| {
        if row == padded {
            return data.to_vec();
        }
        let mut out = Vec::with_capacity(row * h);
        for y in 0..h {
            let start = y * padded;
            out.extend_from_slice(&data[start..start + row]);
        }
        out
    })
}

/// Reads mip 0 while its staging buffer is mapped. `data` contains top-down
/// rows of `row` useful bytes spaced `padded` bytes apart; it is valid only
/// for the duration of `convert`. This lets callers encode directly into
/// their final output without allocating a tightly packed intermediate.
pub fn with_mapped_rows<T>(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    convert: impl FnOnce(&[u8], usize, usize, usize) -> T,
) -> Result<T, ReadbackError> {
    Readback::default().with_mapped_rows(device, queue, texture, convert)
}

/// A reusable staging buffer for serial readbacks on one GPU device. It is
/// deliberately device-scoped: a lost device drops the buffer with its owner.
#[derive(Default)]
pub struct Readback {
    buffer: Option<wgpu::Buffer>,
    capacity: u64,
    #[cfg(test)]
    allocations: usize,
}

impl Readback {
    /// Bytes held by the reusable GPU staging buffer.
    pub fn capacity_bytes(&self) -> u64 {
        self.capacity
    }

    /// Release an oversized staging allocation after its mapping is closed.
    pub fn trim(&mut self, max_bytes: u64) {
        if self.capacity > max_bytes {
            self.buffer = None;
            self.capacity = 0;
        }
    }

    /// Copy this texture into the staging buffer and map it afresh. The
    /// buffer can be reused only after the previous mapping was dropped and
    /// unmapped; this method always completes that sequence synchronously.
    pub fn with_mapped_rows<T>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
        convert: impl FnOnce(&[u8], usize, usize, usize) -> T,
    ) -> Result<T, ReadbackError> {
        let format = texture.format();
        let bpp = format
            .block_copy_size(None)
            .ok_or(ReadbackError::Format(format))?;
        let (w, h) = (texture.width(), texture.height());
        let row = (w * bpp) as usize;
        let padded = row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
        let size = padded as u64 * u64::from(h);
        if self.capacity < size {
            self.buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pm readback"),
                size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }));
            self.capacity = size;
            #[cfg(test)]
            {
                self.allocations += 1;
            }
        }
        let buffer = self.buffer.as_ref().expect("created for this readback");
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("pm readback"),
        });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded as u32),
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
        let slice = buffer.slice(..size);
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
        let out = convert(&data, row, padded, h as usize);
        drop(data);
        buffer.unmap();
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("readback reuse test"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    fn upload(queue: &wgpu::Queue, texture: &wgpu::Texture, bytes: &[u8]) {
        let row = texture.width() * texture.format().block_copy_size(None).unwrap();
        queue.write_texture(
            texture.as_image_copy(),
            bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(texture.height()),
            },
            wgpu::Extent3d {
                width: texture.width(),
                height: texture.height(),
                depth_or_array_layers: 1,
            },
        );
    }

    #[test]
    fn serial_readback_reuses_staging_across_fresh_copies_formats_and_growth() {
        let gpu = crate::Gpu::new("readback reuse test").unwrap();
        let mut readback = Readback::default();
        let rgba8 = source(&gpu.device, wgpu::TextureFormat::Rgba8Unorm, 2, 1);
        for (index, values) in [[1u8, 2, 3, 4, 5, 6, 7, 8], [8, 7, 6, 5, 4, 3, 2, 1]]
            .iter()
            .enumerate()
        {
            upload(&gpu.queue, &rgba8, values);
            let got = readback
                .with_mapped_rows(&gpu.device, &gpu.queue, &rgba8, |data, row, _, _| {
                    data[..row].to_vec()
                })
                .unwrap();
            assert_eq!(
                got.as_slice(),
                values.as_slice(),
                "copy {index} must show the new pixels"
            );
            assert_eq!(readback.allocations, 1);
        }
        let half = source(&gpu.device, wgpu::TextureFormat::Rgba16Float, 1, 1);
        let half_bytes = [0, 0x3c, 0, 0, 0, 0, 0, 0x3c];
        upload(&gpu.queue, &half, &half_bytes);
        let got = readback
            .with_mapped_rows(&gpu.device, &gpu.queue, &half, |data, row, _, _| {
                data[..row].to_vec()
            })
            .unwrap();
        assert_eq!(got.as_slice(), half_bytes.as_slice());
        assert_eq!(readback.allocations, 1, "same padded size across formats");

        let taller = source(&gpu.device, wgpu::TextureFormat::Rgba8Unorm, 2, 2);
        let taller_bytes = [
            10u8, 11, 12, 13, 14, 15, 16, 17, 20, 21, 22, 23, 24, 25, 26, 27,
        ];
        upload(&gpu.queue, &taller, &taller_bytes);
        let got = readback
            .with_mapped_rows(
                &gpu.device,
                &gpu.queue,
                &taller,
                |data, row, stride, height| {
                    (0..height)
                        .flat_map(|y| data[y * stride..y * stride + row].iter().copied())
                        .collect::<Vec<_>>()
                },
            )
            .unwrap();
        assert_eq!(got.as_slice(), taller_bytes.as_slice());
        assert_eq!(readback.allocations, 2, "taller copy grows once");
        readback.trim(512);
        assert_eq!(readback.capacity_bytes(), 512);
        readback.trim(256);
        assert_eq!(readback.capacity_bytes(), 0);
        assert!(readback.buffer.is_none());
        let got = readback
            .with_mapped_rows(&gpu.device, &gpu.queue, &half, |data, row, _, _| {
                data[..row].to_vec()
            })
            .unwrap();
        assert_eq!(got.as_slice(), half_bytes.as_slice());
        assert_eq!(readback.allocations, 3, "trimmed staging is recreated");
    }

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
