//! What the CPU encoder-input conversion costs for one UHD frame, per
//! format and chroma decimation: wall time, and processor time summed over
//! the conversion's threads (the steadier figure on a busy machine), with a
//! checksum of the bytes it produced (a change that only makes it faster
//! leaves every checksum as it was).
//!
//! A measurement, not a gate: run it in a release build,
//! `cargo test --release -p pm-export --test measure_convert -- --ignored --nocapture`.

use std::time::Instant;

use pm_export::convert::{Chroma, ConvertParams, PlaneData, WorkingFrame, convert_tagged};
use pm_export::{AlphaOutput, ColorTags, InputFormat, WorkingEncoding};

const W: u32 = 3840;
const H: u32 = 2160;

/// Detailed, saturated content with a smooth ramp under it, some of it
/// outside what video range codes (the delivery rule's path is exercised).
pub fn picture() -> Vec<f32> {
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut noise = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 40) as f32 / (1u64 << 24) as f32
    };
    let mut out = Vec::with_capacity((W * H * 4) as usize);
    for y in 0..H {
        for x in 0..W {
            let ramp = x as f32 / W as f32;
            let lift = if (x / 64 + y / 64) % 9 == 0 { 0.3 } else { 0.0 };
            out.extend([
                0.6 * ramp + 0.5 * noise() + lift,
                0.5 * (1.0 - ramp) + 0.5 * noise(),
                0.2 + 0.7 * noise(),
                1.0,
            ]);
        }
    }
    out
}

/// Processor time this process has used, user and system, in milliseconds.
fn cpu_ms() -> f64 {
    // SAFETY: `getrusage` fills the zeroed struct it is given.
    let usage = unsafe {
        let mut usage: libc::rusage = std::mem::zeroed();
        libc::getrusage(libc::RUSAGE_SELF, &mut usage);
        usage
    };
    let ms = |t: libc::timeval| t.tv_sec as f64 * 1e3 + t.tv_usec as f64 / 1e3;
    ms(usage.ru_utime) + ms(usage.ru_stime)
}

fn checksum(planes: &[pm_export::convert::Plane]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut eat = |b: u8| {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    };
    for plane in planes {
        match &plane.data {
            PlaneData::U8(v) => v.iter().for_each(|&b| eat(b)),
            PlaneData::U16(v) => v.iter().flat_map(|s| s.to_le_bytes()).for_each(&mut eat),
        }
    }
    hash
}

pub fn measure() {
    let rgba = picture();
    let frame = WorkingFrame {
        width: W,
        height: H,
        rgba: &rgba,
    };
    let pq = ColorTags::Rec2100Pq {
        white_nits: 203,
        metadata: None,
    };
    let hlg = ColorTags::Rec2100Hlg {
        white_nits: 203,
        peak_nits: 1000,
        metadata: None,
    };
    let sdr = ColorTags::Rec709QuickTime;
    let cases = [
        ("420v 8-bit 4:2:0 (H.264), Rec.709", InputFormat::V420, sdr),
        ("yuv420p 8-bit 4:2:0 (VP9), Rec.709", InputFormat::I420, sdr),
        ("x420 10-bit 4:2:0 (HEVC), Rec.709", InputFormat::X420, sdr),
        ("x420 10-bit 4:2:0 (HEVC), PQ", InputFormat::X420, pq),
        ("x420 10-bit 4:2:0 (HEVC), HLG", InputFormat::X420, hlg),
        (
            "v216 10-bit 4:2:2 (ProRes 422), Rec.709",
            InputFormat::V216,
            sdr,
        ),
        ("v216 10-bit 4:2:2 (ProRes 422), PQ", InputFormat::V216, pq),
        (
            "y416 12-bit 4:4:4 (ProRes 4444), Rec.709",
            InputFormat::Y416,
            sdr,
        ),
    ];
    for (name, format, tags) in cases {
        for (dither, chroma) in [
            (false, Chroma::Footage),
            (false, Chroma::Filtered),
            (true, Chroma::Filtered),
        ] {
            let params = ConvertParams {
                working: WorkingEncoding::Standard,
                alpha: AlphaOutput::None,
                tags,
                dither,
                frame: 7,
            };
            let (mut wall, mut cpu) = (Vec::new(), Vec::new());
            let mut sum = 0;
            for _ in 0..7 {
                let (started, used) = (Instant::now(), cpu_ms());
                let out = convert_tagged(&frame, format, &params, tags, chroma).unwrap();
                wall.push(started.elapsed().as_secs_f64() * 1e3);
                cpu.push(cpu_ms() - used);
                sum = checksum(&out.planes);
            }
            wall.sort_by(f64::total_cmp);
            cpu.sort_by(f64::total_cmp);
            eprintln!(
                "MEASURE convert UHD {name}, {} {}: wall best {:.1} ms (median {:.1}), processor best {:.0} ms (median {:.0}), checksum {sum:016x}",
                match chroma {
                    Chroma::Footage => "inverse (point across)",
                    Chroma::Filtered => "filtered",
                },
                if dither { "dithered" } else { "undithered" },
                wall[0],
                wall[wall.len() / 2],
                cpu[0],
                cpu[cpu.len() / 2]
            );
        }
    }
}

#[test]
#[ignore = "measurement: run in a release build with --ignored"]
fn measure_uhd_conversion() {
    measure();
}
