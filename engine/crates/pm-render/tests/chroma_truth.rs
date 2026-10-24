//! Chroma reconstruction against ground truth (QUALITY-1's proof for the
//! upsampling filter, which no standard defines).
//!
//! A 4:4:4 frame (`fixtures/video/chroma-truth444.mp4`, see the README) is
//! subsampled to 4:2:0 and 4:2:2 at left (co-sited) siting by two encoders'
//! filters, and each is drawn by the engine. Every reconstruction is
//! measured against the 4:4:4 original in display codes: the engine's, and
//! the candidate filters evaluated here independently of the engine
//! (Catmull-Rom and linear at the tagged siting, and today's viewer:
//! linear, centered). The engine must be the closest, overall, on natural
//! and graphic content, and at chroma edges.

#[path = "support/doc.rs"]
mod doc;

use pm_media::decode::service::{DecodeService, MediaSource, ServiceConfig};
use pm_media::frame::{DecodedFrame, Layout};
use pm_media::vt::DecodePurpose;
use pm_protocol::color::Precision;
use pm_protocol::common::{Size, Ticks};
use pm_render::{MapResolver, RenderRequest, Renderer, VideoEnd};
use serde_json::json;
use std::path::PathBuf;

const W: usize = 256;
const H: usize = 128;
// BT.709 (H.273 matrix 1).
const KR: f64 = 0.2126;
const KB: f64 = 0.0722;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/video")
        .join(name)
}

/// 8-bit Y′, Cb, Cr codes as decoded, with the chroma plane size.
struct Codes {
    y: Vec<u8>,
    cb: Vec<u8>,
    cr: Vec<u8>,
    cw: usize,
    ch: usize,
}

fn codes(path: &PathBuf) -> Codes {
    let src = MediaSource::open(path).unwrap();
    let service = DecodeService::new(ServiceConfig::default());
    let frame: DecodedFrame = service
        .session(&src, 1, DecodePurpose::Export)
        .unwrap()
        .frame(0)
        .unwrap();
    let f = &frame.format;
    assert_eq!((f.width, f.height, f.bits), (W, H, 8), "{path:?}");
    let (cw, ch) = (W >> f.log2_chroma_w, H >> f.log2_chroma_h);
    frame
        .with_planes(|p| {
            let at = |plane: usize, x: usize, y: usize, bytes: usize, c: usize| unsafe {
                *p[plane].ptr.add(y * p[plane].stride + x * bytes + c)
            };
            let mut out = Codes {
                y: vec![0; W * H],
                cb: vec![0; cw * ch],
                cr: vec![0; cw * ch],
                cw,
                ch,
            };
            for y in 0..H {
                for x in 0..W {
                    out.y[y * W + x] = at(0, x, y, 1, 0);
                }
            }
            for y in 0..ch {
                for x in 0..cw {
                    let (b, r) = match f.layout {
                        Layout::Planar => (at(1, x, y, 1, 0), at(2, x, y, 1, 0)),
                        Layout::Biplanar => (at(1, x, y, 2, 0), at(1, x, y, 2, 1)),
                        other => panic!("{other:?}"),
                    };
                    out.cb[y * cw + x] = b;
                    out.cr[y * cw + x] = r;
                }
            }
            out
        })
        .unwrap()
}

/// E-encoded R′G′B′ in display codes from 8-bit limited BT.709 codes.
fn rgb(y: f64, cb: f64, cr: f64) -> [f64; 3] {
    let (y, u, v) = (
        (y - 16.0) / 219.0,
        (cb - 128.0) / 224.0,
        (cr - 128.0) / 224.0,
    );
    let r = y + 2.0 * (1.0 - KR) * v;
    let b = y + 2.0 * (1.0 - KB) * u;
    let g = (y - KR * r - KB * b) / (1.0 - KR - KB);
    [r, g, b].map(|c| c.clamp(0.0, 1.0) * 255.0)
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kernel {
    Linear,
    CatmullRom,
}

fn weights(kernel: Kernel, t: f64) -> [f64; 4] {
    match kernel {
        Kernel::Linear => [0.0, 1.0 - t, t, 0.0],
        Kernel::CatmullRom => [
            (-t * t * t + 2.0 * t * t - t) / 2.0,
            (3.0 * t * t * t - 5.0 * t * t + 2.0) / 2.0,
            (-3.0 * t * t * t + 4.0 * t * t + t) / 2.0,
            (t * t * t - t * t) / 2.0,
        ],
    }
}

/// A chroma plane at luma resolution: `kernel` across at origin `ox`
/// (luma pixels), linear down at the 4:2:0 row midpoint.
fn upsample(p: &[u8], c: &Codes, kernel: Kernel, ox: f64) -> Vec<f64> {
    let (fx, fy) = (W / c.cw, H / c.ch);
    let at = |i: i64, j: i64| {
        f64::from(
            p[(j.clamp(0, c.ch as i64 - 1) as usize) * c.cw + i.clamp(0, c.cw as i64 - 1) as usize],
        )
    };
    let row = |x: usize, j: i64| {
        let u = (x as f64 - ox) / fx as f64;
        let i = u.floor();
        let w = weights(kernel, u - i);
        (0..4)
            .map(|k| w[k] * at(i as i64 - 1 + k as i64, j))
            .sum::<f64>()
    };
    let oy = if fy == 2 { 0.5 } else { 0.0 };
    let mut out = Vec::with_capacity(W * H);
    for y in 0..H {
        let v = (y as f64 - oy) / fy as f64;
        let j = v.floor();
        let t = v - j;
        for x in 0..W {
            out.push(if fy == 1 {
                row(x, y as i64)
            } else {
                row(x, j as i64) * (1.0 - t) + row(x, j as i64 + 1) * t
            });
        }
    }
    out
}

/// RMS display-code error by region: natural half, graphic half, chroma
/// edges (the truth's chroma varies by more than 8 codes within ±2 px),
/// and all.
fn rms(ours: &[[f64; 3]], truth: &[[f64; 3]], edge: &[bool]) -> [f64; 4] {
    let mut acc = [(0.0, 0usize); 4];
    for (i, (a, t)) in ours.iter().zip(truth).enumerate() {
        let e: f64 = (0..3).map(|c| (a[c] - t[c]).powi(2)).sum();
        let x = i % W;
        for (k, inside) in [x < W / 2, x >= W / 2, edge[i], true]
            .into_iter()
            .enumerate()
        {
            if inside {
                acc[k].0 += e;
                acc[k].1 += 3;
            }
        }
    }
    acc.map(|(s, n)| (s / n as f64).sqrt())
}

fn engine(path: PathBuf) -> Vec<[f64; 3]> {
    let (w, h) = (W as u32, H as u32);
    let mut p = doc::project(
        w,
        h,
        1.0,
        doc::no_fill(),
        vec![doc::layer(
            "v",
            "video",
            json!({"asset":"clip","fit":"stretch","w":w,"h":h,"trim":0,"speed":1}),
            f64::from(w) / 2.0,
            f64::from(h) / 2.0,
            1.0,
        )],
    );
    doc::asset(&mut p, "clip", "video", "mp4", w, h, 1.0);
    p["colorDepth"] = json!(32);
    let mut assets = MapResolver::default();
    assets.paths.insert("clip".into(), path);
    let request = RenderRequest {
        export: true,
        video_end: VideoEnd::Final,
        ..RenderRequest::at_scale(Size::new(w, h), 1.0, Precision::Float32)
    };
    let mut renderer = Renderer::headless().unwrap();
    let frame = renderer
        .render(&doc::load(&p), "Ptest-root", Ticks::ZERO, &request, &assets)
        .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    renderer
        .read_rgba(&frame)
        .unwrap()
        .chunks_exact(4)
        .map(|p| [0, 1, 2].map(|c| f64::from(p[c]).clamp(0.0, 1.0) * 255.0))
        .collect()
}

#[test]
fn the_engine_reconstructs_chroma_closest_to_the_4_4_4_truth() {
    let truth_codes = codes(&fixture("chroma-truth444.mp4"));
    assert_eq!((truth_codes.cw, truth_codes.ch), (W, H));
    let truth: Vec<[f64; 3]> = (0..W * H)
        .map(|i| {
            rgb(
                f64::from(truth_codes.y[i]),
                f64::from(truth_codes.cb[i]),
                f64::from(truth_codes.cr[i]),
            )
        })
        .collect();
    let edge: Vec<bool> = (0..W * H)
        .map(|i| {
            let (x, y) = ((i % W) as i64, (i / W) as i64);
            (-2..=2).any(|dy: i64| {
                (-2..=2).any(|dx: i64| {
                    let j = ((y + dy).clamp(0, H as i64 - 1) as usize) * W
                        + (x + dx).clamp(0, W as i64 - 1) as usize;
                    truth_codes.cb[j].abs_diff(truth_codes.cb[i]) > 8
                        || truth_codes.cr[j].abs_diff(truth_codes.cr[i]) > 8
                })
            })
        })
        .collect();
    let mut failures = Vec::new();
    for clip in [
        "chroma-dec420.mp4",
        "chroma-sws420.mp4",
        "chroma-dec422.mp4",
        "chroma-sws422.mp4",
    ] {
        let c = codes(&fixture(clip));
        assert_eq!(c.y, truth_codes.y, "{clip}: luma is the truth's");
        let candidate = |kernel, ox| {
            let (u, v) = (
                upsample(&c.cb, &c, kernel, ox),
                upsample(&c.cr, &c, kernel, ox),
            );
            let out: Vec<[f64; 3]> = (0..W * H)
                .map(|i| rgb(f64::from(c.y[i]), u[i], v[i]))
                .collect();
            rms(&out, &truth, &edge)
        };
        // Left siting: chroma 0 on luma 0 across; today centers it.
        let centred = (W / c.cw) as f64 / 2.0 - 0.5;
        let catmull_rom = candidate(Kernel::CatmullRom, 0.0);
        let linear = candidate(Kernel::Linear, 0.0);
        let today = candidate(Kernel::Linear, centred);
        let ours = rms(&engine(fixture(clip)), &truth, &edge);
        println!(
            "CHROMA-TRUTH {}",
            json!({"clip": clip, "regions": ["natural", "graphics", "edges", "all"],
                "engine": ours, "catmullRom": catmull_rom, "linear": linear,
                "todayCenteredLinear": today})
        );
        for k in 0..4 {
            let best = catmull_rom[k].min(linear[k]).min(today[k]);
            if ours[k] > best + 0.01 {
                failures.push(format!(
                    "{clip} region {k}: engine RMS {:.3} > best {best:.3}",
                    ours[k]
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
