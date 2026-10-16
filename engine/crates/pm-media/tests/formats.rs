//! Plan section 13's format matrix ("Formats at cutover"), one generated file
//! per format and variant (`tests/corpus/formats.sh`, our LGPL FFmpeg build;
//! output outside the repository). For every file:
//!
//! - video: the probe's facts (codec, native depth, subsampling, alpha,
//!   color tags and section 9 defaults, fields, pixel aspect, clean
//!   aperture, rotation, timecode, reel, identifiers, HDR format, mastering
//!   metadata), and section 2's Seeking bar (random access to every
//!   composition frame equals the sequential decode, every backend);
//! - bit depth: 10- and 12-bit ramps keep more distinct codes than any
//!   8-bit path could;
//! - color: exact bars decode to the codes written;
//! - stills: native depth and exact values against the generator's
//!   formulas or our FFmpeg build's decode;
//! - sequences report their gaps; animations their frames and delays;
//! - audio: every format decodes to the program length at its rate with
//!   its channel layout.
//!
//! ```sh
//! engine/crates/pm-media/tests/corpus/formats.sh
//! cargo test -p pm-media --release --test formats -- --ignored --test-threads=2
//! ```

mod common;

use common::*;
use pm_media::audio::AudioDecoder;
use pm_media::decode::service::MediaSource;
use pm_media::frame::{DecodedFrame, Layout, PlaneFormat};
use pm_media::image::{self, ImageFormat};
use pm_media::interpret::{Interpretation, Origin, resolve_video};
use pm_media::pool::FramePool;
use pm_media::probe::color::HdrFormat;
use pm_media::probe::{AlphaSource, FieldOrder, MediaProbe, VideoProbe};
use pm_media::vt::DecodePurpose;
use pm_protocol::color::{MatrixCoefficients, Primaries, Range, Subsampling, Transfer};
use std::path::{Path, PathBuf};
use std::process::Command;

fn formats() -> PathBuf {
    let dir = std::env::var_os("PM_MEDIA_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var("HOME").unwrap())
                .join("Library/Caches/powermove-engine/media-corpus")
        })
        .join("formats");
    assert!(
        dir.join("video/prores-hq.mov").is_file(),
        "the format corpus is missing at {}; run engine/crates/pm-media/tests/corpus/formats.sh",
        dir.display()
    );
    dir
}

fn probe(path: &Path) -> MediaProbe {
    pm_media::probe::probe(path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .probe
}

fn video(p: &MediaProbe) -> &VideoProbe {
    p.video.as_ref().expect("a video stream")
}

/// What a file's probe must say. `None` fields are not checked.
#[derive(Default, Clone)]
struct Want {
    codec: &'static str,
    bits: u8,
    sub: Option<Subsampling>,
    alpha: Option<AlphaSource>,
    primaries: Option<Primaries>,
    transfer: Option<Transfer>,
    matrix: Option<MatrixCoefficients>,
    range: Option<Range>,
    fields: Option<FieldOrder>,
    pasp: Option<(i64, i64)>,
    size: Option<(u32, u32)>,
}

fn w(codec: &'static str, bits: u8, sub: Subsampling) -> Want {
    Want {
        codec,
        bits,
        sub: Some(sub),
        ..Default::default()
    }
}

use Subsampling::{S420, S422, S444};

fn video_table() -> Vec<(&'static str, Want)> {
    let alpha = |mut x: Want, a: AlphaSource| {
        x.alpha = Some(a);
        x
    };
    let full = |mut x: Want| {
        x.range = Some(Range::Full);
        x
    };
    let fields = |mut x: Want, f: FieldOrder| {
        x.fields = Some(f);
        x
    };
    let pasp = |mut x: Want, p: (i64, i64)| {
        x.pasp = Some(p);
        x
    };
    let rgb = |mut x: Want| {
        x.matrix = Some(MatrixCoefficients::Identity);
        x.range = Some(Range::Full);
        x
    };
    let mut t = vec![
        ("prores-proxy.mov", w("prores", 10, S422)),
        ("prores-lt.mov", w("prores", 10, S422)),
        ("prores-standard.mov", w("prores", 10, S422)),
        ("prores-hq.mov", w("prores", 10, S422)),
        ("prores-4444.mov", w("prores", 12, S444)),
        (
            "prores-4444-alpha.mov",
            alpha(w("prores", 12, S444), AlphaSource::PixelFormat),
        ),
        (
            "prores-xq-alpha.mov",
            alpha(w("prores", 12, S444), AlphaSource::PixelFormat),
        ),
        ("dnxhd-120-1080p25.mov", w("dnxhd", 8, S422)),
        ("dnxhd-185x-1080p25.mxf", w("dnxhd", 10, S422)),
        ("dnxhr-lb.mov", w("dnxhd", 8, S422)),
        ("dnxhr-sq.mov", w("dnxhd", 8, S422)),
        ("dnxhr-hq.mov", w("dnxhd", 8, S422)),
        ("dnxhr-hqx.mov", w("dnxhd", 10, S422)),
        ("dnxhr-444.mxf", w("dnxhd", 10, S444)),
        ("cineform-422.mov", w("cfhd", 10, S422)),
        ("cineform-rgb.mov", rgb(w("cfhd", 12, S444))),
        (
            "cineform-rgba.avi",
            alpha(rgb(w("cfhd", 12, S444)), AlphaSource::PixelFormat),
        ),
        ("h264-vt.mp4", w("h264", 8, S420)),
        ("h264-vt.mov", w("h264", 8, S420)),
        ("hevc-8.mov", w("hevc", 8, S420)),
        ("hevc-10.mp4", w("hevc", 10, S420)),
        ("hevc-422-10.mov", w("hevc", 10, S422)),
        (
            "hevc-alpha.mov",
            alpha(w("hevc", 8, S420), AlphaSource::HevcAlphaLayer),
        ),
        ("av1-8.mp4", w("av1", 8, S420)),
        ("av1-10.mkv", w("av1", 10, S420)),
        ("mjpeg-422.avi", full(w("mjpeg", 8, S422))),
        ("mjpeg-420.mov", full(w("mjpeg", 8, S420))),
        ("mjpeg-444.mkv", full(w("mjpeg", 8, S444))),
        (
            "dv-ntsc.dv",
            pasp(
                fields(w("dvvideo", 8, S422), FieldOrder::BottomFirst),
                (8, 9),
            ),
        ),
        (
            "dv-pal.dv",
            pasp(
                fields(w("dvvideo", 8, S420), FieldOrder::BottomFirst),
                (64, 45),
            ),
        ),
        (
            "dvcpro25-pal.mov",
            pasp(
                fields(w("dvvideo", 8, S422), FieldOrder::BottomFirst),
                (16, 15),
            ),
        ),
        (
            "dvcpro50.mov",
            fields(w("dvvideo", 8, S422), FieldOrder::BottomFirst),
        ),
        (
            "dvcprohd-1080i50.mov",
            fields(w("dvvideo", 8, S422), FieldOrder::TopFirst),
        ),
        ("vp8.webm", w("vp8", 8, S420)),
        (
            "vp8-alpha.webm",
            alpha(w("vp8", 8, S420), AlphaSource::MatroskaBlockAdditional),
        ),
        ("vp9.webm", w("vp9", 8, S420)),
        (
            "vp9-alpha.webm",
            alpha(w("vp9", 8, S420), AlphaSource::MatroskaBlockAdditional),
        ),
        ("vp9-10.webm", w("vp9", 10, S420)),
        ("vp9-444.mkv", w("vp9", 8, S444)),
        (
            "interlaced-tff.mov",
            fields(w("prores", 10, S422), FieldOrder::TopFirst),
        ),
        (
            "interlaced-bff.mxf",
            fields(w("mpeg2video", 8, S422), FieldOrder::BottomFirst),
        ),
        ("meta-pasp.mov", pasp(w("prores", 10, S422), (8, 9))),
    ];
    // DV 4:1:1 has no contract 4 value; the probe reports 4:2:2 and the
    // frame format carries the exact factors (see probe::color).
    for (name, x) in &mut t {
        if name.starts_with("dv-ntsc") || name.starts_with("dvcpro25") {
            x.sub = Some(S422);
        }
    }
    t
}

fn check_want(name: &str, p: &MediaProbe, x: &Want, out: &mut Vec<String>) {
    let v = video(p);
    let c = &v.color.interpreted;
    let mut bad = |what: &str, got: String, want: String| {
        if got != want {
            out.push(format!("{name}: {what} is {got}, want {want}"));
        }
    };
    bad("codec", v.codec.codec.clone(), x.codec.into());
    bad("bit depth", c.bit_depth.to_string(), x.bits.to_string());
    if let Some(s) = x.sub {
        bad(
            "subsampling",
            format!("{:?}", c.subsampling),
            format!("{s:?}"),
        );
    }
    bad("alpha", format!("{:?}", v.alpha), format!("{:?}", x.alpha));
    if let Some(p) = x.primaries {
        bad("primaries", format!("{:?}", c.primaries), format!("{p:?}"));
    }
    if let Some(t) = x.transfer {
        bad("transfer", format!("{:?}", c.transfer), format!("{t:?}"));
    }
    if let Some(m) = x.matrix {
        bad("matrix", format!("{:?}", c.matrix), format!("{m:?}"));
    }
    if let Some(r) = x.range {
        bad("range", format!("{:?}", c.range), format!("{r:?}"));
    }
    let order = x.fields.unwrap_or(FieldOrder::Progressive);
    let got = match v.fields.order {
        FieldOrder::Unknown => FieldOrder::Progressive,
        o => o,
    };
    bad("field order", format!("{got:?}"), format!("{order:?}"));
    bad(
        "pixel aspect",
        format!("{:?}", v.pixel_aspect),
        format!("{:?}", x.pasp.unwrap_or((1, 1))),
    );
    if let Some(s) = x.size {
        bad(
            "size",
            format!("{:?}", (v.width, v.height)),
            format!("{s:?}"),
        );
    }
}

/// Every video variant: probe facts at native depth.
#[test]
#[ignore]
fn video_probe_facts() {
    let dir = formats().join("video");
    let mut failures = vec![];
    for (name, x) in video_table() {
        let p = probe(&dir.join(name));
        check_want(name, &p, &x, &mut failures);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Tags written by the generator, and section 9's defaults for untagged
/// files, each with its origin.
#[test]
#[ignore]
fn color_tags_and_defaults() {
    let dir = formats();
    let color = |f: &str| video(&probe(&dir.join(f))).color.clone();
    let c = color("video/color-601-625.mov");
    assert_eq!(c.interpreted.primaries, Primaries::Bt601_625);
    assert_eq!(c.interpreted.matrix, MatrixCoefficients::Bt601);
    assert_eq!(c.interpreted.transfer, Transfer::DisplayE);
    assert!(matches!(c.origins.primaries, Origin::File { .. }));

    let c = color("video/color-709-full.mp4");
    assert_eq!(c.interpreted.primaries, Primaries::Bt709);
    assert_eq!(c.interpreted.range, Range::Full);
    assert!(
        matches!(c.origins.range, Origin::File { .. }),
        "{:?}",
        c.origins
    );

    let c = color("video/color-p3.mov");
    assert_eq!(c.interpreted.primaries, Primaries::DisplayP3);
    assert_eq!(c.interpreted.transfer, Transfer::DisplayE);
    assert_eq!(c.tagged.transfer, Some(13));

    // Untagged HD and SD: section 9's rules, reported as defaults.
    let c = color("video/h264-vt.mp4");
    assert_eq!(c.tagged.primaries, None, "{:?}", c.tagged);
    assert_eq!(c.interpreted.primaries, Primaries::Bt601_525);
    assert_eq!(c.interpreted.matrix, MatrixCoefficients::Bt601);
    assert!(matches!(c.origins.primaries, Origin::Default { .. }));
    // DNxHR's frame header carries its matrix; the primaries are untagged
    // and 640×360 is SD by height.
    let c = color("video/dnxhr-hq.mov");
    assert_eq!(c.interpreted.primaries, Primaries::Bt601_525);
    assert!(matches!(c.origins.primaries, Origin::Default { .. }));
    assert_eq!(c.interpreted.matrix, MatrixCoefficients::Bt709);
    assert!(matches!(c.origins.matrix, Origin::File { .. }));
    let c = color("video/dnxhd-120-1080p25.mov");
    if c.tagged.primaries.is_none() {
        assert_eq!(c.interpreted.primaries, Primaries::Bt709);
        assert!(matches!(c.origins.primaries, Origin::Default { .. }));
    }
    if c.tagged.matrix.is_none() {
        assert_eq!(c.interpreted.matrix, MatrixCoefficients::Bt709);
    }
    let c = color("video/dv-pal.dv");
    if c.tagged.primaries.is_none() {
        assert_eq!(c.interpreted.primaries, Primaries::Bt601_625, "576 lines");
    }
    // MJPEG is full range by rule; untagged chroma is sited left.
    let c = color("video/mjpeg-420.mov");
    assert_eq!(c.interpreted.range, Range::Full);
    let c = color("video/vp9.webm");
    assert!(matches!(
        c.origins.siting,
        Origin::Default { .. } | Origin::File { .. }
    ));
}

/// HDR10 (with mastering display and content light), HLG, Dolby Vision
/// 8.4, PQ in AV1 and VP9: probe facts and the tone map the reading asks for.
#[test]
#[ignore]
fn hdr_sources() {
    let dir = formats().join("hdr");
    let p = probe(&dir.join("hdr10.mov"));
    let v = video(&p);
    let c = &v.color;
    assert_eq!(c.interpreted.primaries, Primaries::Bt2020);
    assert_eq!(c.interpreted.transfer, Transfer::Pq);
    assert_eq!(c.interpreted.matrix, MatrixCoefficients::Bt2020Ncl);
    assert_eq!(c.interpreted.bit_depth, 10);
    assert_eq!(c.hdr, Some(HdrFormat::Hdr10));
    let m = c.mastering.as_ref().expect("mastering display");
    assert_eq!(m.max_luminance, Some(1000.0));
    assert_eq!(m.min_luminance, Some(0.0001));
    assert_eq!(m.red, Some((0.68, 0.32)));
    assert_eq!(m.green, Some((0.265, 0.69)));
    assert_eq!(m.blue, Some((0.15, 0.06)));
    assert_eq!(m.white, Some((0.3127, 0.329)));
    let l = c.content_light.expect("content light");
    assert_eq!((l.max_cll, l.max_fall), (1000, 400));
    let r = resolve_video(v, &Interpretation::default());
    let tm = r.tone_map.expect("HDR10 is tone mapped in Standard");
    assert_eq!((tm.transfer, tm.peak_nits), (Transfer::Pq, 1000.0));

    let p = probe(&dir.join("hlg.mov"));
    let v = video(&p);
    assert_eq!(v.color.interpreted.transfer, Transfer::Hlg);
    assert_eq!(v.color.hdr, Some(HdrFormat::Hlg));
    assert_eq!(
        resolve_video(v, &Interpretation::default())
            .tone_map
            .unwrap()
            .peak_nits,
        1000.0
    );

    let p = probe(&dir.join("dovi-8.4.mov"));
    assert_eq!(
        video(&p).color.hdr,
        Some(HdrFormat::DolbyVision {
            profile: 8,
            level: 4,
            base_compatibility: 4
        })
    );
    for f in ["pq-av1.mkv", "pq-vp9.webm"] {
        let p = probe(&dir.join(f));
        let c = &video(&p).color;
        assert_eq!(c.interpreted.transfer, Transfer::Pq, "{f}");
        assert_eq!(c.interpreted.primaries, Primaries::Bt2020, "{f}");
        assert_eq!(c.hdr, Some(HdrFormat::Hdr10), "{f}");
    }
    // Interpreting an HDR source as SDR turns the tone map off.
    let (i, _) = Interpretation::from_json(&serde_json::json!({
        "transfer": "displayE", "primaries": "bt709"
    }));
    let p = probe(&dir.join("hdr10.mov"));
    let r = resolve_video(video(&p), &i);
    assert!(r.tone_map.is_none());
    assert!(r.transfer.origin.is_override());
}

/// Timecode (drop frame), reel, content identifier; pixel aspect, clean
/// aperture, rotation; and how each reads with and without overrides.
#[test]
#[ignore]
fn metadata_and_geometry() {
    let dir = formats().join("video");
    let p = probe(&dir.join("meta-timecode.mov"));
    let tc = p.timecode.as_ref().expect("timecode");
    assert_eq!(tc.text, "01:00:00;00");
    assert!(tc.drop_frame);
    assert_eq!(tc.frame, Some(107_892));
    assert_eq!(p.reel.as_deref(), Some("A001"));
    assert_eq!(
        p.content_identifier.as_deref(),
        Some("6B2F5C33-ACDE-4E1C-9A6A-6F1AE3D0F0F1")
    );

    let p = probe(&dir.join("meta-pasp.mov"));
    let r = resolve_video(video(&p), &Interpretation::default());
    assert_eq!(r.pixel_aspect.value, (8, 9));
    assert!(matches!(&r.pixel_aspect.origin, Origin::File { source } if source == "pasp"));
    assert_eq!(r.display_size((720, 480)), (640, 480));

    let p = probe(&dir.join("meta-clap.mov"));
    let v = video(&p);
    let clap = v.clean_aperture.expect("clap");
    assert_eq!(clap.rect, (8, 0, 704, 480));
    // The full encoded frame by default (today's look); the clean aperture
    // on request.
    let r = resolve_video(v, &Interpretation::default());
    assert_eq!(r.clean_aperture.value, None);
    let (i, _) = Interpretation::from_json(&serde_json::json!({ "cleanAperture": true }));
    let r = resolve_video(v, &i);
    assert_eq!(r.clean_aperture.value, Some(clap));
    assert_eq!(r.display_size((720, 480)), (626, 480));
    let (i, _) = Interpretation::from_json(&serde_json::json!({ "pixelAspect": [1, 1] }));
    assert_eq!(resolve_video(v, &i).display_size((720, 480)), (720, 480));

    let p = probe(&dir.join("meta-rotate90.mov"));
    let v = video(&p);
    assert_eq!(v.rotation, Some(90.0));
    let r = resolve_video(v, &Interpretation::default());
    assert_eq!(r.quarter_turns(), 1);
    assert_eq!(r.display_size((320, 180)), (180, 320));

    // Frame rate: the timestamps', or a conformed override.
    let p = probe(&dir.join("vfr.mp4"));
    let v = video(&p);
    assert!(v.variable_rate);
    // Section 13 item 5 with amendment 2: 30 and 29.97 both lie within one
    // timebase unit of the median duration, and the mean (pulled long by the
    // dropped frames) breaks the tie towards 29.97. Reported in
    // docs/engine/formats.md.
    let n = v.nominal_rate.as_ref().unwrap();
    assert_eq!(
        n.candidates,
        vec![pm_time::Rate::FPS_29_97, pm_time::Rate::FPS_30]
    );
    let r = resolve_video(v, &Interpretation::default());
    assert_eq!(r.frame_rate.value, pm_time::Rate::FPS_29_97);
    assert_eq!(r.conformed(), None);
    let (i, _) = Interpretation::from_json(&serde_json::json!({ "frameRate": 24 }));
    let r = resolve_video(v, &i);
    assert_eq!(r.conformed(), Some(pm_time::Rate::FPS_24));

    // Field order: the file's, overridden, or progressive (PsF).
    let p = probe(&dir.join("interlaced-tff.mov"));
    let v = video(&p);
    let r = resolve_video(v, &Interpretation::default());
    assert_eq!(r.field_order.value, FieldOrder::TopFirst);
    let (i, _) = Interpretation::from_json(&serde_json::json!({ "fieldOrder": "progressive" }));
    assert_eq!(
        resolve_video(v, &i).field_order.value,
        FieldOrder::Progressive
    );
}

/// Section 2's Seeking bar on every video variant and container.
#[test]
#[ignore]
fn video_random_access() {
    let dir = formats();
    let mut files = videos(&dir.join("video"), &|n| {
        !n.starts_with("ramp") && !n.starts_with("bars")
    });
    files.extend(videos(&dir.join("hdr"), &|_| true));
    run_all(&files, DecodePurpose::Display);
}

/// Every audio-in-video container also seeks exactly on its video.
#[test]
#[ignore]
fn av_container_random_access() {
    let files = videos(&formats().join("audio"), &|n| n.starts_with("av-"));
    assert_eq!(files.len(), 13, "{files:?}");
    run_all(&files, DecodePurpose::Display);
}

/// The first presented frame through the first backend.
fn first_frame(path: &Path) -> DecodedFrame {
    let src = MediaSource::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let service = pm_media::decode::service::DecodeService::new(Default::default());
    let mut s = service
        .session(&src, 1, DecodePurpose::Display)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    s.frame(0).unwrap()
}

/// Luma codes of row `y` at the format's own depth (n-bit limited range).
fn luma_codes(f: &DecodedFrame, y: usize) -> Vec<i64> {
    let (ya, _) = f.reference().unwrap();
    // Ramps are compared as 10-bit codes (the depth they were written at).
    let (off, div) = (64.0, 876.0);
    let w = f.format.width;
    (0..w)
        .map(|x| (f64::from(ya[y * w + x][0]) * div + off).round() as i64)
        .collect()
}

/// Native depth: a 10-bit ramp (877 codes over 1024 columns) keeps far more
/// distinct codes than the 220 an 8-bit path could, and stays monotonic;
/// 12-bit ProRes keeps codes between the 10-bit ones.
#[test]
#[ignore]
fn native_bit_depth() {
    let dir = formats().join("video");
    for (file, min_distinct) in [
        ("ramp10-prores-hq.mov", 800),
        ("ramp10-dnxhr-hqx.mov", 800),
        ("ramp10-cineform.mov", 700),
        ("ramp10-hevc.mov", 600),
        ("ramp10-av1.mkv", 600),
        ("ramp10-vp9.webm", 600),
    ] {
        let f = first_frame(&dir.join(file));
        assert!(f.format.bits >= 10, "{file}: {}", f.format.label);
        let row = luma_codes(&f, 64);
        let mut distinct = row.clone();
        distinct.dedup();
        let back = row.windows(2).filter(|w| w[1] < w[0] - 1).count();
        eprintln!(
            "{file}: {} distinct of 877, {} steps back, codes {}..{} ({})",
            distinct.len(),
            back,
            row[0],
            row[row.len() - 1],
            f.format.label
        );
        assert!(
            distinct.len() >= min_distinct,
            "{file}: {} distinct",
            distinct.len()
        );
        assert!(back <= 8, "{file}: {back} steps back");
        assert!(
            (row[0] - 64).abs() <= 3 && (row[row.len() - 1] - 940).abs() <= 3,
            "{file}"
        );
    }
    let f = first_frame(&dir.join("ramp12-prores-xq.mov"));
    let (ya, _) = f.reference().unwrap();
    let w = f.format.width;
    let codes: Vec<i64> = (0..w)
        .map(|x| (f64::from(ya[64 * w + x][0]) * 3504.0 + 256.0).round() as i64)
        .collect();
    let off_grid = codes.iter().filter(|c| *c % 4 != 0).count();
    eprintln!(
        "ramp12-prores-xq: {off_grid} of {w} codes between 10-bit levels ({})",
        f.format.label
    );
    assert!(off_grid > w / 2, "12-bit ProRes decoded at 10 bits or less");
    // Its alpha ramp survives at full depth. FFmpeg converted the 12-bit
    // source alpha (4x) to the encoder's 16-bit input as 4x · 16, the file
    // stores 16-bit alpha, and VideoToolbox returns those 16 bits (`y416`,
    // A = v / 65535).
    let bad: Vec<(usize, f64)> = (0..w.min(1024))
        .map(|x| (x, f64::from(ya[64 * w + x][1]) * 65535.0))
        .filter(|&(x, a)| (a - 64.0 * x as f64).abs() > 0.5)
        .collect();
    assert!(
        bad.is_empty(),
        "{} alpha codes differ, first {:?}",
        bad.len(),
        &bad[..bad.len().min(8)]
    );
}

/// 75% bars written with exact BT.709 8-bit codes decode to those codes
/// (scaled to the format's depth) on patch centers.
#[test]
#[ignore]
fn bars_decode_to_their_codes() {
    let y = [180, 168, 145, 134, 63, 51, 28, 16];
    let cb = [128, 44, 147, 63, 193, 109, 212, 128];
    let cr = [128, 136, 44, 52, 204, 212, 120, 128];
    let dir = formats().join("video");
    for (file, tol8) in [
        ("bars709-prores4444.mov", 1.0),
        ("bars709-dnxhr444.mov", 1.0),
        ("bars709-mjpeg.mov", 2.0),
    ] {
        let path = dir.join(file);
        if !path.exists() {
            continue;
        }
        let p = probe(&path);
        let f = first_frame(&path);
        let (ya, cc) = f.reference().unwrap();
        let n = f.format.norm();
        let scale = 2f64.powi(f.format.bits as i32 - 8);
        let full = f.format.range == Range::Full;
        let w = f.format.width;
        let cw = w >> f.format.log2_chroma_w;
        for k in 0..8 {
            let x = 40 * k + 20;
            let yy = 64;
            let to8 = |v: f32, off: f64, div: f64| (f64::from(v) * div + off) / scale;
            let (ly, lc) = if full {
                // MJPEG: full-range codes of the same colors.
                (
                    (f64::from(y[k]) - 16.0) * 255.0 / 219.0,
                    [
                        (f64::from(cb[k]) - 128.0) * 255.0 / 224.0 + 128.0,
                        (f64::from(cr[k]) - 128.0) * 255.0 / 224.0 + 128.0,
                    ],
                )
            } else {
                (f64::from(y[k]), [f64::from(cb[k]), f64::from(cr[k])])
            };
            let got_y = to8(ya[yy * w + x][0], n.y_off, n.y_div);
            let c = cc[(yy >> f.format.log2_chroma_h) * cw + (x >> f.format.log2_chroma_w)];
            let got_c = [to8(c[0], n.c_off, n.c_div), to8(c[1], n.c_off, n.c_div)];
            assert!(
                (got_y - ly).abs() <= tol8
                    && (got_c[0] - lc[0]).abs() <= tol8
                    && (got_c[1] - lc[1]).abs() <= tol8,
                "{file} bar {k}: Y′CbCr {got_y:.2} {:.2} {:.2}, want {ly:.2} {:.2} {:.2}",
                got_c[0],
                got_c[1],
                lc[0],
                lc[1]
            );
        }
        if !full {
            let c = &video(&p).color.interpreted;
            assert_eq!(c.primaries, Primaries::Bt709, "{file}");
            assert_eq!(c.matrix, MatrixCoefficients::Bt709, "{file}");
        }
    }
}

/// A reference decode of a still as RGBA normalized to 0..1: libjpeg-turbo's
/// `djpeg` for JPEG and libwebp's `dwebp` for WebP (the libraries Chromium
/// decodes with), else our FFmpeg build (8-bit images as `rgba`, deeper ones
/// as `rgba64le`, so no conversion rounds).
fn reference_rgba(path: &Path, deep: bool) -> (usize, usize, Vec<f64>) {
    let ext = path.extension().unwrap().to_str().unwrap();
    if ext == "jpg" || ext == "webp" {
        let out = if ext == "jpg" {
            Command::new("djpeg").args(["-pnm"]).arg(path).output()
        } else {
            Command::new("dwebp")
                .arg(path)
                .args(["-pam", "-o", "-"])
                .output()
        }
        .unwrap_or_else(|e| panic!("{ext} reference decoder: {e} (brew install jpeg-turbo webp)"));
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        return netpbm(&out.stdout);
    }
    ffmpeg_rgba(path, deep)
}

/// PGM, PPM, or PAM (8 or 16-bit samples) to RGBA in 0..1.
fn netpbm(b: &[u8]) -> (usize, usize, Vec<f64>) {
    let text = String::from_utf8_lossy(&b[..b.len().min(256)]).to_string();
    let (w, h, ch, maxval, data_at) = if text.starts_with("P7") {
        let end = text.find("ENDHDR\n").unwrap() + 7;
        let field = |k: &str| -> usize {
            text.lines().find(|l| l.starts_with(k)).unwrap()[k.len()..]
                .trim()
                .parse()
                .unwrap()
        };
        (
            field("WIDTH"),
            field("HEIGHT"),
            field("DEPTH"),
            field("MAXVAL"),
            end,
        )
    } else {
        let mut tokens = Vec::new();
        let mut i = 2;
        while tokens.len() < 3 {
            while b[i].is_ascii_whitespace() {
                i += 1;
            }
            let s = i;
            while !b[i].is_ascii_whitespace() {
                i += 1;
            }
            tokens.push(
                std::str::from_utf8(&b[s..i])
                    .unwrap()
                    .parse::<usize>()
                    .unwrap(),
            );
        }
        let ch = if &b[..2] == b"P5" { 1 } else { 3 };
        (tokens[0], tokens[1], ch, tokens[2], i + 1)
    };
    let px = &b[data_at..];
    let bytes = if maxval > 255 { 2 } else { 1 };
    let sample = |i: usize| -> f64 {
        let v = if bytes == 2 {
            u16::from_be_bytes([px[2 * i], px[2 * i + 1]]) as f64
        } else {
            px[i] as f64
        };
        v / maxval as f64
    };
    let mut out = Vec::with_capacity(w * h * 4);
    for i in 0..w * h {
        let v = |c: usize| sample(i * ch + c);
        match ch {
            1 => out.extend([v(0), v(0), v(0), 1.0]),
            3 => out.extend([v(0), v(1), v(2), 1.0]),
            _ => out.extend([v(0), v(1), v(2), v(3)]),
        }
    }
    (w, h, out)
}

fn ffmpeg_rgba(path: &Path, deep: bool) -> (usize, usize, Vec<f64>) {
    let out = Command::new(ffmpeg_bin())
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt"])
        .arg(if deep { "rgba64le" } else { "rgba" })
        .arg("-")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let probe = Command::new(ffmpeg_bin().with_file_name("ffprobe"))
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&probe.stdout);
    let mut it = s.trim().lines().next().unwrap().split(',');
    let w: usize = it.next().unwrap().parse().unwrap();
    let h: usize = it.next().unwrap().parse().unwrap();
    let px = if deep {
        out.stdout
            .chunks_exact(2)
            .map(|b| f64::from(u16::from_le_bytes([b[0], b[1]])) / 65535.0)
            .collect()
    } else {
        out.stdout.iter().map(|&b| f64::from(b) / 255.0).collect()
    };
    (w, h, px)
}

/// A still's RGBA samples normalized to 0..1 (floats as stored).
fn still_rgba(s: &image::StillImage) -> Vec<[f64; 4]> {
    let f = &s.frame;
    assert_eq!(f.format.layout, Layout::Rgba);
    let (w, h) = (f.format.width, f.format.height);
    let max16 = ((1u32 << f.format.bits) - 1) as f64;
    f.with_planes(|pl| {
        let p = &pl[0];
        let mut out = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let px = |c: usize| unsafe {
                    let row = p.ptr.add(y * p.stride);
                    match p.desc.format {
                        PlaneFormat::Rgba8 => f64::from(*row.add(x * 4 + c)) / 255.0,
                        PlaneFormat::Rgba16 => {
                            let b = row.add(x * 8 + c * 2);
                            // Codes of `bits` (12-bit JPEG keeps 0..4095).
                            f64::from(u16::from_le_bytes([*b, *b.add(1)])) / max16
                        }
                        PlaneFormat::Rgba16Float => {
                            let b = row.add(x * 8 + c * 2);
                            pm_media::frame::half_to_f64(u16::from_le_bytes([*b, *b.add(1)]))
                        }
                        PlaneFormat::Rgba32Float => {
                            let b = row.add(x * 16 + c * 4);
                            f64::from(f32::from_le_bytes([*b, *b.add(1), *b.add(2), *b.add(3)]))
                        }
                        other => panic!("{other:?}"),
                    }
                };
                out.push([px(0), px(1), px(2), px(3)]);
            }
        }
        out
    })
    .unwrap()
}

/// (file, format, bit depth, float, alpha, max |Δ| in 8-bit CV against the
/// reference decode, or None).
type StillCase = (&'static str, ImageFormat, u8, bool, bool, Option<f64>);

/// Stills at native depth. Lossless files equal our FFmpeg build's decode
/// exactly; JPEG and WebP equal libjpeg-turbo's and libwebp's decodes (what
/// today's viewer shows); float files equal the generator's values.
#[test]
#[ignore]
fn stills() {
    let dir = formats().join("image");
    let pool = FramePool::new(0);
    // (file, format, bit depth, float, alpha, max |Δ| in 8-bit CV against FFmpeg, or None)
    let table: &[StillCase] = &[
        ("png8-rgb.png", ImageFormat::Png, 8, false, false, Some(0.0)),
        ("png8-rgba.png", ImageFormat::Png, 8, false, true, Some(0.0)),
        (
            "png8-gray.png",
            ImageFormat::Png,
            8,
            false,
            false,
            Some(0.0),
        ),
        (
            "png8-palette.png",
            ImageFormat::Png,
            8,
            false,
            false,
            Some(0.0),
        ),
        (
            "png16-rgb.png",
            ImageFormat::Png,
            16,
            false,
            false,
            Some(0.0),
        ),
        (
            "png16-rgba.png",
            ImageFormat::Png,
            16,
            false,
            true,
            Some(0.0),
        ),
        (
            "tiff8-rgb.tiff",
            ImageFormat::Tiff,
            8,
            false,
            false,
            Some(0.0),
        ),
        (
            "tiff8-rgba-lzw.tiff",
            ImageFormat::Tiff,
            8,
            false,
            true,
            Some(0.0),
        ),
        (
            "tiff16-rgb.tiff",
            ImageFormat::Tiff,
            16,
            false,
            false,
            Some(0.0),
        ),
        ("bmp24.bmp", ImageFormat::Bmp, 8, false, false, Some(0.0)),
        // GIF decodes to RGBA (its palette may hold a transparent index).
        ("gif-still.gif", ImageFormat::Gif, 8, false, true, Some(0.0)),
        ("icon.ico", ImageFormat::Ico, 8, false, true, Some(0.0)),
        (
            "jpeg-444.jpg",
            ImageFormat::Jpeg,
            8,
            false,
            false,
            Some(0.0),
        ),
        (
            "jpeg-422.jpg",
            ImageFormat::Jpeg,
            8,
            false,
            false,
            Some(0.0),
        ),
        (
            "jpeg-420.jpg",
            ImageFormat::Jpeg,
            8,
            false,
            false,
            Some(0.0),
        ),
        (
            "jpeg-gray.jpg",
            ImageFormat::Jpeg,
            8,
            false,
            false,
            Some(0.0),
        ),
        (
            "jpeg-progressive.jpg",
            ImageFormat::Jpeg,
            8,
            false,
            false,
            Some(0.0),
        ),
        (
            "jpeg-12bit.jpg",
            ImageFormat::Jpeg,
            12,
            false,
            false,
            Some(0.0),
        ),
        (
            "jpeg-lossless16.jpg",
            ImageFormat::Jpeg,
            16,
            false,
            false,
            Some(0.0),
        ),
        (
            "webp-lossy.webp",
            ImageFormat::WebP,
            8,
            false,
            false,
            Some(0.0),
        ),
        (
            "webp-lossless-alpha.webp",
            ImageFormat::WebP,
            8,
            false,
            true,
            Some(0.0),
        ),
        ("avif-still.avif", ImageFormat::Avif, 8, false, false, None),
        ("avif-10bit.avif", ImageFormat::Avif, 16, false, false, None),
        ("heic.heic", ImageFormat::Heif, 8, false, false, None),
    ];
    let mut failures = vec![];
    for &(file, format, bits, float, alpha, tol) in table {
        let path = dir.join(file);
        let s = match image::read(&path, &pool) {
            Ok(s) => s,
            Err(e) => {
                failures.push(format!("{file}: {e}"));
                continue;
            }
        };
        if s.format != format || s.bit_depth != bits || s.float != float {
            failures.push(format!(
                "{file}: {:?} {}-bit float {} ({:?}), want {format:?} {bits}-bit float {float}",
                s.format, s.bit_depth, s.float, s.backend
            ));
        }
        let has_alpha = !matches!(s.color.alpha, pm_protocol::color::AlphaMode::Opaque);
        if has_alpha != alpha {
            failures.push(format!("{file}: alpha {:?}", s.color.alpha));
        }
        let Some(tol) = tol else { continue };
        let ours = still_rgba(&s);
        let (fw, fh, theirs) = reference_rgba(&path, s.bit_depth > 8);
        if (fw, fh) != (s.frame.format.width, s.frame.format.height) {
            failures.push(format!(
                "{file}: {}×{} vs FFmpeg {fw}×{fh}",
                s.frame.format.width, s.frame.format.height
            ));
            continue;
        }
        let mut max = 0f64;
        let mut sum = 0f64;
        for (i, px) in ours.iter().enumerate() {
            for c in 0..4 {
                let want = theirs[i * 4 + c];
                let mut got = px[c];
                if s.frame.format.rgba_premultiplied && c < 3 && px[3] > 0.0 {
                    got /= px[3];
                }
                let d = (got - want).abs() * 255.0;
                max = max.max(d);
                sum += d;
            }
        }
        let mean = sum / (ours.len() * 4) as f64;
        eprintln!(
            "{file}: {:?} {}-bit, max |Δ| {max:.3} CV, mean {mean:.4} CV",
            s.backend, s.bit_depth
        );
        if max > tol + 1e-9 {
            failures.push(format!(
                "{file}: max |Δ| {max:.3} CV against FFmpeg (tolerance {tol})"
            ));
        }
    }
    // Float TIFF and OpenEXR: the generator's exact values, above 1 kept.
    let s = image::read(&dir.join("tiff32f-rgb.tiff"), &pool).unwrap();
    assert!(s.float && s.bit_depth == 32);
    let px = still_rgba(&s);
    let (w, h) = (256usize, 16usize);
    for y in [0, 7, 15] {
        for x in [0, 100, 255] {
            let want = [
                2.0 * x as f64 / (w - 1) as f64,
                y as f64 / (h - 1) as f64,
                0.25,
            ];
            let got = px[y * w + x];
            for c in 0..3 {
                assert!(
                    (got[c] - f64::from(want[c] as f32)).abs() < 1e-7,
                    "tiff32f ({x},{y}) {got:?}"
                );
            }
        }
    }
    let s = image::read(&dir.join("exr-float.exr"), &pool).unwrap();
    assert!(s.float);
    let px = still_rgba(&s);
    assert!((px[255][0] - 255.0 / 128.0).abs() < 1e-6, "{:?}", px[255]);
    assert!((px[15 * 256][1] - 1.0).abs() < 1e-6);
    let s = image::read(&dir.join("exr-half-alpha.exr"), &pool).unwrap();
    assert!(s.float && s.bit_depth == 16, "{} {}", s.bit_depth, s.float);
    let px = still_rgba(&s);
    assert!((px[200][3] - 200.0 / 255.0).abs() < 1e-3, "{:?}", px[200]);
    // DPX at 10, 12, 16 bits: the generator's codes.
    for (file, bits, f) in [
        ("dpx10.dpx", 10u8, (|x: usize| x as f64) as fn(usize) -> f64),
        ("dpx12.dpx", 12, |x| (x * 4) as f64),
        ("dpx16.dpx", 16, |x| (x * 64) as f64),
    ] {
        let s = image::read(&dir.join(file), &pool).unwrap();
        assert_eq!(s.bit_depth, bits, "{file}");
        let (ya, cc) = s.frame.reference().unwrap();
        let m = ((1u64 << bits) - 1) as f64;
        let w = s.frame.format.width;
        for x in [0usize, 1, 2, 511, 1023] {
            // PlanarRgb references are [G, A] and [B, R].
            let g = f64::from(ya[3 * w + x][0]) * m;
            let r = f64::from(cc[3 * w + x][1]) * m;
            assert!(
                (g - f(x)).abs() < 0.01 && (r - f(x)).abs() < 0.01,
                "{file} x={x}: {g} {r}"
            );
        }
    }
    let s = image::read(&dir.join("dpx8.dpx"), &pool).unwrap();
    assert_eq!(s.bit_depth, 8);
    // SVG: rasterized at its intrinsic size, premultiplied; and at any size.
    let s = image::read(&dir.join("vector.svg"), &pool).unwrap();
    assert_eq!(s.format, ImageFormat::Svg);
    assert_eq!((s.frame.format.width, s.frame.format.height), (320, 180));
    let px = still_rgba(&s);
    let at = |x: usize, y: usize| px[y * 320 + x];
    // The gradient runs red to blue across the top half.
    assert!(
        at(1, 45)[0] > 0.98 && at(1, 45)[2] < 0.02,
        "{:?}",
        at(1, 45)
    );
    assert!(
        at(318, 45)[2] > 0.98 && at(318, 45)[0] < 0.02,
        "{:?}",
        at(318, 45)
    );
    // The half-transparent green circle, premultiplied.
    let c = at(80, 135);
    assert!(
        (c[1] - 0.5).abs() < 0.01 && (c[3] - 0.5).abs() < 0.01 && c[0] == 0.0,
        "{c:?}"
    );
    // Outside every shape: transparent.
    assert_eq!(at(310, 175), [0.0; 4]);
    let doc = image::svg::SvgDocument::open(&dir.join("vector.svg")).unwrap();
    let big = doc.render(1280, 720, &pool).unwrap();
    assert_eq!(big.frame.format.width, 1280);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Image sequences in every lossless format, with their gaps reported.
#[test]
#[ignore]
fn sequences_report_gaps() {
    let dir = formats().join("sequence");
    for (member, frames, gaps) in [
        ("seq_png_0001.png", 9, vec![(5, 6), (10, 10)]),
        ("seq_exr_0002.exr", 5, vec![(3, 3)]),
        ("seq_dpx_0004.dpx", 6, vec![]),
        ("seq_tiff_0006.tiff", 5, vec![(4, 4)]),
    ] {
        let s = image::sequence::detect(&dir.join(member))
            .unwrap()
            .expect(member);
        assert_eq!(s.frames.len(), frames, "{member}");
        assert_eq!(s.gaps, gaps, "{member}");
        let pool = FramePool::new(0);
        for (_, f) in &s.frames {
            image::read(f, &pool).unwrap_or_else(|e| panic!("{}: {e}", f.display()));
        }
    }
}

/// GIF, APNG, animated WebP and AVIF: frame counts, delays, and every frame
/// decoded and different from its neighbor.
#[test]
#[ignore]
fn animated_images() {
    let dir = formats().join("animated");
    let pool = FramePool::new(0);
    for file in ["anim.gif", "anim.apng", "anim.webp", "anim.avif"] {
        let path = dir.join(file);
        let a = image::animation(&path)
            .unwrap_or_else(|e| panic!("{file}: {e}"))
            .unwrap_or_else(|| panic!("{file}: not animated"));
        assert_eq!(a.frames, 10, "{file}");
        for d in &a.delays {
            assert!((d - 0.1).abs() < 0.011, "{file}: delay {d}");
        }
        let mut last = String::new();
        for i in 0..a.frames {
            let f = image::read_animation_frame(&path, i, &pool).unwrap();
            let h = f.frame.md5_hex().unwrap();
            assert_ne!(h, last, "{file}: frame {i} repeats");
            last = h;
        }
    }
}

/// Our FFmpeg build's decode of a file's first audio stream, per channel
/// (its default priming trim: `initial_padding` and edit lists).
fn ffmpeg_audio(path: &Path, channels: usize) -> Vec<Vec<f32>> {
    let out = Command::new(ffmpeg_bin())
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-map", "0:a:0", "-f", "f32le", "-acodec", "pcm_f32le", "-"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let samples: Vec<f32> = out
        .stdout
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    (0..channels)
        .map(|c| samples.iter().skip(c).step_by(channels).copied().collect())
        .collect()
}

/// Every audio format and audio in every container: rate, channel layout,
/// the program length the container states (section 2's Audio bar, after
/// priming and edit lists), and the samples of our FFmpeg build's own decode
/// (same decoders, so equal up to where each trims priming).
#[test]
#[ignore]
fn audio_formats() {
    let dir = formats().join("audio");
    let mut failures = vec![];
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    assert!(files.len() >= 34, "{} audio files", files.len());
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let mut d = match AudioDecoder::open(&path, None) {
            Ok(d) => d,
            Err(e) => {
                failures.push(format!("{name}: {e}"));
                continue;
            }
        };
        let info = d.info().clone();
        let channels = info.channels();
        let mut out = vec![Vec::new(); channels];
        loop {
            match d.read(&mut out, 8192) {
                Ok(0) => break,
                Ok(_) => {}
                Err(e) => {
                    failures.push(format!("{name}: read: {e}"));
                    break;
                }
            }
        }
        let n = out[0].len() as u64;
        let want_ch = if name.contains("5.1") {
            6
        } else if name.starts_with("wav-s16") || name.starts_with("aiff-s16") {
            1
        } else {
            2
        };
        if info.sample_rate != 48000 {
            failures.push(format!("{name}: {} Hz", info.sample_rate));
        }
        if channels != want_ch {
            failures.push(format!("{name}: {channels} channels"));
        }
        let theirs = ffmpeg_audio(&path, channels);
        // Formats that signal their exact length (PCM, lossless, gapless
        // tags, Opus and Vorbis granules, edit lists) give the 2 s exactly.
        let gapless = [
            "wav-",
            "aiff-",
            "caf-",
            "flac",
            "alac",
            "mp3-",
            "aac-ff",
            "opus",
            "vorbis",
            "weba",
            "av-prores-pcm",
            "av-h264-flac",
            "av-mpeg2-pcm",
            "av-mpeg4-pcm",
            "av-vp9-opus",
            "av-vp8-vorbis",
            "av-dv",
            "av-h263-aac",
            "eac3",
        ];
        if gapless.iter().any(|g| name.starts_with(g)) && n != 96_000 {
            failures.push(format!("{name}: {n} samples, want 96000"));
        }
        let want_n = theirs[0].len() as u64 - info.extra_priming.max(0) as u64;
        if n != want_n {
            failures.push(format!("{name}: {n} samples, FFmpeg decodes {want_n}"));
        }
        // Align on FFmpeg's start: FFmpeg's CLI trims what the demuxer
        // signals; pm-media also trims priming FFmpeg leaves (iTunSMPB, LAME
        // headers, Apple's 2112).
        let extra = info.extra_priming.max(0) as usize;
        let mut max = 0f32;
        let mut compared = 0usize;
        for c in 0..channels {
            let t = theirs[c].get(extra..).unwrap_or(&[]);
            let m = t.len().min(out[c].len());
            compared = compared.max(m);
            for k in 0..m {
                max = max.max((t[k] - out[c][k]).abs());
            }
        }
        eprintln!(
            "{name}: {} {} Hz × {channels} ({:?}), {n} samples (FFmpeg {}, container {:?}), priming {} (+{} beyond FFmpeg), max |Δ| {max:.2e} over {compared}",
            info.codec,
            info.sample_rate,
            info.layout,
            theirs[0].len(),
            info.length,
            info.priming,
            info.extra_priming
        );
        if max > 1e-6 {
            failures.push(format!("{name}: max |Δ| {max:.2e} against FFmpeg's decode"));
        }
        if (compared as u64) + 2048 < n {
            failures.push(format!("{name}: only {compared} of {n} samples compared"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Seeks in every audio format land on their sample: the block read after a
/// seek lines up with the sequential decode at shift 0, exactly for PCM and
/// lossless codecs.
#[test]
#[ignore]
fn audio_seeks() {
    let dir = formats().join("audio");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    let mut failures = vec![];
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let mut d = AudioDecoder::open(&path, None).unwrap();
        let channels = d.info().channels();
        let codec = d.info().codec.clone();
        let mut all = vec![Vec::new(); channels];
        while d.read(&mut all, 8192).unwrap() > 0 {}
        let total = all[0].len() as u64;
        let exact = codec.starts_with("pcm_") || ["flac", "alac"].contains(&codec.as_str());
        let mut worst = 0f32;
        for t in [
            total / 2,
            total / 7,
            1,
            0,
            total - 3000,
            total / 3,
            4321,
            total / 3 + 5000,
        ] {
            if t + 2048 > total {
                continue;
            }
            if let Err(e) = d.seek(t) {
                failures.push(format!("{name}: seek {t}: {e}"));
                continue;
            }
            let mut out = vec![Vec::new(); channels];
            let n = d.read(&mut out, 2048).unwrap();
            if n != 2048 {
                failures.push(format!("{name}: short read at {t}"));
                continue;
            }
            let err = |shift: i64| -> f64 {
                let start = t as i64 + shift;
                if start < 0 || start as u64 + 2048 > total {
                    return f64::MAX;
                }
                (0..n)
                    .map(|i| f64::from(out[0][i] - all[0][start as usize + i]).powi(2))
                    .sum()
            };
            let best = (-64i64..=64)
                .min_by(|a, b| err(*a).total_cmp(&err(*b)))
                .unwrap();
            if best != 0 && err(best) < err(0) {
                failures.push(format!("{name}: seek to {t} off by {best} samples"));
            }
            for c in 0..channels {
                for i in 0..n {
                    worst = worst.max((out[c][i] - all[c][t as usize + i]).abs());
                }
            }
        }
        eprintln!("{name}: seek vs sequential max |Δ| {worst:.2e}");
        if exact && worst != 0.0 {
            failures.push(format!("{name}: lossless seek differs by {worst}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Stills with ICC profiles (section 9): sRGB-equivalent profiles pass
/// through; Display P3 converts in float by matrix (`pm_color::primaries`);
/// every other profile converts through ColorSync, relative colorimetric,
/// into extended linear sRGB. Each matches ColorSync's own conversion to
/// sRGB (sips) within 1 CV wherever that conversion did not clip.
#[test]
#[ignore]
fn icc_stills() {
    use pm_media::image::icc::IccClass;
    let dir = formats().join("image");
    let pool = FramePool::new(0);
    let encode = |v: f64| pm_color::transfer::display_encode_f32(v as f32) as f64;
    for (file, class) in [
        ("icc-adobergb.png", IccClass::Other),
        ("icc-p3.png", IccClass::DisplayP3),
        ("icc-prophoto16.png", IccClass::Other),
        // Apple's "Generic Gray Gamma 2.2" curve has a linear toe and lies
        // within 1/1024 of the sRGB curve everywhere: it passes through.
        ("icc-gray22.png", IccClass::GraySrgb),
    ] {
        let s = image::read(&dir.join(file), &pool).unwrap();
        assert_eq!(s.color.icc_class, Some(class), "{file}");
        let reference = image::read(&dir.join(format!("ref-srgb-{file}")), &pool).unwrap();
        // sips writes an `sRGB` chunk rather than a profile.
        assert!(
            matches!(reference.color.icc_class, None | Some(IccClass::Srgb)),
            "{file} reference: {:?}",
            reference.color.icc_class
        );
        let want = still_rgba(&reference);
        // Display-encoded sRGB values from our conversion.
        let ours: Vec<[f64; 4]> = match class {
            IccClass::GraySrgb => still_rgba(&s),
            IccClass::DisplayP3 => {
                let m = pm_color::primaries::conversion(Primaries::DisplayP3, Primaries::Bt709)
                    .unwrap();
                still_rgba(&s)
                    .iter()
                    .map(|p| {
                        let lin = [0, 1, 2]
                            .map(|c| pm_color::transfer::display_decode_f32(p[c] as f32) as f64);
                        let l = pm_color::primaries::apply(&m, lin);
                        [encode(l[0]), encode(l[1]), encode(l[2]), p[3]]
                    })
                    .collect()
            }
            _ => {
                let c = image::imageio::convert_to_linear_srgb(&s, &pool).unwrap();
                assert!(c.float && c.color.transfer == Transfer::Linear);
                still_rgba(&c)
                    .iter()
                    .map(|p| {
                        let a = if p[3] > 0.0 { p[3] } else { 1.0 };
                        [encode(p[0] / a), encode(p[1] / a), encode(p[2] / a), p[3]]
                    })
                    .collect()
            }
        };
        let mut max = 0f64;
        let mut sum = 0f64;
        let mut compared = 0;
        let mut out_of_gamut = 0;
        for (o, r) in ours.iter().zip(&want) {
            if o[..3]
                .iter()
                .any(|v| !(-0.5 / 255.0..=1.0 + 0.5 / 255.0).contains(v))
            {
                out_of_gamut += 1;
                continue;
            }
            compared += 1;
            for c in 0..3 {
                let d = (o[c] - r[c]).abs() * 255.0;
                max = max.max(d);
                sum += d;
            }
        }
        let mean = sum / (3 * compared.max(1)) as f64;
        eprintln!(
            "{file}: {class:?}, |Δ| max {max:.3} mean {mean:.3} CV against ColorSync over {compared} pixels ({out_of_gamut} outside sRGB kept unclipped)"
        );
        assert!(compared > ours.len() / 2, "{file}");
        // The reference is ColorSync's 8-bit output (±0.5 CV). Display P3
        // converts by matrix from the standard chromaticities, ColorSync from
        // the profile's s15.16 D50 colorants: they differ by up to 1.1 CV.
        let (tol_max, tol_mean) = if class == IccClass::DisplayP3 {
            (1.5, 0.3)
        } else {
            (1.0, 0.3)
        };
        assert!(
            max <= tol_max && mean <= tol_mean,
            "{file}: max {max:.3} mean {mean:.3} CV"
        );
    }
}

/// Interpret Footage's facts: settings with their values, automatic values,
/// and origins; overrides; display size and duration with the reading.
#[test]
#[ignore]
fn interpret_footage_facts() {
    use pm_media::facts::media_facts;
    use pm_protocol::media::{FactOrigin, MediaKind};
    let dir = formats();
    let get = |f: &pm_protocol::media::MediaFacts, m: &str| {
        f.settings
            .iter()
            .find(|s| s.member == m)
            .unwrap_or_else(|| panic!("no {m}: {:?}", f.settings))
            .clone()
    };
    let facts = media_facts(&dir.join("video/meta-pasp.mov"), None).unwrap();
    assert_eq!(facts.kind, MediaKind::Video);
    let pasp = get(&facts, "pixelAspect");
    assert_eq!(pasp.value, "8:9");
    assert_eq!(
        pasp.origin,
        FactOrigin::File {
            source: "pasp".into()
        }
    );
    assert_eq!(facts.display_size, Some([640, 480]));
    let primaries = get(&facts, "primaries");
    assert!(matches!(
        primaries.origin,
        FactOrigin::Default { .. } | FactOrigin::File { .. }
    ));
    // An override: value changes, automatic stays, origin says so.
    let facts = media_facts(
        &dir.join("video/meta-pasp.mov"),
        Some(r#"{"pixelAspect":[1,1],"frameRate":24}"#),
    )
    .unwrap();
    let pasp = get(&facts, "pixelAspect");
    assert_eq!(
        (pasp.value.as_str(), pasp.automatic.as_str()),
        ("1:1", "8:9")
    );
    assert_eq!(pasp.origin, FactOrigin::Override);
    assert_eq!(facts.display_size, Some([720, 480]));
    let rate = get(&facts, "frameRate");
    assert_eq!(
        (rate.value.as_str(), rate.automatic.as_str()),
        ("24/1", "30000/1001")
    );
    assert!((facts.duration.unwrap() - 30.0 / 24.0).abs() < 1e-9);

    let facts = media_facts(&dir.join("hdr/hdr10.mov"), None).unwrap();
    let labels: Vec<&str> = facts.facts.iter().map(|l| l.label.as_str()).collect();
    for l in ["HDR", "Mastering display", "Content light", "Tone mapping"] {
        assert!(labels.contains(&l), "{l}: {labels:?}");
    }
    assert_eq!(get(&facts, "transfer").value, "pq");
    let facts = media_facts(&dir.join("video/interlaced-bff.mxf"), None).unwrap();
    assert_eq!(get(&facts, "fieldOrder").value, "bottomFirst");
    let facts = media_facts(&dir.join("video/meta-timecode.mov"), None).unwrap();
    assert!(
        facts
            .facts
            .iter()
            .any(|l| l.label == "Timecode" && l.value.contains("01:00:00;00"))
    );
    assert!(
        facts
            .facts
            .iter()
            .any(|l| l.label == "Reel" && l.value == "A001")
    );
    let facts = media_facts(
        &dir.join("video/prores-4444-alpha.mov"),
        Some(r#"{"alpha":"ignore"}"#),
    )
    .unwrap();
    let alpha = get(&facts, "alpha");
    assert_eq!(
        (alpha.value.as_str(), alpha.origin.clone()),
        ("ignore", FactOrigin::Override)
    );

    let facts = media_facts(&dir.join("image/icc-adobergb.png"), None).unwrap();
    assert_eq!(facts.kind, MediaKind::Still);
    assert!(
        matches!(get(&facts, "primaries").origin, FactOrigin::File { ref source } if source.contains("ColorSync"))
    );
    let facts = media_facts(&dir.join("sequence/seq_png_0001.png"), None).unwrap();
    assert!(
        facts
            .facts
            .iter()
            .any(|l| l.label == "Missing frames" && l.value == "5–6, 10"),
        "{:?}",
        facts.facts
    );

    let facts = media_facts(&dir.join("audio/wav-5.1.wav"), None).unwrap();
    assert_eq!(facts.kind, MediaKind::Audio);
    assert!(facts.settings.is_empty());
    assert!(
        facts
            .facts
            .iter()
            .any(|l| l.label == "Audio" && l.value.contains("5.1"))
    );

    let bad = media_facts(&dir.join("video/vp9.webm"), Some(r#"{"range":"loud"}"#)).unwrap();
    assert_eq!(bad.issues.len(), 1, "{:?}", bad.issues);
    use pm_protocol::Validate;
    assert!(bad.validate().is_ok());
}
