//! Verify Canvas PNG encoder-input bytes against an external premultiplied
//! binary32 capture. No temporary files or app processes are created.
use pm_export::convert::{ConvertParams, WorkingFrame, convert};
use pm_export::settings::{AlphaOutput, InputFormat, WorkingEncoding};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 4 {
        return Err("usage: canvas_png INPUT_RGBA32F WIDTH HEIGHT OUTPUT_RGBA8".into());
    }
    let bytes = std::fs::read(&args[0])?;
    if bytes.len() % 4 != 0 {
        return Err("unaligned binary32 capture".into());
    }
    let rgba: Vec<_> = bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    let frame = WorkingFrame {
        width: args[1].parse()?,
        height: args[2].parse()?,
        rgba: &rgba,
    };
    let out = convert(
        &frame,
        InputFormat::Rgba8,
        &ConvertParams {
            legacy_png: true,
            working: WorkingEncoding::Standard,
            alpha: AlphaOutput::Straight,
            dither: false,
            frame: 0,
        },
    )?;
    std::fs::write(&args[3], out.planes[0].u8().ok_or("RGBA8 output required")?)?;
    Ok(())
}
