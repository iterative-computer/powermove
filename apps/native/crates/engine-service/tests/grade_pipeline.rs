//! One graded composition read five ways: the paused viewer, the playing
//! viewer, the frame the disk render cache stores, what the scopes count,
//! and the exported files (ProRes 4444 and a 16-bit PNG, decoded by FFmpeg).
//! Every reading is compared with the 16-bit PNG, as display-encoded Rec.709.
//!
//! The grade is an adjustment layer with a LUT, a gamma curve and a lift and
//! gain, over a gradient and flat patches.
//!
//! Stated tolerances, of full scale, with what a debug build measured on
//! 2026-11-12 (Apple silicon, Metal):
//! - the playing viewer against the paused one: identical (measured 0);
//! - the cached frame against the PNG: one 16-bit code (measured 7.6e-6,
//!   half a code: the cache holds the export's own values);
//! - the paused viewer (a half-float surface in linear Display P3, brought
//!   back to encoded Rec.709) against the PNG: 0.0005 (measured 0.00023,
//!   mean 0.00005);
//! - ProRes 4444 (12-bit Y′CbCr through a lossy codec) against the PNG:
//!   0.005 at the worst pixel, 0.0006 on average (measured 0.0032, 0.00031);
//! - scopes: at most 1 pixel in 200 counted in a different level than the
//!   same scope of the PNG (measured 1 in 575 at worst, the waveform).
use pm_engine_service::{jobs::*, *};
use pm_protocol::{
    color::{OutputColor, Precision, QuickTimeTag},
    common::{FrameRate, SessionId, Size, TICKS_PER_SECOND, TickRange, Ticks},
    priority::{AlphaOutput, ExportFormat, ExportSettings, JobSpec, JobState, ProResProfile},
};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const W: u32 = 192;
const H: u32 = 108;

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
        "layers": [
            grade,
            solid("skin", "#C68E6E", 40, 30, 30., 20., 100.),
            solid("red", "#E0261C", 30, 30, 90., 30., 100.),
            solid("green", "#1FB34A", 36, 24, 150., 70., 100.),
            solid("veil", "#FFFFFF", 80, 20, 100., 90., 60.),
        ],
        "assets": {"look": {"id": "look", "name": "Look.cube", "kind": "lut", "format": "cube"}},
        "comps": {}
    });
    pm_model::json::parse(&value.to_string()).unwrap()
}

fn ffmpeg(args: &[&str], input: &Path, output: &Path) -> Vec<u8> {
    let dir = PathBuf::from(
        std::env::var_os("PM_FFMPEG_DIR").expect("source engine/env.sh before this test"),
    );
    let status = std::process::Command::new(dir.join("bin/ffmpeg"))
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
    let end = if format.is_image() {
        1
    } else {
        TICKS_PER_SECOND / 10
    };
    let settings = ExportSettings {
        format,
        alpha,
        quality: pm_protocol::priority::QualityTier::High,
        size: Size::new(W, H),
        frame_rate: FrameRate::new(30, 1),
        range: TickRange {
            start: Ticks::ZERO,
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
    let exe = Path::new(env!("CARGO_BIN_EXE_pm-engine-service"));

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
    let (taps, counted) = async_channel::bounded(1);
    let config = pm_render::scopes::ScopeConfig::PANEL;
    viewer.scopes(Some((config, taps))).unwrap();
    viewer.capture_next().unwrap();
    let paused = wait(&viewer, "a paused frame", |f| {
        f.moment == Moment::Paused && f.pixels.is_some() && f.scale >= 1.
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
                ..FrameSpec::exact(Ticks::ZERO, output)
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

    // The frame the disk render cache stores for playback.
    let start = Instant::now();
    while disk
        .start_fill(0., 0.05, exe, jobs::Activity::default(), |_| {})
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
    let mut stored = vec![];
    let mut folders = vec![frames];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(folder).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                folders.push(path);
            } else if let Ok(data) = std::fs::read(&path)
                && data.len() > 48
                && let Some(frame) = frame_codec::decode(&data[48..], W, H)
            {
                stored.push(frame);
            }
        }
    }
    assert!(!stored.is_empty(), "a rendered frame is on disk");
    let cached: Vec<[f64; 3]> = stored[0]
        .chunks_exact(8)
        .map(|p| {
            let value = |c: usize| f64::from(half::f16::from_le_bytes([p[c * 2], p[c * 2 + 1]]));
            assert_eq!(value(3), 1., "the frame is opaque");
            [value(0), value(1), value(2)]
        })
        .collect();
    let (worst, mean) = difference(&cached, &exported);
    assert!(
        worst < 1. / 65535.,
        "the cached frame against the 16-bit PNG: worst {worst:.6}, mean {mean:.7}"
    );
    viewer.retire();
}
