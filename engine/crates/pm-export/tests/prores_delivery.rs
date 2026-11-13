//! ProRes delivered end to end, as a post house receives it.
//!
//! One composition (a full ramp, patches of known colours, one-pixel
//! saturated lines, a moving element, a half-transparent layer and a layer
//! on a fractional pixel, with sound) is exported through the job engine in
//! every flavour (Proxy, LT, 422, HQ, 4444, 4444 XQ), from 8, 16 and 32-bit
//! projects, at 23.976, 25, 29.97 and 60 fps. Every file is then read three
//! ways: by this crate's container reader (sample entry, `colr`, `gama`,
//! frame durations, timecode track, sound), by `ffprobe` and FFmpeg's
//! decoder, and by Apple's decoder. The decoded picture is held against the
//! frame the engine rendered, and against the patches' codes worked out by
//! hand from their colours.

#![cfg(target_os = "macos")]

mod support;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use pm_export::convert::{Chroma, ConvertParams, EncoderInput, WorkingFrame, convert_tagged};
use pm_export::mac::reader::{decode_frame, video_extensions};
use pm_export::mp4box::Movie;
use pm_export::render::*;
use pm_export::timescale::video_timing;
use pm_export::*;
use pm_protocol::color::{OutputColor, Precision, QuickTimeTag};
use pm_protocol::common::{FrameRate, Size, TICKS_PER_SECOND, TickRange, Ticks};
use pm_protocol::priority::{
    AlphaOutput as WireAlpha, ExportFormat, ExportSettings, ExportTimecode,
    ProResProfile as WireProRes, QualityTier,
};
use pm_render::Renderer;
use pm_time::Rate;
use serde_json::{Value, json};
use support::doc::*;

/// The matrix's frame: small, so 72 exports stay quick.
const W: u32 = 256;
const H: u32 = 64;
const FRAMES: u64 = 6;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-export-prores-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Flavour {
    name: &'static str,
    wire: WireProRes,
    fourcc: &'static str,
    /// `ffprobe`'s profile name.
    profile: &'static str,
    /// Bits a sample carries.
    depth: u32,
    /// At the small test frame: decoded smooth picture may be this many of
    /// its own codes from the rendered frame, and the middle of a flat
    /// patch this many from its colour (the codec's loss at this flavour's
    /// data rate; measured, with a little room).
    smooth: f64,
    patch: f64,
    /// At the small test frame, in any frame: the decoded picture anywhere
    /// (lines and hard edges included) may be this many codes from the
    /// rendered frame, and the two decoders this many from each other
    /// (measured, with a little room).
    whole: f64,
    between: f64,
    /// The flavours a master is made in: their decoded ramps never step
    /// back, and their error is below what a shallower sample could carry.
    master: bool,
}

const FLAVOURS: [Flavour; 6] = [
    Flavour {
        name: "422 Proxy",
        wire: WireProRes::Proxy,
        fourcc: "apco",
        profile: "Proxy",
        depth: 10,
        smooth: 8.0,
        patch: 8.0,
        whole: 1e+09,
        between: 1e+09,
        master: false,
    },
    Flavour {
        name: "422 LT",
        wire: WireProRes::Lt,
        fourcc: "apcs",
        profile: "LT",
        depth: 10,
        smooth: 3.0,
        patch: 1.0,
        whole: 1e+09,
        between: 1e+09,
        master: false,
    },
    Flavour {
        name: "422",
        wire: WireProRes::Standard,
        fourcc: "apcn",
        profile: "Standard",
        depth: 10,
        smooth: 1.5,
        patch: 1.0,
        whole: 1e+09,
        between: 1e+09,
        master: false,
    },
    Flavour {
        name: "422 HQ",
        wire: WireProRes::Hq,
        fourcc: "apch",
        profile: "HQ",
        depth: 10,
        smooth: 1.0,
        patch: 1.0,
        whole: 1e+09,
        between: 1e+09,
        master: true,
    },
    Flavour {
        name: "4444",
        wire: WireProRes::P4444,
        fourcc: "ap4h",
        profile: "4444",
        depth: 12,
        smooth: 2.5,
        patch: 1.0,
        whole: 1e+09,
        between: 1e+09,
        master: true,
    },
    Flavour {
        name: "4444 XQ",
        wire: WireProRes::P4444Xq,
        fourcc: "ap4x",
        profile: "XQ",
        depth: 12,
        smooth: 2.0,
        patch: 1.0,
        whole: 1e+09,
        between: 1e+09,
        master: true,
    },
];

impl Flavour {
    fn is_4444(self) -> bool {
        self.depth == 12
    }

    fn input(self) -> InputFormat {
        if self.is_4444() {
            InputFormat::Y416
        } else {
            InputFormat::V216
        }
    }

    /// (luma offset, luma scale, chroma offset, chroma scale) in its codes.
    fn levels(self) -> (f64, f64, f64, f64) {
        if self.is_4444() {
            (256.0, 3504.0, 2048.0, 3584.0)
        } else {
            (64.0, 876.0, 512.0, 896.0)
        }
    }
}

/// A patch of a known colour: its top-left corner and its hex colour.
const PATCHES: [(u32, &str); 9] = [
    (8, "#FFFFFF"),
    (32, "#808080"),
    (56, "#BFBF00"),
    (80, "#00BFBF"),
    (104, "#00BF00"),
    (128, "#BF00BF"),
    (152, "#BF0000"),
    (176, "#0000BF"),
    (200, "#000000"),
];

fn solid(id: &str, color: &str, w: f64, h: u32, x: f64, y: f64, dur: f64) -> Value {
    layer(
        id,
        "solid",
        json!({ "color": color, "w": w, "h": h }),
        x,
        y,
        dur,
    )
}

/// The composition, `w` × `h` at `fps`, in a project of `depth` bits. Its
/// layers sit in the top-left 256 × 40; the rows from 44 down are ramp alone.
fn document_sized(w: u32, h: u32, depth: u32, fps: f64) -> Value {
    let dur = 4.0;
    let mut layers = vec![audio("tone", "tone", 0.0, dur)];
    for (i, (x, color)) in PATCHES.iter().enumerate() {
        layers.push(solid(
            &format!("patch{i}"),
            color,
            16.0,
            16,
            f64::from(*x),
            4.0,
            dur,
        ));
    }
    // One-pixel saturated lines at an even, an odd and an even column.
    for (id, x, color) in [
        ("red", 226.0, "#FF0000"),
        ("green", 231.0, "#00FF00"),
        ("blue", 236.0, "#0000FF"),
    ] {
        layers.push(solid(id, color, 1.0, 16, x, 4.0, dur));
    }
    // A square moving four pixels a frame.
    let mut mover = solid("mover", "#FF00FF", 12.0, 10, 8.0, 24.0, dur);
    let keys: Vec<Value> = (0..FRAMES)
        .map(|n| {
            json!({ "t": n as f64 / fps, "v": 8.0 + 4.0 * n as f64, "i": format!("k{n}"),
                    "eo": [0.333, 0.0], "ei": [0.667, 1.0], "hold": true })
        })
        .collect();
    mover["p"]["position.x"] = json!({ "v": 8.0, "kf": keys, "expr": null });
    layers.push(mover);
    // Half-transparent white, and a layer whose edges fall inside pixels.
    let mut half = solid("half", "#FFFFFF", 40.0, 10, 100.0, 24.0, dur);
    set(&mut half, "opacity", json!(50));
    layers.push(half);
    layers.push(solid("edge", "#FFD000", 20.5, 10, 160.25, 24.0, dur));
    // Across the frame: black to white over the first 45 %, then a shallow
    // ramp of eight 8-bit levels over the last 45 % (what a project's depth
    // and a flavour's sample depth can carry shows there).
    let ramp = json!({ "type": "linear", "angle": 0, "stops": [
        { "id": "s1", "color": "#000000", "position": 0 },
        { "id": "s2", "color": "#FFFFFF", "position": 45 },
        { "id": "s3", "color": "#606060", "position": 55 },
        { "id": "s4", "color": "#686868", "position": 100 }
    ] });
    let mut doc = project(w, h, dur, ramp, layers);
    doc["fps"] = json!(fps);
    doc["colorDepth"] = json!(depth);
    asset(&mut doc, "tone", "audio", "wav", 0, 0, dur);
    doc
}

fn document(depth: u32, fps: f64) -> Value {
    document_sized(W, H, depth, fps)
}

fn settings(
    flavour: Flavour,
    alpha: WireAlpha,
    (num, den): (u32, u32),
    depth: u32,
    start: u64,
) -> ExportSettings {
    let tick = |frame: u64| {
        Ticks::new(
            (frame as i128 * TICKS_PER_SECOND as i128 * i128::from(den) / i128::from(num)) as i64,
        )
        .unwrap()
    };
    ExportSettings {
        format: ExportFormat::ProRes {
            profile: flavour.wire,
        },
        alpha,
        quality: QualityTier::High,
        size: Size::new(W, H),
        frame_rate: FrameRate::new(num, den),
        range: TickRange {
            start: tick(start),
            end: tick(start + FRAMES),
        },
        audio: true,
        audio_index_playhead: None,
        motion_blur: false,
        color: OutputColor::Rec709QuickTime {
            candidate: QuickTimeTag::Cicp1_13_1,
        },
        precision: match depth {
            8 => Precision::Unorm8,
            16 => Precision::Float16,
            _ => Precision::Float32,
        },
        dither: true,
        draft_from_proxies: false,
        delivery: Default::default(),
    }
}

fn job(doc: &Value, settings: ExportSettings, destination: PathBuf) -> ExportJob {
    let mut media = MediaPaths::default();
    media.insert("tone", repo_root().join("engine/fixtures/tone.wav"));
    job_with(doc, media, settings, destination)
}

fn job_with(
    doc: &Value,
    media: MediaPaths,
    settings: ExportSettings,
    destination: PathBuf,
) -> ExportJob {
    ExportJob {
        document: Arc::new(load(doc)),
        composition: "Ptest-root".into(),
        settings,
        media: Arc::new(media),
        destination,
        overwrite: Overwrite::Fail,
        mode: None,
        seed: 5,
    }
}

fn export(renderer: &mut Renderer, job: &ExportJob) -> Delivered {
    run_export(
        renderer,
        job,
        &Unrestricted,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap_or_else(|e| panic!("{}: {e}", job.destination.display()))
}

/// A picture in a flavour's own codes: luma per pixel, Cb and Cr per chroma
/// sample (`cw` across), alpha 0..1 per pixel when it has any.
#[derive(Clone, Debug)]
struct Picture {
    w: usize,
    luma: Vec<f64>,
    cb: Vec<f64>,
    cr: Vec<f64>,
    cw: usize,
    alpha: Option<Vec<f64>>,
}

impl Picture {
    fn of(input: &EncoderInput, alpha: bool) -> Picture {
        let (w, h) = (input.width as usize, input.height as usize);
        let p = input.planes[0].u16().expect("16-bit plane");
        match input.format {
            InputFormat::V216 => {
                let (mut luma, mut cb, mut cr) = (vec![], vec![], vec![]);
                for q in p.chunks_exact(4) {
                    luma.extend([f64::from(q[1]) / 64.0, f64::from(q[3]) / 64.0]);
                    cb.push(f64::from(q[0]) / 64.0);
                    cr.push(f64::from(q[2]) / 64.0);
                }
                Picture {
                    w,
                    luma,
                    cb,
                    cr,
                    cw: w / 2,
                    alpha: None,
                }
            }
            InputFormat::Y416 => {
                let px = |k: usize, scale: f64| -> Vec<f64> {
                    (0..w * h)
                        .map(|i| f64::from(p[i * 4 + k]) / scale)
                        .collect()
                };
                Picture {
                    w,
                    luma: px(1, 16.0),
                    cb: px(2, 16.0),
                    cr: px(3, 16.0),
                    cw: w,
                    alpha: alpha.then(|| px(0, 65535.0)),
                }
            }
            other => panic!("{other:?} is not a ProRes input"),
        }
    }

    /// Chroma sample of pixel `x`, row `y`.
    fn chroma_at(&self, x: usize, y: usize) -> [f64; 2] {
        let cx = if self.cw == self.w { x } else { x / 2 };
        [self.cb[y * self.cw + cx], self.cr[y * self.cw + cx]]
    }

    /// (largest, root mean square) difference from `other` over the luma
    /// and chroma of rows `rows`, in codes.
    fn distance(&self, other: &Picture, rows: std::ops::Range<usize>) -> (f64, f64) {
        let (w, mut worst, mut sum, mut n) = (self.w, 0.0f64, 0.0, 0usize);
        let mut eat = |a: f64, b: f64| {
            worst = worst.max((a - b).abs());
            sum += (a - b) * (a - b);
            n += 1;
        };
        for y in rows {
            for x in 0..w {
                eat(self.luma[y * w + x], other.luma[y * w + x]);
            }
            for cx in 0..self.cw {
                eat(self.cb[y * self.cw + cx], other.cb[y * self.cw + cx]);
                eat(self.cr[y * self.cw + cx], other.cr[y * self.cw + cx]);
            }
        }
        (worst, (sum / n as f64).sqrt())
    }

    /// The picture with every sample held to the codes a stream may carry
    /// (4 to 1019 at 10 bits, 16 to 4079 at 12).
    fn legal(mut self, flavour: Flavour) -> Picture {
        let (lo, hi) = if flavour.is_4444() {
            (16.0, 4079.0)
        } else {
            (4.0, 1019.0)
        };
        for plane in [&mut self.luma, &mut self.cb, &mut self.cr] {
            for v in plane.iter_mut() {
                *v = v.clamp(lo, hi);
            }
        }
        self
    }

    /// Luma of row `y`.
    fn row(&self, y: usize) -> &[f64] {
        &self.luma[y * self.w..(y + 1) * self.w]
    }
}

/// Frame `n` of `job` as the engine renders it and hands it to the encoder.
fn rendered(renderer: &mut Renderer, job: &ExportJob, n: u64) -> EncoderInput {
    let plan = plan_export(job).unwrap();
    let size = plan.request.output_size;
    let read = render_frame(
        renderer,
        &plan.document,
        &job.composition,
        plan.timeline.tick(n),
        &plan.request,
        &job.media,
        &Unrestricted,
        pass_key(job.seed, size),
        &AtomicBool::new(false),
    )
    .unwrap();
    convert_tagged(
        &WorkingFrame {
            width: size.width,
            height: size.height,
            rgba: &read.rgba,
        },
        plan.spec.input_format(),
        &ConvertParams {
            working: plan.spec.working,
            alpha: plan.spec.alpha,
            tags: plan.spec.tags,
            dither: plan.spec.dither_enabled(),
            frame: n as u32,
        },
        plan.spec.tags,
        Chroma::Filtered,
    )
    .unwrap()
}

/// Frame `n` decoded by Apple's decoder.
fn apple(path: &Path, n: u64, rate: Rate, flavour: Flavour, alpha: bool) -> Picture {
    let decoded = decode_frame(path, n, video_timing(rate).unwrap(), flavour.input())
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    Picture::of(&decoded, alpha)
}

/// Every frame decoded by FFmpeg's decoder, in the stream's own format.
fn ffmpeg(path: &Path, (w, h): (u32, u32), flavour: Flavour, alpha: bool) -> Vec<Picture> {
    let pix = match (flavour.is_4444(), alpha) {
        (false, _) => "yuv422p10le",
        (true, false) => "yuv444p12le",
        (true, true) => "yuva444p12le",
    };
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-map", "0:v:0", "-f", "rawvideo", "-pix_fmt", pix, "pipe:1"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let (w, h) = (w as usize, h as usize);
    let cw = if flavour.is_4444() { w } else { w / 2 };
    let samples = w * h + 2 * cw * h + if alpha { w * h } else { 0 };
    let values: Vec<f64> = out
        .stdout
        .chunks_exact(2)
        .map(|b| f64::from(u16::from_le_bytes([b[0], b[1]])))
        .collect();
    assert!(values.len().is_multiple_of(samples), "{}", path.display());
    values
        .chunks_exact(samples)
        .map(|f| Picture {
            w,
            luma: f[..w * h].to_vec(),
            cb: f[w * h..w * h + cw * h].to_vec(),
            cr: f[w * h + cw * h..w * h + 2 * cw * h].to_vec(),
            cw,
            alpha: alpha.then(|| f[w * h + 2 * cw * h..].iter().map(|v| v / 4095.0).collect()),
        })
        .collect()
}

fn ffprobe(path: &Path) -> Value {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-count_frames",
            "-show_streams",
            "-show_format",
            "-show_entries",
            "frame=media_type,pts,duration",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

fn stream<'a>(probe: &'a Value, kind: &str) -> &'a Value {
    probe["streams"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["codec_type"] == kind)
        .unwrap_or_else(|| panic!("no {kind} stream"))
}

/// The codes of a flat colour, worked out from its hex value by BT.709:
/// (Y′, Cb, Cr) in a flavour's codes.
fn known(color: &str, flavour: Flavour) -> [f64; 3] {
    let c =
        |i: usize| f64::from(u8::from_str_radix(&color[1 + 2 * i..3 + 2 * i], 16).unwrap()) / 255.0;
    let (r, g, b) = (c(0), c(1), c(2));
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let (lo, ls, co, cs) = flavour.levels();
    [
        lo + ls * y,
        co + cs * (b - y) / 1.8556,
        co + cs * (r - y) / 1.5748,
    ]
}

/// The middle of each patch less its known codes (Y′, Cb, Cr), signed.
fn patch_residuals(picture: &Picture, flavour: Flavour) -> Vec<[f64; 3]> {
    // The patches are 16 × 16 at (x, 4), whatever the frame's size.
    PATCHES
        .iter()
        .map(|(x, color)| {
            let want = known(color, flavour);
            let (mut sum, mut n) = ([0.0; 3], 0.0);
            for y in 8..16 {
                for px in *x as usize + 4..*x as usize + 12 {
                    let c = picture.chroma_at(px, y);
                    sum[0] += picture.luma[y * picture.w + px];
                    sum[1] += c[0];
                    sum[2] += c[1];
                    n += 1.0;
                }
            }
            std::array::from_fn(|k| sum[k] / n - want[k])
        })
        .collect()
}

/// How far the middle of each patch is from its known codes, at worst.
fn patches(picture: &Picture, flavour: Flavour) -> f64 {
    patch_residuals(picture, flavour)
        .iter()
        .flatten()
        .fold(0.0, |worst, r| worst.max(r.abs()))
}

/// What the decoded picture of one export measures against the frame the
/// engine rendered, over the frames looked at.
#[derive(Default)]
struct Measured {
    /// Largest and root-mean-square difference over the rows that are ramp
    /// alone, in the flavour's codes, by either decoder.
    smooth: f64,
    rms: f64,
    /// Largest difference anywhere (lines and edges included), Apple's.
    whole: f64,
    /// Largest difference between the two decoders.
    between: f64,
    /// The decoded patches from their known codes, and the rendered ones.
    patch: f64,
    rendered_patch: f64,
    /// The largest step back along the decoded ramps, in whole codes (a
    /// ramp rising less than a code a pixel may alternate between two
    /// neighbouring codes; more than one code back is a reversal).
    back: f64,
    /// Distinct codes on the shallow ramp: rendered, and decoded.
    shallow_rendered: usize,
    shallow_decoded: usize,
    /// The middle of the moving square, in every frame, from the rendered
    /// frame's (it is somewhere else in each).
    mover: f64,
    /// The shallow ramp's column means (over the rows that are ramp alone)
    /// from the rendered frame's, root mean square, and what the rendered
    /// ramp cut to a sample two bits shallower would measure the same way.
    precision: f64,
    shallower: f64,
}

impl Measured {
    fn line(&self) -> String {
        format!(
            "decoded smooth ±{:.2} codes (rms {:.2}), whole frame ±{:.1}, decoders agree ±{:.2}; patches ±{:.2} of their known codes (rendered ±{:.2}), the moving square ±{:.2} in every frame; ramps step back at most {} codes; shallow ramp {} codes rendered, {} decoded, its column means {:.2} codes rms from the rendered ({:.2} for a sample two bits shallower)",
            self.smooth,
            self.rms,
            self.whole,
            self.between,
            self.patch,
            self.rendered_patch,
            self.mover,
            self.back,
            self.shallow_rendered,
            self.shallow_decoded,
            self.precision,
            self.shallower
        )
    }

    /// Holds the measurements against what a flavour from a project of
    /// `depth` bits must deliver, adding what fails to `problems`.
    fn check(
        &self,
        what: &str,
        flavour: Flavour,
        depth: u32,
        (smooth, patch): (f64, f64),
        (whole, between): (f64, f64),
        problems: &mut Vec<String>,
    ) {
        let mut fail = |m: String| problems.push(format!("{what}: {m}"));
        if self.smooth > smooth {
            fail(format!(
                "smooth picture {:.2} codes from the rendered frame",
                self.smooth
            ));
        }
        if self.rendered_patch > 0.6 {
            fail(format!(
                "the rendered patches are {:.2} codes from their colours",
                self.rendered_patch
            ));
        }
        if self.patch > patch {
            fail(format!(
                "decoded patches {:.2} codes from their colours",
                self.patch
            ));
        }
        // The moving square is where it was rendered, in every frame.
        if self.mover > patch.max(smooth) {
            fail(format!(
                "the moving square is {:.2} codes from the rendered frame's",
                self.mover
            ));
        }
        if self.whole > whole {
            fail(format!(
                "the picture is {:.1} codes from the rendered frame somewhere",
                self.whole
            ));
        }
        if self.between > between {
            fail(format!(
                "the decoders disagree by {:.2} codes",
                self.between
            ));
        }
        // The flavour's sample depth is delivered, not a shallower one's:
        // averaged down the ramp's rows, the shallow ramp follows the
        // rendered one more closely than a sample two bits shallower can.
        if self.precision > 0.75 * self.shallower {
            fail(format!(
                "the shallow ramp's column means are {:.2} codes rms from the rendered ones ({:.2} for a sample two bits shallower)",
                self.precision, self.shallower
            ));
        }
        // What the project carries into the frame: the flavour's own codes
        // between the shallow ramp's eight 8-bit levels (27 at 10 bits, 110
        // at 12). That holds for an 8-bit project too: its intermediates
        // are float (`pm-render` keeps extended colour until output), and
        // only an 8-bit output quantizes it to 8 bits.
        let deep = if flavour.is_4444() { 80 } else { 20 };
        if self.shallow_rendered < deep {
            fail(format!(
                "a {depth}-bit project rendered only {} codes on the shallow ramp",
                self.shallow_rendered
            ));
        }
        if flavour.master {
            if self.back > 1.0 {
                fail(format!("a decoded ramp steps back {} codes", self.back));
            }
            // Below what a sample two bits shallower could carry
            // (its rounding alone is 4 / √12 = 1.15 codes rms).
            if self.rms > 0.8 {
                fail(format!("rms {:.2} codes on smooth picture", self.rms));
            }
            // The sample depth is carried, not padded: more codes on the
            // shallow ramp than a sample two bits shallower has there (9
            // at 8 bits, 28 at 10).
            let carried = if flavour.is_4444() { 40 } else { 14 };
            if self.shallow_decoded < carried {
                fail(format!(
                    "only {} codes decoded on the shallow ramp",
                    self.shallow_decoded
                ));
            }
        }
    }
}

/// The rows of the picture that are ramp alone (nothing drawn over them).
const SMOOTH: std::ops::Range<usize> = 44..60;

fn measure(
    renderer: &mut Renderer,
    job: &ExportJob,
    path: &Path,
    rate: Rate,
    flavour: Flavour,
) -> Measured {
    let size = job.settings.size;
    let (w, h) = (size.width as usize, size.height as usize);
    let foreign = ffmpeg(path, (size.width, size.height), flavour, false);
    assert_eq!(foreign.len() as u64, FRAMES, "{}", path.display());
    let mut m = Measured::default();
    // The ramp's two parts, by column: rising, then shallow.
    let (rising, shallow) = (0..w * 45 / 100, w * 56 / 100..w);
    let distinct = |row: &[f64]| {
        row[shallow.clone()]
            .iter()
            .map(|v| v.round() as i64)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    };
    let mut squares = Vec::new();
    // The square is 12 × 10 at (8 + 4n, 24), whatever the frame's size.
    let mover = |picture: &Picture, n: u64| -> [f64; 3] {
        let (mut sum, mut count) = ([0.0; 3], 0.0);
        for y in 27..31 {
            for x in 12 + 4 * n as usize..16 + 4 * n as usize {
                let c = picture.chroma_at(x, y);
                sum[0] += picture.luma[y * w + x];
                sum[1] += c[0];
                sum[2] += c[1];
                count += 1.0;
            }
        }
        sum.map(|v| v / count)
    };
    // Column means of the shallow ramp over the rows that are ramp alone.
    let columns = |picture: &Picture, cut: f64| -> Vec<f64> {
        shallow
            .clone()
            .map(|x| {
                SMOOTH
                    .map(|y| (picture.luma[y * w + x] / cut).round() * cut)
                    .sum::<f64>()
                    / SMOOTH.len() as f64
            })
            .collect()
    };
    let rms = |a: &[f64], b: &[f64]| {
        (a.iter().zip(b).map(|(p, q)| (p - q) * (p - q)).sum::<f64>() / a.len() as f64).sqrt()
    };
    for n in 0..FRAMES {
        let want = Picture::of(&rendered(renderer, job, n), false);
        let own = apple(path, n, rate, flavour, false);
        let other = &foreign[n as usize];
        for decoded in [&own, other] {
            let (got, at) = (mover(decoded, n), mover(&want, n));
            for k in 0..3 {
                m.mover = m.mover.max((got[k] - at[k]).abs());
            }
        }
        let exact = columns(&want, 1e-9);
        m.precision = m.precision.max(rms(&columns(&own, 1e-9), &exact));
        m.shallower = rms(&columns(&want, 4.0), &exact);
        for decoded in [&own, other] {
            let (worst, rms) = decoded.distance(&want, SMOOTH);
            m.smooth = m.smooth.max(worst);
            squares.push(rms * rms);
            m.patch = m.patch.max(patches(decoded, flavour));
        }
        m.whole = m.whole.max(own.distance(&want, 0..h).0);
        m.between = m.between.max(own.distance(other, 0..h).0);
        m.rendered_patch = m.rendered_patch.max(patches(&want, flavour));
        let row = own.row(50);
        for part in [&row[rising.clone()], &row[shallow.clone()]] {
            for p in part.windows(2) {
                m.back = m.back.max(p[0].round() - p[1].round());
            }
        }
        m.shallow_rendered = m.shallow_rendered.max(distinct(want.row(50)));
        m.shallow_decoded = m.shallow_decoded.max(distinct(row));
        // No level shift: the ramp starts at black and reaches white.
        let (lo, ls, ..) = flavour.levels();
        let peak = row.iter().cloned().fold(0.0, f64::max);
        assert!(
            (row[0] - lo).abs() < 0.02 * ls && (peak - lo - ls).abs() < 0.02 * ls,
            "{}: the ramp runs from {:.1} to {peak:.1}",
            path.display(),
            row[0]
        );
    }
    m.rms = (squares.iter().sum::<f64>() / squares.len() as f64).sqrt();
    m
}

/// One export's facts, for the report.
struct Row {
    flavour: &'static str,
    depth: u32,
    rate: String,
    line: String,
}

const RATES: [(u32, u32, &str); 4] = [
    (24000, 1001, "23.976"),
    (25, 1, "25"),
    (30000, 1001, "29.97"),
    (60, 1, "60"),
];

#[test]
fn every_flavour_from_every_project_depth_at_every_rate() {
    let dir = scratch("matrix");
    let mut renderer = Renderer::headless().unwrap();
    let mut rows: Vec<Row> = Vec::new();
    let mut problems: Vec<String> = Vec::new();
    for flavour in FLAVOURS {
        for depth in [8u32, 16, 32] {
            for (num, den, label) in RATES {
                let what = format!(
                    "ProRes {} from a {depth}-bit project at {label} fps",
                    flavour.name
                );
                let fps = f64::from(num) / f64::from(den);
                let rate = Rate::new(i64::from(num), i64::from(den)).unwrap();
                let path = dir.join(format!("{}-{depth}-{num}.mov", flavour.fourcc));
                let doc = document(depth, fps);
                let job = job(
                    &doc,
                    settings(flavour, WireAlpha::None, (num, den), depth, 0),
                    path.clone(),
                );
                export(&mut renderer, &job);

                // The container, read by this crate's own reader.
                let movie = Movie::read(&path).unwrap();
                let video = movie.track(b"vide").expect("a video track");
                let visual = video.visual.clone().expect("a visual sample entry");
                let timing = video_timing(rate).unwrap();
                assert_eq!(
                    String::from_utf8_lossy(&video.sample_entry),
                    flavour.fourcc,
                    "{what}: sample entry"
                );
                assert_eq!(visual.depth, 24, "{what}: opaque sample entry depth");
                assert_eq!(
                    (visual.width, visual.height),
                    (W as u16, H as u16),
                    "{what}"
                );
                let colr = visual.colr.clone().expect("a colr box");
                assert_eq!(
                    (&colr.kind, colr.primaries, colr.transfer, colr.matrix),
                    (b"nclc", 1, 13, 1),
                    "{what}: colr"
                );
                assert_eq!(video.media_timescale, timing.timescale as u32, "{what}");
                assert_eq!(
                    video.stts,
                    vec![(FRAMES as u32, timing.frame_duration as u32)],
                    "{what}: every frame lasts one frame"
                );
                assert_eq!(video.sample_count, FRAMES, "{what}");

                // The timecode track: the composition's own time, drop-frame
                // where the rate has it.
                let tmcd_track = movie.track(b"tmcd").expect("a timecode track");
                let tmcd = tmcd_track.tmcd.clone().expect("a tmcd sample entry");
                let drop = den == 1001 && num == 30000;
                assert_eq!(tmcd.drop_frame(), drop, "{what}: drop-frame flag");
                assert_eq!(
                    u32::from(tmcd.frames_per_second),
                    fps.round() as u32,
                    "{what}: timecode frames per second"
                );
                assert_eq!(tmcd.start_frame, Some(0), "{what}: start timecode");
                assert_eq!(
                    (tmcd.timescale, tmcd.frame_duration),
                    (timing.timescale as u32, timing.frame_duration as u32),
                    "{what}: timecode timing"
                );
                assert!(
                    video
                        .references
                        .iter()
                        .any(|(kind, ids)| kind == b"tmcd" && ids.contains(&tmcd_track.id)),
                    "{what}: the picture refers to its timecode track"
                );

                // The sound: 24-bit PCM, 48 kHz, stereo, as long as the
                // picture to the sample.
                let sound = movie.track(b"soun").expect("an audio track");
                let audio = sound.audio.clone().expect("an audio sample entry");
                assert_eq!(
                    (
                        audio.channels,
                        audio.sample_rate,
                        audio.bits_per_channel,
                        audio.float
                    ),
                    (2, 48_000.0, 24, false),
                    "{what}: sound"
                );
                let plan = plan_export(&job).unwrap();
                let samples = export_audio_samples(&plan.timeline, 48_000);
                let picture_samples = FRAMES as f64 * 48_000.0 * f64::from(den) / f64::from(num);
                assert_eq!(sound.sample_count as i64, samples, "{what}: audio samples");
                assert!(
                    (samples as f64 - picture_samples).abs() <= 1.0,
                    "{what}: sound {samples} samples, picture {picture_samples:.2}"
                );

                // ffprobe's reading of the same file.
                let probe = ffprobe(&path);
                let v = stream(&probe, "video");
                let a = stream(&probe, "audio");
                let want_pix = if flavour.is_4444() {
                    "yuv444p12le"
                } else {
                    "yuv422p10le"
                };
                assert_eq!(v["codec_name"], "prores", "{what}");
                assert_eq!(v["codec_tag_string"], flavour.fourcc, "{what}");
                assert_eq!(v["profile"], flavour.profile, "{what}");
                assert_eq!(v["pix_fmt"], want_pix, "{what}");
                assert_eq!(
                    v["bits_per_raw_sample"],
                    flavour.depth.to_string().as_str(),
                    "{what}: bits per sample"
                );
                assert_eq!(v["color_primaries"], "bt709", "{what}");
                assert_eq!(v["color_transfer"], "iec61966-2-1", "{what}");
                assert_eq!(v["color_space"], "bt709", "{what}");
                assert!(
                    v["color_range"] == "tv" || v["color_range"].is_null(),
                    "{what}: range {}",
                    v["color_range"]
                );
                assert!(
                    v["sample_aspect_ratio"] == "1:1" || v["sample_aspect_ratio"].is_null(),
                    "{what}: pixel aspect {}",
                    v["sample_aspect_ratio"]
                );
                assert_eq!(v["field_order"], "progressive", "{what}");
                assert_eq!(v["nb_read_frames"], FRAMES.to_string().as_str(), "{what}");
                assert_eq!(
                    v["r_frame_rate"],
                    format!("{num}/{den}").as_str(),
                    "{what}: frame rate"
                );
                let durations: Vec<i64> = probe["frames"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|f| f["media_type"] == "video")
                    .map(|f| f["duration"].as_i64().unwrap())
                    .collect();
                assert_eq!(
                    durations,
                    vec![timing.frame_duration; FRAMES as usize],
                    "{what}: frame durations"
                );
                assert_eq!(a["codec_name"], "pcm_s24le", "{what}");
                assert_eq!(a["sample_rate"], "48000", "{what}");
                assert_eq!(a["channels"], 2, "{what}");
                assert_eq!(a["channel_layout"], "stereo", "{what}");
                let timecode = stream(&probe, "data")["tags"]["timecode"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                let want_timecode = if drop { "00:00:00;00" } else { "00:00:00:00" };
                assert_eq!(timecode, want_timecode, "{what}: timecode");

                // Apple's reading of the format.
                let extensions = video_extensions(&path).unwrap();
                let ext = |key: &str| {
                    extensions
                        .iter()
                        .find(|(k, _)| k.contains(key))
                        .map(|(_, v)| v.clone())
                        .unwrap_or_default()
                };
                assert!(
                    ext("ColorPrimaries").contains("709"),
                    "{what}: {extensions:?}"
                );
                assert!(
                    ext("TransferFunction").contains("sRGB")
                        || ext("TransferFunction").contains("61966"),
                    "{what}: {extensions:?}"
                );
                assert!(ext("YCbCrMatrix").contains("709"), "{what}: {extensions:?}");

                // The picture: both decoders against the rendered frame.
                let m = measure(&mut renderer, &job, &path, rate, flavour);
                let line = format!(
                    "{} {} colr 1-13-1 gama {:?}; {FRAMES} frames of {}/{}; tmcd {} at {} fps; PCM 24-bit 48 kHz stereo, {samples} samples; {}",
                    flavour.fourcc,
                    v["pix_fmt"].as_str().unwrap(),
                    visual.gamma,
                    timing.frame_duration,
                    timing.timescale,
                    timecode,
                    tmcd.frames_per_second,
                    m.line()
                );
                eprintln!("MEASURE {what}: {line}");
                assert_eq!(visual.gamma, None, "{what}: no gama atom beside colr");
                m.check(
                    &what,
                    flavour,
                    depth,
                    (flavour.smooth, flavour.patch),
                    (flavour.whole, flavour.between),
                    &mut problems,
                );
                rows.push(Row {
                    flavour: flavour.name,
                    depth,
                    rate: label.to_owned(),
                    line,
                });
            }
        }
    }
    if let Some(report) = std::env::var_os("PM_PRORES_REPORT") {
        let mut text = String::from("| flavour | project | fps | facts |\n|---|---|---|---|\n");
        for r in &rows {
            text.push_str(&format!(
                "| {} | {}-bit | {} | {} |\n",
                r.flavour, r.depth, r.rate, r.line
            ));
        }
        std::fs::write(report, text).unwrap();
    }
    assert_eq!(rows.len(), 72);
    assert!(problems.is_empty(), "{problems:#?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Start timecode and reel as a post house sets them.
#[test]
fn start_timecode_and_reel_are_written_as_asked() {
    let dir = scratch("timecode");
    let mut renderer = Renderer::headless().unwrap();
    let hq = FLAVOURS[3];
    // (rate, start, drop-frame, the frame number it is, what ffprobe reads)
    let cases = [
        ((30000u32, 1001u32), "01:00:00;00", true, 107_892u32),
        ((30000, 1001), "01:00:00:00", false, 108_000),
        ((24000, 1001), "10:00:00:00", false, 864_000),
        ((25, 1), "09:59:50:00", false, 899_750),
        ((60, 1), "00:59:59:59", false, 215_999),
    ];
    for ((num, den), start, drop_frame, frame) in cases {
        let what = format!("{start} at {num}/{den}");
        let path = dir.join(format!("{frame}-{num}.mov"));
        let mut s = settings(hq, WireAlpha::None, (num, den), 32, 0);
        s.delivery.timecode = Some(ExportTimecode {
            start: start.into(),
            drop_frame,
            reel: Some("A001C003".into()),
        });
        let doc = document(32, f64::from(num) / f64::from(den));
        export(&mut renderer, &job(&doc, s, path.clone()));
        let movie = Movie::read(&path).unwrap();
        let tmcd = movie.track(b"tmcd").unwrap().tmcd.clone().unwrap();
        let probe = ffprobe(&path);
        let read = stream(&probe, "data")["tags"]["timecode"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let reel = stream(&probe, "data")["tags"]["reel_name"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        eprintln!(
            "MEASURE timecode {what}: tmcd start frame {:?}, drop-frame {}, reel {:?}; ffprobe reads {read} reel {reel:?}",
            tmcd.start_frame,
            tmcd.drop_frame(),
            tmcd.reel
        );
        assert_eq!(tmcd.start_frame, Some(frame), "{what}");
        assert_eq!(tmcd.drop_frame(), drop_frame, "{what}");
        assert_eq!(tmcd.reel.as_deref(), Some("A001C003"), "{what}");
        assert_eq!(read, start, "{what}: ffprobe");
        assert_eq!(reel, "A001C003", "{what}: ffprobe reel");
    }
    // A work area that starts later is labelled with the composition's own
    // time at its first frame.
    let (num, den) = (30000u32, 1001u32);
    let path = dir.join("work-area.mov");
    let doc = document(32, f64::from(num) / f64::from(den));
    export(
        &mut renderer,
        &job(
            &doc,
            settings(hq, WireAlpha::None, (num, den), 32, 45),
            path.clone(),
        ),
    );
    let tmcd = Movie::read(&path)
        .unwrap()
        .track(b"tmcd")
        .unwrap()
        .tmcd
        .clone()
        .unwrap();
    let probe = ffprobe(&path);
    let read = stream(&probe, "data")["tags"]["timecode"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    eprintln!(
        "MEASURE timecode of a work area starting at frame 45 of 29.97: start frame {:?}, ffprobe reads {read}",
        tmcd.start_frame
    );
    assert_eq!(tmcd.start_frame, Some(45));
    assert_eq!(read, "00:00:01;15");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The matrix runs at a small frame, where a flavour's data rate is tiny.
/// At HD the masters hold the brief's bar: smooth picture within one code
/// of the rendered frame, patches within one code of their colours.
#[test]
fn the_picture_holds_at_hd_in_every_flavour() {
    let dir = scratch("hd");
    let mut renderer = Renderer::headless().unwrap();
    let mut problems = Vec::new();
    let rate = Rate::new(25, 1).unwrap();
    for flavour in FLAVOURS {
        let what = format!("ProRes {} at 1920 × 1080", flavour.name);
        let path = dir.join(format!("{}.mov", flavour.fourcc));
        let mut s = settings(flavour, WireAlpha::None, (25, 1), 32, 0);
        s.size = Size::new(1920, 1080);
        let job = job(&document_sized(1920, 1080, 32, 25.0), s, path.clone());
        let delivered = export(&mut renderer, &job);
        let m = measure(&mut renderer, &job, &path, rate, flavour);
        eprintln!(
            "MEASURE {what}: {} Mb/s; {}",
            delivered.report.bytes as f64 * 8.0 * 25.0 / FRAMES as f64 / 1e6,
            m.line()
        );
        // (smooth, patch) in the flavour's codes at a real frame size.
        let (smooth, patch) = match flavour.name {
            "422 Proxy" => (4.0, 8.0),
            "422 LT" => (2.0, 1.0),
            "422" => (1.5, 1.0),
            "422 HQ" => (1.0, 1.0),
            "4444" => (2.0, 1.0),
            _ => (1.5, 1.0),
        };
        m.check(
            &what,
            flavour,
            32,
            (smooth, patch),
            (flavour.whole, flavour.between),
            &mut problems,
        );
    }
    assert!(problems.is_empty(), "{problems:#?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// "Rec.709" (1-1-1) instead of the default label is the same picture with
/// another label, in every flavour.
#[test]
fn the_rec709_label_is_a_label_in_every_flavour() {
    let dir = scratch("label");
    let mut renderer = Renderer::headless().unwrap();
    let rate = Rate::new(25, 1).unwrap();
    for flavour in FLAVOURS {
        let doc = document(32, 25.0);
        let mut decoded = Vec::new();
        for (name, color, transfer, ffprobe_transfer) in [
            (
                "1-13-1",
                OutputColor::Rec709QuickTime {
                    candidate: QuickTimeTag::Cicp1_13_1,
                },
                13,
                "iec61966-2-1",
            ),
            ("1-1-1", OutputColor::Rec709Broadcast, 1, "bt709"),
        ] {
            let what = format!("ProRes {} labelled {name}", flavour.name);
            let path = dir.join(format!("{}-{name}.mov", flavour.fourcc));
            let mut s = settings(flavour, WireAlpha::None, (25, 1), 32, 0);
            s.color = color;
            export(&mut renderer, &job(&doc, s, path.clone()));
            let visual = Movie::read(&path)
                .unwrap()
                .track(b"vide")
                .unwrap()
                .visual
                .clone()
                .unwrap();
            let colr = visual.colr.clone().unwrap();
            let probe = ffprobe(&path);
            let v = stream(&probe, "video");
            eprintln!(
                "MEASURE {what}: colr {} {}-{}-{}, gama {:?}; ffprobe {} / {} / {}",
                String::from_utf8_lossy(&colr.kind),
                colr.primaries,
                colr.transfer,
                colr.matrix,
                visual.gamma,
                v["color_primaries"],
                v["color_transfer"],
                v["color_space"]
            );
            assert_eq!(
                (colr.primaries, colr.transfer, colr.matrix),
                (1, transfer, 1),
                "{what}"
            );
            assert_eq!(v["color_transfer"], ffprobe_transfer, "{what}");
            assert_eq!(visual.gamma, None, "{what}: no gama atom beside colr");
            decoded.push(apple(&path, 0, rate, flavour, false));
        }
        let (worst, _) = decoded[0].distance(&decoded[1], 0..H as usize);
        assert_eq!(
            worst, 0.0,
            "ProRes {}: the label changed the picture",
            flavour.name
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Alpha in 4444 and 4444 XQ: carried exactly, straight or premultiplied as
/// asked, and absent (an opaque file) when not asked for.
#[test]
fn alpha_is_exact_in_4444_and_xq_straight_and_premultiplied() {
    let dir = scratch("alpha");
    let mut renderer = Renderer::headless().unwrap();
    let rate = Rate::new(25, 1).unwrap();
    let doc = document(32, 25.0);
    let w = W as usize;
    for flavour in FLAVOURS.into_iter().filter(|f| f.is_4444()) {
        for (mode, wire) in [
            ("straight", WireAlpha::Straight),
            ("premultiplied", WireAlpha::Premultiplied),
        ] {
            let what = format!("ProRes {} with {mode} alpha", flavour.name);
            let path = dir.join(format!("{}-{mode}.mov", flavour.fourcc));
            let job = job(&doc, settings(flavour, wire, (25, 1), 32, 0), path.clone());
            export(&mut renderer, &job);
            let visual = Movie::read(&path)
                .unwrap()
                .track(b"vide")
                .unwrap()
                .visual
                .clone()
                .unwrap();
            let probe = ffprobe(&path);
            let extensions = video_extensions(&path).unwrap();
            let label = extensions
                .iter()
                .find(|(k, _)| k.contains("Alpha"))
                .map(|(k, v)| format!("{k} = {v}"))
                .unwrap_or_else(|| "none".into());
            assert_eq!(visual.depth, 32, "{what}: sample entry depth");
            assert_eq!(stream(&probe, "video")["pix_fmt"], "yuva444p12le", "{what}");
            // The format says which alpha it is, as a reader is told.
            assert!(
                label.contains("AlphaChannelMode")
                    && label.to_lowercase().contains(mode)
                    && label.to_lowercase().contains("straight") == (wire == WireAlpha::Straight),
                "{what}: the format's alpha mode is {label}"
            );

            let want = Picture::of(&rendered(&mut renderer, &job, 0), true);
            let own = apple(&path, 0, rate, flavour, true);
            let other = &ffmpeg(&path, (W, H), flavour, true)[0];
            let (wa, oa, fa) = (
                want.alpha.as_ref().unwrap(),
                own.alpha.as_ref().unwrap(),
                other.alpha.as_ref().unwrap(),
            );
            // In 16-bit codes, by Apple's decoder; in 12-bit, by FFmpeg's.
            let exact = wa
                .iter()
                .zip(oa)
                .map(|(a, b)| ((a - b) * 65535.0).abs())
                .fold(0.0, f64::max);
            let foreign = wa
                .iter()
                .zip(fa)
                .map(|(a, b)| ((a - b) * 4095.0).abs())
                .fold(0.0, f64::max);
            // Known coverage: nothing on the ramp rows (the background is
            // not part of an alpha export), a patch, the half-transparent
            // layer, and the layer that starts a quarter into a pixel.
            let at = |x: usize, y: usize| oa[y * w + x];
            let coverage = [at(128, 50), at(16, 12), at(120, 28), at(160, 28)];
            // Colour under the half-transparent white layer.
            let (lo, ls, ..) = flavour.levels();
            let under = (own.luma[28 * w + 120] - lo) / ls;
            eprintln!(
                "MEASURE {what}: depth {}, {}; alpha {exact:.1} of 65,535 from the rendered frame by Apple's decoder, {foreign:.2} of 4,095 by FFmpeg's; coverage empty {:.4}, patch {:.4}, half {:.5}, quarter-in edge {:.4}; white at half coverage reads {under:.4}; label {label}",
                visual.depth,
                stream(&probe, "video")["pix_fmt"],
                coverage[0],
                coverage[1],
                coverage[2],
                coverage[3]
            );
            assert_eq!(exact, 0.0, "{what}: alpha is exact");
            // FFmpeg hands back the 16-bit alpha cut to 12 bits.
            assert!(
                foreign <= 1.0,
                "{what}: FFmpeg reads alpha {foreign:.2} codes off"
            );
            assert_eq!(coverage[0], 0.0, "{what}");
            assert_eq!(coverage[1], 1.0, "{what}");
            assert!((coverage[2] - 0.5).abs() < 1e-4, "{what}: {}", coverage[2]);
            assert!((coverage[3] - 0.75).abs() < 1e-3, "{what}: {}", coverage[3]);
            // Straight: the colour is the layer's own (white); premultiplied:
            // white times its coverage.
            let want_under = if wire == WireAlpha::Straight {
                1.0
            } else {
                0.5
            };
            assert!(
                (under - want_under).abs() < 0.004,
                "{what}: white at half coverage reads {under:.4}"
            );
            assert!(
                patches(&own, flavour) <= 1.0 && patches(other, flavour) <= 1.0,
                "{what}: opaque patches keep their colours"
            );
        }
        // Not asked for: an opaque file, read as opaque.
        let path = dir.join(format!("{}-opaque.mov", flavour.fourcc));
        export(
            &mut renderer,
            &job(
                &doc,
                settings(flavour, WireAlpha::None, (25, 1), 32, 0),
                path.clone(),
            ),
        );
        let visual = Movie::read(&path)
            .unwrap()
            .track(b"vide")
            .unwrap()
            .visual
            .clone()
            .unwrap();
        let own = apple(&path, 0, rate, flavour, true);
        assert_eq!(visual.depth, 24, "ProRes {} opaque", flavour.name);
        assert!(
            own.alpha.unwrap().iter().all(|a| *a == 1.0),
            "ProRes {} opaque: decoded alpha is full",
            flavour.name
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A composition showing the movie `path` full frame, untouched.
fn untouched(path: &Path) -> (Value, MediaPaths) {
    let dur = FRAMES as f64 / 25.0;
    let mut doc = project(
        W,
        H,
        dur,
        no_fill(),
        vec![layer(
            "v",
            "video",
            json!({ "asset": "clip", "fit": "stretch", "w": W, "h": H, "trim": 0, "speed": 1 }),
            f64::from(W) / 2.0,
            f64::from(H) / 2.0,
            dur,
        )],
    );
    doc["fps"] = json!(25);
    doc["colorDepth"] = json!(32);
    asset(&mut doc, "clip", "video", "mov", W, H, dur);
    let mut media = MediaPaths::default();
    media.insert("clip", path.to_path_buf());
    (doc, media)
}

/// A delivered file comes back in as footage with what it carries (depth,
/// timecode, reel, sound), and re-exporting it untouched three times over
/// stays within the codec's own loss.
#[test]
fn a_delivered_file_comes_back_and_survives_three_generations() {
    use pm_export::pipeline::test_sinks::MemorySink;
    let dir = scratch("generations");
    let mut renderer = Renderer::headless().unwrap();
    let rate = Rate::new(25, 1).unwrap();
    let mut problems = Vec::new();
    for flavour in FLAVOURS {
        let what = format!("ProRes {}", flavour.name);
        let first = dir.join(format!("{}-1.mov", flavour.fourcc));
        let mut s = settings(flavour, WireAlpha::None, (25, 1), 32, 0);
        s.delivery.timecode = Some(ExportTimecode {
            start: "01:00:00:00".into(),
            drop_frame: false,
            reel: Some("A001".into()),
        });
        export(&mut renderer, &job(&document(32, 25.0), s, first.clone()));

        // What the engine's own import reads from it.
        let source = pm_media::decode::service::MediaSource::open(&first).unwrap();
        let probe = &source.probe;
        let video = probe.video.as_ref().unwrap();
        let sound = probe
            .streams
            .iter()
            .find_map(|s| s.audio.as_ref())
            .expect("the file's sound");
        eprintln!(
            "MEASURE {what} imported: codec {:?}; alpha {:?}; {} frames; timecode {:?}, reel {:?}; sound {} {} Hz {} channels {:?}",
            video.codec,
            video.alpha,
            video.frames,
            probe.timecode,
            probe.reel,
            sound.codec,
            sound.sample_rate,
            sound.channels,
            sound.channel_layout
        );
        assert_eq!(video.frames as u64, FRAMES, "{what}");
        assert_eq!(
            probe
                .timecode
                .as_ref()
                .map(|t| (t.text.as_str(), t.drop_frame, t.frame)),
            Some(("01:00:00:00", false, Some(90_000))),
            "{what}: timecode read back"
        );
        assert_eq!(
            probe.reel.as_deref(),
            Some("A001"),
            "{what}: reel read back"
        );
        assert_eq!((sound.sample_rate, sound.channels), (48_000, 2), "{what}");

        // The encoder input of an untouched re-export against the file's
        // own decoded samples: what import and pass-through carry.
        let (doc, media) = untouched(&first);
        let mut again = settings(flavour, WireAlpha::None, (25, 1), 32, 0);
        again.audio = false;
        let job2 = job_with(
            &doc,
            media,
            again.clone(),
            dir.join(format!("{}-2.mov", flavour.fourcc)),
        );
        let handed = run_export_into(
            &mut renderer,
            &job2,
            plan_export(&job2).unwrap(),
            MemorySink::default(),
            &Unrestricted,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap()
        .frames;
        // A decoder may return samples outside the legal codes (a lossy
        // flavour's overshoot at hard edges); the exporter holds them to
        // the codes a stream may carry, as it does for any frame.
        let decoded_first = apple(&first, 0, rate, flavour, false).legal(flavour);
        let handed_first = Picture::of(&handed[0], false);
        let (carried, _) = handed_first.distance(&decoded_first, 0..H as usize);
        let (unclipped, _) =
            handed_first.distance(&apple(&first, 0, rate, flavour, false), 0..H as usize);

        // Generations two and three, each from the one before.
        let mut files = vec![first.clone()];
        for generation in 2..=3 {
            let path = dir.join(format!("{}-{generation}.mov", flavour.fourcc));
            let (doc, media) = untouched(files.last().unwrap());
            export(
                &mut renderer,
                &job_with(&doc, media, again.clone(), path.clone()),
            );
            files.push(path);
        }
        let pictures: Vec<Vec<Picture>> = files
            .iter()
            .map(|f| {
                (0..FRAMES)
                    .map(|n| apple(f, n, rate, flavour, false))
                    .collect()
            })
            .collect();
        let drift = |a: usize, b: usize| {
            (0..FRAMES as usize)
                .map(|k| pictures[a][k].distance(&pictures[b][k], 0..H as usize))
                .fold((0.0f64, 0.0f64), |m, d| (m.0.max(d.0), m.1.max(d.1)))
        };
        let (second, third) = (drift(1, 0), drift(2, 0));
        eprintln!(
            "MEASURE {what} generations: an untouched re-export hands the encoder the file's own samples to ±{carried:.2} codes (±{unclipped:.2} before the decoder's out-of-range samples are held to legal codes); generation 2 is ±{:.2} (rms {:.2}) from generation 1, generation 3 ±{:.2} (rms {:.2})",
            second.0, second.1, third.0, third.1
        );
        // 4:2:2 footage into a 4:2:2 encoder is the file's own samples.
        let carried_bound = if flavour.is_4444() { 1.0 } else { 0.0 };
        if carried > carried_bound {
            problems.push(format!(
                "{what}: an untouched re-export changes the samples by {carried:.2} codes before encoding"
            ));
        }
        // Each generation re-quantizes what the one before decoded to. LT,
        // 422, HQ and 4444 code it to the same samples: every frame of
        // generations two and three is generation one's. XQ and Proxy move
        // a few samples (measured: ±1.5 codes, rms 0.08; ±7.75, rms 0.25),
        // and no further in the third generation than in the second.
        let (most, rms) = match flavour.name {
            "422 Proxy" => (10.0, 0.4),
            "4444 XQ" => (2.0, 0.12),
            _ => (0.0, 0.0),
        };
        for (generation, drift) in [(2, second), (3, third)] {
            if drift.0 > most || drift.1 > rms {
                problems.push(format!(
                    "{what}: generation {generation} is ±{:.2} (rms {:.2}) from generation 1 in some frame, where ±{most} (rms {rms}) is its loss",
                    drift.0, drift.1
                ));
            }
        }
        if third.0 > second.0 + 2.0 {
            problems.push(format!(
                "{what}: generation 3 drifts further (±{:.2}) than generation 2 (±{:.2})",
                third.0, second.0
            ));
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// ProRes from an HDR project: BT.2100 PQ and HLG labels, the mastering
/// display and light level a grading suite reads, and the picture.
#[test]
fn hdr_projects_deliver_pq_and_hlg_prores() {
    let dir = scratch("hdr");
    let mut renderer = Renderer::headless().unwrap();
    let rate = Rate::new(25, 1).unwrap();
    let mut doc = document(32, 25.0);
    doc["color"] = json!({ "mode": "hdr", "masteringPeak": 1000 });
    for flavour in FLAVOURS.into_iter().filter(|f| f.master) {
        for (name, color, cicp, transfer) in [
            ("PQ", OutputColor::Rec2100Pq, (9, 16, 9), "smpte2084"),
            ("HLG", OutputColor::Rec2100Hlg, (9, 18, 9), "arib-std-b67"),
        ] {
            let what = format!("ProRes {} {name}", flavour.name);
            let path = dir.join(format!("{}-{name}.mov", flavour.fourcc));
            let mut s = settings(flavour, WireAlpha::None, (25, 1), 32, 0);
            s.color = color;
            let job = job(&doc, s, path.clone());
            export(&mut renderer, &job);
            let visual = Movie::read(&path)
                .unwrap()
                .track(b"vide")
                .unwrap()
                .visual
                .clone()
                .unwrap();
            let colr = visual.colr.clone().expect("a colr box");
            let probe = ffprobe(&path);
            let v = stream(&probe, "video");
            let want = Picture::of(&rendered(&mut renderer, &job, 0), false);
            let own = apple(&path, 0, rate, flavour, false);
            let other = &ffmpeg(&path, (W, H), flavour, false)[0];
            let smooth = own
                .distance(&want, SMOOTH)
                .0
                .max(other.distance(&want, SMOOTH).0);
            eprintln!(
                "MEASURE {what}: colr {}-{}-{}; mastering display {}, light level {}; ffprobe {} / {} / {}, side data {}; decoded smooth ±{smooth:.2} codes",
                colr.primaries,
                colr.transfer,
                colr.matrix,
                visual.mdcv.is_some(),
                visual.clli.is_some(),
                v["color_primaries"],
                v["color_transfer"],
                v["color_space"],
                v["side_data_list"]
            );
            assert_eq!((colr.primaries, colr.transfer, colr.matrix), cicp, "{what}");
            // The mastering display a 1,000-nit HDR project states, written
            // out by hand: P3 primaries (green, blue, red) and D65 white in
            // steps of 0.00002, then 1,000 and 0 cd/m² in steps of 0.0001.
            let mastering: [u8; 24] = [
                0x33, 0xC2, 0x86, 0xC4, // green 0.265, 0.690
                0x1D, 0x4C, 0x0B, 0xB8, // blue 0.150, 0.060
                0x84, 0xD0, 0x3E, 0x80, // red 0.680, 0.320
                0x3D, 0x13, 0x40, 0x42, // white 0.3127, 0.3290
                0x00, 0x98, 0x96, 0x80, // 10,000,000 × 0.0001 cd/m²
                0x00, 0x00, 0x00, 0x00,
            ];
            assert_eq!(visual.mdcv, Some(mastering), "{what}: mastering display");
            // The composition's light level is not measured (no footage
            // states one), so none is claimed.
            assert_eq!(visual.clli, None, "{what}: content light level");
            let display = v["side_data_list"]
                .as_array()
                .and_then(|list| {
                    list.iter()
                        .find(|d| d["side_data_type"] == "Mastering display metadata")
                })
                .unwrap_or_else(|| panic!("{what}: ffprobe reads no mastering display"));
            assert_eq!(
                (
                    display["red_x"].as_str(),
                    display["green_y"].as_str(),
                    display["white_point_x"].as_str(),
                    display["max_luminance"].as_str(),
                    display["min_luminance"].as_str()
                ),
                (
                    Some("34000/50000"),
                    Some("34500/50000"),
                    Some("15635/50000"),
                    Some("10000000/10000"),
                    Some("0/10000")
                ),
                "{what}: the mastering display as ffprobe reads it"
            );
            assert_eq!(v["color_primaries"], "bt2020", "{what}");
            assert_eq!(v["color_transfer"], transfer, "{what}");
            assert_eq!(v["color_space"], "bt2020nc", "{what}");
            assert!(
                smooth <= flavour.smooth,
                "{what}: smooth picture {smooth:.2} codes from the rendered frame"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Each flavour spends its own data rate: on a detailed HD picture the file
/// is as large as the flavour's published target rate (Apple's ProRes white
/// paper, 1920 × 1080 at 25 fps), where a simple picture needs far less.
#[test]
fn detailed_picture_reaches_each_flavours_data_rate() {
    let dir = scratch("rate");
    let (w, h, frames) = (1920u32, 1080u32, 5u64);
    // Fine detail everywhere: a different value every pixel, over a ramp.
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let mut noise = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 40) as f32 / (1u64 << 24) as f32
    };
    let rgba: Vec<f32> = (0..w * h)
        .flat_map(|i| {
            let ramp = (i % w) as f32 / w as f32;
            [
                0.2 + 0.4 * ramp + 0.4 * noise(),
                0.2 + 0.6 * noise(),
                0.7 - 0.4 * ramp + 0.3 * noise(),
                1.0,
            ]
        })
        .collect();
    // Target megabits a second at 1920 × 1080, 25 fps.
    let targets = [38.0, 85.0, 122.0, 184.0, 275.0, 415.0];
    let mut problems = Vec::new();
    for (flavour, target) in FLAVOURS.into_iter().zip(targets) {
        let path = dir.join(format!("{}.mov", flavour.fourcc));
        let mut job = pm_export::export(ExportSpec {
            width: w,
            height: h,
            rate: Rate::new(25, 1).unwrap(),
            frames,
            codec: Codec::ProRes(match flavour.wire {
                WireProRes::Proxy => ProResProfile::Proxy,
                WireProRes::Lt => ProResProfile::Lt,
                WireProRes::Standard => ProResProfile::Standard,
                WireProRes::Hq => ProResProfile::Hq,
                WireProRes::P4444 => ProResProfile::P4444,
                WireProRes::P4444Xq => ProResProfile::P4444Xq,
            }),
            container: Container::Mov,
            tags: ColorTags::Rec709QuickTime,
            alpha: AlphaOutput::None,
            working: WorkingEncoding::Standard,
            dither: Dither::Default,
            audio: None,
            timecode: None,
            destination: path.clone(),
            overwrite: Overwrite::Replace,
            mode: None,
        })
        .unwrap();
        for _ in 0..frames {
            job.push_frame(&WorkingFrame {
                width: w,
                height: h,
                rgba: &rgba,
            })
            .unwrap();
        }
        // The delivery's own check of the file (block means of the decoded
        // picture against what the encoder was handed) passes: detail a
        // lean flavour codes coarsely is not a broken file.
        let delivered = match job.finish() {
            Ok(delivered) => delivered,
            Err(e) => {
                problems.push(format!("ProRes {}: {e}", flavour.name));
                continue;
            }
        };
        let rate = delivered.report.bytes as f64 * 8.0 * 25.0 / frames as f64 / 1e6;
        eprintln!(
            "MEASURE ProRes {} on detailed 1920 × 1080 at 25 fps: {rate:.0} Mb/s (target {target:.0}); block means within {:.2} codes of the encoder input",
            flavour.name,
            delivered.report.max_pixel_error_cv()
        );
        if !(0.85 * target..=1.15 * target).contains(&rate) {
            problems.push(format!(
                "ProRes {}: {rate:.0} Mb/s, target {target:.0}",
                flavour.name
            ));
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The best straight line through a channel's decoded patch values against
/// their known codes: (gain, offset at the channel's middle code, what is
/// left over at worst).
fn fitted(residuals: &[[f64; 3]], flavour: Flavour, k: usize) -> (f64, f64, f64) {
    let (lo, ls, co, _) = flavour.levels();
    let middle = if k == 0 { lo + ls / 2.0 } else { co };
    let points: Vec<(f64, f64)> = PATCHES
        .iter()
        .zip(residuals)
        .map(|((_, color), r)| {
            let want = known(color, flavour)[k];
            (want - middle, want + r[k] - middle)
        })
        .collect();
    let n = points.len() as f64;
    let (sx, sy) = points
        .iter()
        .fold((0.0, 0.0), |s, p| (s.0 + p.0, s.1 + p.1));
    let (sxx, sxy) = points
        .iter()
        .fold((0.0, 0.0), |s, p| (s.0 + p.0 * p.0, s.1 + p.0 * p.1));
    let gain = (n * sxy - sx * sy) / (n * sxx - sx * sx);
    let offset = (sy - gain * sx) / n;
    let left = points
        .iter()
        .map(|p| (p.1 - offset - gain * p.0).abs())
        .fold(0.0, f64::max);
    (gain, offset, left)
}

/// ProRes 422 Proxy puts flat patches up to 6.7 codes from their colours.
/// Whether that is the encoder's quantisation or a mistake in the matrix or
/// range on the way to it shows in the sign and pattern of the error: a
/// range mistake is a gain (0.86 or 1.17) and an offset (64 codes) common to
/// every patch, a matrix mistake moves the saturated patches by tens of
/// codes and leaves the greys alone. The same frame is also coded by
/// Apple's Proxy encoder from FFmpeg's own conversion (none of this crate's
/// colour code), and by FFmpeg's own Proxy encoder.
#[test]
fn the_proxy_error_on_flat_patches_is_the_encoders_quantisation() {
    let dir = scratch("proxy");
    let mut renderer = Renderer::headless().unwrap();
    let rate = Rate::new(25, 1).unwrap();
    let (proxy, hq) = (FLAVOURS[0], FLAVOURS[3]);
    let table = |residuals: &[[f64; 3]]| -> String {
        PATCHES
            .iter()
            .zip(residuals)
            .map(|((_, color), r)| format!("{color} {:+.2} {:+.2} {:+.2}", r[0], r[1], r[2]))
            .collect::<Vec<_>>()
            .join(" | ")
    };
    let mut problems = Vec::new();
    for (w, h) in [(W, H), (1920u32, 1080u32)] {
        let doc = document_sized(w, h, 32, 25.0);
        let export_as = |renderer: &mut Renderer, flavour: Flavour| {
            let path = dir.join(format!("{}-{w}.mov", flavour.fourcc));
            let mut s = settings(flavour, WireAlpha::None, (25, 1), 32, 0);
            s.size = Size::new(w, h);
            s.audio = false;
            let job = job(&doc, s, path.clone());
            export(renderer, &job);
            (job, path)
        };
        let (job, ours) = export_as(&mut renderer, proxy);
        let (_, master) = export_as(&mut renderer, hq);
        // What the encoder was handed, and what each file decodes to.
        let handed = patch_residuals(
            &Picture::of(&rendered(&mut renderer, &job, 0), false),
            proxy,
        );
        let decoded = patch_residuals(&apple(&ours, 0, rate, proxy, false), proxy);
        let foreign = patch_residuals(&ffmpeg(&ours, (w, h), proxy, false)[0], proxy);
        let best = patch_residuals(&apple(&master, 0, rate, hq, false), hq);
        eprintln!(
            "MEASURE Proxy at {w} × {h}, patch (Y′ Cb Cr) less its known codes: handed to the encoder: {}",
            table(&handed)
        );
        eprintln!(
            "MEASURE Proxy at {w} × {h}, our file by Apple's decoder: {}",
            table(&decoded)
        );
        eprintln!(
            "MEASURE Proxy at {w} × {h}, our file by FFmpeg's decoder: {}",
            table(&foreign)
        );
        eprintln!(
            "MEASURE 422 HQ at {w} × {h}, the same frame: {}",
            table(&best)
        );
        for (k, name) in ["Y′", "Cb", "Cr"].into_iter().enumerate() {
            let (gain, offset, left) = fitted(&decoded, proxy, k);
            let mean = decoded.iter().map(|r| r[k]).sum::<f64>() / decoded.len() as f64;
            eprintln!(
                "MEASURE Proxy at {w} × {h}, {name} over the nine patches: gain {gain:.4}, offset {offset:+.2} codes at mid-scale, {left:.2} codes left over at worst; mean error {mean:+.2} codes"
            );
            // A range mistake: gain 0.855 or 1.169, offset tens of codes.
            if (gain - 1.0).abs() > 0.01 || offset.abs() > 2.0 {
                problems.push(format!(
                    "{w} × {h} {name}: gain {gain:.4}, offset {offset:+.2}: the levels are off"
                ));
            }
        }
        // A matrix mistake (BT.601 for BT.709) moves the luma of the
        // saturated patches by tens of codes: every patch's luma is right.
        let luma = decoded.iter().fold(0.0f64, |m, r| m.max(r[0].abs()));
        if luma > 1.0 {
            problems.push(format!("{w} × {h}: a patch's luma is {luma:.2} codes off"));
        }
        // The encoder was handed the right codes.
        let worst_handed = handed.iter().flatten().fold(0.0f64, |m, r| m.max(r.abs()));
        if worst_handed > 0.6 {
            problems.push(format!(
                "{w} × {h}: the encoder was handed patches {worst_handed:.2} codes off"
            ));
        }
        // The same frame through tools that share none of this crate's
        // colour code: the working frame as 16-bit RGB, converted by FFmpeg
        // (BT.709, video range), coded by Apple's Proxy encoder and by
        // FFmpeg's own.
        let plan = plan_export(&job).unwrap();
        let read = render_frame(
            &mut renderer,
            &plan.document,
            &job.composition,
            plan.timeline.tick(0),
            &plan.request,
            &job.media,
            &Unrestricted,
            pass_key(job.seed, plan.request.output_size),
            &AtomicBool::new(false),
        )
        .unwrap();
        let rgb: Vec<u8> = read
            .rgba
            .chunks_exact(4)
            .flat_map(|p| [p[0], p[1], p[2]])
            .flat_map(|c| ((f64::from(c).clamp(0.0, 1.0) * 65535.0).round() as u16).to_le_bytes())
            .collect();
        let raw = dir.join(format!("frame-{w}.rgb48"));
        std::fs::write(&raw, &rgb).unwrap();
        for (label, encoder) in [
            (
                "Apple's Proxy encoder from FFmpeg's conversion",
                vec![
                    "-c:v",
                    "prores_videotoolbox",
                    "-profile:v",
                    "proxy",
                    "-pix_fmt",
                    "p210le",
                ],
            ),
            (
                "FFmpeg's own Proxy encoder",
                vec![
                    "-c:v",
                    "prores_ks",
                    "-profile:v",
                    "0",
                    "-pix_fmt",
                    "yuv422p10le",
                ],
            ),
        ] {
            let path = dir.join(format!("{}-{w}.mov", encoder[1]));
            let size = format!("{w}x{h}");
            let out = Command::new("ffmpeg")
                .args(["-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgb48le", "-s", size.as_str(), "-r", "25", "-i"])
                .arg(&raw)
                .args(["-vf", "scale=in_range=full:out_range=limited:out_color_matrix=bt709:flags=accurate_rnd+full_chroma_int"])
                .args(&encoder)
                .args(["-frames:v", "1"])
                .arg(&path)
                .output()
                .unwrap();
            if !out.status.success() {
                eprintln!(
                    "MEASURE Proxy at {w} × {h}, {label}: not available here ({})",
                    String::from_utf8_lossy(&out.stderr).trim()
                );
                continue;
            }
            let theirs = patch_residuals(&ffmpeg(&path, (w, h), proxy, false)[0], proxy);
            let apart = theirs
                .iter()
                .flatten()
                .zip(foreign.iter().flatten())
                .map(|(a, b)| (a - b).abs())
                .fold(0.0, f64::max);
            let worst = theirs.iter().flatten().fold(0.0f64, |m, r| m.max(r.abs()));
            eprintln!("MEASURE Proxy at {w} × {h}, {label}: {}", table(&theirs));
            eprintln!(
                "MEASURE Proxy at {w} × {h}, {label}: patches ±{worst:.2} of their known codes, ±{apart:.2} from our file's (same decoder)"
            );
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
    let _ = std::fs::remove_dir_all(&dir);
}
