//! Reusable, direct-to-frame readback for export's float working targets.
//!
//! The general `read_f32` path copies the mapped bytes into a raw `Vec`,
//! decodes into another `Vec<f32>`, then the export copies that into its frame.
//! An export already owns the destination, so decode mapped rows into it and
//! keep one staging buffer for successive bands and frames.

use std::sync::OnceLock;

use pm_gpu::readback::{Readback, ReadbackError, half_to_f32};

/// One export's serial staging buffer. Keep this across frames, on the render
/// thread; a device restart creates a new renderer and a new readback.
#[derive(Default)]
pub(crate) struct ExportReadback {
    staging: Readback,
}

impl ExportReadback {
    /// Decode an RGBA8 (8-bit project depth), RGBA16F or RGBA32F target
    /// into an existing top-down band.
    /// `destination` has exactly `width * height * 4` samples.
    pub(crate) fn read_into(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
        destination: &mut [f32],
    ) -> Result<(), ReadbackError> {
        let format = texture.format();
        if !matches!(
            format,
            wgpu::TextureFormat::Rgba8Unorm
                | wgpu::TextureFormat::Rgba16Float
                | wgpu::TextureFormat::Rgba32Float
        ) {
            return Err(ReadbackError::Format(format));
        }
        assert_eq!(
            destination.len(),
            texture.width() as usize * texture.height() as usize * 4,
            "the export readback destination must match the texture"
        );
        self.staging
            .with_mapped_rows(device, queue, texture, |data, row, padded, height| {
                decode_mapped_rows(format, data, row, padded, height, destination);
            })
    }
}

fn half_table() -> &'static [f32; 1 << 16] {
    static TABLE: OnceLock<Box<[f32; 1 << 16]>> = OnceLock::new();
    TABLE.get_or_init(|| Box::new(std::array::from_fn(|bits| half_to_f32(bits as u16))))
}

/// `data` is valid only while mapped. The caller owns the destination, so
/// there is no raw-byte or decoded-frame allocation in this hot path.
fn decode_mapped_rows(
    format: wgpu::TextureFormat,
    data: &[u8],
    row: usize,
    padded: usize,
    height: usize,
    destination: &mut [f32],
) {
    let bytes_per_sample = match format {
        wgpu::TextureFormat::Rgba8Unorm => 1,
        wgpu::TextureFormat::Rgba16Float => 2,
        wgpu::TextureFormat::Rgba32Float => 4,
        _ => unreachable!("read_into checked the texture format"),
    };
    debug_assert_eq!(row % bytes_per_sample, 0);
    debug_assert!(padded >= row && data.len() >= padded * height);
    let samples_per_row = row / bytes_per_sample;
    debug_assert_eq!(destination.len(), samples_per_row * height);
    match format {
        // The unorm decode `pm_gpu::readback::read_f32` uses.
        wgpu::TextureFormat::Rgba8Unorm => {
            crate::par::rows(destination, samples_per_row, |y, output| {
                let start = y * padded;
                for (sample, &byte) in output.iter_mut().zip(&data[start..start + row]) {
                    *sample = f32::from(byte) / 255.0;
                }
            });
        }
        wgpu::TextureFormat::Rgba16Float => {
            let table = half_table();
            crate::par::rows(destination, samples_per_row, |y, output| {
                let start = y * padded;
                for (sample, bytes) in output
                    .iter_mut()
                    .zip(data[start..start + row].chunks_exact(2))
                {
                    *sample = table[u16::from_le_bytes([bytes[0], bytes[1]]) as usize];
                }
            });
        }
        wgpu::TextureFormat::Rgba32Float => {
            crate::par::rows(destination, samples_per_row, |y, output| {
                let start = y * padded;
                for (sample, bytes) in output
                    .iter_mut()
                    .zip(data[start..start + row].chunks_exact(4))
                {
                    *sample = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                }
            });
        }
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_rows_preserve_every_bit_pattern_with_padding() {
        // Every binary16 code, including signed zero, subnormals, infinities,
        // and NaN payloads, takes the same conversion as `read_f32`.
        let (width, height) = (3usize, 1usize << 16);
        let row = width * 4 * 2;
        let padded = 32;
        let mut bytes = vec![0xa5; padded * height];
        for i in 0..width * height * 4 {
            let bits = (i & 0xffff) as u16;
            let y = i / (width * 4);
            let x = i % (width * 4);
            bytes[y * padded + x * 2..y * padded + x * 2 + 2].copy_from_slice(&bits.to_le_bytes());
        }
        let mut decoded = vec![0.0; width * height * 4];
        decode_mapped_rows(
            wgpu::TextureFormat::Rgba16Float,
            &bytes,
            row,
            padded,
            height,
            &mut decoded,
        );
        for (i, sample) in decoded.iter().enumerate() {
            assert_eq!(
                sample.to_bits(),
                half_to_f32((i & 0xffff) as u16).to_bits(),
                "binary16 code at sample {i}"
            );
        }
    }

    #[test]
    fn float_rows_preserve_bits_and_skip_padding() {
        let (width, height) = (3usize, 35usize);
        let row = width * 4 * 4;
        let padded = 64;
        let values = [
            0x0000_0000u32,
            0x8000_0000,
            0x3f00_0000,
            0x7f80_0000,
            0xff80_0000,
            0x7fc1_2345,
            0xffc1_2345,
            0x0000_0001,
        ];
        let mut bytes = vec![0xa5; padded * height];
        for i in 0..width * height * 4 {
            let y = i / (width * 4);
            let x = i % (width * 4);
            bytes[y * padded + x * 4..y * padded + x * 4 + 4]
                .copy_from_slice(&values[i % values.len()].to_le_bytes());
        }
        let mut decoded = vec![0.0; width * height * 4];
        decode_mapped_rows(
            wgpu::TextureFormat::Rgba32Float,
            &bytes,
            row,
            padded,
            height,
            &mut decoded,
        );
        for (i, sample) in decoded.iter().enumerate() {
            assert_eq!(sample.to_bits(), values[i % values.len()], "sample {i}");
        }
    }

    #[test]
    fn gpu_readback_matches_existing_decoder_across_reused_staging() {
        let gpu = pm_gpu::Gpu::new("export readback equivalence").unwrap();
        let mut readback = ExportReadback::default();
        for format in [
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureFormat::Rgba32Float,
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureFormat::Rgba16Float,
        ] {
            let bytes_per_sample = match format {
                wgpu::TextureFormat::Rgba8Unorm => 1,
                wgpu::TextureFormat::Rgba16Float => 2,
                _ => 4,
            };
            let (width, height) = (32u32, 20u32);
            let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("export readback equivalence"),
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
            });
            let samples = width as usize * height as usize * 4;
            let mut bytes = Vec::with_capacity(samples * bytes_per_sample);
            for i in 0..samples {
                let value = ((i * 73) % 1009) as f32 / 1009.0;
                if bytes_per_sample == 1 {
                    bytes.push((value * 255.0).round() as u8);
                } else if bytes_per_sample == 2 {
                    bytes.extend_from_slice(&pm_gpu::readback::f32_to_half(value).to_le_bytes());
                } else {
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
            }
            gpu.queue.write_texture(
                texture.as_image_copy(),
                &bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * 4 * bytes_per_sample as u32),
                    rows_per_image: Some(height),
                },
                texture.size(),
            );
            let mut actual = vec![0.0; samples];
            readback
                .read_into(&gpu.device, &gpu.queue, &texture, &mut actual)
                .unwrap();
            let expected = pm_gpu::readback::read_f32(&gpu.device, &gpu.queue, &texture).unwrap();
            assert_eq!(
                actual.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                expected.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
            );
        }
    }
}
