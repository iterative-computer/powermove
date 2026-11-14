//! A render that stalls (a busy machine, a playing viewer, an extension)
//! makes the encoder finish the frames it holds back (`Pipeline::idle`). The
//! file must still hold every frame in its place, for every VideoToolbox
//! format: its own file check says so.

#![cfg(target_os = "macos")]

use pm_export::convert::WorkingFrame;
use pm_export::*;
use pm_time::Rate;

const W: u32 = 320;
const H: u32 = 180;
const FRAMES: u64 = 24;

/// A bar moving across a ramp: each frame unlike its neighbours.
fn picture(n: u64) -> Vec<f32> {
    (0..H)
        .flat_map(|y| (0..W).map(move |x| (x, y)))
        .flat_map(|(x, y)| {
            let ramp = 0.2 + 0.5 * y as f32 / H as f32;
            let bar = (x as i64 - 10 * n as i64).abs() < 12;
            let v = if bar { 0.95 } else { ramp };
            [v, v * 0.7, 1.0 - v, if x < W / 2 { 1.0 } else { 0.6 }]
        })
        .collect()
}

fn export(name: &str, codec: Codec, alpha: AlphaOutput, idle_after: &[u64]) -> Result<(), String> {
    let dir = std::env::temp_dir().join(format!(
        "pm-export-idle-{name}-{}-{}",
        idle_after.len(),
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut job = pm_export::export(ExportSpec {
        width: W,
        height: H,
        rate: Rate::new(24, 1).unwrap(),
        frames: FRAMES,
        codec,
        container: Container::Mov,
        tags: ColorTags::Rec709QuickTime,
        alpha,
        working: WorkingEncoding::Standard,
        dither: Dither::Off,
        audio: None,
        timecode: None,
        destination: dir.join(format!("{name}.mov")),
        overwrite: Overwrite::Fail,
        mode: None,
    })
    .unwrap();
    for n in 0..FRAMES {
        let rgba = picture(n);
        job.push_frame(&WorkingFrame {
            width: W,
            height: H,
            rgba: &rgba,
        })
        .unwrap();
        if idle_after.contains(&n) {
            job.idle().map_err(|e| e.to_string())?;
        }
    }
    let result = job.finish().map(|_| ()).map_err(|e| e.to_string());
    if result.is_ok() && std::env::var_os("PM_STRESS_HASH").is_some() {
        let bytes = std::fs::read(dir.join(format!("{name}.mov"))).unwrap();
        // The compressed samples only: the movie header holds its creation time.
        let at = bytes.windows(4).position(|w| w == b"mdat").unwrap() - 4;
        let size = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        let samples = &bytes[at + 8..at + size];
        let mdat = samples.len();
        let sum = samples.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
            (h ^ u64::from(*b)).wrapping_mul(0x100_0000_01b3)
        });
        eprintln!("MEASURE {name}: {mdat} bytes, hash {sum:016x}");
        if let Some(keep) = std::env::var_os("PM_STRESS_KEEP") {
            std::fs::write(
                std::path::Path::new(&keep).join(format!("{name}.mov")),
                &bytes,
            )
            .unwrap();
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[test]
fn a_stalled_render_keeps_every_frame_in_place() {
    let hevc = |ten_bit| Codec::Hevc {
        bitrate_bps: 8_000_000,
        ten_bit,
    };
    let cases = [
        ("hevc-alpha", hevc(false), AlphaOutput::Straight),
        ("hevc", hevc(false), AlphaOutput::None),
        ("hevc10", hevc(true), AlphaOutput::None),
        (
            "h264",
            Codec::H264 {
                bitrate_bps: 8_000_000,
            },
            AlphaOutput::None,
        ),
        (
            "prores4444",
            Codec::ProRes(ProResProfile::P4444),
            AlphaOutput::Straight,
        ),
    ];
    let mut failed = vec![];
    for (name, codec, alpha) in cases {
        for idle in [vec![], vec![5], vec![5, 11, 17]] {
            let result = export(name, codec, alpha, &idle);
            eprintln!("MEASURE {name} idle after {idle:?}: {result:?}");
            if let Err(e) = result {
                failed.push(format!("{name} idle after {idle:?}: {e}"));
            }
        }
    }
    assert!(failed.is_empty(), "{failed:#?}");
}

/// HEVC with alpha, larger and longer, many times (`PM_STRESS_RUNS`): a
/// picture the encoder puts in the wrong place fails the file check.
#[test]
#[ignore]
fn hevc_alpha_keeps_every_frame_in_place_many_times() {
    let runs: usize = std::env::var("PM_STRESS_RUNS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);
    let mut failed = vec![];
    for run in 0..runs {
        let codec = Codec::Hevc {
            bitrate_bps: 8_000_000,
            ten_bit: false,
        };
        if let Err(e) = export(&format!("stress{run}"), codec, AlphaOutput::Straight, &[]) {
            eprintln!("MEASURE run {run}: {e}");
            failed.push(e);
        }
    }
    eprintln!("MEASURE {} of {runs} runs failed", failed.len());
    assert!(failed.is_empty(), "{failed:#?}");
}
