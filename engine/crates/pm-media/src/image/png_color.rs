//! PNG color chunks with the PNG Third Edition precedence (W3C PNG 3rd
//! edition, §4.3 and §11.3.2): `cICP` over `iCCP`, `iCCP` over `sRGB`, and
//! `sRGB` over `cHRM` and `gAMA`. Only chunks before the first `IDAT`
//! count.
//!
//! Without a higher-precedence chunk, `gAMA` alone is the transfer: stored
//! samples are linear light raised to the file gamma `g`, so decoding raises
//! them to `1 / g` (a pure power, no sRGB snapping), and `cHRM` alone gives
//! the primaries and white point. A `cHRM` whose chromaticities do not
//! define primaries (a zero `y`, or a singular matrix) is ignored, as a
//! malformed ancillary chunk is; its `gAMA` still applies.

use super::ImageColor;
use crate::error::Result;
use std::io::Read;
use std::path::Path;

/// The color chunks of a PNG before `IDAT`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PngChunks {
    pub cicp: Option<[u8; 4]>,
    pub iccp: bool,
    pub srgb: bool,
    pub chrm: Option<[f32; 8]>,
    pub gamma: Option<f32>,
    /// `mDCV` maximum luminance, cd/m².
    pub mastering_peak: Option<f64>,
}

impl PngChunks {
    pub fn read(path: &Path) -> Result<Option<PngChunks>> {
        let mut file = std::io::BufReader::new(std::fs::File::open(path)?);
        let mut signature = [0; 8];
        if file.read_exact(&mut signature).is_err() || signature != *b"\x89PNG\r\n\x1a\n" {
            return Ok(None);
        }
        let mut c = PngChunks::default();
        loop {
            let mut header = [0; 8];
            if file.read_exact(&mut header).is_err() {
                break;
            }
            let len = u32::from_be_bytes(header[..4].try_into().unwrap());
            let ty: [u8; 4] = header[4..].try_into().unwrap();
            let body = match &ty {
                b"IDAT" | b"IEND" => break,
                b"cICP" | b"cHRM" | b"gAMA" | b"mDCV" | b"mDCv" if len <= 32 => {
                    let mut b = vec![0; len as usize];
                    file.read_exact(&mut b)?;
                    file.seek_relative(4)?;
                    b
                }
                _ => {
                    match &ty {
                        b"iCCP" => c.iccp = true,
                        b"sRGB" => c.srgb = true,
                        _ => {}
                    }
                    file.seek_relative(i64::from(len) + 4)?;
                    continue;
                }
            };
            let u32_at = |i: usize| u32::from_be_bytes(body[i..i + 4].try_into().unwrap());
            match (&ty, body.len()) {
                (b"cICP", 4) => c.cicp = Some(body[..4].try_into().unwrap()),
                (b"gAMA", 4) if u32_at(0) > 0 => c.gamma = Some(u32_at(0) as f32 / 100_000.0),
                (b"cHRM", 32) => {
                    // White, red, green, blue in the chunk; red, green,
                    // blue, white here.
                    let v: Vec<f32> = (0..8).map(|i| u32_at(i * 4) as f32 / 100_000.0).collect();
                    let ch = [v[2], v[3], v[4], v[5], v[6], v[7], v[0], v[1]];
                    if valid_chromaticities(&ch) {
                        c.chrm = Some(ch);
                    }
                }
                (b"mDCV" | b"mDCv", 24) => {
                    let peak = f64::from(u32_at(16)) * 0.0001;
                    c.mastering_peak = (peak > 0.0).then_some(peak);
                }
                _ => {}
            }
        }
        Ok(Some(c))
    }

    /// Applies the chunks that take precedence to a still's color facts
    /// (`icc` holds the decoder's `iCCP` profile).
    pub fn apply(&self, color: ImageColor) -> ImageColor {
        let mut color = ImageColor {
            mastering_peak: self.mastering_peak,
            ..color
        };
        if let Some(cicp) = self.cicp {
            color = ImageColor {
                cicp: Some(cicp),
                ..color.with_icc(None)
            };
        } else if !self.iccp && !self.srgb {
            color.chromaticities = self.chrm;
            color.gamma = self.gamma;
        }
        color
    }
}

/// Chromaticities that define primaries: every `y` positive and the
/// primaries linearly independent.
fn valid_chromaticities(c: &[f32; 8]) -> bool {
    if c.iter().any(|v| !v.is_finite()) || [c[1], c[3], c[5], c[7]].iter().any(|&y| y <= 0.0) {
        return false;
    }
    let col = |x: f32, y: f32| [f64::from(x / y), 1.0, f64::from((1.0 - x - y) / y)];
    let (r, g, b) = (col(c[0], c[1]), col(c[2], c[3]), col(c[4], c[5]));
    let det = r[0] * (g[1] * b[2] - g[2] * b[1]) - g[0] * (r[1] * b[2] - r[2] * b[1])
        + b[0] * (r[1] * g[2] - r[2] * g[1]);
    det.abs() > 1e-9
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_protocol::color::{AlphaMode, Transfer};

    fn chunk(ty: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut c = (body.len() as u32).to_be_bytes().to_vec();
        c.extend_from_slice(ty);
        c.extend_from_slice(body);
        c.extend_from_slice(&[0; 4]); // The CRC is not read.
        c
    }

    fn png(chunks: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
        v.extend(chunk(b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0]));
        for (ty, body) in chunks {
            v.extend(chunk(ty, body));
        }
        v.extend(chunk(b"IEND", &[]));
        v
    }

    fn chrm(white: [u32; 2], prim: [u32; 6]) -> Vec<u8> {
        white
            .iter()
            .chain(&prim)
            .flat_map(|v| v.to_be_bytes())
            .collect()
    }

    #[test]
    fn third_edition_precedence() {
        let dir = tempfile::Builder::new()
            .prefix("pm-png-color-")
            .tempdir()
            .unwrap();
        let g = |v: u32| v.to_be_bytes();
        let p3 = chrm([31270, 32900], [68000, 32000, 26500, 69000, 15000, 6000]);
        let zero = vec![0; 32];
        let profile = Some(vec![1, 2, 3]);
        type Chunks<'a> = &'a [(&'a [u8; 4], &'a [u8])];
        // (chunks, the decoder's iCCP profile, cicp, chromaticities, gamma, profile kept)
        let cases: [(Chunks, bool, Option<[u8; 4]>, bool, Option<f32>, bool); 9] = [
            (&[], false, None, false, None, false),
            (
                &[(b"gAMA", &g(70_000))],
                false,
                None,
                false,
                Some(0.7),
                false,
            ),
            (&[(b"cHRM", &p3)], false, None, true, None, false),
            (
                &[(b"cHRM", &p3), (b"gAMA", &g(70_000))],
                false,
                None,
                true,
                Some(0.7),
                false,
            ),
            // A malformed cHRM is ignored; its gAMA still applies.
            (
                &[(b"cHRM", &zero), (b"gAMA", &g(70_000))],
                false,
                None,
                false,
                Some(0.7),
                false,
            ),
            (
                &[(b"sRGB", &[0]), (b"gAMA", &g(45_455)), (b"cHRM", &p3)],
                false,
                None,
                false,
                None,
                false,
            ),
            (
                &[(b"iCCP", b"x\0\0"), (b"gAMA", &g(70_000))],
                true,
                None,
                false,
                None,
                true,
            ),
            (
                &[(b"iCCP", b"x\0\0"), (b"cICP", &[9, 16, 0, 1])],
                true,
                Some([9, 16, 0, 1]),
                false,
                None,
                false,
            ),
            // After IDAT: ignored.
            (
                &[(b"IDAT", &[]), (b"gAMA", &g(70_000))],
                false,
                None,
                false,
                None,
                false,
            ),
        ];
        for (i, (chunks, iccp, cicp, chromaticities, gamma, kept)) in cases.into_iter().enumerate()
        {
            let path = dir.path().join(format!("{i}.png"));
            std::fs::write(&path, png(chunks)).unwrap();
            let c = PngChunks::read(&path).unwrap().unwrap();
            let base = ImageColor::plain(Transfer::DisplayE, AlphaMode::Straight)
                .with_icc(if iccp { profile.clone() } else { None });
            let color = c.apply(base);
            assert_eq!(color.cicp, cicp, "case {i}");
            assert_eq!(color.chromaticities.is_some(), chromaticities, "case {i}");
            assert_eq!(color.gamma, gamma, "case {i}");
            assert_eq!(color.icc.is_some(), kept, "case {i}");
        }
        std::fs::write(dir.path().join("not.png"), b"GIF89a").unwrap();
        assert_eq!(PngChunks::read(&dir.path().join("not.png")).unwrap(), None);
    }

    #[test]
    fn mastering_display_peak() {
        let dir = tempfile::Builder::new()
            .prefix("pm-png-mdcv-")
            .tempdir()
            .unwrap();
        let mut body = vec![0u8; 16];
        body.extend(10_000_000u32.to_be_bytes());
        body.extend(50u32.to_be_bytes());
        let path = dir.path().join("m.png");
        std::fs::write(&path, png(&[(b"mDCV", &body)])).unwrap();
        let c = PngChunks::read(&path).unwrap().unwrap();
        assert_eq!(c.mastering_peak, Some(1000.0));
    }
}
