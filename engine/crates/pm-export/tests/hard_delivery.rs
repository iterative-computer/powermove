//! A correct export is never refused.
//!
//! The file check fails an export only on evidence that no codec loss at
//! any supported data rate can explain (`docs/engine/delivery.md`, "The
//! file check"). Each case here encodes a correct picture with the real
//! encoder through the export job at a hard place for that rule, and the
//! export must deliver: H.264 and HEVC at the lowest rate the export
//! admits (0.1 Mb/s) and at low tiers, HD and UHD, with moving grain and
//! small contrasting text; full-frame primary and secondary slates in
//! every ProRes flavour; PQ and HLG; soft-edged alpha; very dark and flat
//! frames; one-frame exports; 12-bit ProRes 4444 XQ; and long-GOP motion
//! with cuts and a flash frame that the encoder reorders. Every case prints
//! what the check measured (`MEASURE`), which is what its allowances are
//! held against.

#![cfg(target_os = "macos")]

use std::path::PathBuf;

use pm_export::convert::WorkingFrame;
use pm_export::*;
use pm_time::Rate;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-export-hard-{name}-{}", std::process::id()));
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

type Picture = dyn Fn(u64, u32, u32) -> Vec<f32> + Sync;

/// Small contrasting text: glyph-like strokes two pixels wide in 12 × 18
/// cells, white on a dark band and red and blue on the picture, moving
/// three pixels a frame.
fn text(x: u32, y: u32, n: u64, h: u32) -> Option<[f32; 3]> {
    let line = |top: u32| (top..top + 18).contains(&y);
    let (bands, colours) = (
        [h * 2 / 3, h * 2 / 3 + 24, h / 5],
        [[0.95, 0.95, 0.95], [0.9, 0.1, 0.1], [0.15, 0.25, 0.95]],
    );
    let band = bands.iter().position(|top| line(*top))?;
    let x = x + 3 * n as u32;
    let (cell, gx, gy) = (x / 12, x % 12, y - bands[band]);
    if gx >= 10 || hash(cell, band as u32, 7) < -0.35 {
        return None;
    }
    let stroke = |a: u32, b: u32| (a..a + 2).contains(&b);
    let pick = |k: u32| hash(cell, k, 11) > 0.0;
    let on = (pick(0) && stroke(0, gx))
        || (pick(1) && stroke(8, gx))
        || (pick(2) && stroke(0, gy))
        || (pick(3) && stroke(8, gy))
        || (pick(4) && stroke(16, gy))
        || (pick(5) && gx.abs_diff(gy * 10 / 18) < 2);
    on.then_some(colours[band])
}

/// A picture with grain new in every frame, a texture panning one pixel a
/// frame, a dark band for white text, and the text.
fn grain_and_text(n: u64, w: u32, h: u32) -> Vec<f32> {
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let fy = y as f32 / h as f32;
        let dark = (h * 2 / 3..h * 2 / 3 + 18).contains(&y);
        for x in 0..w {
            let pan = (x + n as u32) % 23;
            let texture = 0.05 * (pan as f32 / 11.5 - 1.0).abs();
            let grain = 0.10 * hash(x, y, n);
            let base = if dark { 0.06 } else { 0.18 + 0.4 * fy } + texture + grain;
            let rgb = text(x, y, n, h).unwrap_or([
                base + 0.05 * (x as f32 / w as f32),
                base,
                base - 0.04 * fy,
            ]);
            out.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 1.0]);
        }
    }
    out
}

/// [`grain_and_text`] that cuts to another picture at frame 30 and flashes
/// white at frame 45.
fn cuts(n: u64, w: u32, h: u32) -> Vec<f32> {
    if n == 45 {
        return [1.0f32; 4].repeat((w * h) as usize);
    }
    let mut frame = grain_and_text(n, w, h);
    if n >= 30 {
        for p in frame.chunks_exact_mut(4) {
            let [r, g, b] = [p[0], p[1], p[2]];
            p[..3].copy_from_slice(&[1.0 - b, 0.8 - 0.6 * r, g * 0.7]);
        }
    }
    frame
}

/// One colour over the whole frame.
fn slate(rgb: [f32; 3]) -> impl Fn(u64, u32, u32) -> Vec<f32> + Sync {
    move |_, w, h| [rgb[0], rgb[1], rgb[2], 1.0].repeat((w * h) as usize)
}

/// Near black, with faint grain.
fn dark(n: u64, w: u32, h: u32) -> Vec<f32> {
    (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .flat_map(|(x, y)| {
            let v = 0.02 + 0.006 * hash(x, y, n);
            [v, v, v * 1.1, 1.0]
        })
        .collect()
}

/// A soft-edged coloured disc on nothing, moving, with grain (premultiplied).
fn soft_alpha(n: u64, w: u32, h: u32) -> Vec<f32> {
    let (cx, cy) = (w as f32 * (0.3 + 0.05 * n as f32), h as f32 * 0.5);
    let radius = h as f32 * 0.3;
    (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .flat_map(|(x, y)| {
            let far = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
            let a = (1.0 - (far - radius) / 40.0).clamp(0.0, 1.0);
            let v = 0.5 + 0.05 * hash(x, y, n);
            let rgb = [v + 0.3 * (x as f32 / w as f32), v * 0.8, 1.0 - v];
            [rgb[0] * a, rgb[1] * a, rgb[2] * a, a]
        })
        .collect()
}

/// Smooth ramps that only twelve bits carry, with very faint grain.
fn ramps(n: u64, w: u32, h: u32) -> Vec<f32> {
    (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .flat_map(|(x, y)| {
            let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
            let g = 0.0015 * hash(x, y, n);
            [fx + g, 0.3 + 0.4 * fy + g, 1.0 - fx * 0.8 + g, 1.0]
        })
        .collect()
}

/// Linear light for PQ and HLG: highlights up to six times diffuse white
/// over a dim grainy picture.
fn hdr(n: u64, w: u32, h: u32) -> Vec<f32> {
    let mut frame = grain_and_text(n, w, h);
    for (i, p) in frame.chunks_exact_mut(4).enumerate() {
        let (x, y) = ((i as u32 % w) as f32, (i as u32 / w) as f32);
        let spot = (-((x - w as f32 * 0.6 - 4.0 * n as f32).powi(2)
            + (y - h as f32 * 0.3).powi(2))
            / (h as f32 * 0.05).powi(2))
        .exp();
        for c in &mut p[..3] {
            *c = c.max(0.0).powf(2.2) * 0.8 + 6.0 * spot;
        }
    }
    frame
}

struct Case<'a> {
    name: String,
    codec: Codec,
    container: Container,
    tags: ColorTags,
    working: WorkingEncoding,
    alpha: AlphaOutput,
    size: (u32, u32),
    frames: u64,
    picture: &'a Picture,
}

fn case<'a>(
    name: &str,
    codec: Codec,
    size: (u32, u32),
    frames: u64,
    picture: &'a Picture,
) -> Case<'a> {
    let container = if matches!(codec, Codec::ProRes(_)) {
        Container::Mov
    } else {
        Container::Mp4
    };
    Case {
        name: name.into(),
        codec,
        container,
        tags: ColorTags::Rec709QuickTime,
        working: WorkingEncoding::Standard,
        alpha: AlphaOutput::None,
        size,
        frames,
        picture,
    }
}

/// Exports `c` through the job, which checks its file; `None` when it was
/// delivered.
fn deliver(c: &Case, dir: &std::path::Path) -> Option<String> {
    let (w, h) = c.size;
    let destination = dir.join(format!(
        "{}.{}",
        c.name.replace([' ', '/', '.', '·'], "-"),
        c.container.extension()
    ));
    let spec = ExportSpec {
        width: w,
        height: h,
        rate: Rate::new(25, 1).unwrap(),
        frames: c.frames,
        codec: c.codec,
        container: c.container,
        tags: c.tags,
        alpha: c.alpha,
        working: c.working,
        dither: Dither::Default,
        audio: None,
        timecode: None,
        destination: destination.clone(),
        overwrite: Overwrite::Replace,
        mode: None,
    };
    let mut job = match pm_export::export(spec) {
        Ok(job) => job,
        Err(e) => return Some(format!("{}: not started: {e}", c.name)),
    };
    for n in 0..c.frames {
        let rgba = (c.picture)(n, w, h);
        if let Err(e) = job.push_frame(&WorkingFrame {
            width: w,
            height: h,
            rgba: &rgba,
        }) {
            return Some(format!("{}: frame {n}: {e}", c.name));
        }
    }
    let outcome = match job.finish() {
        Ok(d) => {
            let p = d.report.picture;
            let opt = |v: Option<f64>| v.map_or("-".into(), |v| format!("{v:.3}"));
            eprintln!(
                "MEASURE {}: delivered {:.2} Mb/s; typical {:.3} CV, largest {:.3} (cell {:.2}), identity {}, kept {}, gain off {}, frame means {:+.2}, shared {:+.2}, registration {}; blocks: mean {:.2} CV, most foreign {:.1}; notes {:?}",
                c.name,
                d.report.bytes as f64 * 8.0 * 25.0 / c.frames as f64 / 1e6,
                p.typical_cv,
                p.frame_distance_cv,
                p.frame_cell_cv,
                opt(p.identity),
                opt(p.kept),
                opt(p.gain),
                p.frame_bias_cv,
                p.level_cv,
                opt(p.registration),
                d.report.max_pixel_error_cv(),
                d.report
                    .pixels
                    .iter()
                    .map(|b| b.foreign_cv)
                    .fold(0.0, f64::max),
                d.report.notes,
            );
            None
        }
        Err(e) => {
            eprintln!("MEASURE {}: REFUSED: {e}", c.name);
            Some(format!("{}: refused: {e}", c.name))
        }
    };
    let _ = std::fs::remove_file(&destination);
    outcome
}

fn run(name: &str, cases: Vec<Case>) {
    let dir = scratch(name);
    let failures: Vec<String> = cases.iter().filter_map(|c| deliver(c, &dir)).collect();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(failures.is_empty(), "{failures:#?}");
}

const HD: (u32, u32) = (1920, 1080);
const UHD: (u32, u32) = (3840, 2160);

fn h264(mbps: f64) -> Codec {
    Codec::H264 {
        bitrate_bps: (mbps * 1e6) as u32,
    }
}

fn hevc(mbps: f64, ten_bit: bool) -> Codec {
    Codec::Hevc {
        bitrate_bps: (mbps * 1e6) as u32,
        ten_bit,
    }
}

const PRORES: [(&str, ProResProfile); 6] = [
    ("Proxy", ProResProfile::Proxy),
    ("LT", ProResProfile::Lt),
    ("422", ProResProfile::Standard),
    ("HQ", ProResProfile::Hq),
    ("4444", ProResProfile::P4444),
    ("4444 XQ", ProResProfile::P4444Xq),
];

/// Moving grain and small text, HD and UHD, at the lowest admitted rate and
/// the low tiers.
#[test]
fn grain_and_text_at_low_rates_are_delivered() {
    let mut cases = vec![];
    for (size, label, frames) in [(HD, "HD", 12), (UHD, "UHD", 6)] {
        for mbps in [0.1, 1.0, 4.0] {
            for (codec, kind) in [
                (h264(mbps), "H.264"),
                (hevc(mbps, false), "HEVC"),
                (hevc(mbps, true), "HEVC Main10"),
            ] {
                cases.push(case(
                    &format!("{kind} {mbps} Mb/s {label} grain and text"),
                    codec,
                    size,
                    frames,
                    &grain_and_text,
                ));
            }
        }
    }
    run("grain", cases);
}

/// Long-GOP motion with a cut and a flash frame, reordered by the encoder.
#[test]
fn long_gop_cuts_and_flashes_are_delivered() {
    let mut cases = vec![];
    for mbps in [0.1, 1.0, 4.0] {
        for (codec, kind) in [
            (h264(mbps), "H.264"),
            (hevc(mbps, false), "HEVC"),
            (hevc(mbps, true), "HEVC Main10"),
        ] {
            cases.push(case(
                &format!("{kind} {mbps} Mb/s cuts"),
                codec,
                (1280, 720),
                60,
                &cuts,
            ));
        }
    }
    run("cuts", cases);
}

const SLATES: [(&str, [f32; 3]); 6] = [
    ("red", [1.0, 0.0, 0.0]),
    ("green", [0.0, 1.0, 0.0]),
    ("blue", [0.0, 0.0, 1.0]),
    ("cyan", [0.0, 1.0, 1.0]),
    ("magenta", [1.0, 0.0, 1.0]),
    ("yellow", [1.0, 1.0, 0.0]),
];

/// Full-frame saturated slates: every frame of a slate is quantised the
/// same way, so whatever the codec does to that colour is in every frame.
#[test]
fn saturated_slates_are_delivered() {
    let pictures: Vec<_> = SLATES.iter().map(|(_, rgb)| slate(*rgb)).collect();
    let mut cases = vec![];
    for ((colour, _), picture) in SLATES.iter().zip(&pictures) {
        for (flavour, profile) in PRORES {
            cases.push(case(
                &format!("ProRes {flavour} {colour} slate"),
                Codec::ProRes(profile),
                HD,
                3,
                picture,
            ));
        }
        for (codec, kind) in [
            (h264(0.1), "H.264 0.1 Mb/s"),
            (hevc(0.1, false), "HEVC 0.1 Mb/s"),
            (hevc(0.1, true), "HEVC Main10 0.1 Mb/s"),
        ] {
            cases.push(case(
                &format!("{kind} {colour} slate"),
                codec,
                HD,
                3,
                picture,
            ));
        }
    }
    run("slates", cases);
}

/// Very dark and flat frames, and one-frame exports, in every codec.
#[test]
fn dark_flat_and_single_frames_are_delivered() {
    let grey = slate([0.5, 0.5, 0.5]);
    let mut codecs: Vec<(String, Codec)> = PRORES
        .iter()
        .map(|(name, p)| (format!("ProRes {name}"), Codec::ProRes(*p)))
        .collect();
    for mbps in [0.1, 4.0] {
        codecs.push((format!("H.264 {mbps} Mb/s"), h264(mbps)));
        codecs.push((format!("HEVC {mbps} Mb/s"), hevc(mbps, false)));
        codecs.push((format!("HEVC Main10 {mbps} Mb/s"), hevc(mbps, true)));
    }
    let mut cases = vec![];
    for (name, codec) in &codecs {
        cases.push(case(&format!("{name} dark"), *codec, HD, 6, &dark));
        cases.push(case(&format!("{name} flat grey"), *codec, HD, 6, &grey));
        cases.push(case(
            &format!("{name} one frame"),
            *codec,
            HD,
            1,
            &grain_and_text,
        ));
        cases.push(case(
            &format!("{name} one dark frame"),
            *codec,
            HD,
            1,
            &dark,
        ));
    }
    run("dark", cases);
}

/// Soft-edged alpha in every format that carries it, straight and
/// premultiplied.
#[test]
fn soft_alpha_is_delivered() {
    let mut cases = vec![];
    for (name, codec, container) in [
        (
            "ProRes 4444",
            Codec::ProRes(ProResProfile::P4444),
            Container::Mov,
        ),
        (
            "ProRes 4444 XQ",
            Codec::ProRes(ProResProfile::P4444Xq),
            Container::Mov,
        ),
        ("HEVC 0.1 Mb/s", hevc(0.1, false), Container::Mov),
        ("HEVC 4 Mb/s", hevc(4.0, false), Container::Mov),
    ] {
        for alpha in [AlphaOutput::Straight, AlphaOutput::Premultiplied] {
            let mut c = case(
                &format!("{name} {alpha:?} soft alpha"),
                codec,
                HD,
                6,
                &soft_alpha,
            );
            c.container = container;
            c.alpha = alpha;
            cases.push(c);
        }
    }
    run("alpha", cases);
}

/// Twelve-bit ramps in ProRes 4444 XQ and 4444.
#[test]
fn twelve_bit_ramps_are_delivered() {
    let cases = [ProResProfile::P4444Xq, ProResProfile::P4444]
        .into_iter()
        .map(|p| {
            case(
                &format!("{p:?} 12-bit ramps"),
                Codec::ProRes(p),
                UHD,
                4,
                &ramps,
            )
        })
        .collect();
    run("ramps", cases);
}

/// PQ and HLG, in HEVC Main10 at low rates and in ProRes.
#[test]
fn pq_and_hlg_are_delivered() {
    let mut cases = vec![];
    for (label, tags) in [
        (
            "PQ",
            ColorTags::Rec2100Pq {
                white_nits: 203,
                metadata: None,
            },
        ),
        (
            "HLG",
            ColorTags::Rec2100Hlg {
                white_nits: 203,
                peak_nits: 1000,
                metadata: None,
            },
        ),
    ] {
        for (name, codec) in [
            ("HEVC Main10 0.1 Mb/s", hevc(0.1, true)),
            ("HEVC Main10 4 Mb/s", hevc(4.0, true)),
            ("ProRes 422 HQ", Codec::ProRes(ProResProfile::Hq)),
            ("ProRes Proxy", Codec::ProRes(ProResProfile::Proxy)),
            ("ProRes 4444", Codec::ProRes(ProResProfile::P4444)),
        ] {
            let mut c = case(&format!("{name} {label}"), codec, HD, 6, &hdr);
            c.tags = tags;
            c.working = WorkingEncoding::Linear;
            cases.push(c);
        }
    }
    run("hdr", cases);
}

/// A ramp under heavy noise (±0.3 on every pixel and channel, new in every
/// frame): a codec at its data-rate limit drops most of it.
fn heavy_noise(n: u64, w: u32, h: u32) -> Vec<f32> {
    (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .flat_map(|(x, y)| {
            let ramp = x as f32 / w as f32;
            let noise = |c: u32| 0.6 * hash(x, y, n * 4 + u64::from(c));
            [
                0.3 + 0.4 * ramp + noise(0),
                0.5 + noise(1),
                0.7 - 0.4 * ramp + noise(2),
                1.0,
            ]
        })
        .collect()
}

/// Heavy noise and grain with text in every ProRes flavour, and heavy noise
/// in the long-GOP codecs at the lowest rate and the low tier.
#[test]
fn heavy_noise_and_text_are_delivered_in_every_format() {
    let mut cases = vec![];
    for (flavour, profile) in PRORES {
        let codec = Codec::ProRes(profile);
        cases.push(case(
            &format!("ProRes {flavour} heavy noise"),
            codec,
            HD,
            5,
            &heavy_noise,
        ));
        cases.push(case(
            &format!("ProRes {flavour} grain and text"),
            codec,
            HD,
            5,
            &grain_and_text,
        ));
    }
    for mbps in [0.1, 4.0] {
        for (codec, kind) in [
            (h264(mbps), "H.264"),
            (hevc(mbps, false), "HEVC"),
            (hevc(mbps, true), "HEVC Main10"),
        ] {
            cases.push(case(
                &format!("{kind} {mbps} Mb/s heavy noise"),
                codec,
                HD,
                5,
                &heavy_noise,
            ));
        }
    }
    run("noise", cases);
}

/// The shift of most of the picture every frame shares, in the codec's
/// codes: what the report measured, or what a refusal names.
fn shared_shift(c: &Case, dir: &std::path::Path) -> f64 {
    let (w, h) = c.size;
    let destination = dir.join(format!("measure.{}", c.container.extension()));
    let mut job = pm_export::export(ExportSpec {
        width: w,
        height: h,
        rate: Rate::new(25, 1).unwrap(),
        frames: c.frames,
        codec: c.codec,
        container: c.container,
        tags: c.tags,
        alpha: c.alpha,
        working: c.working,
        dither: Dither::Default,
        audio: None,
        timecode: None,
        destination: destination.clone(),
        overwrite: Overwrite::Replace,
        mode: None,
    })
    .unwrap();
    for n in 0..c.frames {
        let rgba = (c.picture)(n, w, h);
        job.push_frame(&WorkingFrame {
            width: w,
            height: h,
            rgba: &rgba,
        })
        .unwrap();
    }
    let shift = match job.finish() {
        Ok(d) => d.report.picture.level_cv.abs(),
        Err(e) => e
            .to_string()
            .split(" is ")
            .skip(1)
            .filter_map(|s| s.split(' ').next()?.parse::<f64>().ok())
            .fold(0.0, |a: f64, v| a.max(v.abs())),
    };
    let _ = std::fs::remove_file(&destination);
    shift
}

/// How far quantisation moves the whole picture of every frame, by data
/// rate per pixel: what `level_tolerance` is set from.
#[test]
#[ignore = "measurement: run with --ignored --nocapture"]
fn measure_the_shared_shift_by_rate() {
    let dir = scratch("rates");
    let grey = slate([0.5, 0.5, 0.5]);
    let slates: Vec<_> = SLATES.iter().map(|(_, rgb)| slate(*rgb)).collect();
    let mut pictures: Vec<(&str, &Picture, u64)> = vec![
        ("grey", &grey, 3),
        ("dark", &dark, 3),
        ("grain and text", &grain_and_text, 6),
    ];
    for ((name, _), p) in SLATES.iter().zip(&slates) {
        pictures.push((name, p, 3));
    }
    for (size, label) in [(HD, "HD"), (UHD, "UHD")] {
        for mbps in [0.1, 0.2, 0.4, 0.8, 1.6, 3.2, 6.4] {
            if size == UHD && ![0.4, 1.6].contains(&mbps) {
                continue;
            }
            let bpp = mbps * 1e6 / (25.0 * size.0 as f64 * size.1 as f64);
            for (codec, kind) in [
                (h264(mbps), "H.264"),
                (hevc(mbps, false), "HEVC"),
                (hevc(mbps, true), "HEVC Main10"),
            ] {
                let worst = pictures
                    .iter()
                    .map(|(name, picture, frames)| {
                        let c = case(name, codec, size, *frames, *picture);
                        (shared_shift(&c, &dir), *name)
                    })
                    .fold((0.0, ""), |a, b| if b.0 > a.0 { b } else { a });
                eprintln!(
                    "MEASURE {kind} {label} {mbps} Mb/s ({bpp:.4} bits a pixel): largest shared shift {:.2} codes ({})",
                    worst.0, worst.1
                );
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
