//! The file check refuses a wrong picture and delivers a right one.
//!
//! Each case writes a real file with the real encoder from frames that
//! differ from the ones the export meant to deliver (the evidence is taken
//! from the intended frames, as the sink takes it), and the file is then
//! held to that evidence the way a finished export is: a level shift, a
//! picture displaced by a pixel, a flattened block, a stale frame, a
//! swapped pair of frames and a garbage block must each be refused, by all
//! six ProRes flavours, H.264 and HEVC, while the same detailed frames
//! encoded as they are must be delivered.

#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};

use pm_export::convert::WorkingFrame;
use pm_export::pipeline::test_sinks::MemorySink;
use pm_export::verify::{Evidence, VerifyReport};
use pm_export::*;
use pm_time::Rate;

const W: u32 = 1280;
const H: u32 = 720;
const FRAMES: u64 = 8;
/// The centre block of the nine sampled ones, and the middle sampled frame.
const BLOCK_AT: (u32, u32) = (624, 352);
const MIDDLE: usize = (FRAMES / 2) as usize;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-export-check-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn hash(x: u32, y: u32, n: u64) -> f32 {
    let mut v = u64::from(x)
        .wrapping_mul(0x9e37_79b9_7f4a_7c15)
        .wrapping_add(u64::from(y).wrapping_mul(0xc2b2_ae3d_27d4_eb4f))
        .wrapping_add(n.wrapping_mul(0x1656_67b1_9e37_79f9));
    v ^= v >> 29;
    v = v.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    v ^= v >> 32;
    (v & 0xffff) as f32 / 65535.0 - 0.5
}

/// A detailed moving picture: slow waves and a soft-edged disc that crosses
/// the frame, under fine texture that pans three pixels a frame.
fn detailed(n: u64) -> Vec<f32> {
    let centre = (160.0 + 130.0 * n as f32, 250.0 + 30.0 * n as f32);
    (0..H)
        .flat_map(|y| (0..W).map(move |x| (x, y)))
        .flat_map(|(x, y)| {
            let (fx, fy) = (x as f32, y as f32);
            let pan = fx + 3.0 * n as f32;
            let wave = 0.14 * (pan / 5.9).sin() + 0.10 * (fy / 4.6).sin();
            let far = ((fx - centre.0).powi(2) + (fy - centre.1).powi(2)).sqrt();
            let disc = 0.22 * (1.0 - ((far - 140.0) / 12.0).clamp(0.0, 1.0));
            let grain = 0.10 * hash(x + 3 * n as u32, y, 0);
            let base = 0.42 + wave + disc + grain;
            [
                base + 0.06 * (fy / 37.0).sin(),
                base,
                base - 0.08 * (pan / 53.0).cos(),
                1.0,
            ]
        })
        .collect()
}

/// The review's picture: luma alternating between 10-bit codes 400 and 600
/// from pixel to pixel, neutral chroma, the same in every frame but for a
/// level that steps with the frame so frames can be told apart.
fn alternating(n: u64) -> Vec<f32> {
    let level = |code: f32| (code - 64.0) / 876.0;
    (0..H)
        .flat_map(|y| (0..W).map(move |x| (x, y)))
        .flat_map(|(x, y)| {
            let v = level(if (x + y) % 2 == 0 { 400.0 } else { 600.0 } + 6.0 * n as f32);
            [v, v, v, 1.0]
        })
        .collect()
}

#[derive(Clone, Copy)]
struct Format {
    name: &'static str,
    codec: Codec,
    container: Container,
    alpha: AlphaOutput,
}

fn formats() -> Vec<Format> {
    let mut out: Vec<Format> = [
        ("ProRes 422 Proxy", ProResProfile::Proxy),
        ("ProRes 422 LT", ProResProfile::Lt),
        ("ProRes 422", ProResProfile::Standard),
        ("ProRes 422 HQ", ProResProfile::Hq),
        ("ProRes 4444", ProResProfile::P4444),
        ("ProRes 4444 XQ", ProResProfile::P4444Xq),
    ]
    .into_iter()
    .map(|(name, profile)| Format {
        name,
        codec: Codec::ProRes(profile),
        container: Container::Mov,
        alpha: AlphaOutput::None,
    })
    .collect();
    out.push(Format {
        name: "H.264 12 Mb/s",
        codec: Codec::H264 {
            bitrate_bps: 12_000_000,
        },
        container: Container::Mp4,
        alpha: AlphaOutput::None,
    });
    out.push(Format {
        name: "HEVC Main10 12 Mb/s",
        codec: Codec::Hevc {
            bitrate_bps: 12_000_000,
            ten_bit: true,
        },
        container: Container::Mp4,
        alpha: AlphaOutput::None,
    });
    out
}

/// The formats that carry alpha.
fn alpha_formats() -> Vec<Format> {
    vec![
        Format {
            name: "ProRes 4444 with alpha",
            codec: Codec::ProRes(ProResProfile::P4444),
            container: Container::Mov,
            alpha: AlphaOutput::Straight,
        },
        Format {
            name: "ProRes 4444 XQ with alpha",
            codec: Codec::ProRes(ProResProfile::P4444Xq),
            container: Container::Mov,
            alpha: AlphaOutput::Straight,
        },
        Format {
            name: "HEVC with alpha 12 Mb/s",
            codec: Codec::Hevc {
                bitrate_bps: 12_000_000,
                ten_bit: false,
            },
            container: Container::Mov,
            alpha: AlphaOutput::Straight,
        },
    ]
}

fn spec(format: Format, destination: PathBuf) -> ExportSpec {
    ExportSpec {
        width: W,
        height: H,
        rate: Rate::new(25, 1).unwrap(),
        frames: FRAMES,
        codec: format.codec,
        container: format.container,
        tags: ColorTags::Rec709QuickTime,
        alpha: format.alpha,
        working: WorkingEncoding::Standard,
        dither: Dither::Off,
        audio: None,
        timecode: None,
        destination,
        overwrite: Overwrite::Replace,
        mode: None,
    }
}

/// What an export of `frames` keeps of them: the evidence its sink takes
/// from the encoder input.
fn evidence(format: Format, frames: &[Vec<f32>]) -> Evidence {
    let spec = spec(format, PathBuf::from("/unused"));
    let mut pipeline = Pipeline::new(spec, MemorySink::default()).unwrap();
    for rgba in frames {
        pipeline
            .push_frame(&WorkingFrame {
                width: W,
                height: H,
                rgba,
            })
            .unwrap();
    }
    let mut evidence = Evidence::default();
    for (n, input) in pipeline.finish().unwrap().frames.iter().enumerate() {
        evidence.add(n as u64, FRAMES, input);
    }
    evidence
}

/// Encodes `frames` to a file with the real encoder (the export's own check
/// holds the file to these same frames, and passes).
fn write(format: Format, frames: &[Vec<f32>], dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(format!("{name}.{}", format.container.extension()));
    let mut job = pm_export::export(spec(format, path.clone())).unwrap();
    for rgba in frames {
        job.push_frame(&WorkingFrame {
            width: W,
            height: H,
            rgba,
        })
        .unwrap();
    }
    job.finish()
        .unwrap_or_else(|e| panic!("{} {name}: {e}", format.name))
        .path
}

/// The file at `path` held to the evidence of `intended`.
fn examine(format: Format, path: &Path, intended: &Evidence) -> (VerifyReport, Vec<String>) {
    pm_export::mac::examine_file(
        path,
        &spec(format, path.to_path_buf()),
        intended.clone(),
        0,
        None,
    )
    .unwrap()
}

fn pixel(frame: &mut [f32], x: u32, y: u32) -> &mut [f32] {
    let i = ((y * W + x) * 4) as usize;
    &mut frame[i..i + 3]
}

/// Every frame lifted by ten 10-bit luma codes.
fn level_shift(frames: &mut [Vec<f32>]) {
    for frame in frames {
        for p in frame.chunks_exact_mut(4) {
            for c in &mut p[..3] {
                *c += 10.0 / 876.0;
            }
        }
    }
}

/// Every frame read as if its full-range codes were video range: the
/// picture squeezed into 16..235 of 255 (a gain of 0.86 and a lift of the
/// blacks), as a range mistake in a decoder or a tag would deliver it.
fn range_mistake(frames: &mut [Vec<f32>]) {
    for frame in frames {
        for p in frame.chunks_exact_mut(4) {
            for c in &mut p[..3] {
                *c = (16.0 + *c * 219.0) / 255.0;
            }
        }
    }
}

/// Every frame's picture one pixel to the right.
fn one_pixel_shift(frames: &mut [Vec<f32>]) {
    for frame in frames {
        for row in frame.chunks_exact_mut((W * 4) as usize) {
            row.copy_within(0..((W - 1) * 4) as usize, 4);
        }
    }
}

/// The centre sampled block of the middle frame, replaced by its mean.
fn flattened_block(frames: &mut [Vec<f32>]) {
    let frame = &mut frames[MIDDLE];
    let mut mean = [0.0f32; 3];
    for y in BLOCK_AT.1..BLOCK_AT.1 + 16 {
        for x in BLOCK_AT.0..BLOCK_AT.0 + 16 {
            for (m, c) in mean.iter_mut().zip(pixel(frame, x, y).iter()) {
                *m += c / 256.0;
            }
        }
    }
    for y in BLOCK_AT.1..BLOCK_AT.1 + 16 {
        for x in BLOCK_AT.0..BLOCK_AT.0 + 16 {
            pixel(frame, x, y).copy_from_slice(&mean);
        }
    }
}

/// A frame that is not one of the sampled three shows the frame before it.
fn stale_frame(frames: &mut [Vec<f32>]) {
    frames[2] = frames[1].clone();
}

/// Two frames that are not sampled change places.
fn swapped_pair(frames: &mut [Vec<f32>]) {
    frames.swap(5, 6);
}

/// The centre sampled block of the middle frame, replaced by noise of the
/// block's own mean and strength.
fn garbage_block(frames: &mut [Vec<f32>]) {
    let frame = &mut frames[MIDDLE];
    let mut mean = [0.0f32; 3];
    for y in BLOCK_AT.1..BLOCK_AT.1 + 16 {
        for x in BLOCK_AT.0..BLOCK_AT.0 + 16 {
            for (m, c) in mean.iter_mut().zip(pixel(frame, x, y).iter()) {
                *m += c / 256.0;
            }
        }
    }
    // Noise that is the same over 4 × 4 pixels, so it has a low-frequency
    // picture of its own, and that sums to nothing over the block.
    let noise = |x: u32, y: u32| hash(x / 4, y / 4, 99);
    let offset = (0..16)
        .flat_map(|y| (0..16).map(move |x| (x, y)))
        .map(|(x, y)| noise(BLOCK_AT.0 + x, BLOCK_AT.1 + y))
        .sum::<f32>()
        / 256.0;
    for y in BLOCK_AT.1..BLOCK_AT.1 + 16 {
        for x in BLOCK_AT.0..BLOCK_AT.0 + 16 {
            let v = 0.5 * (noise(x, y) - offset);
            let value = mean.map(|m| m + v);
            pixel(frame, x, y).copy_from_slice(&value);
        }
    }
}

/// A green block of 128 × 128 pixels in a frame that is not sampled, away
/// from the sampled places.
fn foreign_block(frames: &mut [Vec<f32>]) {
    let frame = &mut frames[3];
    for y in 150..278 {
        for x in 760..888 {
            pixel(frame, x, y).copy_from_slice(&[0.0, 0.8, 0.0]);
        }
    }
}

type Tamper = fn(&mut [Vec<f32>]);

/// A wrong picture, and what the check says of it. `local` cases damage one
/// block of a frame that a long-GOP codec predicts: a skipped or
/// intra-predicted block shows a picture that is not the frame's own at a
/// low rate, so those codecs note it and keep the file; ProRes, which codes
/// each block from its own samples, refuses it.
struct Case {
    what: &'static str,
    tamper: Tamper,
    expected: &'static [&'static str],
    local: bool,
}

fn tampers() -> Vec<Case> {
    vec![
        Case {
            what: "a level shift of 10 codes",
            tamper: level_shift,
            expected: &["luma is +", "from what was rendered in every frame"],
            local: false,
        },
        Case {
            what: "a range mistake",
            tamper: range_mistake,
            expected: &["luma's range is scaled by 0.86 in every frame"],
            local: false,
        },
        Case {
            what: "a picture one pixel to the right",
            tamper: one_pixel_shift,
            expected: &["displaced by (1, 0)"],
            local: false,
        },
        Case {
            what: "a flattened block",
            tamper: flattened_block,
            expected: &["frame 4 at (624, 352): a block lost its detail"],
            local: true,
        },
        Case {
            what: "a stale frame",
            tamper: stale_frame,
            expected: &["frame 2 shows the picture of frame 1"],
            local: false,
        },
        Case {
            what: "a swapped pair of frames",
            tamper: swapped_pair,
            expected: &[
                "frame 5 shows the picture of frame 6",
                "frame 6 shows the picture of frame 5",
            ],
            local: false,
        },
        Case {
            what: "a garbage block",
            tamper: garbage_block,
            expected: &["frame 4 at (624, 352): a block holds a picture that was not rendered"],
            local: true,
        },
        Case {
            what: "a foreign block away from the sampled places",
            tamper: foreign_block,
            expected: &["frame 3 is not the picture that was rendered"],
            local: false,
        },
    ]
}

/// The check's verdict on `problems`/`notes` for `case` in `format`.
fn judged(format: Format, case: &Case, problems: &[String], notes: &[String]) -> Option<String> {
    let intra = matches!(format.codec, Codec::ProRes(_));
    // Ten codes of 10 bits are within what quantisation at 0.1 Mb/s does to
    // every frame of a correct H.264 or HEVC Main10 file (up to five codes
    // of each, `hard_delivery.rs`): not certain, so delivered.
    if case.tamper as usize == level_shift as usize && !intra {
        return (!problems.is_empty()).then(|| {
            format!(
                "{}: {} is within the codec's quantisation and was refused: {problems:?}",
                format.name, case.what
            )
        });
    }
    if case.local && !intra {
        return (!problems.is_empty() || notes.is_empty()).then(|| {
            format!(
                "{}: {} was to be noted with the file kept: {problems:?} {notes:?}",
                format.name, case.what
            )
        });
    }
    let found = case
        .expected
        .iter()
        .all(|e| problems.iter().any(|p| p.contains(e)));
    (!found).then(|| {
        format!(
            "{}: {} was not refused for it: {problems:?}",
            format.name, case.what
        )
    })
}

/// A picture whose frames differ only in colour: mid-grey luma under fine
/// grain, the left half bluish and the right half yellowish at the same
/// luma (Cb ±0.1, Cr neutral), the halves changing sides from frame to
/// frame. Every frame has the same luma grid and the same channel means.
fn colour_halves(n: u64) -> Vec<f32> {
    // Rec. 709 R′G′B′ of Y′ = 0.5, Cb = ±0.1, Cr = 0.
    let tint = |cb: f32| [0.5, 0.5 - 0.1873 * cb, 0.5 + 1.8556 * cb];
    (0..H)
        .flat_map(|y| (0..W).map(move |x| (x, y)))
        .flat_map(|(x, y)| {
            let left = x < W / 2;
            let cb = if left == n.is_multiple_of(2) {
                0.1
            } else {
                -0.1
            };
            let grain = 0.04 * hash(x, y, n);
            let [r, g, b] = tint(cb).map(|c| c + grain);
            [r, g, b, 1.0]
        })
        .collect()
}

/// Frame 2 with its colour gone: each pixel's R′G′B′ set to its own luma.
fn colour_lost(frames: &mut [Vec<f32>]) {
    for p in frames[2].chunks_exact_mut(4) {
        let luma = 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
        p[..3].fill(luma);
    }
}

/// A grey picture under grain whose alpha is 0.3 on one half and 0.9 on the
/// other, the halves changing sides from frame to frame (premultiplied
/// working values, as the renderer hands them).
fn alpha_halves(n: u64) -> Vec<f32> {
    (0..H)
        .flat_map(|y| (0..W).map(move |x| (x, y)))
        .flat_map(|(x, y)| {
            let left = x < W / 2;
            let a = if left == n.is_multiple_of(2) {
                0.3
            } else {
                0.9
            };
            let v = 0.45 + 0.04 * hash(x, y, n);
            [v * a, v * a, v * a, a]
        })
        .collect()
}

/// Frame 2's alpha halves top and bottom instead of left and right: the
/// same average alpha, in other places.
fn alpha_misplaced(frames: &mut [Vec<f32>]) {
    for y in 0..H {
        for x in 0..W {
            let i = ((y * W + x) * 4) as usize;
            let p = &mut frames[2][i..i + 4];
            let a = if y < H / 2 { 0.3 } else { 0.9 };
            let v = p[0] / p[3];
            p.copy_from_slice(&[v * a, v * a, v * a, a]);
        }
    }
}

/// Frames that differ only in colour (or only in where their alpha is) are
/// each held to their own colour: a frame that lost its colour, a stale
/// frame and a swapped pair that keep every frame's luma and means are
/// refused, while the same frames encoded as they are are delivered.
#[test]
fn a_frame_with_lost_or_another_frames_colour_or_alpha_is_refused() {
    let dir = scratch("colour");
    let mut failures = vec![];
    let colour: [(&str, Tamper, &[u64]); 3] = [
        ("frame 2 without its colour", colour_lost, &[2]),
        ("a colour-only stale frame", stale_frame, &[2]),
        ("a colour-only swapped pair", swapped_pair, &[5, 6]),
    ];
    let alpha: [(&str, Tamper, &[u64]); 3] = [
        (
            "frame 2 with its alpha in other places",
            alpha_misplaced,
            &[2],
        ),
        ("an alpha-only stale frame", stale_frame, &[2]),
        ("an alpha-only swapped pair", swapped_pair, &[5, 6]),
    ];
    let runs = formats()
        .into_iter()
        .map(|f| (f, frames_of(colour_halves), &colour))
        .chain(
            alpha_formats()
                .into_iter()
                .map(|f| (f, frames_of(alpha_halves), &alpha)),
        );
    for (format, frames, cases) in runs {
        let intended = evidence(format, &frames);
        let right = write(format, &frames, &dir, "right");
        let (report, problems) = examine(format, &right, &intended);
        eprintln!("MEASURE {}, as encoded: {}", format.name, summary(&report));
        if !problems.is_empty() {
            failures.push(format!("{}: refused as encoded: {problems:?}", format.name));
        }
        for (what, tamper, wrong_frames) in cases.iter() {
            let mut wrong = frames.clone();
            tamper(&mut wrong);
            let path = write(format, &wrong, &dir, "wrong");
            let (report, problems) = examine(format, &path, &intended);
            eprintln!(
                "MEASURE {}, {what}: {} problems; {}; first: {}",
                format.name,
                problems.len(),
                summary(&report),
                problems.first().map_or("", |p| p.as_str())
            );
            for n in wrong_frames.iter() {
                if !problems.iter().any(|p| p.contains(&format!("frame {n} "))) {
                    failures.push(format!(
                        "{}: {what}: frame {n} was not refused: {problems:?}",
                        format.name
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
    let _ = std::fs::remove_dir_all(&dir);
}

fn summary(report: &VerifyReport) -> String {
    let p = &report.picture;
    let least = |values: Vec<f64>| values.into_iter().fold(f64::INFINITY, f64::min);
    let detail = least(report.pixels.iter().filter_map(|c| c.detail).collect());
    let foreign = report
        .pixels
        .iter()
        .map(|c| c.foreign_cv)
        .fold(0.0, f64::max);
    let opt = |v: Option<f64>| v.map_or("-".into(), |v| format!("{v:.2}"));
    format!(
        "{} frames: typical distance {:.2} CV, largest {:.2} (one cell {:.2}); identity ratio {}; least kept {}; gain off {}; a frame's means {:+.2} CV, most of the picture shared by every frame {:+.2} CV; blocks: mean {:.2} CV, least luma detail kept {detail:.2}, most foreign {foreign:.1} CV, registration {}",
        p.frames_checked,
        p.typical_cv,
        p.frame_distance_cv,
        p.frame_cell_cv,
        opt(p.identity),
        opt(p.kept),
        opt(p.gain),
        p.frame_bias_cv,
        p.level_cv,
        report.max_pixel_error_cv(),
        opt(p.registration),
    )
}

fn frames_of(picture: fn(u64) -> Vec<f32>) -> Vec<Vec<f32>> {
    (0..FRAMES).map(picture).collect()
}

#[test]
fn a_detailed_picture_is_delivered_and_every_wrong_one_is_refused() {
    let dir = scratch("detailed");
    let frames = frames_of(detailed);
    let mut failures = vec![];
    for format in formats() {
        let intended = evidence(format, &frames);
        let right = write(format, &frames, &dir, "right");
        let (report, problems) = examine(format, &right, &intended);
        eprintln!("MEASURE {}, as encoded: {}", format.name, summary(&report));
        if !problems.is_empty() {
            failures.push(format!("{}: refused as encoded: {problems:?}", format.name));
        }
        for case in tampers() {
            let mut wrong = frames.clone();
            (case.tamper)(&mut wrong);
            let path = write(format, &wrong, &dir, "wrong");
            let (report, problems) = examine(format, &path, &intended);
            eprintln!(
                "MEASURE {}, {}: {} problems, {} notes; {}; first: {}",
                format.name,
                case.what,
                problems.len(),
                report.notes.len(),
                summary(&report),
                problems
                    .first()
                    .or(report.notes.first())
                    .map_or("", |p| p.as_str())
            );
            failures.extend(judged(format, &case, &problems, &report.notes));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The review's case: blocks of equal numbers of two luma codes 200 apart
/// (activity 100 codes, so a block's mean has 15 codes of room), lifted by
/// ten codes, flattened, or moved a pixel.
#[test]
fn alternating_detail_does_not_hide_a_level_shift_a_flat_block_or_a_moved_picture() {
    let dir = scratch("alternating");
    let frames = frames_of(alternating);
    let mut failures = vec![];
    for format in formats()
        .into_iter()
        .filter(|f| matches!(f.codec, Codec::ProRes(_)))
    {
        let intended = evidence(format, &frames);
        let right = write(format, &frames, &dir, "right");
        let (report, problems) = examine(format, &right, &intended);
        eprintln!(
            "MEASURE {}, alternating codes as encoded: {}",
            format.name,
            summary(&report)
        );
        if !problems.is_empty() {
            failures.push(format!("{}: refused as encoded: {problems:?}", format.name));
        }
        // What the flavour keeps of pixel-alternating detail: Proxy at this
        // size keeps a tenth of it, so its file has no such detail to tell
        // a flat block or a moved picture by.
        let kept = report
            .pixels
            .iter()
            .filter_map(|p| p.detail)
            .fold(f64::INFINITY, f64::min);
        let carries = kept >= 0.5;
        if !carries && format.codec != Codec::ProRes(ProResProfile::Proxy) {
            failures.push(format!("{} keeps {kept:.2} of the detail", format.name));
        }
        let cases: [(&str, Tamper, &str, bool); 3] = [
            (
                "a level shift of 10 codes",
                level_shift,
                "from what was rendered in every frame",
                true,
            ),
            (
                "a flattened block",
                flattened_block,
                "a block lost its detail",
                carries,
            ),
            (
                "a picture one pixel to the right",
                one_pixel_shift,
                "displaced by",
                carries,
            ),
        ];
        for (what, tamper, expected, told) in cases {
            let mut wrong = frames.clone();
            tamper(&mut wrong);
            let path = write(format, &wrong, &dir, "wrong");
            let (report, problems) = examine(format, &path, &intended);
            eprintln!(
                "MEASURE {}, alternating codes, {what}: {} problems; {}; first: {}",
                format.name,
                problems.len(),
                summary(&report),
                problems.first().map_or("", |p| p.as_str())
            );
            if told && !problems.iter().any(|p| p.contains(expected)) {
                failures.push(format!(
                    "{}: {what} was not refused for it: {problems:?}",
                    format.name
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// What the check costs a frame: the evidence taken as a frame is encoded,
/// and the decode of every frame afterwards, against decoding three.
#[test]
#[ignore = "measurement: run with --ignored --nocapture to print the cost"]
fn measure_the_check_per_frame() {
    let dir = scratch("cost");
    const N: u64 = 24;
    for (w, h) in [(1920u32, 1080u32), (3840, 2160)] {
        let frames: Vec<Vec<f32>> = (0..N)
            .map(|n| {
                (0..h)
                    .flat_map(|y| (0..w).map(move |x| (x, y)))
                    .flat_map(|(x, y)| {
                        let bar = if (x + 40 * n as u32) % 640 < 80 {
                            0.2
                        } else {
                            0.0
                        };
                        let v = 0.25 + 0.4 * x as f32 / w as f32 + bar + 0.08 * hash(x, y, n);
                        [v, v * 0.9, 0.8 - v * 0.5, 1.0]
                    })
                    .collect()
            })
            .collect();
        for format in formats()
            .into_iter()
            .filter(|f| ["ProRes 422 HQ", "H.264 12 Mb/s", "HEVC Main10 12 Mb/s"].contains(&f.name))
        {
            let path = dir.join(format!("cost.{}", format.container.extension()));
            let mut spec = spec(format, path.clone());
            (spec.width, spec.height, spec.frames) = (w, h, N);
            // The encoder inputs, to time the evidence on its own.
            let mut pipeline = Pipeline::new(spec.clone(), MemorySink::default()).unwrap();
            let mut job = pm_export::export(spec.clone()).unwrap();
            for rgba in &frames {
                let frame = WorkingFrame {
                    width: w,
                    height: h,
                    rgba,
                };
                pipeline.push_frame(&frame).unwrap();
                job.push_frame(&frame).unwrap();
            }
            let finishing = std::time::Instant::now();
            job.finish().unwrap();
            let finish = finishing.elapsed().as_secs_f64() * 1e3;
            let inputs = pipeline.finish().unwrap().frames;
            let taking = std::time::Instant::now();
            let mut evidence = Evidence::default();
            for (n, input) in inputs.iter().enumerate() {
                evidence.add(n as u64, N, input);
            }
            let take = taking.elapsed().as_secs_f64() * 1e3 / N as f64;
            let examining = std::time::Instant::now();
            let (report, problems) =
                pm_export::mac::examine_file(&path, &spec, evidence, 0, None).unwrap();
            let examine = examining.elapsed().as_secs_f64() * 1e3;
            assert!(problems.is_empty(), "{problems:?}");
            assert_eq!(report.picture.frames_checked, N);
            // Before: the first, middle and last frame, each with its own reader.
            let sampling = std::time::Instant::now();
            for n in pm_export::verify::sampled_frames(N) {
                let frame = pm_export::mac::reader::decode_frame(
                    &path,
                    n,
                    spec.timing().unwrap(),
                    spec.input_format(),
                )
                .unwrap();
                std::hint::black_box(pm_export::verify::block_means(&frame, n));
            }
            let sample = sampling.elapsed().as_secs_f64() * 1e3;
            eprintln!(
                "MEASURE {w}x{h} {}, {N} frames: evidence taken at encode {take:.2} ms a frame; the file held to it {examine:.0} ms ({:.1} ms a frame, every frame decoded); three frames decoded as before {sample:.0} ms; the export's finish (encoder flush, write, check) {finish:.0} ms",
                format.name,
                examine / N as f64
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
