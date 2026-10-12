//! 8-bit RGBA PNG input and output (top-down rows).

use anyhow::{Result, bail};
use std::path::Path;

#[derive(Clone)]
pub struct Rgba8 {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

pub fn write_png(path: &Path, w: u32, h: u32, rgba: &[u8]) -> Result<()> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut enc = png::Encoder::new(file, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(rgba)?;
    Ok(())
}

pub fn read_png(path: &Path) -> Result<Rgba8> {
    let file = std::io::BufReader::new(std::fs::File::open(path)?);
    let mut reader = png::Decoder::new(file).read_info()?;
    let mut buf = vec![0u8; reader.output_buffer_size().unwrap_or(0)];
    let info = reader.next_frame(&mut buf)?;
    if info.bit_depth != png::BitDepth::Eight {
        bail!("{}: expected 8-bit PNG", path.display());
    }
    let data = match info.color_type {
        png::ColorType::Rgba => buf[..info.buffer_size()].to_vec(),
        png::ColorType::Rgb => buf[..info.buffer_size()].chunks(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        c => bail!("{}: unsupported color type {c:?}", path.display()),
    };
    Ok(Rgba8 { width: info.width, height: info.height, data })
}
