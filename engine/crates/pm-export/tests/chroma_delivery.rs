//! Chroma decimation of delivered video, through the job engine's export
//! pipeline (render, convert, encode, mux, verify) and read back through the
//! codec: thin saturated lines at even and odd columns and rows, and a line
//! moving one pixel a frame, exported to H.264 (`420v`), HEVC Main10 (`x420`)
//! and ProRes 422 HQ (`v216`).
//!
//! The reference is the declared band-limited decimation: the tent (bilinear)
//! kernel of a 2:1 reduction sited where each format's chroma sits: across,
//! co-sited `[1, 2, 1] / 4` on the even column; down (4:2:0, siting type 0),
//! `[1, 3, 3, 1] / 8` centred between rows `2y` and `2y + 1`. Each tap set
//! weighs even and odd source pixels equally (½ each), so a line's chroma is
//! carried with the same weight at either parity and a moving line does not
//! flicker; no tap is negative, so no edge gets a complementary fringe.
//!
//! Untouched footage is the exception: a 4:2:0 clip drawn 1:1 over the whole
//! frame is decimated by the exact inverse of the renderer's upsampling, so
//! it passes through with its own chroma.

#![cfg(target_os = "macos")]

mod support;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use pm_export::convert::{EncoderInput, PlaneData, WorkingFrame, rgb_to_ycbcr};
use pm_export::mac::reader::decode_frame;
use pm_export::pipeline::test_sinks::MemorySink;
use pm_export::render::*;
use pm_export::timescale::video_timing;
use pm_export::*;
use pm_protocol::color::{OutputColor, Precision, QuickTimeTag};
use pm_protocol::common::{FrameRate, Size, TICKS_PER_SECOND, TickRange, Ticks};
use pm_protocol::priority::{
    AlphaOutput as WireAlpha, ExportFormat, ExportSettings, ProResProfile as WireProRes,
    QualityTier,
};
use pm_render::Renderer;
use pm_time::Rate;
use serde_json::{Value, json};
use support::doc::*;

const W: u32 = 128;
const H: u32 = 64;
const RED: [f64; 3] = [1.0, 0.0, 0.0];

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-export-chroma-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[derive(Clone, Copy, Debug)]
struct Format {
    wire: ExportFormat,
    input: InputFormat,
    /// Decoded chroma may differ from the reference by this many codes of
    /// the format's own depth (codec loss on a tiny, saturated frame).
    tolerance: f64,
}

fn formats() -> [Format; 3] {
    [
        Format {
            wire: ExportFormat::H264,
            input: InputFormat::V420,
            tolerance: 4.0,
        },
        Format {
            wire: ExportFormat::Hevc {
                ten_bit: true,
                container: pm_protocol::priority::Container::Mov,
            },
            input: InputFormat::X420,
            tolerance: 4.0,
        },
        Format {
            wire: ExportFormat::ProRes {
                profile: WireProRes::Hq,
            },
            input: InputFormat::V216,
            tolerance: 2.0,
        },
    ]
}

fn settings(format: ExportFormat, frames: u64) -> ExportSettings {
    ExportSettings {
        format,
        alpha: WireAlpha::None,
        quality: QualityTier::Max,
        size: Size::new(W, H),
        frame_rate: FrameRate::new(24, 1),
        range: TickRange {
            start: Ticks::ZERO,
            end: Ticks::new(frames as i64 * TICKS_PER_SECOND / 24).unwrap(),
        },
        audio: false,
        audio_index_playhead: None,
        motion_blur: false,
        color: OutputColor::Rec709QuickTime {
            candidate: QuickTimeTag::Cicp1_13_1,
        },
        precision: Precision::Float32,
        dither: false,
        draft_from_proxies: false,
        delivery: Default::default(),
    }
}

/// Held keyframes: the value `v(n)` from frame `n` on (24 fps).
fn held(frames: u64, v: impl Fn(u64) -> f64) -> Value {
    let kf: Vec<Value> = (0..frames)
        .map(|n| {
            json!({ "t": n as f64 / 24.0, "v": v(n), "i": format!("k{n}"),
                    "eo": [0.333, 0.0], "ei": [0.667, 1.0], "hold": true })
        })
        .collect();
    json!({ "v": v(0), "kf": kf, "expr": null })
}

/// A 1 px red line over black (a solid placed by its top-left corner):
/// vertical at column `x(n)` in frame `n`, or horizontal at row `y(n)`.
#[derive(Clone, Copy)]
enum Line {
    Column(fn(u64) -> u32),
    Row(fn(u64) -> u32),
}

fn document(line: Line, frames: u64) -> Value {
    let dur = frames as f64 / 24.0;
    let (w, h, x, y) = match line {
        Line::Column(_) => (1, H, 0.0, 0.0),
        Line::Row(_) => (W, 1, 0.0, 0.0),
    };
    let mut l = layer(
        "line",
        "solid",
        json!({ "color": "#FF0000", "w": w, "h": h }),
        x,
        y,
        dur,
    );
    match line {
        Line::Column(at) => set(&mut l, "position.x", held(frames, |n| f64::from(at(n)))),
        Line::Row(at) => set(&mut l, "position.y", held(frames, |n| f64::from(at(n)))),
    }
    let mut doc = project(W, H, dur, no_fill(), vec![l]);
    doc["fps"] = json!(24);
    doc
}

fn export(line: Line, frames: u64, format: Format, name: &str) -> Vec<EncoderInput> {
    let dir = scratch(name);
    let destination = dir.join(match format.wire {
        ExportFormat::H264 => "out.mp4",
        _ => "out.mov",
    });
    let job = ExportJob {
        document: Arc::new(load(&document(line, frames))),
        composition: "Ptest-root".into(),
        settings: settings(format.wire, frames),
        media: Arc::new(MediaPaths::default()),
        destination: destination.clone(),
        overwrite: Overwrite::Fail,
        mode: None,
        seed: 3,
    };
    let mut renderer = Renderer::headless().unwrap();
    run_export(
        &mut renderer,
        &job,
        &Unrestricted,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap_or_else(|e| panic!("{name}: {e}"));
    let decoded = (0..frames)
        .map(|n| {
            decode_frame(
                &destination,
                n,
                video_timing(Rate::FPS_24).unwrap(),
                format.input,
            )
            .unwrap()
        })
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    decoded
}

/// A decoded frame's planes in normalized units: luma per pixel, (Cb, Cr) per
/// chroma sample, and the chroma grid's size.
struct Planes {
    luma: Vec<f64>,
    chroma: Vec<[f64; 2]>,
    cw: usize,
    ch: usize,
    /// One code of the format's chroma, normalized.
    code: f64,
}

fn planes(input: &EncoderInput) -> Planes {
    let (w, h) = (input.width as usize, input.height as usize);
    let u8s = |k: usize| match &input.planes[k].data {
        PlaneData::U8(v) => v.clone(),
        PlaneData::U16(_) => panic!("8-bit plane expected"),
    };
    let u16s = |k: usize| match &input.planes[k].data {
        PlaneData::U16(v) => v.clone(),
        PlaneData::U8(_) => panic!("16-bit plane expected"),
    };
    match input.format {
        InputFormat::V420 => {
            let (l, c) = (u8s(0), u8s(1));
            Planes {
                luma: l.iter().map(|&v| (f64::from(v) - 16.0) / 219.0).collect(),
                chroma: c
                    .chunks_exact(2)
                    .map(|p| [p[0], p[1]].map(|v| (f64::from(v) - 128.0) / 224.0))
                    .collect(),
                cw: w / 2,
                ch: h / 2,
                code: 1.0 / 224.0,
            }
        }
        InputFormat::X420 => {
            let (l, c) = (u16s(0), u16s(1));
            Planes {
                luma: l
                    .iter()
                    .map(|&v| (f64::from(v >> 6) - 64.0) / 876.0)
                    .collect(),
                chroma: c
                    .chunks_exact(2)
                    .map(|p| [p[0], p[1]].map(|v| (f64::from(v >> 6) - 512.0) / 896.0))
                    .collect(),
                cw: w / 2,
                ch: h / 2,
                code: 1.0 / 896.0,
            }
        }
        InputFormat::V216 => {
            let p = u16s(0);
            let y = |v: u16| (f64::from(v) - 4096.0) / 56064.0;
            let c = |v: u16| (f64::from(v) - 32768.0) / 57344.0;
            let mut luma = Vec::with_capacity(w * h);
            let mut chroma = Vec::with_capacity(w * h / 2);
            for q in p.chunks_exact(4) {
                luma.extend([y(q[1]), y(q[3])]);
                chroma.push([c(q[0]), c(q[2])]);
            }
            Planes {
                luma,
                chroma,
                cw: w / 2,
                ch: h,
                code: 64.0 / 57344.0,
            }
        }
        other => panic!("{other:?} is not a subsampled video input"),
    }
}

/// Tent decimation of a full-resolution channel to the format's chroma grid,
/// with edge samples repeated.
fn reference(full: impl Fn(usize, usize) -> f64, cw: usize, ch: usize) -> Vec<f64> {
    let (w, h) = (W as i64, H as i64);
    let across = [(-1i64, 0.25), (0, 0.5), (1, 0.25)];
    let down: &[(i64, f64)] = if ch == H as usize {
        &[(0, 1.0)]
    } else {
        &[(-1, 0.125), (0, 0.375), (1, 0.375), (2, 0.125)]
    };
    let mut out = Vec::with_capacity(cw * ch);
    for cy in 0..ch as i64 {
        let row = if ch == H as usize { cy } else { 2 * cy };
        for cx in 0..cw as i64 {
            let mut acc = 0.0;
            for &(dy, wy) in down {
                for &(dx, wx) in &across {
                    let x = (2 * cx + dx).clamp(0, w - 1) as usize;
                    let y = (row + dy).clamp(0, h - 1) as usize;
                    acc += wx * wy * full(x, y);
                }
            }
            out.push(acc);
        }
    }
    out
}

struct Measured {
    /// Largest difference from the reference, in codes.
    worst: f64,
    /// Lowest Cr anywhere, in codes (red's Cr is positive: a negative value is
    /// a complementary fringe).
    fringe: f64,
    /// Sum of Cr over the chroma samples, in codes, per chroma row (a vertical
    /// line) or per chroma column (a horizontal line), averaged.
    carried: f64,
}

fn measure(decoded: &EncoderInput, line: Line, n: u64) -> Measured {
    let p = planes(decoded);
    let red = rgb_to_ycbcr(RED);
    let on = |x: usize, y: usize| match line {
        Line::Column(at) => x == at(n) as usize,
        Line::Row(at) => y == at(n) as usize,
    };
    // The line is where it should be: red's luma on it, black beside it.
    for y in 0..H as usize {
        for x in 0..W as usize {
            let want = if on(x, y) { red[0] } else { 0.0 };
            let got = p.luma[y * W as usize + x];
            assert!(
                (got - want).abs() < 6.0 / 219.0,
                "luma at {x},{y} frame {n}: {got} against {want}; lit {:?}",
                (0..W as usize * H as usize)
                    .filter(|&i| p.luma[i] > 0.05)
                    .map(|i| (i % W as usize, i / W as usize))
                    .take(8)
                    .collect::<Vec<_>>()
            );
        }
    }
    let cb = reference(|x, y| if on(x, y) { red[1] } else { 0.0 }, p.cw, p.ch);
    let cr = reference(|x, y| if on(x, y) { red[2] } else { 0.0 }, p.cw, p.ch);
    let mut worst = 0.0f64;
    for (i, got) in p.chroma.iter().enumerate() {
        worst = worst
            .max((got[0] - cb[i]).abs() / p.code)
            .max((got[1] - cr[i]).abs() / p.code);
    }
    let fringe = p.chroma.iter().map(|c| c[1] / p.code).fold(0.0, f64::min);
    let total: f64 = p.chroma.iter().map(|c| c[1] / p.code).sum();
    let carried = match line {
        Line::Column(_) => total / p.ch as f64,
        Line::Row(_) => total / p.cw as f64,
    };
    Measured {
        worst,
        fringe,
        carried,
    }
}

fn check(line: Line, frames: u64, label: &str) {
    for format in formats() {
        let name = format!("{label}-{:?}", format.input).to_lowercase();
        let decoded = export(line, frames, format, &name);
        let measured: Vec<Measured> = (0..frames)
            .map(|n| measure(&decoded[n as usize], line, n))
            .collect();
        let worst = measured.iter().map(|m| m.worst).fold(0.0, f64::max);
        let fringe = measured.iter().map(|m| m.fringe).fold(0.0, f64::min);
        let carried: Vec<f64> = measured.iter().map(|m| m.carried).collect();
        let spread = carried.iter().cloned().fold(f64::MIN, f64::max)
            - carried.iter().cloned().fold(f64::MAX, f64::min);
        eprintln!(
            "MEASURE chroma {label} {:?}: worst {worst:.1} codes from the tent reference, lowest Cr {fringe:.1}, Cr carried per frame {carried:.1?} (spread {spread:.1})",
            format.input
        );
        assert!(
            worst <= format.tolerance,
            "{label} {:?}: chroma {worst:.1} codes from the band-limited reference",
            format.input
        );
        assert!(
            fringe >= -format.tolerance,
            "{label} {:?}: a complementary fringe of {fringe:.1} codes",
            format.input
        );
        assert!(
            spread <= format.tolerance,
            "{label} {:?}: the chroma a line carries depends on its parity ({carried:.1?})",
            format.input
        );
    }
}

#[test]
fn one_pixel_columns_keep_their_colour_at_either_parity() {
    // Frame 0 at an even column, frame 1 at an odd one.
    check(Line::Column(|n| 40 + n as u32), 2, "columns");
}

#[test]
fn one_pixel_rows_keep_their_colour_at_either_parity() {
    check(Line::Row(|n| 20 + n as u32), 2, "rows");
}

#[test]
fn a_line_moving_a_pixel_a_frame_does_not_flicker() {
    check(Line::Column(|n| 61 + n as u32), 6, "moving");
}

/// Detailed colour: a different saturated hue every pixel or two.
fn busy(x: u32, y: u32, n: u64) -> [f32; 3] {
    let h = (x * 7 + y * 13 + n as u32 * 5) % 6;
    let v = if (x + y).is_multiple_of(3) { 0.9 } else { 0.35 };
    match h {
        0 => [v, 0.1, 0.1],
        1 => [0.1, v, 0.1],
        2 => [0.1, 0.1, v],
        3 => [v, v, 0.1],
        4 => [0.1, v, v],
        _ => [v, 0.1, v],
    }
}

#[test]
fn untouched_420_footage_keeps_its_chroma_and_a_moved_clip_is_filtered() {
    let dir = scratch("footage");
    // A 4:2:0 H.264 source (Rec.709, chroma type 0) written by the exporter.
    let source = dir.join("source.mp4");
    let frames = 2u64;
    let spec = ExportSpec {
        width: W,
        height: H,
        rate: Rate::FPS_24,
        frames,
        codec: Codec::H264 {
            bitrate_bps: 40_000_000,
        },
        container: Container::Mp4,
        tags: ColorTags::Rec709QuickTime,
        alpha: AlphaOutput::None,
        working: WorkingEncoding::Standard,
        dither: Dither::Off,
        audio: None,
        timecode: None,
        destination: source.clone(),
        overwrite: Overwrite::Replace,
        mode: None,
    };
    let mut writer = pm_export::export(spec).unwrap();
    for n in 0..frames {
        let rgba: Vec<f32> = (0..H)
            .flat_map(|y| {
                (0..W).flat_map(move |x| {
                    let [r, g, b] = busy(x, y, n);
                    [r, g, b, 1.0]
                })
            })
            .collect();
        writer
            .push_frame(&WorkingFrame {
                width: W,
                height: H,
                rgba: &rgba,
            })
            .unwrap();
    }
    writer.finish().unwrap();
    let decoded = decode_frame(
        &source,
        0,
        video_timing(Rate::FPS_24).unwrap(),
        InputFormat::V420,
    )
    .unwrap();
    let reference = planes(&decoded);

    let deliver = |x: f64| {
        let dur = frames as f64 / 24.0;
        let mut doc = project(
            W,
            H,
            dur,
            no_fill(),
            vec![layer(
                "v",
                "video",
                json!({ "asset": "clip", "fit": "stretch", "w": W, "h": H, "trim": 0, "speed": 1 }),
                x,
                f64::from(H) / 2.0,
                dur,
            )],
        );
        doc["fps"] = json!(24);
        doc["colorDepth"] = json!(32);
        asset(&mut doc, "clip", "video", "mp4", W, H, dur);
        let mut media = MediaPaths::default();
        media.insert("clip", source.clone());
        let job = ExportJob {
            document: Arc::new(load(&doc)),
            composition: "Ptest-root".into(),
            settings: settings(ExportFormat::H264, frames),
            media: Arc::new(media),
            destination: PathBuf::from("/unused.mp4"),
            overwrite: Overwrite::Fail,
            mode: None,
            seed: 3,
        };
        let plan = plan_export(&job).unwrap();
        let mut renderer = Renderer::headless().unwrap();
        let out = run_export_into(
            &mut renderer,
            &job,
            plan,
            MemorySink::default(),
            &Unrestricted,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();
        planes(&out.frames[0])
    };
    let worst = |p: &Planes| {
        p.chroma
            .iter()
            .zip(&reference.chroma)
            .map(|(a, b)| ((a[0] - b[0]).abs().max((a[1] - b[1]).abs()) / p.code).round())
            .fold(0.0, f64::max)
    };
    // Drawn 1:1 (a video layer is placed by its centre): its own chroma.
    let untouched = deliver(f64::from(W) / 2.0);
    let luma_exact = untouched
        .luma
        .iter()
        .zip(&reference.luma)
        .all(|(a, b)| ((a - b) * 219.0).abs() < 0.5);
    eprintln!(
        "MEASURE untouched 4:2:0 footage through the job: luma exact {luma_exact}, chroma within {} CV",
        worst(&untouched)
    );
    assert!(luma_exact);
    assert!(worst(&untouched) <= 1.0);
    // Moved by one pixel the clip is resampled: rendered, so filtered (its
    // chroma no longer equals the source's sample for sample).
    let moved = deliver(f64::from(W) / 2.0 + 1.0);
    eprintln!(
        "MEASURE moved clip: chroma {} CV from the source",
        worst(&moved)
    );
    assert!(worst(&moved) > 8.0);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// One chroma decimation for each continuous piece of picture
// ---------------------------------------------------------------------------

/// A source clip written by the exporter: `picture(x, y)` in every frame
/// (intra-coded, so every frame decodes to the same values).
fn still_clip(path: &std::path::Path, codec: Codec, picture: impl Fn(u32, u32) -> [f32; 3]) {
    let frames = 8u64;
    let spec = ExportSpec {
        width: W,
        height: H,
        rate: Rate::FPS_24,
        frames,
        codec,
        container: Container::Mov,
        tags: ColorTags::Rec709QuickTime,
        alpha: AlphaOutput::None,
        working: WorkingEncoding::Standard,
        dither: Dither::Off,
        audio: None,
        timecode: None,
        destination: path.to_path_buf(),
        overwrite: Overwrite::Replace,
        mode: None,
    };
    let rgba: Vec<f32> = (0..H)
        .flat_map(|y| (0..W).map(move |x| (x, y)))
        .flat_map(|(x, y)| {
            let [r, g, b] = picture(x, y);
            [r, g, b, 1.0]
        })
        .collect();
    let mut writer = pm_export::export(spec).unwrap();
    for _ in 0..frames {
        writer
            .push_frame(&WorkingFrame {
                width: W,
                height: H,
                rgba: &rgba,
            })
            .unwrap();
    }
    writer.finish().unwrap();
}

/// A video layer showing `asset` 1:1 over the whole frame from frame `from`
/// for `frames` frames.
fn clip_layer(id: &str, asset: &str, from: u64, frames: u64) -> Value {
    let mut l = layer(
        id,
        "video",
        json!({ "asset": asset, "fit": "stretch", "w": W, "h": H, "trim": 0, "speed": 1 }),
        f64::from(W) / 2.0,
        f64::from(H) / 2.0,
        frames as f64 / 24.0,
    );
    l["from"] = json!(from as f64 / 24.0);
    l
}

/// The encoder input of every frame of `layers` (bottom to top) exported to
/// ProRes 422 HQ through the job engine.
fn deliver(layers: Vec<Value>, clips: &[(&str, &std::path::Path)], frames: u64) -> Vec<Planes> {
    deliver_as(
        layers,
        clips,
        settings(
            ExportFormat::ProRes {
                profile: WireProRes::Hq,
            },
            frames,
        ),
        |_| {},
    )
}

fn deliver_as(
    layers: Vec<Value>,
    clips: &[(&str, &std::path::Path)],
    settings: ExportSettings,
    edit: impl Fn(&mut Value),
) -> Vec<Planes> {
    let (delivered, _) = deliver_and_filter(layers, clips, settings, edit, None);
    delivered.iter().map(planes).collect()
}

/// Every frame's encoder input as the job delivers it, and the same
/// rendered (and postprocessed) frames decimated band-limited.
fn deliver_and_filter(
    layers: Vec<Value>,
    clips: &[(&str, &std::path::Path)],
    settings: ExportSettings,
    edit: impl Fn(&mut Value),
    post: Option<Arc<dyn FramePost>>,
) -> (Vec<EncoderInput>, Vec<EncoderInput>) {
    let dur = 8.0 / 24.0;
    // The document lists layers top first.
    let mut doc = project(W, H, dur, no_fill(), layers.into_iter().rev().collect());
    doc["fps"] = json!(24);
    doc["colorDepth"] = json!(32);
    let mut media = MediaPaths::default();
    for (id, path) in clips {
        asset(&mut doc, id, "video", "mov", W, H, dur);
        media.insert(*id, path.to_path_buf());
    }
    edit(&mut doc);
    let job = ExportJob {
        document: Arc::new(load(&doc)),
        composition: "Ptest-root".into(),
        settings,
        media: Arc::new(media),
        destination: PathBuf::from("/unused.mov"),
        overwrite: Overwrite::Fail,
        mode: None,
        seed: 3,
    };
    let planned = || {
        let mut plan = plan_export(&job).unwrap_or_else(|e| panic!("{e}"));
        plan.frame_post = post.clone();
        plan
    };
    let mut renderer = Renderer::headless().unwrap();
    let cancel = AtomicBool::new(false);
    let delivered = run_export_into(
        &mut renderer,
        &job,
        planned(),
        MemorySink::default(),
        &Unrestricted,
        &cancel,
        &mut |_| {},
    )
    .unwrap()
    .frames;
    let plan = planned();
    let size = plan.request.output_size;
    let filtered = (0..plan.timeline.frames)
        .map(|n| {
            let time = plan.timeline.tick(n);
            let read = render_frame(
                &mut renderer,
                &plan.document,
                &job.composition,
                time,
                &plan.request,
                &job.media,
                &Unrestricted,
                pass_key(job.seed, size),
                &cancel,
            )
            .unwrap();
            let rgba = match &post {
                None => read.rgba,
                Some(post) => post
                    .frame(n, time, W, H, plan.spec.working, read.rgba)
                    .unwrap(),
            };
            pm_export::convert::convert_tagged(
                &WorkingFrame {
                    width: W,
                    height: H,
                    rgba: &rgba,
                },
                plan.spec.input_format(),
                &pm_export::convert::ConvertParams {
                    working: plan.spec.working,
                    alpha: plan.spec.alpha,
                    tags: plan.spec.tags,
                    dither: false,
                    frame: n as u32,
                },
                plan.spec.tags,
                pm_export::convert::Chroma::Filtered,
            )
            .unwrap()
        })
        .collect();
    (delivered, filtered)
}

/// The largest chroma difference between two frames, in codes, over the
/// chroma samples `keep` selects, with `b` scaled by `gain`.
fn chroma_step(a: &Planes, b: &Planes, gain: f64, keep: impl Fn(usize, usize) -> bool) -> f64 {
    let mut worst = 0.0f64;
    for cy in 0..a.ch {
        for cx in 0..a.cw {
            if keep(cx, cy) {
                let (p, q) = (a.chroma[cy * a.cw + cx], b.chroma[cy * a.cw + cx]);
                worst = worst
                    .max((p[0] - gain * q[0]).abs() / a.code)
                    .max((p[1] - gain * q[1]).abs() / a.code);
            }
        }
    }
    worst
}

fn busy_still(x: u32, y: u32) -> [f32; 3] {
    busy(x, y, 0)
}

/// The decoded 4:2:2 source's own chroma.
fn source_planes(path: &std::path::Path) -> Planes {
    planes(
        &decode_frame(
            path,
            0,
            video_timing(Rate::FPS_24).unwrap(),
            InputFormat::V216,
        )
        .unwrap(),
    )
}

#[test]
fn a_fade_reaching_full_opacity_does_not_change_chroma_sharpness() {
    let dir = scratch("fade");
    let source = dir.join("clip.mov");
    still_clip(&source, Codec::ProRes(ProResProfile::Hq), busy_still);
    // 96 % to 100 % over black, then held: one continuous piece of picture.
    let opacity = |n: u64| (96 + n).min(100) as f64;
    let frames = 8;
    let mut clip = clip_layer("v", "clip", 0, frames);
    set(&mut clip, "opacity", held(frames, opacity));
    let out = deliver(vec![clip], &[("clip", &source)], frames);
    let steps: Vec<f64> = (0..frames as usize - 1)
        .map(|n| {
            let gain = opacity(n as u64) / opacity(n as u64 + 1);
            chroma_step(&out[n], &out[n + 1], gain, |_, _| true)
        })
        .collect();
    eprintln!(
        "MEASURE fade 96..100 % then held, 4:2:2 footage to ProRes 422 HQ: chroma step between frames beyond the opacity change {steps:.1?} codes (10-bit)"
    );
    for (n, step) in steps.iter().enumerate() {
        assert!(
            *step <= 1.5,
            "chroma changes by {step:.1} codes between frames {n} and {} beyond the fade itself",
            n + 1
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_title_entering_over_footage_does_not_change_the_footage_chroma() {
    let dir = scratch("title");
    let source = dir.join("clip.mov");
    still_clip(&source, Codec::ProRes(ProResProfile::Hq), busy_still);
    let frames = 8;
    // A 16 × 8 title in the top-left corner from frame 3 to frame 5.
    let mut title = layer(
        "title",
        "solid",
        json!({ "color": "#FFFFFF", "w": 16, "h": 8 }),
        0.0,
        0.0,
        3.0 / 24.0,
    );
    title["from"] = json!(3.0 / 24.0);
    let out = deliver(
        vec![clip_layer("v", "clip", 0, frames), title],
        &[("clip", &source)],
        frames,
    );
    // Away from the title the picture is the same in every frame.
    let away = |cx: usize, cy: usize| cx >= 12 || cy >= 12;
    let steps: Vec<f64> = (0..frames as usize - 1)
        .map(|n| chroma_step(&out[n], &out[n + 1], 1.0, away))
        .collect();
    eprintln!(
        "MEASURE title over footage in frames 3..6 of 8: footage chroma step between frames {steps:.1?} codes (10-bit)"
    );
    assert!(
        steps.iter().all(|s| *s <= 1.0),
        "footage chroma changes as the title enters or leaves: {steps:.1?}"
    );
    // The title is in the picture: the frames differ under it.
    assert!(chroma_step(&out[2], &out[3], 1.0, |cx, cy| cx < 6 && cy < 6) > 8.0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_untouched_clip_keeps_its_chroma_up_to_a_cut_to_a_touched_one() {
    let dir = scratch("cut");
    let (a, b) = (dir.join("a.mov"), dir.join("b.mov"));
    still_clip(&a, Codec::ProRes(ProResProfile::Hq), busy_still);
    still_clip(&b, Codec::ProRes(ProResProfile::Hq), |x, y| {
        busy(x + 3, y + 1, 2)
    });
    let frames = 8;
    // Clip A alone for four frames, then a cut to clip B, which fades in
    // its last two frames (keyframe times count from the layer's start).
    let mut second = clip_layer("b", "other", 4, 4);
    set(
        &mut second,
        "opacity",
        held(4, |n| if n < 2 { 100.0 } else { 90.0 }),
    );
    let out = deliver(
        vec![clip_layer("a", "clip", 0, 4), second],
        &[("clip", &a), ("other", &b)],
        frames,
    );
    let own = source_planes(&a);
    let first: Vec<f64> = (0..4)
        .map(|n| chroma_step(&out[n], &own, 1.0, |_, _| true))
        .collect();
    eprintln!("MEASURE untouched clip before a cut: chroma from its source {first:.1?} codes");
    assert!(
        first.iter().all(|s| *s == 0.0),
        "the untouched clip's chroma is its source's, exactly: {first:.1?}"
    );
    // The second clip is one piece of picture: filtered from its first frame.
    let other = source_planes(&b);
    let second: Vec<f64> = (4..8)
        .map(|n| chroma_step(&out[n], &other, 1.0, |_, _| true))
        .collect();
    eprintln!(
        "MEASURE the clip after the cut, fading in its last frames: chroma from its source {second:.1?} codes"
    );
    assert!(second[0] > 8.0, "{second:.1?}");
    assert!(chroma_step(&out[4], &out[5], 1.0, |_, _| true) <= 1.0);
    assert!(
        chroma_step(&out[5], &out[7], 1.0, |_, _| true) > 4.0,
        "the fade is in the file"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// The inverse only when delivery leaves the footage's colour as it is
// ---------------------------------------------------------------------------

/// The largest difference between two encoder inputs' samples, in the
/// format's own codes, and whether every sample is equal.
fn planes_differ(a: &EncoderInput, b: &EncoderInput) -> f64 {
    let (a, b) = (planes(a), planes(b));
    let luma = a
        .luma
        .iter()
        .zip(&b.luma)
        .map(|(p, q)| (p - q).abs() / a.code)
        .fold(0.0, f64::max);
    luma.max(chroma_step(&a, &b, 1.0, |_, _| true))
}

/// A 4:2:0 Rec.709 clip of fine coloured detail, alone and untouched.
fn sdr_420_clip(dir: &std::path::Path) -> PathBuf {
    let source = dir.join("clip.mov");
    still_clip(
        &source,
        Codec::H264 {
            bitrate_bps: 40_000_000,
        },
        busy_still,
    );
    source
}

fn hdr_project(doc: &mut Value) {
    doc["color"] = json!({ "mode": "hdr", "masteringPeak": 1000 });
}

/// An untouched SDR clip in an HDR project, delivered with `color`: the
/// delivery changes its colour, so its chroma is filtered like any
/// rendered frame's.
fn sdr_footage_is_filtered(name: &str, format: ExportFormat, color: OutputColor) {
    let dir = scratch(name);
    let source = sdr_420_clip(&dir);
    let frames = 4;
    let mut s = settings(format, frames);
    s.color = color;
    let (delivered, filtered) = deliver_and_filter(
        vec![clip_layer("v", "clip", 0, frames)],
        &[("clip", &source)],
        s,
        hdr_project,
        None,
    );
    let worst = delivered
        .iter()
        .zip(&filtered)
        .map(|(d, f)| planes_differ(d, f))
        .fold(0.0, f64::max);
    eprintln!(
        "MEASURE untouched SDR 4:2:0 footage delivered as {name}: {worst:.1} codes from the band-limited decimation of the delivered colour"
    );
    assert_eq!(delivered.len(), frames as usize);
    assert!(
        delivered == filtered,
        "{name}: {worst:.1} codes from the band-limited decimation"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

fn hevc10() -> ExportFormat {
    ExportFormat::Hevc {
        ten_bit: true,
        container: pm_protocol::priority::Container::Mov,
    }
}

#[test]
fn sdr_footage_delivered_as_pq_is_filtered() {
    sdr_footage_is_filtered("pq", hevc10(), OutputColor::Rec2100Pq);
}

#[test]
fn sdr_footage_delivered_as_hlg_is_filtered() {
    sdr_footage_is_filtered("hlg", hevc10(), OutputColor::Rec2100Hlg);
}

#[test]
fn sdr_footage_tone_mapped_out_of_an_hdr_project_is_filtered() {
    sdr_footage_is_filtered(
        "tone",
        ExportFormat::H264,
        OutputColor::Rec709QuickTime {
            candidate: QuickTimeTag::Cicp1_13_1,
        },
    );
}

/// Draws a one-pixel red column on every frame.
struct Stroke(usize);

impl FramePost for Stroke {
    fn frame(
        &self,
        _: u64,
        _: Ticks,
        width: u32,
        _: u32,
        _: WorkingEncoding,
        mut rgba: Vec<f32>,
    ) -> Result<Vec<f32>, JobError> {
        for row in rgba.chunks_exact_mut(width as usize * 4) {
            row[self.0 * 4..self.0 * 4 + 4].copy_from_slice(&[1.0, 0.0, 0.0, 1.0]);
        }
        Ok(rgba)
    }
}

#[test]
fn a_postprocessed_frame_of_untouched_footage_is_filtered() {
    let dir = scratch("post");
    let source = dir.join("clip.mov");
    // Grey footage: the stroke is the only colour in the frame.
    still_clip(&source, Codec::ProRes(ProResProfile::Hq), |_, _| [0.4; 3]);
    let frames = 4;
    let (delivered, filtered) = deliver_and_filter(
        vec![clip_layer("v", "clip", 0, frames)],
        &[("clip", &source)],
        settings(
            ExportFormat::ProRes {
                profile: WireProRes::Hq,
            },
            frames,
        ),
        |_| {},
        // An odd column: point sampling at the even columns drops it.
        Some(Arc::new(Stroke(41))),
    );
    let p = planes(&delivered[0]);
    let carried: f64 = (0..p.cw).map(|cx| p.chroma[cx][1] / p.code).sum();
    let red = rgb_to_ycbcr(RED)[2] / p.code;
    eprintln!(
        "MEASURE a red stroke drawn on untouched footage at an odd column: Cr carried {carried:.1} codes of {:.1} (half the stroke's own)",
        red / 2.0
    );
    assert!(
        (carried - red / 2.0).abs() <= 2.0,
        "the stroke's colour is carried: {carried:.1} of {:.1}",
        red / 2.0
    );
    assert!(delivered == filtered);
    let _ = std::fs::remove_dir_all(&dir);
}
