//! The f64 reference of a still (gate G-I1, docs/engine/gate-policy.md):
//! the decoder's samples converted by the standards with `pm_reference`
//! math only, never the engine's color code.
//!
//! - Y′CbCr (AVIF): H.273 dequantization and matrix; chroma reconstructed
//!   with Lanczos-3 at its H.273 location (left when untagged, as video).
//! - ICC.1 matrix/TRC profiles through the D50 PCS, relative colorimetric,
//!   unclipped.
//! - H.273 code points: primaries; the sRGB curve for the sRGB and BT.709
//!   transfer family (the display reading video uses); PQ and HLG per
//!   BT.2100, tone mapped with the BT.2390 EETF from the content peak to SDR
//!   white at 203 cd/m² (BT.2408), HLG through its OOTF for a 1000 cd/m²
//!   display.
//! - PNG `cHRM` (white adapted to D65 by Bradford) and `gAMA` (a pure
//!   power, PNG Third Edition); float samples are linear.
//! - Alpha: straight color is premultiplied after conversion; stored
//!   premultiplied color is divided by alpha in its stored encoding first.
//!
//! The result is premultiplied display sRGB, each channel clipped to 0…1 as
//! output stores it. Readings with no f64 model here (ColorSync conversions
//! from CMYK and other models, device CMYK, table-based or gray profiles)
//! are NaN: unreferenced.

#![allow(dead_code)]

use pm_media::frame::{DecodedFrame, Layout, PlaneFormat};
use pm_media::image::icc::IccClass;
use pm_media::image::{ImageFormat, StillImage};
use pm_protocol::color::AlphaMode;
use pm_reference::chroma::reconstruct_lanczos3;
use pm_reference::color::{
    Curve, Extent, MatrixTrc, Primaries as P, Transfer as T, apply, bradford, dequantize_chroma,
    dequantize_luma, eetf_bt2390, hlg_ootf, invert, matrix_coefficients, mul, white_xyz,
    ycbcr_to_rgb,
};
use std::path::Path;

/// A still's reference: `rgba` premultiplied display sRGB (NaN where not
/// modelled), oriented as the file says.
pub struct Reference {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<f64>,
    /// What the reference models (a ledger category).
    pub reading: String,
}

const SDR_WHITE: f64 = 203.0;

/// One sample of plane `p` at (x, y), as its code.
fn code(f: &DecodedFrame, p: usize, x: usize, y: usize, c: usize, comps: usize) -> f64 {
    f.with_planes(|pl| {
        let plane = &pl[p];
        let i = y * plane.stride;
        // SAFETY: (x, y) lies inside the plane's `height` rows of `stride`.
        unsafe {
            match plane.desc.format {
                PlaneFormat::R8 | PlaneFormat::Rgba8 => {
                    f64::from(*plane.ptr.add(i + x * comps + c))
                }
                _ => {
                    let at = plane.ptr.add(i + (x * comps + c) * 2);
                    f64::from(u16::from_le_bytes([*at, *at.add(1)]))
                }
            }
        }
    })
    .unwrap()
}

/// Straight or stored RGBA samples in 0…1 (Y′CbCr through H.273), and the
/// alpha mode they are in.
fn samples(still: &StillImage) -> Vec<[f64; 4]> {
    let f = &still.frame;
    let (w, h) = (f.format.width, f.format.height);
    match f.format.layout {
        Layout::Rgba => pm_media::image::imageio::rgba_unit(f)
            .unwrap()
            .chunks_exact(4)
            .map(|p| [p[0], p[1], p[2], p[3]].map(f64::from))
            .collect(),
        Layout::Planar | Layout::PlanarRgb => {
            let bits = u32::from(f.format.bits);
            let full = still.color.cicp.is_none_or(|c| c[3] == 1);
            let (lw, lh) = (f.format.log2_chroma_w, f.format.log2_chroma_h);
            let (cw, ch) = (w.div_ceil(1 << lw), h.div_ceil(1 << lh));
            let alpha_max = f64::from((1u32 << f.format.alpha_bits.max(1)) - 1);
            let has_alpha = f.format.planes.len() == 4;
            let (fx, fy) = (1usize << lw, 1usize << lh);
            // H.273 chroma location (left when untagged, as video).
            let origin = match (still.color.chroma_location, fy) {
                (Some(1), _) => ((fx as f64 - 1.0) / 2.0, (fy as f64 - 1.0) / 2.0),
                (Some(2), _) | (_, 1) => (0.0, 0.0),
                _ => (0.0, (fy as f64 - 1.0) / 2.0),
            };
            let rgb = f.format.layout == Layout::PlanarRgb;
            let chroma = |p: usize| -> Vec<f64> {
                let v: Vec<f64> = (0..ch)
                    .flat_map(|y| (0..cw).map(move |x| (x, y)))
                    .map(|(x, y)| {
                        let c = code(f, p, x, y, 0, 1);
                        if rgb {
                            dequantize_luma(c, bits, full)
                        } else {
                            dequantize_chroma(c, bits, full)
                        }
                    })
                    .collect();
                reconstruct_lanczos3(&v, (cw, ch), (w, h), (fx, fy), origin)
            };
            let (cb, cr) = (chroma(1), chroma(2));
            let k = matrix_coefficients(match still.color.cicp.map_or(6, |c| c[2]) {
                2 => 6,
                m => m,
            });
            (0..w * h)
                .map(|i| {
                    let (x, y) = (i % w, i / w);
                    let luma = dequantize_luma(code(f, 0, x, y, 0, 1), bits, full);
                    let c = if rgb {
                        [cr[i], luma, cb[i]]
                    } else {
                        ycbcr_to_rgb([luma, cb[i], cr[i]], k.flatten().expect("a linear matrix"))
                    };
                    let a = if has_alpha {
                        code(f, 3, x, y, 0, 1) / alpha_max
                    } else {
                        1.0
                    };
                    [c[0], c[1], c[2], a]
                })
                .collect()
        }
        other => panic!("{other:?} stills"),
    }
}

fn srgb_encode(l: f64) -> f64 {
    T::Srgb.from_linear(l)
}

/// The conversion of one straight color, in its stored encoding, to linear
/// Rec.709; None where the reference does not model the reading.
type Convert = Box<dyn Fn([f64; 3]) -> [f64; 3]>;

fn conversion(still: &StillImage) -> Option<(Convert, String)> {
    let color = &still.color;
    let to709 = |p: &P| mul(&invert(&P::BT709.to_xyz()), &p.to_xyz());
    if still.frame.format.label.contains("CMYK") {
        return None;
    }
    match color.icc_class {
        Some(IccClass::ConvertedToSrgb) => return None,
        Some(_) => {
            let icc = color.icc.as_ref()?;
            let src = MatrixTrc::parse_icc(icc).ok()?;
            let srgb_linear = MatrixTrc::from_primaries(&P::BT709, Curve::Gamma(1.0));
            return Some((
                Box::new(move |c| {
                    pm_reference::color::convert(&src, &srgb_linear, c, Extent::Unclipped)
                }),
                "ICC.1 matrix/TRC, relative colorimetric".into(),
            ));
        }
        None => {}
    }
    if let Some(cicp) = color.cicp {
        let primaries = match cicp[0] {
            1 | 2 => P::BT709,
            p => P::from_h273(p)?,
        };
        let m = to709(&primaries);
        let peak = color.mastering_peak.unwrap_or(1000.0);
        return Some(match cicp[1] {
            16 => (
                Box::new(move |c| {
                    let sdr = c.map(|e| {
                        let e = eetf_bt2390(e.clamp(0.0, 1.0), peak, SDR_WHITE);
                        T::Pq.to_linear(e) * 10_000.0 / SDR_WHITE
                    });
                    apply(&m, sdr)
                }),
                "PQ, BT.2390 EETF to 203 cd/m²".into(),
            ),
            18 => (
                Box::new(move |c| {
                    let scene = c.map(|e| T::Hlg.to_linear(e.clamp(0.0, 1.0)));
                    let display = hlg_ootf(scene, 1000.0).map(|v| v * 1000.0);
                    let sdr = display.map(|nits| {
                        let e = eetf_bt2390(T::Pq.from_linear(nits / 10_000.0), 1000.0, SDR_WHITE);
                        T::Pq.to_linear(e) * 10_000.0 / SDR_WHITE
                    });
                    apply(&m, sdr)
                }),
                "HLG, BT.2100 OOTF and BT.2390 EETF to 203 cd/m²".into(),
            ),
            1 | 2 | 6 | 13 | 14 | 15 => (
                Box::new(move |c| apply(&m, c.map(|v| T::Srgb.to_linear(v)))),
                "H.273 primaries, sRGB curve".into(),
            ),
            _ => return None,
        });
    }
    if color.chromaticities.is_some() || color.gamma.is_some() {
        let ch = color.chromaticities.map(|c| c.map(f64::from));
        let m = match ch {
            Some(c) => {
                let p = P {
                    r: [c[0], c[1]],
                    g: [c[2], c[3]],
                    b: [c[4], c[5]],
                    white: [c[6], c[7]],
                };
                let adapt = bradford(white_xyz(p.white), white_xyz(P::BT709.white));
                mul(&invert(&P::BT709.to_xyz()), &mul(&adapt, &p.to_xyz()))
            }
            None => to709(&P::BT709),
        };
        let gamma = color.gamma.map(f64::from);
        return Some((
            Box::new(move |c| {
                let l = c.map(|v| match gamma {
                    Some(g) => v.signum() * v.abs().powf(1.0 / g),
                    None => T::Srgb.to_linear(v),
                });
                apply(&m, l)
            }),
            "PNG cHRM/gAMA".into(),
        ));
    }
    if still.float {
        return Some((Box::new(|c| c), "linear float".into()));
    }
    Some((Box::new(|c| c.map(|v| T::Srgb.to_linear(v))), "sRGB".into()))
}

/// EXIF orientation of packed pixels (EXIF's definitions).
fn orient(px: &[f64], w: usize, h: usize, o: u8) -> (Vec<f64>, usize, usize) {
    let (ow, oh) = if o >= 5 { (h, w) } else { (w, h) };
    let mut out = Vec::with_capacity(px.len());
    for oy in 0..oh {
        for ox in 0..ow {
            let (sx, sy) = match o {
                2 => (w - 1 - ox, oy),
                3 => (w - 1 - ox, h - 1 - oy),
                4 => (ox, h - 1 - oy),
                5 => (oy, ox),
                6 => (oy, h - 1 - ox),
                7 => (w - 1 - oy, h - 1 - ox),
                8 => (w - 1 - oy, ox),
                _ => (ox, oy),
            };
            out.extend_from_slice(&px[(sy * w + sx) * 4..][..4]);
        }
    }
    (out, ow, oh)
}

pub fn reference(path: &Path) -> Reference {
    let pool = pm_media::pool::FramePool::new(0);
    let still =
        pm_media::image::read(path, &pool).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_ne!(still.format, ImageFormat::Svg);
    let (w, h) = (still.frame.format.width, still.frame.format.height);
    let converted = conversion(&still);
    let premultiplied = matches!(
        still.color.alpha,
        AlphaMode::Premultiplied | AlphaMode::PremultipliedWithColor { .. }
    ) || still.frame.format.rgba_premultiplied;
    let opaque = still.color.alpha == AlphaMode::Opaque;
    let mut rgba = Vec::with_capacity(w * h * 4);
    for s in samples(&still) {
        let a = if opaque { 1.0 } else { s[3] };
        let Some((f, _)) = &converted else {
            rgba.extend_from_slice(&[f64::NAN; 4]);
            continue;
        };
        let stored = [s[0], s[1], s[2]];
        let straight = if premultiplied && a > 0.0 {
            stored.map(|v| v / a)
        } else {
            stored
        };
        let e = f(straight).map(srgb_encode);
        for v in e {
            rgba.push((v * a).clamp(0.0, 1.0));
        }
        rgba.push(a.clamp(0.0, 1.0));
    }
    let (rgba, w, h) = if still.oriented {
        (rgba, w, h)
    } else {
        orient(&rgba, w, h, still.orientation)
    };
    Reference {
        width: w,
        height: h,
        rgba,
        reading: converted.map_or_else(|| "unreferenced".into(), |c| c.1),
    }
}
