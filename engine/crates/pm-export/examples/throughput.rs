//! Export throughput of the CPU encoder-input path plus VideoToolbox and AVAssetWriter.
//!
//!   cargo run --release -p pm-export --example throughput -- <hq|4444|h264> <width> <height> <frames>
//!
//! Writes into a temporary folder and removes it. Prints convert time (the CPU
//! reference conversion, which the GPU encoder-input pass replaces in production),
//! the whole export, and the verification it includes.

#[cfg(target_os = "macos")]
fn main() {
    use std::time::Instant;

    use pm_export::convert::{ConvertParams, convert};
    use pm_export::*;
    use pm_time::Rate;

    let args: Vec<String> = std::env::args().collect();
    let codec = match args.get(1).map(String::as_str) {
        Some("4444") => Codec::ProRes(ProResProfile::P4444),
        Some("h264") => Codec::H264 {
            bitrate_bps: 16_000_000,
        },
        _ => Codec::ProRes(ProResProfile::Hq),
    };
    let width: u32 = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(1920);
    let height: u32 = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(1080);
    let frames: u64 = args.get(4).and_then(|v| v.parse().ok()).unwrap_or(48);
    let dir = std::env::temp_dir().join(format!("pm-export-throughput-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let container = if matches!(codec, Codec::H264 { .. }) {
        Container::Mp4
    } else {
        Container::Mov
    };
    let spec = ExportSpec {
        width,
        height,
        rate: Rate::FPS_23_976,
        frames,
        codec,
        container,
        tags: ColorTags::Rec709QuickTime,
        alpha: AlphaOutput::None,
        working: WorkingEncoding::Standard,
        dither: Dither::Default,
        audio: None,
        timecode: None,
        destination: dir.join(format!("out.{}", container.extension())),
        overwrite: Overwrite::Fail,
        mode: None,
    };
    let picture = |f: u32| -> Vec<f32> {
        let mut v = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                let u = (x as f32 / width as f32 + f as f32 * 0.01).fract();
                v.extend([u, y as f32 / height as f32, 1.0 - u, 1.0]);
            }
        }
        v
    };
    // Convert alone, for the CPU share (the dither tile is generated once, first).
    let _ = pm_export::dither::tile();
    let px = picture(0);
    let t = Instant::now();
    let params = ConvertParams {
        working: WorkingEncoding::Standard,
        alpha: AlphaOutput::None,
        dither: spec.dither_enabled(),
        frame: 0,
    };
    let _ = convert(
        &WorkingFrame {
            width,
            height,
            rgba: &px,
        },
        codec.input_format(false),
        &params,
    )
    .unwrap();
    let convert_ms = t.elapsed().as_secs_f64() * 1e3;
    let t = Instant::now();
    let mut job = export(spec).unwrap();
    for f in 0..frames as u32 {
        let px = picture(f);
        job.push_frame(&WorkingFrame {
            width,
            height,
            rgba: &px,
        })
        .unwrap();
    }
    let done = job.finish().unwrap();
    let total = t.elapsed().as_secs_f64();
    let bytes = std::fs::metadata(&done.path).unwrap().len();
    println!(
        "{} {width}x{height}, {frames} frames: {:.1} fps end to end ({:.1} ms/frame, CPU convert {:.1} ms/frame), {:.1} MB, hardware {:?}, max block error {:.3} CV",
        done.report.codec,
        frames as f64 / total,
        total * 1e3 / frames as f64,
        convert_ms,
        bytes as f64 / 1e6,
        done.report.hardware_encoder,
        done.report.max_pixel_error_cv()
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("VideoToolbox export runs on macOS only.");
}
