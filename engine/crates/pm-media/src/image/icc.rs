//! ICC profile classification. Section 9: stills bypass ColorSync when the
//! profile is equivalent to sRGB (exact pass-through); Display P3 stills
//! convert in float. Equivalence is decided from the profile's colorants and
//! tone curves, not its name.

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum IccClass {
    /// sRGB primaries (D50-adapted colorants) and the sRGB curve.
    Srgb,
    /// Display P3 primaries and the sRGB curve.
    DisplayP3,
    /// Gray with the sRGB curve.
    GraySrgb,
    /// Anything else: converted by the color pipeline.
    Other,
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn s15(b: &[u8], at: usize) -> Option<f64> {
    Some(be32(b, at)? as i32 as f64 / 65536.0)
}

fn tag<'a>(icc: &'a [u8], sig: &[u8; 4]) -> Option<&'a [u8]> {
    let n = be32(icc, 128)? as usize;
    for i in 0..n.min(1000) {
        let e = 132 + i * 12;
        if icc.get(e..e + 4)? == sig {
            let off = be32(icc, e + 4)? as usize;
            let len = be32(icc, e + 8)? as usize;
            return icc.get(off..off + len);
        }
    }
    None
}

fn xyz(icc: &[u8], sig: &[u8; 4]) -> Option<[f64; 3]> {
    let t = tag(icc, sig)?;
    if t.get(..4)? != b"XYZ " {
        return None;
    }
    Some([s15(t, 8)?, s15(t, 12)?, s15(t, 16)?])
}

/// The sRGB EOTF (IEC 61966-2-1).
fn srgb(v: f64) -> f64 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

/// Whether a `curv` or `para` tone curve is the sRGB curve (within 1/1024
/// over the whole input range).
fn is_srgb_curve(t: &[u8]) -> bool {
    let eval: Box<dyn Fn(f64) -> Option<f64>> = match t.get(..4) {
        Some(b"curv") => {
            let Some(n) = be32(t, 8) else { return false };
            let n = n as usize;
            if n < 2 {
                return false; // identity or a pure gamma: not sRGB
            }
            Box::new(move |x: f64| {
                let p = x * (n - 1) as f64;
                let i = (p.floor() as usize).min(n - 2);
                let f = p - i as f64;
                let a = u16::from_be_bytes(t.get(12 + 2 * i..14 + 2 * i)?.try_into().ok()?) as f64 / 65535.0;
                let b = u16::from_be_bytes(t.get(14 + 2 * i..16 + 2 * i)?.try_into().ok()?) as f64 / 65535.0;
                Some(a + (b - a) * f)
            })
        }
        Some(b"para") => {
            let Some(kind) = t.get(8..10).map(|k| u16::from_be_bytes([k[0], k[1]])) else { return false };
            if kind != 3 && kind != 4 {
                return false;
            }
            let Some(g) = s15(t, 12) else { return false };
            let (Some(a), Some(b), Some(c), Some(d)) = (s15(t, 16), s15(t, 20), s15(t, 24), s15(t, 28)) else { return false };
            let (e, f) = if kind == 4 { (s15(t, 32).unwrap_or(0.0), s15(t, 36).unwrap_or(0.0)) } else { (0.0, 0.0) };
            Box::new(move |x: f64| Some(if x >= d { (a * x + b).max(0.0).powf(g) + e } else { c * x + f }))
        }
        _ => return false,
    };
    (0..=256).all(|i| {
        let x = i as f64 / 256.0;
        eval(x).is_some_and(|y| (y - srgb(x)).abs() <= 1.0 / 1024.0)
    })
}

/// Classifies an ICC profile.
pub fn classify(icc: &[u8]) -> IccClass {
    if icc.len() < 132 {
        return IccClass::Other;
    }
    let space = &icc[16..20];
    let close = |a: [f64; 3], b: [f64; 3]| a.iter().zip(&b).all(|(x, y)| (x - y).abs() <= 0.002);
    if space == b"GRAY" {
        return match tag(icc, b"kTRC") {
            Some(t) if is_srgb_curve(t) => IccClass::GraySrgb,
            _ => IccClass::Other,
        };
    }
    if space != b"RGB " {
        return IccClass::Other;
    }
    let (Some(r), Some(g), Some(b)) = (xyz(icc, b"rXYZ"), xyz(icc, b"gXYZ"), xyz(icc, b"bXYZ")) else {
        return IccClass::Other;
    };
    let curves = [b"rTRC", b"gTRC", b"bTRC"].iter().all(|s| tag(icc, s).is_some_and(is_srgb_curve));
    if !curves {
        return IccClass::Other;
    }
    // D50-adapted colorants (ICC v4 Bradford adaptation from D65).
    const SRGB: [[f64; 3]; 3] = [[0.4361, 0.2225, 0.0139], [0.3851, 0.7169, 0.0971], [0.1431, 0.0606, 0.7141]];
    const P3: [[f64; 3]; 3] = [[0.5151, 0.2412, -0.0011], [0.2920, 0.6922, 0.0419], [0.1571, 0.0666, 0.7841]];
    if close(r, SRGB[0]) && close(g, SRGB[1]) && close(b, SRGB[2]) {
        IccClass::Srgb
    } else if close(r, P3[0]) && close(g, P3[1]) && close(b, P3[2]) {
        IccClass::DisplayP3
    } else {
        IccClass::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal RGB profile with the given colorants and a `para` sRGB curve.
    pub(crate) fn profile(colorants: [[f64; 3]; 3], gamma_only: bool) -> Vec<u8> {
        let fx = |v: f64| ((v * 65536.0).round() as i32).to_be_bytes();
        let mut tags: Vec<([u8; 4], Vec<u8>)> = vec![];
        for (sig, c) in [(*b"rXYZ", colorants[0]), (*b"gXYZ", colorants[1]), (*b"bXYZ", colorants[2])] {
            let mut t = b"XYZ \0\0\0\0".to_vec();
            for v in c {
                t.extend_from_slice(&fx(v));
            }
            tags.push((sig, t));
        }
        let mut para = b"para\0\0\0\0".to_vec();
        if gamma_only {
            para.extend_from_slice(&[0, 0, 0, 0]);
            para.extend_from_slice(&fx(2.2));
        } else {
            para.extend_from_slice(&[0, 3, 0, 0]);
            for v in [2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045] {
                para.extend_from_slice(&fx(v));
            }
        }
        for sig in [*b"rTRC", *b"gTRC", *b"bTRC"] {
            tags.push((sig, para.clone()));
        }
        let mut icc = vec![0u8; 128];
        icc[16..20].copy_from_slice(b"RGB ");
        icc.extend_from_slice(&(tags.len() as u32).to_be_bytes());
        let mut off = 132 + 12 * tags.len();
        let mut data = vec![];
        for (sig, t) in &tags {
            icc.extend_from_slice(sig);
            icc.extend_from_slice(&(off as u32).to_be_bytes());
            icc.extend_from_slice(&(t.len() as u32).to_be_bytes());
            data.extend_from_slice(t);
            off += t.len();
        }
        icc.extend(data);
        let len = icc.len() as u32;
        icc[..4].copy_from_slice(&len.to_be_bytes());
        icc
    }

    #[test]
    fn classifies_by_colorants_and_curves() {
        let srgb = [[0.4361, 0.2225, 0.0139], [0.3851, 0.7169, 0.0971], [0.1431, 0.0606, 0.7141]];
        let p3 = [[0.5151, 0.2412, -0.0011], [0.2920, 0.6922, 0.0419], [0.1571, 0.0666, 0.7841]];
        assert_eq!(classify(&profile(srgb, false)), IccClass::Srgb);
        assert_eq!(classify(&profile(p3, false)), IccClass::DisplayP3);
        assert_eq!(classify(&profile(srgb, true)), IccClass::Other, "gamma 2.2 is not the sRGB curve");
        assert_eq!(classify(&[0u8; 40]), IccClass::Other);
    }

    #[test]
    fn system_profiles() {
        let dir = std::path::Path::new("/System/Library/ColorSync/Profiles");
        let read = |n: &str| std::fs::read(dir.join(n)).unwrap_or_else(|e| panic!("{n}: {e}"));
        assert_eq!(classify(&read("sRGB Profile.icc")), IccClass::Srgb);
        assert_eq!(classify(&read("Display P3.icc")), IccClass::DisplayP3);
        assert_eq!(classify(&read("AdobeRGB1998.icc")), IccClass::Other);
        assert_eq!(classify(&read("ITU-709.icc")), IccClass::Other);
        assert_eq!(classify(&read("Generic CMYK Profile.icc")), IccClass::Other);
    }
}
