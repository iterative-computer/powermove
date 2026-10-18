//! Section 25's video class against its FFmpeg reference, measured for the
//! Phase 3a gate (docs/engine/phase3a-gate.md): "the engine within 1 CV of an
//! FFmpeg plus zimg reference using the same chroma filter".
//!
//! Two sets of sources. The e2e fixture videos (`apps/desktop/e2e/fixtures`)
//! are flat: every frame is one Y'CbCr triple (ffmpeg `signalstats` over 24
//! frames, e.g. `prores422.mov` Y = 324, U = 360, V = 960 throughout), so they
//! check the matrix and range conversion only and never exercise chroma
//! upsampling, siting, or the filter. The chroma-edge sources are generated
//! here with the pinned FFmpeg from `testsrc2` (saturated bars, a moving
//! pattern, text: chroma codes 16 to 240 with edges both ways) in 4:2:0,
//! 4:2:2 and 4:4:4, through ProRes, H.264 and HEVC Main 10 (VideoToolbox
//! encoders), FFV1, VP9, and AV1, with
//! left, center and top-left siting; they are the class as written.
//!
//! Each source is drawn alone by the renderer at its native size (one layer,
//! `contain`, centered, so every output pixel is one source pixel) at 32-bit,
//! and compared with frame n decoded by FFmpeg and converted by zimg to
//! 32-bit float RGB (no clipping) with the matrix, range and siting the engine
//! reads from the file (`Renderer::video_color`) and the engine's filter
//! (pm-color `chroma.rs`): 4:2:2 linear across (zimg `bilinear`); 4:2:0
//! linear between rows and Catmull-Rom across, which zimg cannot do in one
//! pass (one filter for both axes), so the reference runs two zscale passes:
//! 4:2:0 to 4:2:2 at 16 bits with `bilinear` (vertical only), then 4:2:2 to
//! RGB with `bicubic` b = 0, c = 0.5 (horizontal only), both with the file's
//! siting. ProRes is also compared with FFmpeg's VideoToolbox decode (the
//! engine decodes ProRes with VideoToolbox only), to separate the decoder
//! from the chroma path. The corpus's own generated H.264 media (the newest
//! `motion-*` capture), when present, is measured the same way. Alpha sources compare color where the reference is opaque and
//! alpha everywhere. Interior figures leave out a 2-pixel border, where the
//! two edge extensions may differ.
//!
//! A measurement of the gate, not a regression test:
//! `cargo test --release -p pm-cli --test phase3a_video_class -- --ignored --nocapture`.

use std::path::{Path, PathBuf};
use std::process::Command;

use pm_protocol::color::{
    ChromaSiting, ColorDescription, MatrixCoefficients, Precision, Range, Subsampling,
};
use pm_protocol::common::{Size, Ticks};
use pm_render::{MapResolver, RenderRequest, Renderer};
use serde_json::{Value, json};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn ffmpeg() -> PathBuf {
    std::env::var_os("PM_FFMPEG_DIR")
        .map(|d| PathBuf::from(d).join("bin/ffmpeg"))
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from("ffmpeg"))
}

fn ch(v: Value) -> Value {
    json!({ "v": v, "kf": [], "expr": null })
}

fn document(w: u32, h: u32, fps: f64, dur: f64) -> pm_model::Document {
    // Today's `videoPlaybackTime` clamps the source time to the asset's
    // duration less an end padding (0.04 s in the viewer, one frame for
    // final output; `pm_render::retime::playback_time`), so the last 0.04 s
    // of an asset hold the frame before. The asset is declared longer than
    // the frames measured so every measured frame is the frame at its time.
    let asset_dur = dur + 0.04 + 1.0 / fps;
    let p = json!({
        "perspective": ch(json!(50)), "position.z": ch(json!(0)), "anchor.z": ch(json!(0)),
        "scale.z": ch(json!(100)), "rotation.x": ch(json!(0)), "rotation.y": ch(json!(0)),
        "orientation.x": ch(json!(0)), "orientation.y": ch(json!(0)), "orientation.z": ch(json!(0)),
        "anchor.x": ch(json!(0)), "anchor.y": ch(json!(0)),
        "position.x": ch(json!(f64::from(w) / 2.0)), "position.y": ch(json!(f64::from(h) / 2.0)),
        "scale.x": ch(json!(100)), "scale.y": ch(json!(100)),
        "rotation": ch(json!(0)), "opacity": ch(json!(100)), "skew": ch(json!(0))
    });
    let layer = json!({
        "id": "v", "type": "video", "name": "v", "from": 0, "dur": dur, "on": true,
        "lock": false, "shy": false, "collapsed": true, "threeD": false, "solo": false,
        "color": "#3B62E8", "blend": "normal", "mblur": false, "parent": null, "group": null,
        "matteSource": null, "p": p, "fx": [], "transitionIn": null, "transitionOut": null,
        "masks": [], "d": { "asset": "clip", "fit": "contain", "trim": 0, "speed": 1,
        "embeddedAudio": false, "w": w, "h": h }, "locked_intent": {}
    });
    let doc = json!({
        "id": "Pvideo", "name": "video", "w": w, "h": h, "fps": fps, "dur": dur, "bg": "#000000",
        "backgroundFill": { "type": "none", "angle": 0, "stops": [{ "id": "s1", "color": "#000000", "position": 0 }] },
        "compId": "Pvideo-root", "compName": "video", "markers": [], "work": [0, dur], "params": {},
        "revision": 0, "edits": [], "created": 0, "shutter": 0.5, "layers": [layer], "comps": {},
        "assets": { "clip": { "id": "clip", "name": "clip", "kind": "video", "w": w, "h": h,
            "dur": asset_dur, "fps": fps, "persisted": true } }
    });
    pm_model::Document::new(Some(pm_model::json::parse(&doc.to_string()).unwrap()), 1)
}

/// Frame `n` decoded by FFmpeg and converted by zimg to 32-bit float RGBA,
/// interleaved, with the engine's reading of the file and its chroma filter.
///
/// `repeat_edges` extends the decoded planes by 8 pixels of repeated edge
/// samples (`fillborders` `smear`) before zimg runs and crops them after, so
/// the filter sees the engine's edge rule (pm-color `chroma.rs`: taps outside
/// the plane are clamped, edge samples repeat) instead of zimg's own.
/// `videotoolbox` decodes with FFmpeg's VideoToolbox hwaccel instead of its
/// software decoder (a diagnostic for ProRes, which the engine decodes with
/// VideoToolbox), converting the downloaded frame to planar 16-bit at its own
/// subsampling (no resampling) before zimg.
fn reference(
    path: &Path,
    n: u32,
    color: &Reading,
    alpha: bool,
    repeat_edges: bool,
    videotoolbox: bool,
) -> Vec<f32> {
    let Reading {
        matrix,
        range,
        chromal,
        subsampling,
    } = color;
    let out_fmt = if alpha { "gbrapf32le" } else { "gbrpf32le" };
    let to_rgb = |filter: &str| {
        format!(
            "zscale=matrixin={matrix}:rangein={range}:range=full:chromalin={chromal}:chromal={chromal}:filter={filter}:dither=none,format={out_fmt}"
        )
    };
    let chain = match subsampling {
        Subsampling::S444 | Subsampling::S422 => to_rgb("bilinear"),
        Subsampling::S420 => {
            let mid = if alpha { "yuva422p16le" } else { "yuv422p16le" };
            format!(
                "zscale=rangein={range}:range={range}:chromalin={chromal}:chromal={chromal}:filter=bilinear:dither=none,format={mid},{}",
                to_rgb("bicubic:param_a=0:param_b=0.5")
            )
        }
    };
    let planar16 = match (subsampling, alpha) {
        (Subsampling::S444, true) => "yuva444p16le",
        (Subsampling::S444, false) => "yuv444p16le",
        (Subsampling::S422, true) => "yuva422p16le",
        (Subsampling::S422, false) => "yuv422p16le",
        (Subsampling::S420, true) => "yuva420p16le",
        (Subsampling::S420, false) => "yuv420p16le",
    };
    let mut vf = format!("select=eq(n\\,{n})");
    if videotoolbox {
        vf += &format!(",format={planar16}");
    }
    if repeat_edges {
        vf += ",pad=iw+16:ih+16:8:8,fillborders=left=8:right=8:top=8:bottom=8:mode=smear";
    }
    vf += &format!(",{chain}");
    if repeat_edges {
        vf += ",crop=iw-16:ih-16:8:8";
    }
    let mut cmd = Command::new(ffmpeg());
    cmd.args(["-v", "error"]);
    if videotoolbox {
        cmd.args(["-hwaccel", "videotoolbox"]);
    }
    let out = cmd
        .arg("-i")
        .arg(path)
        .args(["-vf", &vf, "-frames:v", "1", "-f", "rawvideo", "-"])
        .output()
        .expect("ffmpeg runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let planar: Vec<f32> = out
        .stdout
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    let planes = if alpha { 4 } else { 3 };
    let px = planar.len() / planes;
    // Planes G, B, R (, A).
    (0..px)
        .flat_map(|i| {
            [
                planar[2 * px + i],
                planar[i],
                planar[px + i],
                if alpha { planar[3 * px + i] } else { 1.0 },
            ]
        })
        .collect()
}

/// Frame `n` decoded by FFmpeg (software) to its native planes, upsampled
/// and converted in f64 by this test's own implementation of the plan's
/// filter (section 9: 4:2:2 linear across; 4:2:0 Catmull-Rom across and
/// linear between rows; siting-aware; taps clamped to the plane) with no
/// clipping anywhere. A diagnostic beside zimg, whose word-type resize
/// clamps chroma overshoot at the 16-bit code floor and ceiling. Opaque
/// sources only; RGB interleaved, alpha 1.
fn reference_f64(path: &Path, n: u32, color: &Tags, w: u32, h: u32) -> Vec<f64> {
    let depth = probe_depth(path);
    let sub = match color.subsampling {
        Subsampling::S444 => "444",
        Subsampling::S422 => "422",
        Subsampling::S420 => "420",
    };
    let fmt = if depth == 8 {
        format!("yuv{sub}p")
    } else {
        format!("yuv{sub}p{depth}le")
    };
    let out = Command::new(ffmpeg())
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-vf", &format!("select=eq(n\\,{n})"), "-frames:v", "1"])
        .args(["-pix_fmt", &fmt, "-f", "rawvideo", "-"])
        .output()
        .expect("ffmpeg runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let samples: Vec<f64> = if depth == 8 {
        out.stdout.iter().map(|&b| f64::from(b)).collect()
    } else {
        out.stdout
            .chunks_exact(2)
            .map(|b| f64::from(u16::from_le_bytes([b[0], b[1]])))
            .collect()
    };
    // Luma pixels per chroma sample.
    let (fx, fy): (u32, u32) = match color.subsampling {
        Subsampling::S444 => (1, 1),
        Subsampling::S422 => (2, 1),
        Subsampling::S420 => (2, 2),
    };
    let (cw, ch) = (w.div_ceil(fx) as usize, h.div_ceil(fy) as usize);
    let (w, h) = (w as usize, h as usize);
    let y_plane = &samples[..w * h];
    let u_plane = &samples[w * h..w * h + cw * ch];
    let v_plane = &samples[w * h + cw * ch..w * h + 2 * cw * ch];
    // Chroma sample (0, 0) on the luma grid.
    let ox = if fx == 1 || color.siting.horizontally_cosited() {
        0.0
    } else {
        0.5
    };
    let oy = if fy == 1 {
        0.0
    } else {
        match color.siting {
            ChromaSiting::TopLeft | ChromaSiting::Top => 0.0,
            ChromaSiting::BottomLeft | ChromaSiting::Bottom => 1.0,
            ChromaSiting::Left | ChromaSiting::Center => 0.5,
        }
    };
    let taps = |pos: f64, size: usize, cubic: bool| -> Vec<(usize, f64)> {
        let i0 = pos.floor();
        let t = pos - i0;
        let at = |k: f64| (i0 + k).clamp(0.0, size as f64 - 1.0) as usize;
        if cubic {
            let w = [
                -0.5 * t * t * t + t * t - 0.5 * t,
                1.5 * t * t * t - 2.5 * t * t + 1.0,
                -1.5 * t * t * t + 2.0 * t * t + 0.5 * t,
                0.5 * t * t * t - 0.5 * t * t,
            ];
            (0..4).map(|k| (at(k as f64 - 1.0), w[k])).collect()
        } else {
            vec![(at(0.0), 1.0 - t), (at(1.0), t)]
        }
    };
    let cubic_x = color.subsampling == Subsampling::S420;
    let scale = f64::from(1u32 << (depth - 8));
    let max = f64::from((1u32 << depth) - 1);
    let (kr, kb) = match color.matrix {
        MatrixCoefficients::Bt709 => (0.2126, 0.0722),
        MatrixCoefficients::Bt2020Ncl => (0.2627, 0.0593),
        _ => (0.299, 0.114),
    };
    let kg = 1.0 - kr - kb;
    let mut rgb = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let ty = taps((y as f64 - oy) / f64::from(fy), ch, false);
        for x in 0..w {
            let tx = taps((x as f64 - ox) / f64::from(fx), cw, cubic_x);
            let (mut u, mut v) = (0.0, 0.0);
            for &(j, wy) in &ty {
                for &(i, wx) in &tx {
                    u += wy * wx * u_plane[j * cw + i];
                    v += wy * wx * v_plane[j * cw + i];
                }
            }
            let yy = y_plane[y * w + x];
            let (yn, cb, cr) = if color.range == Range::Full {
                (
                    yy / max,
                    (u - 128.0 * scale) / max,
                    (v - 128.0 * scale) / max,
                )
            } else {
                (
                    (yy - 16.0 * scale) / (219.0 * scale),
                    (u - 128.0 * scale) / (224.0 * scale),
                    (v - 128.0 * scale) / (224.0 * scale),
                )
            };
            let r = yn + 2.0 * (1.0 - kr) * cr;
            let b = yn + 2.0 * (1.0 - kb) * cb;
            let g = (yn - kr * r - kb * b) / kg;
            rgb.extend([r, g, b, 1.0]);
        }
    }
    rgb
}

/// The frames measured from each generated source and fixture.
const SAMPLED: [u32; 4] = [0, 7, 15, 23];

/// Frame count of the first video stream.
fn probe_frames(path: &Path) -> u32 {
    let out = Command::new(ffprobe())
        .args(["-v", "error", "-select_streams", "v:0", "-count_frames"])
        .args(["-show_entries", "stream=nb_read_frames", "-of", "csv=p=0"])
        .arg(path)
        .output()
        .expect("ffprobe runs");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0)
}

/// Bits per sample of the first video stream's pixel format.
fn probe_depth(path: &Path) -> u32 {
    let out = Command::new(ffprobe())
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries"])
        .args(["stream=pix_fmt", "-of", "csv=p=0"])
        .arg(path)
        .output()
        .expect("ffprobe runs");
    let fmt = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if fmt.contains("12") {
        12
    } else if fmt.contains("10") {
        10
    } else {
        8
    }
}

fn ffprobe() -> PathBuf {
    let p = ffmpeg().with_file_name("ffprobe");
    if p.exists() {
        p
    } else {
        PathBuf::from("ffprobe")
    }
}

/// Width, height, and frame rate of the first video stream.
fn probe(path: &Path) -> (u32, u32, f64) {
    let out = Command::new(ffprobe())
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries"])
        .args(["stream=width,height,r_frame_rate", "-of", "csv=p=0"])
        .arg(path)
        .output()
        .expect("ffprobe runs");
    let text = String::from_utf8_lossy(&out.stdout);
    let f: Vec<&str> = text.trim().split(',').collect();
    let (num, den) = f[2].split_once('/').unwrap_or((f[2], "1"));
    (
        f[0].parse().unwrap(),
        f[1].parse().unwrap(),
        num.parse::<f64>().unwrap() / den.parse::<f64>().unwrap(),
    )
}

/// The corpus's generated H.264 sources (`motion-*` captures), which the
/// Chromium comparison of `retime/speed` and `space25d/precomps` decodes.
fn corpus_sources() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME") else {
        return vec![];
    };
    let base = PathBuf::from(home).join("Library/Application Support/Powermove Engine Corpus");
    let mut captures: Vec<PathBuf> = std::fs::read_dir(&base)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("motion-"))
        })
        .collect();
    captures.sort();
    let Some(latest) = captures.pop() else {
        return vec![];
    };
    ["corpus-frames-24.mp4", "corpus-frames-30.mp4"]
        .iter()
        .map(|n| latest.join("generated/media").join(n))
        .filter(|p| p.exists())
        .collect()
}

/// What a file's tags say, read by ffprobe and interpreted with section 9's
/// rules for untagged sources, written here from the plan rather than from
/// `pm-media`'s code: untagged matrix BT.601 at heights up to 576, else
/// BT.709; MJPEG, `yuvj` and RGB full range, other Y′CbCr limited; untagged
/// siting left (type 0). The references are built from this reading, and the
/// engine's reading (`Renderer::video_color`) is asserted equal to it, so a
/// file whose tags the engine misreads fails instead of being compared with
/// a reference built from the same misreading.
#[derive(Clone, Debug)]
struct Tags {
    matrix: MatrixCoefficients,
    range: Range,
    siting: ChromaSiting,
    subsampling: Subsampling,
    /// ffprobe's own words, for the report.
    raw: String,
}

fn file_tags(path: &Path) -> Tags {
    let out = Command::new(ffprobe())
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries"])
        .args([
            "stream=codec_name,height,pix_fmt,color_space,color_range,color_primaries,color_transfer,chroma_location",
            "-of",
            "default=nw=1",
        ])
        .arg(path)
        .output()
        .expect("ffprobe runs");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let field = |k: &str| -> String {
        text.lines()
            .find_map(|l| l.strip_prefix(&format!("{k}=")))
            .unwrap_or("")
            .trim()
            .to_string()
    };
    let (codec, pix_fmt) = (field("codec_name"), field("pix_fmt"));
    let height: u32 = field("height").parse().expect("a height");
    let rgb = ["gbr", "rgb", "bgr"].iter().any(|p| pix_fmt.starts_with(p));
    let matrix = if rgb {
        MatrixCoefficients::Identity
    } else {
        match field("color_space").as_str() {
            "bt709" => MatrixCoefficients::Bt709,
            "smpte170m" | "bt470bg" => MatrixCoefficients::Bt601,
            "bt2020nc" => MatrixCoefficients::Bt2020Ncl,
            "" | "unknown" | "gbr" => {
                if height <= 576 {
                    MatrixCoefficients::Bt601
                } else {
                    MatrixCoefficients::Bt709
                }
            }
            other => panic!("{}: matrix {other} is not in the class", path.display()),
        }
    };
    let jpeg_like = codec == "mjpeg" || codec == "mjpegb" || pix_fmt.starts_with("yuvj");
    let range = match field("color_range").as_str() {
        _ if rgb || jpeg_like => Range::Full,
        "pc" => Range::Full,
        _ => Range::Limited,
    };
    let siting = match field("chroma_location").as_str() {
        "center" => ChromaSiting::Center,
        "topleft" => ChromaSiting::TopLeft,
        "top" => ChromaSiting::Top,
        "bottomleft" => ChromaSiting::BottomLeft,
        "bottom" => ChromaSiting::Bottom,
        _ => ChromaSiting::Left,
    };
    let subsampling = if pix_fmt.contains("444") {
        Subsampling::S444
    } else if pix_fmt.contains("422") {
        Subsampling::S422
    } else if pix_fmt.contains("420") || pix_fmt.starts_with("nv12") {
        Subsampling::S420
    } else {
        panic!(
            "{}: pixel format {pix_fmt} is not in the class",
            path.display()
        )
    };
    let raw = format!(
        "{pix_fmt}, matrix {}, range {}, primaries {}, transfer {}, siting {}",
        or_untagged(field("color_space")),
        or_untagged(field("color_range")),
        or_untagged(field("color_primaries")),
        or_untagged(field("color_transfer")),
        or_untagged(field("chroma_location")),
    );
    Tags {
        matrix,
        range,
        siting,
        subsampling,
        raw,
    }
}

fn or_untagged(v: String) -> String {
    if v.is_empty() || v == "unknown" || v == "unspecified" {
        "untagged".into()
    } else {
        v
    }
}

/// Whether the engine's reading equals the tags' (siting where the source
/// is subsampled, vertical siting where it is 4:2:0).
fn reading_matches(engine: &ColorDescription, tags: &Tags) -> bool {
    let siting = match tags.subsampling {
        Subsampling::S444 => true,
        Subsampling::S422 => {
            engine.siting.horizontally_cosited() == tags.siting.horizontally_cosited()
        }
        Subsampling::S420 => engine.siting == tags.siting,
    };
    engine.matrix == tags.matrix
        && engine.range == tags.range
        && engine.subsampling == tags.subsampling
        && siting
}

/// The tags' reading, in zscale's names.
struct Reading {
    matrix: &'static str,
    range: &'static str,
    chromal: &'static str,
    subsampling: Subsampling,
}

/// Generates the chroma-edge sources into `dir` with the pinned FFmpeg.
/// Returns (file, encoding) pairs.
fn generate_edge_sources(dir: &Path) -> Vec<(PathBuf, &'static str)> {
    const SRC: &str = "testsrc2=s=160x90:r=30:d=1";
    let alpha_src = "testsrc2=s=160x90:r=30:d=1,format=rgb24[c];\
        color=c=white:s=160x90:r=30:d=1,format=gray,geq=lum='if(lt(mod(X+2*Y,40),20),255,0)'[a];\
        [c][a]alphamerge";
    let sets: [(&str, &str, bool, &[&str]); 9] = [
        (
            "edges-prores422hq.mov",
            "ProRes 422 HQ (VideoToolbox), 4:2:2 10-bit, left",
            false,
            &[
                "-c:v",
                "prores_videotoolbox",
                "-profile:v",
                "hq",
                "-pix_fmt",
                "p210le",
            ],
        ),
        (
            "edges-prores4444.mov",
            "ProRes 4444 (VideoToolbox), 4:4:4 12-bit",
            false,
            &[
                "-c:v",
                "prores_videotoolbox",
                "-profile:v",
                "4444",
                "-pix_fmt",
                "p410le",
            ],
        ),
        (
            "edges-prores4444-alpha.mov",
            "ProRes 4444 with alpha edges (VideoToolbox)",
            true,
            &[
                "-c:v",
                "prores_videotoolbox",
                "-profile:v",
                "4444",
                "-pix_fmt",
                "ayuv64le",
            ],
        ),
        (
            "edges-ffv1-422p10.mkv",
            "FFV1, 4:2:2 10-bit, left",
            false,
            &["-c:v", "ffv1", "-pix_fmt", "yuv422p10le"],
        ),
        (
            "edges-ffv1-444.mkv",
            "FFV1, 4:4:4 8-bit",
            false,
            &["-c:v", "ffv1", "-pix_fmt", "yuv444p"],
        ),
        (
            "edges-ffv1-420-left.mkv",
            "FFV1, 4:2:0 8-bit, left",
            false,
            &[
                "-c:v",
                "ffv1",
                "-pix_fmt",
                "yuv420p",
                "-chroma_sample_location",
                "left",
            ],
        ),
        (
            "edges-ffv1-420-center.mkv",
            "FFV1, 4:2:0 8-bit, center",
            false,
            &[
                "-c:v",
                "ffv1",
                "-pix_fmt",
                "yuv420p",
                "-chroma_sample_location",
                "center",
            ],
        ),
        (
            "edges-ffv1-420-topleft.mkv",
            "FFV1, 4:2:0 8-bit, top-left",
            false,
            &[
                "-c:v",
                "ffv1",
                "-pix_fmt",
                "yuv420p",
                "-chroma_sample_location",
                "topleft",
            ],
        ),
        (
            "edges-vp9-420.webm",
            "VP9 lossless, 4:2:0 8-bit",
            false,
            &[
                "-c:v",
                "libvpx-vp9",
                "-lossless",
                "1",
                "-pix_fmt",
                "yuv420p",
            ],
        ),
    ];
    let mut out = Vec::new();
    let mut run = |file: &str, label: &'static str, alpha: bool, codec: &[&str]| {
        let path = dir.join(file);
        let mut cmd = Command::new(ffmpeg());
        cmd.args(["-hide_banner", "-v", "error", "-y"]);
        if alpha {
            cmd.args(["-filter_complex", alpha_src]);
        } else {
            cmd.args(["-f", "lavfi", "-i", SRC]);
        }
        cmd.args(codec).args(BT601_TV);
        let status = cmd.arg(&path).output().expect("ffmpeg runs");
        if status.status.success() {
            out.push((path, label));
        } else {
            println!(
                "{file}: not generated ({})",
                String::from_utf8_lossy(&status.stderr).trim()
            );
        }
    };
    for (file, label, alpha, codec) in sets {
        run(file, label, alpha, codec);
    }
    run(
        "edges-av1-420.mp4",
        "AV1 (SVT, CRF 10), 4:2:0 8-bit, left",
        false,
        &[
            "-c:v",
            "libsvtav1",
            "-pix_fmt",
            "yuv420p",
            "-crf",
            "10",
            "-svtav1-params",
            "lp=2",
        ],
    );
    run(
        "edges-h264-vt.mp4",
        "H.264 (VideoToolbox), 4:2:0 8-bit, left",
        false,
        &[
            "-c:v",
            "h264_videotoolbox",
            "-pix_fmt",
            "nv12",
            "-q:v",
            "90",
        ],
    );
    run(
        "edges-hevc-main10-vt.mp4",
        "HEVC Main 10 (VideoToolbox), 4:2:0 10-bit, left",
        false,
        &[
            "-c:v",
            "hevc_videotoolbox",
            "-profile:v",
            "main10",
            "-pix_fmt",
            "p010le",
            "-q:v",
            "90",
            "-tag:v",
            "hvc1",
        ],
    );
    // Tagged other than BT.601 limited: the samples are converted from
    // testsrc2's RGB with the tagged matrix and range (`scale`), then the
    // frames carry the tags (`setparams`), which the encoders and muxers
    // write (Matroska `Colour`, MOV `colr`, HEVC VUI).
    let tagged = |matrix: &str, primaries: &str, trc: &str, range: &str, fmt: &str| {
        let scale_range = if range == "pc" { "full" } else { "limited" };
        let scale_matrix = if matrix == "bt2020nc" {
            "bt2020"
        } else {
            matrix
        };
        format!(
            "scale=out_color_matrix={scale_matrix}:out_range={scale_range},format={fmt},\
             setparams=colorspace={matrix}:color_primaries={primaries}:color_trc={trc}:range={range}"
        )
    };
    let tagged_sets: [(&str, &'static str, String, &[&str]); 5] = [
        (
            "edges-ffv1-420-bt709.mkv",
            "FFV1, 4:2:0 8-bit, BT.709 limited, left (tagged)",
            tagged("bt709", "bt709", "bt709", "tv", "yuv420p"),
            &["-c:v", "ffv1", "-chroma_sample_location", "left"],
        ),
        (
            "edges-ffv1-420-bt709-full.mkv",
            "FFV1, 4:2:0 8-bit, BT.709 full range",
            tagged("bt709", "bt709", "bt709", "pc", "yuv420p"),
            // FFV1's range is written from the encoder option, not the frame.
            &["-c:v", "ffv1", "-color_range", "pc"],
        ),
        (
            "edges-ffv1-420p10-bt2020nc.mkv",
            "FFV1, 4:2:0 10-bit, BT.2020 NCL matrix, BT.709 primaries, limited",
            tagged("bt2020nc", "bt709", "bt709", "tv", "yuv420p10le"),
            &["-c:v", "ffv1"],
        ),
        (
            "edges-prores422hq-bt709.mov",
            "ProRes 422 HQ (VideoToolbox), 4:2:2 10-bit, BT.709 limited",
            tagged("bt709", "bt709", "bt709", "tv", "p210le"),
            &["-c:v", "prores_videotoolbox", "-profile:v", "hq"],
        ),
        (
            "edges-hevc-main10-bt2020.mp4",
            "HEVC Main 10 (VideoToolbox), 4:2:0 10-bit, BT.2020 NCL with BT.2020 primaries",
            tagged("bt2020nc", "bt2020", "bt2020-10", "tv", "p010le"),
            &[
                "-c:v",
                "hevc_videotoolbox",
                "-profile:v",
                "main10",
                "-q:v",
                "90",
                "-tag:v",
                "hvc1",
            ],
        ),
    ];
    for (file, label, vf, codec) in tagged_sets {
        let path = dir.join(file);
        let status = Command::new(ffmpeg())
            .args([
                "-hide_banner",
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                SRC,
            ])
            .args(["-vf", &vf])
            .args(codec)
            .arg(&path)
            .output()
            .expect("ffmpeg runs");
        if status.status.success() {
            out.push((path, label));
        } else {
            println!(
                "{file}: not generated ({})",
                String::from_utf8_lossy(&status.stderr).trim()
            );
        }
    }
    out
}

/// The tags of the untagged-primaries sources: BT.601 matrix, limited range
/// (primaries and transfer stay untagged with this FFmpeg; the engine reads
/// them by height, BT.601 525-line, drawn as Rec.709 in Standard).
const BT601_TV: [&str; 8] = [
    "-colorspace",
    "smpte170m",
    "-color_primaries",
    "smpte170m",
    "-color_trc",
    "smpte170m",
    "-color_range",
    "tv",
];

/// The generated sources, untagged-primaries and tagged.
const GENERATED: usize = 17;

/// VideoToolbox's ProRes decode against FFmpeg's software decoder, plane by
/// plane over every frame: both downloaded as planar 16-bit at the source's
/// own subsampling (no resampling, no matrix), differences given in codes of
/// the source's bit depth.
fn decoder_planes(path: &Path, alpha: bool) -> String {
    let tags = file_tags(path);
    let depth = probe_depth(path);
    let sub = match tags.subsampling {
        Subsampling::S444 => "444",
        Subsampling::S422 => "422",
        Subsampling::S420 => "420",
    };
    let fmt = format!("yuv{}{sub}p16le", if alpha { "a" } else { "" });
    let decode = |videotoolbox: bool| -> Vec<u16> {
        let mut cmd = Command::new(ffmpeg());
        cmd.args(["-v", "error"]);
        if videotoolbox {
            cmd.args(["-hwaccel", "videotoolbox"]);
        }
        let out = cmd
            .arg("-i")
            .arg(path)
            .args(["-vf", &format!("format={fmt}"), "-f", "rawvideo", "-"])
            .output()
            .expect("ffmpeg runs");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        out.stdout
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect()
    };
    let (sw, vt) = (decode(false), decode(true));
    assert_eq!(sw.len(), vt.len(), "{}: decoded sizes", path.display());
    let (w, h, _) = probe(path);
    let (fx, fy) = match tags.subsampling {
        Subsampling::S444 => (1, 1),
        Subsampling::S422 => (2, 1),
        Subsampling::S420 => (2, 2),
    };
    let luma = (w * h) as usize;
    let chroma = (w.div_ceil(fx) * h.div_ceil(fy)) as usize;
    let planes: Vec<(&str, usize)> = [("Y", luma), ("Cb", chroma), ("Cr", chroma)]
        .into_iter()
        .chain(alpha.then_some(("A", luma)))
        .collect();
    let frame: usize = planes.iter().map(|p| p.1).sum();
    let frames = sw.len() / frame;
    let unit = f64::from(1u32 << (16 - depth));
    let mut parts = Vec::new();
    let mut offset = 0;
    for (plane, len) in &planes {
        let (mut max, mut over) = (0.0f64, 0usize);
        for f in 0..frames {
            let start = f * frame + offset;
            for i in start..start + len {
                let d = (f64::from(sw[i]) - f64::from(vt[i])).abs() / unit;
                max = max.max(d);
                if d > 1.0 {
                    over += 1;
                }
            }
        }
        parts.push(format!("{plane} max {max:.2} ({over} over 1)"));
        offset += len;
    }
    format!("{frames} frames, {depth}-bit codes: {}", parts.join(", "))
}

#[derive(Default)]
struct Row {
    name: String,
    encoding: String,
    kind: &'static str,
    siting: String,
    edges: bool,
    frames: u32,
    color_px: usize,
    max_cv: f64,
    max_interior_cv: f64,
    over_one: usize,
    over_one_interior: usize,
    /// Against the reference with repeated edges (the engine's edge rule).
    max_repeat_cv: f64,
    over_one_repeat: usize,
    /// ProRes: against FFmpeg's VideoToolbox decode, repeated edges.
    max_vt_cv: Option<f64>,
    /// Against the f64 reference (no clipping); opaque sources.
    max_f64_cv: Option<f64>,
    over_one_f64: usize,
    /// Where the repeated-edges maximum is, with both values in CV.
    worst_pixel: String,
    alpha_max_cv: f64,
    rendered: bool,
    /// ffprobe's tags, and whether the engine's reading equals them.
    tags: String,
    reading_ok: bool,
    /// `pm_render::media::probe_video`'s refusal: the source routes to the
    /// legacy engine and is not drawn here.
    refused: Option<String>,
    /// ProRes: VideoToolbox's decode against FFmpeg's software decode, per
    /// plane (Y, Cb, Cr, A): max difference in codes of the source's depth,
    /// and samples over 1 code, over every frame.
    planes: Option<String>,
}

/// Measures frames `frames` of `path` (every frame index below the
/// source's length).
fn measure(
    r: &mut Renderer,
    path: &Path,
    encoding: &str,
    edges: bool,
    frames: &[u32],
) -> Option<Row> {
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    // A source the renderer refuses (`video_color` answers unsupported) is
    // still read by the probe; its reading is checked, and it is recorded as
    // routed to the legacy engine.
    let (engine_color, refused) = match r.video_color(path) {
        Ok(c) => (c, pm_render::media::probe_video(path).err()),
        Err(e) => match pm_media::decode::service::MediaSource::open(path) {
            Ok(src) => (src.color, Some(e.to_string())),
            Err(open) => {
                println!("{name}: the engine does not read it ({e}; probe: {open})");
                return None;
            }
        },
    };
    let color = file_tags(path);
    let reading_ok = reading_matches(&engine_color, &color);
    println!(
        "{name}: tags {}; read as {:?} {:?} {:?} {:?}; engine {:?} {:?} {:?} {:?} {:?}{}",
        color.raw,
        color.matrix,
        color.range,
        color.siting,
        color.subsampling,
        engine_color.matrix,
        engine_color.range,
        engine_color.siting,
        engine_color.subsampling,
        engine_color.primaries,
        if reading_ok { "" } else { "  MISREAD" }
    );
    let (w, h, fps) = probe(path);
    let mut row = Row {
        name: name.clone(),
        encoding: encoding.into(),
        siting: format!("{:?} {:?} {:?}", color.matrix, color.range, color.siting),
        edges,
        tags: color.raw.clone(),
        reading_ok,
        ..Row::default()
    };
    if let Some(e) = refused {
        println!("{name}: routes to the legacy engine ({e})");
        row.refused = Some(e);
        return Some(row);
    }
    let reading = Reading {
        matrix: match color.matrix {
            MatrixCoefficients::Bt709 => "709",
            MatrixCoefficients::Bt601 => "170m",
            MatrixCoefficients::Bt2020Ncl => "2020_ncl",
            MatrixCoefficients::Identity => "input",
        },
        range: if color.range == Range::Full {
            "full"
        } else {
            "limited"
        },
        chromal: match color.siting {
            ChromaSiting::Left => "left",
            ChromaSiting::Center => "center",
            ChromaSiting::TopLeft => "topleft",
            ChromaSiting::Top => "top",
            ChromaSiting::BottomLeft => "bottomleft",
            ChromaSiting::Bottom => "bottom",
        },
        subsampling: color.subsampling,
    };
    let kind = match color.subsampling {
        Subsampling::S444 => "4:4:4",
        Subsampling::S422 => "4:2:2",
        Subsampling::S420 => "4:2:0",
    };
    let alpha = name.contains("alpha");
    let prores = name.contains("prores");
    let last = frames.iter().copied().max().unwrap_or(0);
    let doc = document(w, h, fps, f64::from(last + 1) / fps);
    let mut resolver = MapResolver::default();
    resolver.paths.insert("clip".into(), path.to_path_buf());
    row.kind = kind;
    if prores {
        row.planes = Some(decoder_planes(path, alpha));
        println!(
            "{name}: VideoToolbox against FFmpeg's decode: {}",
            row.planes.as_ref().unwrap()
        );
    }
    for &n in frames {
        let t = (f64::from(n) + 0.5) / fps;
        let request = RenderRequest::at_scale(Size::new(w, h), 1.0, Precision::Float32);
        let ticks = Ticks::new((t * pm_time::TICKS_PER_SECOND as f64).round() as i64).unwrap();
        let frame = match r.render(&doc, "Pvideo-root", ticks, &request, &resolver) {
            Ok(f) if f.failures.is_empty() => f,
            Ok(f) => {
                println!("{name}: layer failures {:?}", f.failures);
                break;
            }
            Err(e) => {
                println!("{name}: not rendered ({e})");
                break;
            }
        };
        let ours = r.read_rgba(&frame).expect("readback");
        let zimg_edges = reference(path, n, &reading, alpha, false, false);
        let repeated = reference(path, n, &reading, alpha, true, false);
        let exact = (!alpha).then(|| reference_f64(path, n, &color, w, h));
        if let Some(exact) = &exact {
            let max_against = |k: u32| -> f64 {
                let exact = reference_f64(path, k, &color, w, h);
                ours.chunks_exact(4)
                    .zip(exact.chunks_exact(4))
                    .map(|(o, q)| {
                        (0..3)
                            .map(|c| (f64::from(o[c]) - q[c]).abs() * 255.0)
                            .fold(0.0, f64::max)
                    })
                    .fold(0.0, f64::max)
            };
            let mut frame_max: f64 = 0.0;
            for (o, q) in ours.chunks_exact(4).zip(exact.chunks_exact(4)) {
                let worst = (0..3)
                    .map(|c| (f64::from(o[c]) - q[c]).abs() * 255.0)
                    .fold(0.0, f64::max);
                frame_max = frame_max.max(worst);
                row.max_f64_cv = Some(row.max_f64_cv.unwrap_or(0.0).max(worst));
                if worst > 1.0 {
                    row.over_one_f64 += 1;
                }
            }
            if frame_max > 1.0 {
                // Which source frame the engine drew instead.
                let best = (n.saturating_sub(3)..=n + 3)
                    .filter(|&k| k != n)
                    .map(|k| (k, max_against(k)))
                    .min_by(|a, b| a.1.total_cmp(&b.1));
                if let Some((k, m)) = best {
                    println!(
                        "  frame {n}: max {frame_max:.3} CV; nearest source frame {k} at {m:.3} CV"
                    );
                }
            }
        }
        if prores {
            let vt = reference(path, n, &reading, alpha, true, true);
            for (o, q) in ours.chunks_exact(4).zip(vt.chunks_exact(4)) {
                // VideoToolbox decodes full alpha to 0.99995, not 1.
                if q[3] < 0.999 {
                    continue;
                }
                let worst = (0..3)
                    .map(|c| (f64::from(o[c]) - f64::from(q[c])).abs() * 255.0)
                    .fold(0.0, f64::max);
                row.max_vt_cv = Some(row.max_vt_cv.unwrap_or(0.0).max(worst));
            }
        }
        assert_eq!(zimg_edges.len(), ours.len(), "{name}: frame size");
        assert_eq!(repeated.len(), ours.len(), "{name}: frame size");
        for (i, (o, q)) in ours
            .chunks_exact(4)
            .zip(repeated.chunks_exact(4))
            .enumerate()
        {
            if q[3] < 1.0 {
                continue;
            }
            let worst = (0..3)
                .map(|c| (f64::from(o[c]) - f64::from(q[c])).abs() * 255.0)
                .fold(0.0, f64::max);
            if worst > row.max_repeat_cv {
                let cv =
                    |v: &[f32]| [v[0], v[1], v[2]].map(|c| (f64::from(c) * 2550.0).round() / 10.0);
                let f64_cv = exact
                    .as_ref()
                    .map(|e| {
                        let p = &e[4 * i..4 * i + 3];
                        format!(
                            ", f64 reference {:?}",
                            [p[0], p[1], p[2]].map(|c| (c * 2550.0).round() / 10.0)
                        )
                    })
                    .unwrap_or_default();
                row.worst_pixel = format!(
                    "frame {n} ({}, {}): engine {:?}, zimg reference {:?}{f64_cv}",
                    i as u32 % w,
                    i as u32 / w,
                    cv(o),
                    cv(q)
                );
            }
            row.max_repeat_cv = row.max_repeat_cv.max(worst);
            if worst > 1.0 {
                row.over_one_repeat += 1;
            }
        }
        row.frames += 1;
        for (i, (o, q)) in ours
            .chunks_exact(4)
            .zip(zimg_edges.chunks_exact(4))
            .enumerate()
        {
            let (x, y) = ((i as u32) % w, (i as u32) / w);
            let interior = x >= 2 && y >= 2 && x + 2 < w && y + 2 < h;
            let ra = f64::from(q[3]);
            row.alpha_max_cv = row.alpha_max_cv.max((f64::from(o[3]) - ra).abs() * 255.0);
            if ra < 1.0 {
                continue;
            }
            row.color_px += 1;
            let mut worst: f64 = 0.0;
            for c in 0..3 {
                // The engine's output is premultiplied; opaque here.
                worst = worst.max((f64::from(o[c]) - f64::from(q[c])).abs() * 255.0);
            }
            row.max_cv = row.max_cv.max(worst);
            if worst > 1.0 {
                row.over_one += 1;
            }
            if interior {
                row.max_interior_cv = row.max_interior_cv.max(worst);
                if worst > 1.0 {
                    row.over_one_interior += 1;
                }
            }
        }
    }
    row.rendered = row.frames as usize == frames.len();
    if row.max_repeat_cv > 1.0 {
        println!("  worst: {}", row.worst_pixel);
    }
    println!(
        "{:<28} {} {} [{}] {}: {} frames, {} opaque px, max {:.3} CV ({} px over 1), interior max {:.3} CV ({} px over 1); repeated edges max {:.3} CV ({} px over 1){}{}",
        row.name,
        row.kind,
        row.siting,
        row.encoding,
        if row.edges { "chroma edges" } else { "flat" },
        row.frames,
        row.color_px,
        row.max_cv,
        row.over_one,
        row.max_interior_cv,
        row.over_one_interior,
        row.max_repeat_cv,
        row.over_one_repeat,
        row.max_f64_cv
            .map(|v| format!(
                "; f64 reference max {v:.3} CV ({} px over 1)",
                row.over_one_f64
            ))
            .unwrap_or_default()
            + row
                .max_vt_cv
                .map(|v| format!("; against VideoToolbox's decode max {v:.3} CV"))
                .unwrap_or_default()
                .as_str(),
        if alpha {
            format!(", alpha max {:.3} CV", row.alpha_max_cv)
        } else {
            String::new()
        }
    );
    Some(row)
}

#[test]
#[ignore = "gate measurement (docs/engine/phase3a-gate.md); run with --ignored"]
fn video_class_against_ffmpeg_and_zimg() {
    let fixtures = repo().join("apps/desktop/e2e/fixtures");
    let mut r = Renderer::headless().expect("a Metal device");
    let mut rows = Vec::new();
    println!(
        "Flat e2e fixtures (one Y'CbCr triple per frame: the BT.601 limited conversion only):"
    );
    for name in [
        "prores422.mov",
        "prores4444-alpha.mov",
        "converted.mkv",
        "h264-aac.mov",
        "h264-aac.mp4",
        "hevc.mov",
        "hevc.mp4",
        "vp8-opus.webm",
        "vp9-opus.webm",
        "av1.mp4",
    ] {
        rows.extend(measure(
            &mut r,
            &fixtures.join(name),
            "e2e fixture",
            false,
            &SAMPLED,
        ));
    }
    let dir = std::env::temp_dir().join(format!("pm-p3a-video-class-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    println!("Chroma-edge sources (testsrc2, generated):");
    let generated = generate_edge_sources(&dir);
    for (path, encoding) in &generated {
        rows.extend(measure(&mut r, path, encoding, true, &SAMPLED));
    }
    std::fs::remove_dir_all(&dir).ok();
    println!("Corpus H.264 sources (the motion capture's generated media):");
    let corpus = corpus_sources();
    for path in &corpus {
        // Every frame: the Chromium comparison's failing frames
        // (`retime/speed` t = 2.9, `space25d/precomps` t = 1.5 and 2.6) map
        // to frames of these sources.
        let (_, _, fps) = probe(path);
        let count = probe_frames(path);
        let all: Vec<u32> = (0..count).collect();
        assert!(count > 0 && fps > 0.0);
        rows.extend(measure(
            &mut r,
            path,
            "corpus H.264, every frame",
            true,
            &all,
        ));
    }
    println!("Readings (ffprobe tags under section 9's rules; the engine's reading):");
    for row in &rows {
        println!(
            "  {:<32} {:<28} {}{}",
            row.name,
            row.siting,
            row.tags,
            match (&row.refused, row.reading_ok) {
                (_, false) => "  MISREAD by the engine".to_string(),
                (Some(e), true) => format!("  routes to legacy: {e}"),
                (None, true) => String::new(),
            }
        );
    }
    let misread: Vec<&str> = rows
        .iter()
        .filter(|r| !r.reading_ok)
        .map(|r| r.name.as_str())
        .collect();
    let refused: Vec<(&str, &str)> = rows
        .iter()
        .filter_map(|r| r.refused.as_deref().map(|e| (r.name.as_str(), e)))
        .collect();
    let edged: Vec<&Row> = rows
        .iter()
        .filter(|r| r.edges && r.refused.is_none())
        .collect();
    let unrendered: Vec<&str> = edged
        .iter()
        .filter(|r| !r.rendered)
        .map(|r| r.name.as_str())
        .collect();
    let worst = |rs: &[&Row], f: fn(&Row) -> f64| rs.iter().map(|r| f(r)).fold(0.0, f64::max);
    let flat: Vec<&Row> = rows.iter().filter(|r| !r.edges).collect();
    println!(
        "flat fixtures: worst {:.3} CV; chroma-edge sources ({} of {} generated, {} rendered): worst {:.3} CV, interior {:.3} CV, repeated edges {:.3} CV, alpha {:.3} CV (bar 1)",
        worst(&flat, |r| r.max_cv),
        generated.len() + corpus.len(),
        GENERATED + corpus.len(),
        edged.len() - unrendered.len(),
        worst(&edged, |r| r.max_cv),
        worst(&edged, |r| r.max_interior_cv),
        worst(&edged, |r| r.max_repeat_cv),
        worst(&edged, |r| r.alpha_max_cv),
    );
    assert_eq!(
        generated.len(),
        GENERATED,
        "every chroma-edge source is generated"
    );
    assert!(
        misread.is_empty(),
        "the engine misreads the tags of {misread:?}"
    );
    // Only the BT.2020-primaries source routes (wide gamut waits for
    // Interpret and wide-gamut projects; `pm_render::media::video_supported`).
    assert!(
        refused.len() == 1
            && refused[0].0 == "edges-hevc-main10-bt2020.mp4"
            && refused[0].1.contains("Bt2020 primaries"),
        "routed to legacy: {refused:?}"
    );
    assert!(unrendered.is_empty(), "not rendered: {unrendered:?}");
    // Judged: FFmpeg's software decode plus zimg with the engine's filter and
    // edge rule, color and alpha.
    let all = worst(&edged, |r| r.max_repeat_cv.max(r.alpha_max_cv));
    assert!(
        all <= 1.0,
        "the video class against FFmpeg plus zimg: {all:.3} CV"
    );
}
