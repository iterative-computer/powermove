//! Blender surface import for the job engine (plan sections 9 and 17, contract 11).
//!
//! Surfaces are 16-bit RGBA PNGs with straight alpha whose color codes are already E
//! (AgX to piecewise sRGB). The engine ignores the PNG `gAMA`, `sRGB`, and `iCCP` chunks
//! (it never color-converts a surface) and premultiplies in E, the Standard working
//! encoding, at f32. Decoding is exact: every 16-bit code maps to `code / 65535`.

use anyhow::{Context, Result, bail};
use std::path::Path;

pub struct Surface16 {
    pub width: u32,
    pub height: u32,
    /// Straight RGBA, 16 bits per channel.
    pub rgba: Vec<u16>,
    pub bit_depth: u8,
    /// Ancillary chunk types present (diagnostics; color chunks are ignored).
    pub chunks: Vec<String>,
}

pub fn chunk_types(bytes: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 8;
    while i + 8 <= bytes.len() {
        let n = u32::from_be_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
        out.push(String::from_utf8_lossy(&bytes[i + 4..i + 8]).into_owned());
        i += 12 + n;
    }
    out
}

pub fn read(path: &Path) -> Result<Surface16> {
    let bytes = std::fs::read(path).with_context(|| path.display().to_string())?;
    decode(&bytes)
}

/// Decodes RGBA PNG (8- or 16-bit) to 16-bit straight RGBA, never applying gamma or
/// color chunks. 8-bit codes widen by ×257 (exact).
pub fn decode(bytes: &[u8]) -> Result<Surface16> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::IDENTITY);
    let mut reader = decoder.read_info()?;
    let mut buf = vec![0u8; reader.output_buffer_size().context("size")?];
    let info = reader.next_frame(&mut buf)?;
    if info.color_type != png::ColorType::Rgba {
        bail!("surface must be RGBA, got {:?}", info.color_type);
    }
    let n = (info.width * info.height * 4) as usize;
    let rgba: Vec<u16> = match info.bit_depth {
        png::BitDepth::Sixteen => buf[..n * 2].chunks_exact(2).map(|b| u16::from_be_bytes([b[0], b[1]])).collect(),
        png::BitDepth::Eight => buf[..n].iter().map(|&b| b as u16 * 257).collect(),
        d => bail!("unsupported bit depth {d:?}"),
    };
    Ok(Surface16 { width: info.width, height: info.height, rgba, bit_depth: info.bit_depth as u8, chunks: chunk_types(bytes) })
}

impl Surface16 {
    pub fn px(&self, x: u32, y: u32) -> [u16; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2], self.rgba[i + 3]]
    }

    /// The compositor's input: premultiplied E-encoded f32 RGBA.
    pub fn premultiplied(&self) -> Vec<[f32; 4]> {
        self.rgba
            .chunks_exact(4)
            .map(|p| {
                let a = p[3] as f32 / 65535.0;
                [p[0] as f32 / 65535.0 * a, p[1] as f32 / 65535.0 * a, p[2] as f32 / 65535.0 * a, a]
            })
            .collect()
    }

    /// Bounding box of pixels with alpha > 0: (x0, y0, x1, y1) inclusive.
    pub fn alpha_bbox(&self) -> Option<(u32, u32, u32, u32)> {
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
        let mut any = false;
        for y in 0..self.height {
            for x in 0..self.width {
                if self.px(x, y)[3] > 0 {
                    any = true;
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                }
            }
        }
        any.then_some((x0, y0, x1, y1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode16(w: u32, h: u32, px: &[u16]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut e = png::Encoder::new(&mut out, w, h);
            e.set_color(png::ColorType::Rgba);
            e.set_depth(png::BitDepth::Sixteen);
            // A gAMA chunk the import must ignore.
            e.set_source_gamma(png::ScaledFloat::new(1.0 / 1.8));
            let mut wr = e.write_header().unwrap();
            let bytes: Vec<u8> = px.iter().flat_map(|v| v.to_be_bytes()).collect();
            wr.write_image_data(&bytes).unwrap();
        }
        out
    }

    #[test]
    fn decode_is_exact_and_ignores_gamma() {
        let px: Vec<u16> = (0..4 * 64).map(|i| (i as u32 * 257 % 65536) as u16).collect();
        let s = decode(&encode16(8, 8, &px)).unwrap();
        assert_eq!(s.rgba, px);
        assert!(s.chunks.iter().any(|c| c == "gAMA"));
        let pm = s.premultiplied();
        let a = px[3] as f32 / 65535.0;
        assert_eq!(pm[0][0], px[0] as f32 / 65535.0 * a);
    }
}
