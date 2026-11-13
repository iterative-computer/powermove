//! One graded composition read five ways: the paused viewer, the playing
//! viewer, playback from the disk render cache (the stored frame, and the
//! picture a playing frame presents from it), what the scopes count, and the
//! exported files (ProRes 4444 and a 16-bit PNG, decoded by FFmpeg). Every
//! reading is compared with the 16-bit PNG, as display-encoded Rec.709.
//!
//! The grade is an adjustment layer with a LUT, a gamma curve and a lift and
//! gain, over a gradient, flat patches and a moving clip whose range flag is
//! wrong and is corrected in Interpret Footage (full-range codes in a file
//! that says limited). Every reading is taken three frames in, so each path
//! also has to choose the same frame of the clip.
//!
//! Stated tolerances, of full scale, with what a debug build measured on
//! 2026-11-12 (Apple silicon, Metal):
//! - the playing viewer against the paused one: identical (measured 0);
//! - the cached frame against the PNG: one 16-bit code (measured 7.6e-6,
//!   half a code: the cache holds the export's own values);
//! - the picture a playing frame presents from the cache, against the paused
//!   viewer: identical (measured 0), and playback shows cached frames;
//! - the paused viewer (a half-float surface in linear Display P3, brought
//!   back to encoded Rec.709) against the PNG: 0.0005 (measured 0.00023,
//!   mean 0.00005);
//! - ProRes 4444 (12-bit Y′CbCr through a lossy codec) against the PNG:
//!   0.005 at the worst pixel, 0.0006 on average (measured 0.0032, 0.00031);
//! - scopes: at most 1 pixel in 200 counted in a different level than the
//!   same scope of the PNG (measured 1 in 575 at worst, the waveform).
use pm_engine_service::{jobs::*, *};
use pm_protocol::{
    clock::{SeekMode, TransportCommand as C},
    color::{OutputColor, Precision, QuickTimeTag},
    common::{FrameRate, PlaybackRate, SessionId, Size, TICKS_PER_SECOND, TickRange, Ticks},
    priority::{AlphaOutput, ExportFormat, ExportSettings, JobSpec, JobState, ProResProfile},
};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const W: u32 = 192;
const H: u32 = 108;
/// Where every reading is taken: three frames in.
const AT: i64 = TICKS_PER_SECOND / 10;
/// The clip's frame there, and its size.
const CLIP_FRAME: usize = 3;
const CLIP: (u32, u32) = (64, 48);

/// The luma codes of the clip's four grey bands at frame `n`: the whole of
/// 0..255, as a full-range camera or screen recorder writes.
fn clip_luma(n: usize) -> [u8; 4] {
    [16 + n as u8, 128 - 4 * n as u8, 235, 255 - n as u8]
}

fn ffmpeg_bin() -> PathBuf {
    PathBuf::from(std::env::var_os("PM_FFMPEG_DIR").expect("source engine/env.sh before this test"))
        .join("bin/ffmpeg")
}

/// A clip whose range flag is wrong: 8-bit Y′CbCr with full-range codes, in
/// a file that says limited. Lossless, so the file holds these codes.
fn mistagged_clip(dir: &Path) -> PathBuf {
    let (w, h) = (CLIP.0 as usize, CLIP.1 as usize);
    let mut raw = Vec::new();
    for n in 0..12 {
        let bands = clip_luma(n);
        for _ in 0..h {
            for x in 0..w {
                raw.push(bands[x / 16]);
            }
        }
        raw.extend(std::iter::repeat_n(128u8, w * h / 2));
    }
    let input = dir.join("codes.yuv");
    std::fs::write(&input, raw).unwrap();
    let output = dir.join("Clip.mkv");
    let status = std::process::Command::new(ffmpeg_bin())
        .args(["-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "yuv420p"])
        .args(["-s", "64x48", "-r", "30", "-i"])
        .arg(&input)
        .args(["-c:v", "ffv1", "-color_range", "tv"])
        .arg(&output)
        .status()
        .unwrap();
    assert!(status.success(), "ffmpeg wrote the clip");
    output
}

fn clip_asset(interpret: Option<Value>) -> Value {
    let mut asset = json!({"id": "clip", "name": "Clip.mkv", "kind": "video", "format": "mkv",
        "w": CLIP.0, "h": CLIP.1, "dur": 0.4, "fps": 30, "persisted": true});
    if let Some(interpret) = interpret {
        asset["interpret"] = interpret;
    }
    asset
}

fn clip_layer(w: u32, h: u32, x: f64, y: f64) -> Value {
    layer(
        "footage",
        "video",
        json!({"asset": "clip", "fit": "stretch", "w": w, "h": h, "trim": 0, "speed": 1}),
        x,
        y,
        100.,
    )
}

fn channel(v: Value) -> Value {
    json!({"v": v, "kf": [], "expr": null})
}

fn layer(id: &str, ty: &str, d: Value, x: f64, y: f64, opacity: f64) -> Value {
    let mut p = serde_json::Map::new();
    for (key, value) in [
        ("position.x", x),
        ("position.y", y),
        ("position.z", 0.),
        ("anchor.x", 0.),
        ("anchor.y", 0.),
        ("anchor.z", 0.),
        ("scale.x", 100.),
        ("scale.y", 100.),
        ("scale.z", 100.),
        ("rotation", 0.),
        ("rotation.x", 0.),
        ("rotation.y", 0.),
        ("orientation.x", 0.),
        ("orientation.y", 0.),
        ("orientation.z", 0.),
        ("opacity", opacity),
        ("skew", 0.),
        ("perspective", 50.),
    ] {
        p.insert(key.into(), channel(json!(value)));
    }
    json!({"id": id, "type": ty, "name": id, "from": 0, "dur": 3, "on": true,
        "blend": "normal", "masks": [], "fx": [], "p": p, "d": d})
}

/// A LUT that bends and mixes the channels, as a creative look does.
fn look() -> String {
    let size = 17;
    let at = |i: usize| i as f64 / (size - 1) as f64;
    let curve = |v: f64| v * v * (3. - 2. * v);
    let mut text = format!("LUT_3D_SIZE {size}\n");
    for b in 0..size {
        for g in 0..size {
            for r in 0..size {
                let (r, g, b) = (at(r), at(g), at(b));
                text += &format!(
                    "{:.7} {:.7} {:.7}\n",
                    0.85 * curve(r) + 0.1 * g + 0.03,
                    0.9 * curve(g) + 0.06 * b,
                    0.8 * b.powf(1.1) + 0.12 * r + 0.02,
                );
            }
        }
    }
    text
}

fn project() -> pm_model::Value {
    let solid = |id: &str, color: &str, w: u32, h: u32, x: f64, y: f64, opacity: f64| {
        layer(
            id,
            "solid",
            json!({"color": color, "w": w, "h": h}),
            x,
            y,
            opacity,
        )
    };
    let mut grade = layer("grade", "adjustment", json!({}), 0., 0., 100.);
    grade["fx"] = json!([
        {"id": "lut", "type": "applyColorLut", "version": 2, "on": true,
         "p": {"amount": channel(json!(100)), "inputSpace": channel(json!(0)),
               "outputSpace": channel(json!(0))},
         "inputs": {"lut": {"kind": "asset", "assetId": "look"}}},
        {"id": "curve", "type": "levels", "version": 2, "on": true,
         "p": {"gamma": channel(json!(1.25))}, "inputs": {}},
        {"id": "liftgain", "type": "levels", "version": 2, "on": true,
         "p": {"outBlack": channel(json!(0.04)), "outWhite": channel(json!(0.93))},
         "inputs": {}},
    ]);
    let value = json!({
        "id": "P", "name": "P", "compId": "root", "compName": "P", "w": W, "h": H,
        "fps": 30, "dur": 3, "bg": "#000000", "work": [0, 3],
        "backgroundFill": {"type": "linear", "angle": 35, "stops": [
            {"id": "a", "color": "#05060A", "position": 0},
            {"id": "b", "color": "#2F6FD0", "position": 40},
            {"id": "c", "color": "#E8A46A", "position": 75},
            {"id": "d", "color": "#FAFAF2", "position": 100}]},
        "requiresEngine": "native",
        "layers": [
            grade,
            clip_layer(CLIP.0, CLIP.1, 150., 30.),
            solid("skin", "#C68E6E", 40, 30, 30., 20., 100.),
            solid("red", "#E0261C", 30, 30, 90., 30., 100.),
            solid("green", "#1FB34A", 36, 24, 150., 70., 100.),
            solid("veil", "#FFFFFF", 80, 20, 100., 90., 60.),
        ],
        "assets": {
            "look": {"id": "look", "name": "Look.cube", "kind": "lut", "format": "cube"},
            "clip": clip_asset(Some(json!({"range": "full"}))),
        },
        "comps": {}
    });
    pm_model::json::parse(&value.to_string()).unwrap()
}

fn ffmpeg(args: &[&str], input: &Path, output: &Path) -> Vec<u8> {
    let status = std::process::Command::new(ffmpeg_bin())
        .args(["-v", "error", "-y", "-i"])
        .arg(input)
        .args(args)
        .arg(output)
        .status()
        .unwrap();
    assert!(status.success(), "ffmpeg {args:?}");
    std::fs::read(output).unwrap()
}

fn export(
    snapshot: &Snapshot,
    media: &MediaCatalog,
    destination: &Path,
    format: ExportFormat,
    color: OutputColor,
    alpha: AlphaOutput,
) {
    export_sized(snapshot, media, destination, format, color, alpha, (W, H));
}

/// Exports from [`AT`]: one frame of a still, a tenth of a second of video.
fn export_sized(
    snapshot: &Snapshot,
    media: &MediaCatalog,
    destination: &Path,
    format: ExportFormat,
    color: OutputColor,
    alpha: AlphaOutput,
    (width, height): (u32, u32),
) {
    let end = AT
        + if format.is_image() {
            1
        } else {
            TICKS_PER_SECOND / 10
        };
    let settings = ExportSettings {
        format,
        alpha,
        quality: pm_protocol::priority::QualityTier::High,
        size: Size::new(width, height),
        frame_rate: FrameRate::new(30, 1),
        range: TickRange {
            start: Ticks::new(AT).unwrap(),
            end: Ticks::new(end).unwrap(),
        },
        audio: false,
        audio_index_playhead: None,
        motion_blur: false,
        color,
        precision: Precision::Float16,
        dither: false,
        draft_from_proxies: false,
        delivery: Default::default(),
    };
    let request = ExportRequest::new(
        "grade-export".into(),
        snapshot,
        JobSpec::Export {
            session: SessionId::new(1).unwrap(),
            composition: snapshot.open_comp().unwrap().as_str().into(),
            settings,
        },
        media.clone(),
        destination.to_owned(),
    )
    .unwrap();
    let status = ExportHandle::start(Path::new(env!("CARGO_BIN_EXE_pm-engine-service")), request)
        .unwrap()
        .wait()
        .unwrap();
    assert_eq!(status.state, JobState::Done, "{status:?}");
}

/// The viewer's linear Display P3 pixels as display-encoded Rec.709.
fn encoded_rec709(pixels: &[[f64; 3]]) -> Vec<[f64; 3]> {
    use pm_protocol::color::{Primaries, Transfer};
    let to_709 = pm_color::primaries::conversion(Primaries::DisplayP3, Primaries::Bt709).unwrap();
    pixels
        .iter()
        .map(|p| {
            pm_color::primaries::apply(&to_709, *p)
                .map(|v| pm_color::transfer::from_linear(Transfer::DisplayE, v).unwrap())
        })
        .collect()
}

/// The largest and the mean difference of two frames.
fn difference(a: &[[f64; 3]], b: &[[f64; 3]]) -> (f64, f64) {
    assert_eq!(a.len(), b.len());
    let mut worst = 0f64;
    let mut sum = 0.;
    for (x, y) in a.iter().zip(b) {
        for c in 0..3 {
            let d = (x[c] - y[c]).abs();
            worst = worst.max(d);
            sum += d;
        }
    }
    (worst, sum / (a.len() * 3) as f64)
}

fn wait(viewer: &Viewer, why: &str, done: impl Fn(&FrameReport) -> bool) -> FrameReport {
    let start = Instant::now();
    loop {
        let status = viewer.status();
        assert!(status.error.is_none(), "{why}: {:?}", status.error);
        if let Some(frame) = status.frame
            && done(&frame)
        {
            return frame;
        }
        assert!(start.elapsed() < Duration::from_secs(60), "{why}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Pixels that fall in different levels of two histograms, as a share.
fn moved(a: &[u32], b: &[u64], pixels: u64) -> f64 {
    let different: u64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| u64::from(*x).abs_diff(*y))
        .sum();
    different as f64 / 2. / pixels as f64
}

#[test]
fn a_graded_frame_is_the_same_in_the_viewer_the_cache_the_scopes_and_the_export() {
    let dir = tempfile::tempdir().unwrap();
    let cube = dir.path().join("Look.cube");
    std::fs::write(&cube, look()).unwrap();
    let snapshot = pm_kernel::document::DocumentStore::new_untitled(project()).snapshot();
    let mut media = MediaCatalog::default();
    media.originals.insert("look".into(), cube);
    media
        .originals
        .insert("clip".into(), mistagged_clip(dir.path()));
    let exe = Path::new(env!("CARGO_BIN_EXE_pm-engine-service"));
    let at = Ticks::new(AT).unwrap();

    // The exported files.
    let png = dir.path().join("still.png");
    export(
        &snapshot,
        &media,
        &png,
        ExportFormat::PngStill { sixteen_bit: true },
        OutputColor::SrgbImage,
        // Without alpha, a still carries the composition's background, as
        // the viewer shows it.
        AlphaOutput::None,
    );
    let raw = ffmpeg(
        &["-f", "rawvideo", "-pix_fmt", "rgba64le"],
        &png,
        &dir.path().join("png.raw"),
    );
    assert_eq!(raw.len(), (W * H * 8) as usize);
    let code = |bytes: &[u8]| f64::from(u16::from_le_bytes([bytes[0], bytes[1]]));
    let exported: Vec<[f64; 3]> = raw
        .chunks_exact(8)
        .map(|p| {
            assert_eq!(code(&p[6..]), 65535., "the frame is opaque");
            [code(p), code(&p[2..]), code(&p[4..])].map(|v| v / 65535.)
        })
        .collect();
    let spread = exported
        .iter()
        .flatten()
        .fold((1f64, 0f64), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
    assert!(
        spread.0 < 0.08 && spread.1 > 0.85,
        "the picture spans the range: {spread:?}"
    );

    let mov = dir.path().join("master.mov");
    export(
        &snapshot,
        &media,
        &mov,
        ExportFormat::ProRes {
            profile: ProResProfile::P4444,
        },
        OutputColor::Rec709QuickTime {
            candidate: QuickTimeTag::Cicp1_13_1,
        },
        AlphaOutput::None,
    );
    // The decoder's own 12-bit Y′CbCr planes (no conversion by FFmpeg), then
    // BT.709 limited range to R′G′B′ here.
    let planes = ffmpeg(
        &[
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "yuv444p12le",
        ],
        &mov,
        &dir.path().join("mov.raw"),
    );
    let count = (W * H) as usize;
    assert_eq!(planes.len(), count * 6);
    let plane = |n: usize, i: usize| code(&planes[(n * count + i) * 2..]);
    let prores: Vec<[f64; 3]> = (0..count)
        .map(|i| {
            let y = (plane(0, i) - 256.) / 3504.;
            let (cb, cr) = ((plane(1, i) - 2048.) / 3584., (plane(2, i) - 2048.) / 3584.);
            [
                y + 1.5748 * cr,
                y - 0.187_324 * cb - 0.468_124 * cr,
                y + 1.8556 * cb,
            ]
        })
        .collect();
    let (worst, mean) = difference(&prores, &exported);
    assert!(
        worst < 0.005 && mean < 0.0006,
        "ProRes 4444 against the 16-bit PNG: worst {worst:.5}, mean {mean:.6}"
    );

    // The viewer, paused then playing, at full quality.
    let service = EngineService::start(pm_gpu::Gpu::new("grade-pipeline").unwrap()).unwrap();
    let disk = service.render_cache();
    let frames = dir.path().join("frames");
    disk.set_storage(Some((frames.clone(), 1 << 30))).unwrap();
    let viewer = service
        .viewer(ViewerConfig {
            composition: snapshot.open_comp().unwrap(),
            snapshot: snapshot.clone(),
            media: media.clone(),
            output: Output {
                width: W,
                height: H,
                display: 0,
                headroom: 1.,
                crop: None,
            },
            layer: None,
            rate: pm_time::Rate::FPS_30,
            end: Some(3 * TICKS_PER_SECOND),
            audio: false,
        })
        .unwrap();
    viewer.quality(Quality::Full).unwrap();
    viewer
        .command(C::Seek {
            time: at,
            mode: SeekMode::Exact,
        })
        .unwrap();
    wait(&viewer, "the playhead three frames in", |f| f.time == at);
    // The scopes count the frames drawn from here on: all at the playhead.
    let (taps, counted) = async_channel::bounded(1);
    let config = pm_render::scopes::ScopeConfig::PANEL;
    viewer.scopes(Some((config, taps))).unwrap();
    viewer.capture_next().unwrap();
    let paused = wait(&viewer, "a paused frame", |f| {
        f.time == at && f.moment == Moment::Paused && f.pixels.is_some() && f.scale >= 1.
    });
    let paused = encoded_rec709(&paused.pixels.unwrap());
    let (worst, mean) = difference(&paused, &exported);
    assert!(
        worst < 0.0005,
        "the paused viewer against the 16-bit PNG: worst {worst:.5}, mean {mean:.6}"
    );

    // What the scopes counted, against the same counts of the exported PNG.
    let start = Instant::now();
    let scope = loop {
        match counted.try_recv() {
            Ok(frame) => break frame.unwrap(),
            Err(_) => {
                assert!(start.elapsed() < Duration::from_secs(20), "no scope frame");
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    };
    let reference = pm_color::scopes::compute(
        &exported,
        W as usize,
        H as usize,
        pm_color::scopes::ScopeConfig {
            columns: config.columns as usize,
            levels: config.levels as usize,
            vector_bins: config.vector_bins as usize,
            ..Default::default()
        },
    )
    .unwrap();
    let pixels = u64::from(W * H);
    assert_eq!(scope.pixels, pixels);
    let luma = moved(
        &scope.luma_histogram,
        &reference.luma_histogram.bins,
        pixels,
    );
    let waveform = moved(&scope.waveform, &reference.waveform, pixels);
    let vector = moved(&scope.vectorscope, &reference.vectorscope, pixels);
    assert!(
        luma < 0.005 && waveform < 0.005 && vector < 0.005,
        "scope pixels in a different level than the export's: luma {luma:.4}, waveform {waveform:.4}, vectorscope {vector:.4}"
    );
    for c in 0..3 {
        let channel = moved(&scope.histogram[c], &reference.histogram[c].bins, pixels);
        let parade = moved(&scope.parade[c], &reference.parade[c], pixels);
        assert!(
            channel < 0.005 && parade < 0.005,
            "channel {c}: histogram {channel:.4}, parade {parade:.4}"
        );
    }
    viewer.scopes(None).unwrap();

    // The frame the worker draws while playing: the same request, at the
    // quality a person fixes with Full. (A capture is always drawn as a
    // paused frame, so the playing path is read through the engine itself.)
    let scene = Scene {
        doc: snapshot.document.clone(),
        comp: snapshot.open_comp().unwrap(),
        media: std::sync::Arc::new(media.export_paths(false)),
        content: 1,
    };
    let output = Output {
        width: W,
        height: H,
        display: 0,
        headroom: 1.,
        crop: None,
    };
    let mut engine = Headless::new().unwrap();
    let playing = engine
        .frame_spec_settled(
            &scene,
            FrameSpec {
                moment: Moment::Playing,
                exact: false,
                ..FrameSpec::exact(at, output)
            },
            &Default::default(),
        )
        .unwrap();
    let playing = encoded_rec709(&playing);
    let (worst, mean) = difference(&playing, &paused);
    assert!(
        worst < 1e-6,
        "the playing viewer against the paused one: worst {worst:e}, mean {mean:e}"
    );

    // The frames the disk render cache stores for playback: the first second.
    let start = Instant::now();
    while disk
        .start_fill(0., 1., exe, jobs::Activity::default(), |_| {})
        .is_err()
    {
        assert!(start.elapsed() < Duration::from_secs(30), "scene not read");
        std::thread::sleep(Duration::from_millis(20));
    }
    while disk.bars().running || disk.bars().done == 0 {
        assert!(
            start.elapsed() < Duration::from_secs(120),
            "{:?}",
            disk.bars()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(disk.bars().error, None);

    // Playback shows them. The viewer, played through the rendered second:
    // a playing frame there is a cached one. (This is the first play, so no
    // frame of it is in the viewer's own memory.)
    let played = (0..3).any(|_| {
        viewer
            .command(C::Seek {
                time: Ticks::ZERO,
                mode: SeekMode::Exact,
            })
            .unwrap();
        wait(&viewer, "back at the start", |f| f.time == Ticks::ZERO);
        viewer
            .command(C::Play {
                rate: PlaybackRate::NORMAL,
            })
            .unwrap();
        let start = Instant::now();
        let mut cached = false;
        while !cached && start.elapsed() < Duration::from_secs(2) {
            let status = viewer.status();
            assert!(status.error.is_none(), "{:?}", status.error);
            cached = status.frame.is_some_and(|f| {
                f.moment == Moment::Playing
                    && f.cache_hit
                    && f.time.get() > 0
                    && f.time.get() < TICKS_PER_SECOND
            });
            std::thread::sleep(Duration::from_millis(1));
        }
        viewer.command(C::Pause).unwrap();
        cached
    });
    assert!(
        played,
        "playback through a rendered range shows cached frames"
    );

    // The picture a playing frame presents from the cache: the viewer's own
    // scene, through the engine's frame path with the cache it reads, until
    // the read has landed.
    let rendered = disk.observed_scene().expect("the composition is observed");
    let mut reader = Headless::new().unwrap();
    reader.read_rendered_frames(disk.clone());
    let start = Instant::now();
    let presented = loop {
        let (pixels, from_disk) = reader
            .frame_spec_read(
                &rendered,
                FrameSpec {
                    moment: Moment::Playing,
                    exact: false,
                    ..FrameSpec::exact(at, output)
                },
                &Default::default(),
            )
            .unwrap();
        if from_disk {
            break encoded_rec709(&pixels);
        }
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "the playing frame was drawn, not read from the cache"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    let (worst, mean) = difference(&presented, &paused);
    assert!(
        worst < 1e-4,
        "cached playback against the paused viewer: worst {worst:e}, mean {mean:e}"
    );
    let (worst, mean) = difference(&presented, &exported);
    assert!(
        worst < 0.0005,
        "cached playback against the 16-bit PNG: worst {worst:.5}, mean {mean:.6}"
    );

    // The stored frame itself, read through the cache's own reader.
    let stored = disk
        .stored_frame(&rendered, at)
        .expect("the frame three frames in is on disk");
    let cached: Vec<[f64; 3]> = stored
        .chunks_exact(8)
        .map(|p| {
            let value = |c: usize| f64::from(half::f16::from_le_bytes([p[c * 2], p[c * 2 + 1]]));
            assert_eq!(value(3), 1., "the frame is opaque");
            [value(0), value(1), value(2)]
        })
        .collect();
    assert_eq!(cached.len(), (W * H) as usize);
    let (worst, mean) = difference(&cached, &exported);
    assert!(
        worst < 1. / 65535.,
        "the cached frame against the 16-bit PNG: worst {worst:.6}, mean {mean:.7}"
    );
    viewer.retire();
}

/// Interpret Footage › Range reaches the viewer and the export alike. The
/// clip's file says limited and holds full-range codes: as the file says it
/// is read as limited everywhere, and with Range set to Full every code is
/// read as code / 255, in the paused viewer and in the exported 16-bit PNG.
#[test]
fn a_corrected_range_is_read_the_same_in_the_viewer_and_the_export() {
    let dir = tempfile::tempdir().unwrap();
    let mut media = MediaCatalog::default();
    media
        .originals
        .insert("clip".into(), mistagged_clip(dir.path()));
    let service = EngineService::start(pm_gpu::Gpu::new("range-pipeline").unwrap()).unwrap();
    let at = Ticks::new(AT).unwrap();
    let (w, h) = CLIP;
    for (case, (interpret, full)) in [
        (None, false),
        (Some(json!({"range": "full"})), true),
        (Some(json!({"range": "limited"})), false),
    ]
    .into_iter()
    .enumerate()
    {
        let value = json!({
            "id": "P", "name": "P", "compId": "root", "compName": "P", "w": w, "h": h,
            "fps": 30, "dur": 3, "bg": "#000000", "work": [0, 3], "requiresEngine": "native",
            "layers": [clip_layer(w, h, f64::from(w) / 2., f64::from(h) / 2.)],
            "assets": {"clip": clip_asset(interpret.clone())},
            "comps": {}
        });
        let snapshot = pm_kernel::document::DocumentStore::new_untitled(
            pm_model::json::parse(&value.to_string()).unwrap(),
        )
        .snapshot();
        let expected: Vec<f64> = clip_luma(CLIP_FRAME)
            .into_iter()
            .map(|code| {
                let code = f64::from(code);
                if full {
                    code / 255.
                } else {
                    (code - 16.) / 219.
                }
            })
            .collect();
        // The middle of each band, away from its edges.
        let bands = |pixels: &[[f64; 3]]| -> Vec<[f64; 3]> {
            (0..4)
                .map(|band| pixels[(h as usize / 2) * w as usize + band * 16 + 8])
                .collect()
        };
        let check = |what: &str, got: Vec<[f64; 3]>, tolerance: f64| {
            for (band, expected) in expected.iter().enumerate() {
                // What a file can hold: the PNG stores 0..1.
                let expected = expected.clamp(0., 1.);
                for value in got[band] {
                    assert!(
                        (value.clamp(0., 1.) - expected).abs() < tolerance,
                        "{what}, {interpret:?}, band {band}: {value}, expected {expected}"
                    );
                }
            }
        };

        let png = dir.path().join(format!("range-{case}.png"));
        export_sized(
            &snapshot,
            &media,
            &png,
            ExportFormat::PngStill { sixteen_bit: true },
            OutputColor::SrgbImage,
            AlphaOutput::None,
            CLIP,
        );
        let raw = ffmpeg(
            &["-f", "rawvideo", "-pix_fmt", "rgba64le"],
            &png,
            &dir.path().join("range.raw"),
        );
        let code = |bytes: &[u8]| f64::from(u16::from_le_bytes([bytes[0], bytes[1]])) / 65535.;
        let exported: Vec<[f64; 3]> = raw
            .chunks_exact(8)
            .map(|p| [code(p), code(&p[2..]), code(&p[4..])])
            .collect();
        // The export renders in half float: half a stored code below 1.
        check("the exported PNG", bands(&exported), 2.5e-4);

        let viewer = service
            .viewer(ViewerConfig {
                composition: snapshot.open_comp().unwrap(),
                snapshot: snapshot.clone(),
                media: media.clone(),
                output: Output {
                    width: w,
                    height: h,
                    display: 0,
                    headroom: 1.,
                    crop: None,
                },
                layer: None,
                rate: pm_time::Rate::FPS_30,
                end: Some(3 * TICKS_PER_SECOND),
                audio: false,
            })
            .unwrap();
        viewer.quality(Quality::Full).unwrap();
        viewer
            .command(C::Seek {
                time: at,
                mode: SeekMode::Exact,
            })
            .unwrap();
        wait(&viewer, "the playhead three frames in", |f| f.time == at);
        viewer.capture_next().unwrap();
        let paused = wait(&viewer, "a paused frame", |f| {
            f.time == at && f.moment == Moment::Paused && f.pixels.is_some() && f.scale >= 1.
        });
        let paused = encoded_rec709(&paused.pixels.unwrap());
        check("the paused viewer", bands(&paused), 0.0005);
        // The limited reading puts the brightest band above white; the
        // viewer's surface keeps that and an SDR file stops at white.
        let shown: Vec<[f64; 3]> = paused.iter().map(|p| p.map(|v| v.clamp(0., 1.))).collect();
        let (worst, mean) = difference(&shown, &exported);
        assert!(
            worst < 0.0005,
            "{interpret:?}: the viewer against the PNG: worst {worst:.5}, mean {mean:.6}"
        );
        viewer.retire();
    }
}

/// The v1 tests' clip: 1280×720 FFV1 with no colour tag, four saturated
/// bands (R′G′B′ codes converted by FFmpeg to 4:2:0 Y′CbCr). HD, so the
/// native reading takes Rec.709 and a v1 project's (Electron's) Rec.601.
const V1: (u32, u32) = (64, 36);
fn untagged_hd_clip(dir: &Path) -> PathBuf {
    let (w, h) = (1280usize, 720usize);
    let colours = [
        [200u8, 40, 40],
        [40, 200, 40],
        [40, 40, 200],
        [180, 160, 60],
    ];
    let mut row = Vec::with_capacity(w * 3);
    for x in 0..w {
        row.extend(colours[x * 4 / w]);
    }
    let input = dir.join("hd.rgb");
    std::fs::write(&input, row.repeat(h * 12)).unwrap();
    let output = dir.join("Untagged.mkv");
    let status = std::process::Command::new(ffmpeg_bin())
        .args(["-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgb24"])
        .args(["-s", "1280x720", "-r", "30", "-i"])
        .arg(&input)
        .args(["-c:v", "ffv1", "-pix_fmt", "yuv420p"])
        .arg(&output)
        .status()
        .unwrap();
    assert!(status.success(), "ffmpeg wrote the clip");
    output
}

/// A v1 project around that clip, as v1.2 stored it: the asset has no
/// `timing` and no colour marker.
fn v1_project(clip: &Path) -> Value {
    let (w, h) = V1;
    json!({
        "id": "P", "name": "Synthetic v1", "compId": "root", "compName": "Main", "w": w, "h": h,
        "fps": 30, "dur": 3, "bg": "#000000", "work": [0, 3],
        "layers": [clip_layer(w, h, f64::from(w) / 2., f64::from(h) / 2.)],
        "assets": {"clip": {"id": "clip", "name": "Untagged.mkv", "kind": "video", "format": "mkv",
            "w": 1280, "h": 720, "dur": 0.4, "fps": 30, "sourcePath": clip}},
        "comps": {}
    })
}

/// The paused, exact frame of `snapshot` at [`AT`], as display-encoded Rec.709.
fn still_frame(
    engine: &mut Headless,
    snapshot: &Snapshot,
    media: pm_export::render::MediaPaths,
) -> Vec<[f64; 3]> {
    let (w, h) = V1;
    let scene = Scene {
        doc: snapshot.document.clone(),
        comp: snapshot.open_comp().unwrap(),
        media: std::sync::Arc::new(media),
        content: 1,
    };
    let output = Output {
        width: w,
        height: h,
        display: 0,
        headroom: 1.,
        crop: None,
    };
    let pixels = engine
        .frame_spec_settled(
            &scene,
            FrameSpec::exact(Ticks::new(AT).unwrap(), output),
            &Default::default(),
        )
        .unwrap();
    encoded_rec709(&pixels)
}

/// A v1 project reads its video the same whether it is opened from the v1
/// library or from a `.pmv` file it was saved to: both carry the v1 video
/// reading (`videoColor`), so a Save As and reopen never changes the picture.
#[test]
fn a_v1_project_reads_its_video_the_same_from_the_library_and_from_its_file() {
    use pm_kernel::{
        Kernel, KernelConfig,
        persistence::{DiskFormat, encode_disk_value},
        session::{SessionSource, ShellCall, ShellReply},
    };
    let dir = tempfile::tempdir().unwrap();
    let clip = untagged_hd_clip(dir.path());
    let store = dir.path().join("store");
    std::fs::create_dir(&store).unwrap();
    let slot = json!({"v": "1.2.0", "proj": v1_project(&clip)});
    let slot = pm_model::json::parse(&slot.to_string()).unwrap();
    std::fs::write(
        store.join("project.P.json"),
        encode_disk_value(&slot, DiskFormat::V1_2).unwrap(),
    )
    .unwrap();
    std::fs::write(
        store.join("projects.json"),
        r#"[{"id":"P","name":"Synthetic v1"}]"#,
    )
    .unwrap();
    std::fs::write(
        store.join("projectHistory.P.json"),
        r#"{"version":1,"paths":[],"index":0,"entries":[]}"#,
    )
    .unwrap();
    let opened = |k: &mut Kernel, id: &str, source: SessionSource| match k
        .shell(ShellCall::Open {
            id: id.into(),
            source,
            electron_profile: None,
        })
        .unwrap()
    {
        ShellReply::Opened { session, .. } => session,
        other => panic!("{other:?}"),
    };
    let mut k = Kernel::new(KernelConfig::default());
    k.shell(ShellCall::AttachLibrary {
        profile: dir.path().to_path_buf(),
        format: DiskFormat::V1_2,
        electron_profile: None,
    })
    .unwrap();
    let session = opened(
        &mut k,
        "P",
        SessionSource::Library {
            profile: dir.path().to_path_buf(),
            project: "P".into(),
        },
    );
    let from_library = k.project("P").unwrap().store().snapshot();
    let file = dir.path().join("Saved.pmv");
    k.shell(ShellCall::SaveAs {
        session: session.clone(),
        path: file.clone(),
        replace: false,
    })
    .unwrap();
    k.shell(ShellCall::Close(session)).unwrap();
    drop(k);
    let mut k = Kernel::new(KernelConfig::default());
    opened(&mut k, "F", SessionSource::File(file));
    let from_file = k.project("F").unwrap().store().snapshot();

    let mut media = MediaCatalog::default();
    media.originals.insert("clip".into(), clip.clone());
    let mut engine = Headless::new().unwrap();
    let library = still_frame(&mut engine, &from_library, media.export_paths(false));
    let reopened = still_frame(&mut engine, &from_file, media.export_paths(false));
    let (worst, _) = difference(&library, &reopened);
    assert!(worst < 1e-6, "library against the saved file: {worst:e}");
    let marker = |s: &Snapshot| {
        let doc = pm_model::canonical::realize_project(s.document.to_value().unwrap());
        doc.get("assets")
            .and_then(|a| a.get("clip"))
            .and_then(|c| c.get("videoColor"))
            .and_then(|v| v.as_str().map(str::to_owned))
    };
    assert_eq!(
        marker(&from_library).as_deref(),
        Some("electron"),
        "from the library"
    );
    assert_eq!(
        marker(&from_file).as_deref(),
        Some("electron"),
        "from the file"
    );

    // The reading that would have been shown natively is another picture.
    let native = pm_kernel::document::DocumentStore::new_untitled(
        pm_model::json::parse(
            &{
                let mut p = v1_project(&clip);
                p["assets"]["clip"]["timing"] = json!(2);
                p
            }
            .to_string(),
        )
        .unwrap(),
    )
    .snapshot();
    let native = still_frame(&mut engine, &native, media.export_paths(false));
    let (apart, _) = difference(&library, &native);
    assert!(
        apart > 0.03,
        "the v1 reading differs from the native one: {apart}"
    );
}

/// A proxy really made from `original` by the app's generator (ProRes Proxy
/// at full size).
fn proxy_of(dir: &Path, original: &Path) -> PathBuf {
    use pm_protocol::priority::{ProxyFormat, ProxySize};
    let codecs = PathBuf::from(std::env::var_os("PM_FFMPEG_DIR").unwrap()).join("bin");
    let store = pm_proxy::ProxyStore::open(dir.join("proxies")).unwrap();
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let hash = blake3::hash(&std::fs::read(original).unwrap())
        .to_hex()
        .to_string();
    pm_proxy::generate_with_disk_probe(
        pm_proxy::GenerationRequest {
            source: original,
            source_hash: &hash,
            format: ProxyFormat::ProResProxy,
            size: ProxySize::Full,
            ffmpeg: &codecs.join("ffmpeg"),
            ffprobe: &codecs.join("ffprobe"),
            store: &store,
            control: pm_proxy::GenerationControl {
                cancel: &cancel,
                pause: &|| false,
                on_pause: &|_| {},
                on_progress: &|_| {},
            },
        },
        &|_: &Path| -> pm_proxy::Result<(u64, u64)> { Ok((40 << 30, 100 << 30)) },
    )
    .unwrap()
    .stored
    .path
}

/// A clip read as in Powermove 1 (`videoColor: "electron"`, the asset of a
/// loaded v1 project) shows one picture everywhere: the paused viewer, a
/// playing frame, the frame the disk render cache stores, the clip drawn
/// from its generated proxy (Proxy Preferred/Only) and the exported 16-bit
/// PNG. Measured on 2026-11-13 (debug, Metal): see the eprintln lines.
#[test]
fn a_clip_read_as_in_powermove_1_is_the_same_in_the_viewer_the_cache_the_proxy_and_the_export() {
    let dir = tempfile::tempdir().unwrap();
    let clip = untagged_hd_clip(dir.path());
    let mut project = v1_project(&clip);
    project["assets"]["clip"]["videoColor"] = json!("electron");
    project["requiresEngine"] = json!("native");
    let snapshot = pm_kernel::document::DocumentStore::new_untitled(
        pm_model::json::parse(&project.to_string()).unwrap(),
    )
    .snapshot();
    let mut media = MediaCatalog::default();
    media.originals.insert("clip".into(), clip.clone());
    let (w, h) = V1;
    let at = Ticks::new(AT).unwrap();
    let exe = Path::new(env!("CARGO_BIN_EXE_pm-engine-service"));

    let png = dir.path().join("v1.png");
    export_sized(
        &snapshot,
        &media,
        &png,
        ExportFormat::PngStill { sixteen_bit: true },
        OutputColor::SrgbImage,
        AlphaOutput::None,
        V1,
    );
    let raw = ffmpeg(
        &["-f", "rawvideo", "-pix_fmt", "rgba64le"],
        &png,
        &dir.path().join("v1.raw"),
    );
    let code = |bytes: &[u8]| f64::from(u16::from_le_bytes([bytes[0], bytes[1]])) / 65535.;
    let exported: Vec<[f64; 3]> = raw
        .chunks_exact(8)
        .map(|p| [code(p), code(&p[2..]), code(&p[4..])])
        .collect();
    let mut engine = Headless::new().unwrap();
    // The export shows the v1 reading, not the native one.
    let native = {
        let mut p = project.clone();
        p["assets"]["clip"]
            .as_object_mut()
            .unwrap()
            .remove("videoColor");
        p["assets"]["clip"]["timing"] = json!(2);
        pm_kernel::document::DocumentStore::new_untitled(
            pm_model::json::parse(&p.to_string()).unwrap(),
        )
        .snapshot()
    };
    let native = still_frame(&mut engine, &native, media.export_paths(false));
    let (apart, _) = difference(&exported, &native);
    assert!(apart > 0.03, "the export reads the clip as in v1: {apart}");

    // The middle of each band, away from edges and chroma transitions.
    let bands = |pixels: &[[f64; 3]]| -> Vec<[f64; 3]> {
        (0..4)
            .map(|band| pixels[(h as usize / 2) * w as usize + band * 16 + 8])
            .collect()
    };
    // The paused viewer.
    let service = EngineService::start(pm_gpu::Gpu::new("v1-reading").unwrap()).unwrap();
    let disk = service.render_cache();
    disk.set_storage(Some((dir.path().join("frames"), 1 << 30)))
        .unwrap();
    let output = Output {
        width: w,
        height: h,
        display: 0,
        headroom: 1.,
        crop: None,
    };
    let viewer = service
        .viewer(ViewerConfig {
            composition: snapshot.open_comp().unwrap(),
            snapshot: snapshot.clone(),
            media: media.clone(),
            output,
            layer: None,
            rate: pm_time::Rate::FPS_30,
            end: Some(3 * TICKS_PER_SECOND),
            audio: false,
        })
        .unwrap();
    viewer.quality(Quality::Full).unwrap();
    viewer
        .command(C::Seek {
            time: at,
            mode: SeekMode::Exact,
        })
        .unwrap();
    wait(&viewer, "the playhead three frames in", |f| f.time == at);
    viewer.capture_next().unwrap();
    let paused = wait(&viewer, "a paused frame", |f| {
        f.time == at && f.moment == Moment::Paused && f.pixels.is_some() && f.scale >= 1.
    });
    let paused = encoded_rec709(&paused.pixels.unwrap());
    let (worst, mean) = difference(&paused, &exported);
    eprintln!("paused viewer vs PNG: worst {worst:.5}, mean {mean:.6}");
    assert!(
        worst < 0.0005,
        "the paused viewer against the PNG: {worst:.5}"
    );

    // A playing frame.
    let scene = Scene {
        doc: snapshot.document.clone(),
        comp: snapshot.open_comp().unwrap(),
        media: std::sync::Arc::new(media.export_paths(false)),
        content: 1,
    };
    let playing = engine
        .frame_spec_settled(
            &scene,
            FrameSpec {
                moment: Moment::Playing,
                exact: false,
                ..FrameSpec::exact(at, output)
            },
            &Default::default(),
        )
        .unwrap();
    // A playing frame may take a cheaper resampler for this 20:1 reduction
    // (band edges move by a few hundredths); the reading is what is compared,
    // in the middle of each band.
    let (worst, _) = difference(&bands(&encoded_rec709(&playing)), &bands(&paused));
    eprintln!("playing vs paused (band centres): worst {worst:e}");
    assert!(
        worst < 1e-6,
        "the playing frame against the paused one: {worst:e}"
    );

    // The frame the disk render cache stores.
    let start = Instant::now();
    while disk
        .start_fill(0., 1., exe, jobs::Activity::default(), |_| {})
        .is_err()
    {
        assert!(start.elapsed() < Duration::from_secs(30), "scene not read");
        std::thread::sleep(Duration::from_millis(20));
    }
    while disk.bars().running || disk.bars().done == 0 {
        assert!(
            start.elapsed() < Duration::from_secs(120),
            "{:?}",
            disk.bars()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(disk.bars().error, None);
    let rendered = disk.observed_scene().expect("the composition is observed");
    let stored = disk
        .stored_frame(&rendered, at)
        .expect("the frame three frames in is on disk");
    let cached: Vec<[f64; 3]> = stored
        .chunks_exact(8)
        .map(|p| {
            let value = |c: usize| f64::from(half::f16::from_le_bytes([p[c * 2], p[c * 2 + 1]]));
            [value(0), value(1), value(2)]
        })
        .collect();
    let (worst, mean) = difference(&cached, &exported);
    eprintln!("cached frame vs PNG: worst {worst:.6}, mean {mean:.7}");
    assert!(
        worst < 1. / 65535.,
        "the cached frame against the PNG: {worst:.6}"
    );
    viewer.retire();

    // The clip drawn from its generated proxy, which is tagged with the
    // native reading; in the middle of each band, away from chroma edges.
    let proxy = proxy_of(dir.path(), &clip);
    let mut proxied = media.clone();
    proxied.proxies.insert("clip".into(), proxy);
    let from_proxy = still_frame(&mut engine, &snapshot, proxied.export_paths(true));
    let (worst, _) = difference(&bands(&from_proxy), &bands(&exported));
    eprintln!("proxy vs PNG (band centres): worst {worst:.5}");
    // ProRes Proxy on flat bands, then the v1 reading's 8-bit upload.
    assert!(worst < 0.012, "the proxy against the PNG: {worst:.5}");
}
