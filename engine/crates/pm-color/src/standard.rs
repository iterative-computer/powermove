//! The Standard pipeline (section 9, "The rule"; section 2, "Color
//! invariants, Standard").
//!
//! In Standard (linear blending off) intermediates are premultiplied,
//! E-encoded values with Rec.709 primaries. A hex color's code values are
//! the working values (`code / 255`), sources tagged E pass through, and an
//! output encoded with E writes the working values back as codes. This
//! module holds the hex color reading the legacy engine uses
//! (`PM.hex2rgb`, `PM.hexAlpha`) and the Standard invariants as functions
//! the tests (and the parity lane) check.

use crate::transfer::{code_max, unit_to_code};
use crate::ycbcr::{Matrix, Quantizer};
use pm_protocol::color::Range;

/// A color from a document hex string, straight alpha, in the working
/// encoding (E). Components can fall outside 0..1 only for malformed
/// strings the legacy parser reads that way (a leading `-`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HexColor {
    pub rgb: [f64; 3],
    pub alpha: f64,
}

impl HexColor {
    /// Premultiplied working RGBA with the layer opacity applied, as the
    /// legacy solid draw uniform (`u_color = rgb · a, a = opacity · α`).
    pub fn premultiplied(&self, opacity: f64) -> [f64; 4] {
        let a = opacity * self.alpha;
        [self.rgb[0] * a, self.rgb[1] * a, self.rgb[2] * a, a]
    }
}

/// `hexDigits`: drop the first `#`, expand 3- and 4-digit forms.
fn hex_digits(hex: &str) -> Vec<u16> {
    let units: Vec<u16> = hex.encode_utf16().collect();
    let mut s = units;
    if let Some(i) = s.iter().position(|&u| u == u16::from(b'#')) {
        s.remove(i);
    }
    if s.len() == 3 || s.len() == 4 {
        s.iter().flat_map(|&u| [u, u]).collect()
    } else {
        s
    }
}

fn is_js_whitespace(u: u16) -> bool {
    matches!(
        u,
        0x09..=0x0D | 0x20 | 0xA0 | 0x1680 | 0x2000..=0x200A | 0x2028 | 0x2029 | 0x202F | 0x205F
            | 0x3000 | 0xFEFF
    )
}

/// JavaScript `parseInt(s, 16)` over UTF-16 units; `None` is NaN.
fn parse_int_16(s: &[u16]) -> Option<f64> {
    let mut i = 0;
    while i < s.len() && is_js_whitespace(s[i]) {
        i += 1;
    }
    let mut sign = 1.0;
    if i < s.len() && (s[i] == u16::from(b'+') || s[i] == u16::from(b'-')) {
        if s[i] == u16::from(b'-') {
            sign = -1.0;
        }
        i += 1;
    }
    if i + 1 < s.len() && s[i] == u16::from(b'0') && (s[i + 1] | 0x20) == u16::from(b'x') {
        i += 2;
    }
    let mut value = 0.0f64;
    let mut any = false;
    while i < s.len() {
        let d = match s[i] {
            u @ 0x30..=0x39 => u - 0x30,
            u @ 0x41..=0x46 => u - 0x41 + 10,
            u @ 0x61..=0x66 => u - 0x61 + 10,
            _ => break,
        };
        value = value * 16.0 + f64::from(d);
        any = true;
        i += 1;
    }
    any.then_some(sign * value)
}

/// `(parseInt(slice, 16) || 0)`: NaN and −0 become 0.
fn channel(s: &[u16], from: usize) -> f64 {
    let end = (from + 2).min(s.len());
    let slice = if from < s.len() { &s[from..end] } else { &[] };
    match parse_int_16(slice) {
        Some(v) if v != 0.0 => v,
        _ => 0.0,
    }
}

/// `PM.hex2rgb` and `PM.hexAlpha`: `#RGB`, `#RGBA`, `#RRGGBB`, `#RRGGBBAA`,
/// channels read by position, unreadable channels 0, alpha 1 unless eight
/// digits are present.
pub fn parse_hex(hex: &str) -> HexColor {
    let s = hex_digits(hex);
    let rgb = [0, 2, 4].map(|i| channel(&s, i) / 255.0);
    let alpha = if s.len() == 8 {
        channel(&s, 6) / 255.0
    } else {
        1.0
    };
    HexColor { rgb, alpha }
}

/// Full-range RGB output codes of an E-encoded working color (every
/// Standard RGB output, PNG, TIFF, viewer): clipped to 0..1 and rounded
/// half up. Code values of a hex color come back exactly at 8 bits.
pub fn rgb_codes(working_rgb: [f64; 3], bits: u8) -> [u32; 3] {
    working_rgb.map(|v| unit_to_code(v, bits))
}

/// The ideal n-bit code of an 8-bit code value (`code / 255` scaled to
/// 2ⁿ − 1 and rounded half up): what a 10-bit or 16-bit output of a hex
/// color should contain.
pub fn ideal_code(code8: u8, bits: u8) -> u32 {
    let v = f64::from(code8) * code_max(bits) as f64 / 255.0;
    libm::floor(v + 0.5) as u32
}

/// Y′CbCr output codes of an E-encoded working color: the Rec.709 matrix
/// (or another), quantized, rounded half to even as the encoder-input pass
/// does (WGSL `round`), and clipped to the codable range.
pub fn ycbcr_codes(working_rgb: [f64; 3], matrix: Matrix, quantizer: Quantizer) -> [u32; 3] {
    let ycc = matrix.rgb_to_ycbcr(working_rgb);
    [
        quantizer.luma_code(ycc[0]),
        quantizer.chroma_code(ycc[1]),
        quantizer.chroma_code(ycc[2]),
    ]
}

/// A reference decode of Y′CbCr codes back to full-range 8-bit RGB codes
/// (what a player shows for the export).
pub fn decode_ycbcr_to_rgb8(codes: [u32; 3], matrix: Matrix, quantizer: Quantizer) -> [u32; 3] {
    let y = quantizer.luma_value(codes[0]);
    let cb = quantizer.chroma_value(codes[1]);
    let cr = quantizer.chroma_value(codes[2]);
    matrix.ycbcr_to_rgb([y, cb, cr]).map(|v| unit_to_code(v, 8))
}

/// Section 2 Standard invariant (2) for one hex color and one output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolidOutput {
    /// sRGB PNG, 8-bit: exact.
    SrgbPng8,
    /// Full-range RGB at 10 bits and above: within 1 of the ideal code.
    FullRangeRgb(u8),
    /// 8-bit limited-range Y′CbCr (Rec.709): within 2 after a reference
    /// decode, greys exactly neutral.
    LimitedYcbcr8,
}

/// The largest code difference invariant (2) allows for `output`.
pub const fn solid_tolerance(output: SolidOutput) -> u32 {
    match output {
        SolidOutput::SrgbPng8 => 0,
        SolidOutput::FullRangeRgb(_) => 1,
        SolidOutput::LimitedYcbcr8 => 2,
    }
}

/// Checks Standard invariant (2) for a hex color: the codes the output
/// decodes to against the hex codes, and grey neutrality for Y′CbCr.
/// Returns the largest difference seen, or an error describing a failure.
pub fn check_solid(rgb8: [u8; 3], output: SolidOutput) -> Result<u32, String> {
    let working = rgb8.map(|c| f64::from(c) / 255.0);
    let tolerance = solid_tolerance(output);
    let diff = |a: u32, b: u32| a.abs_diff(b);
    let worst = match output {
        SolidOutput::SrgbPng8 => {
            let codes = rgb_codes(working, 8);
            (0..3)
                .map(|i| diff(codes[i], u32::from(rgb8[i])))
                .max()
                .unwrap()
        }
        SolidOutput::FullRangeRgb(bits) => {
            let codes = rgb_codes(working, bits);
            (0..3)
                .map(|i| diff(codes[i], ideal_code(rgb8[i], bits)))
                .max()
                .unwrap()
        }
        SolidOutput::LimitedYcbcr8 => {
            let q = Quantizer::new(8, Range::Limited);
            let codes = ycbcr_codes(working, Matrix::BT709, q);
            if rgb8[0] == rgb8[1] && rgb8[1] == rgb8[2] && (codes[1] != 128 || codes[2] != 128) {
                return Err(format!("grey {rgb8:?} is not neutral: {codes:?}"));
            }
            let back = decode_ycbcr_to_rgb8(codes, Matrix::BT709, q);
            (0..3)
                .map(|i| diff(back[i], u32::from(rgb8[i])))
                .max()
                .unwrap()
        }
    };
    if worst > tolerance {
        Err(format!(
            "{rgb8:?} through {output:?} differs by {worst} (allowed {tolerance})"
        ))
    } else {
        Ok(worst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_reading_matches_the_legacy_parser() {
        let c = parse_hex("#4C8DFF");
        assert_eq!(c.rgb, [76.0 / 255.0, 141.0 / 255.0, 1.0]);
        assert_eq!(c.alpha, 1.0);
        let c = parse_hex("#3fcf8e80");
        assert_eq!(c.rgb, [63.0 / 255.0, 207.0 / 255.0, 142.0 / 255.0]);
        assert_eq!(c.alpha, 128.0 / 255.0);
        // Short forms expand per digit.
        assert_eq!(parse_hex("#f80").rgb, parse_hex("#ff8800").rgb);
        assert_eq!(parse_hex("#f808").alpha, 136.0 / 255.0);
        // Unreadable channels are 0; parseInt stops at the first non-digit.
        assert_eq!(parse_hex("#zz1g00").rgb, [0.0, 1.0 / 255.0, 0.0]);
        assert_eq!(parse_hex("").rgb, [0.0; 3]);
        assert_eq!(parse_hex("garbage").alpha, 1.0);
        // Only the first '#' is removed; a missing '#' is fine.
        assert_eq!(parse_hex("ff0000").rgb, [1.0, 0.0, 0.0]);
        // parseInt accepts a sign.
        assert_eq!(parse_hex("#-1ff00").rgb[0], -1.0 / 255.0);
    }

    #[test]
    fn premultiplied_solid_matches_the_legacy_uniform() {
        let c = parse_hex("#FF000080");
        let p = c.premultiplied(0.5);
        let a = 0.5 * 128.0 / 255.0;
        assert_eq!(p, [a, 0.0, 0.0, a]);
    }

    #[test]
    fn invariant_2_srgb_png_is_exact_for_every_code() {
        for c in 0..=255u8 {
            assert_eq!(
                check_solid([c, 255 - c, c / 3], SolidOutput::SrgbPng8),
                Ok(0)
            );
        }
    }

    #[test]
    fn invariant_2_high_depth_rgb_within_one() {
        for bits in [10, 12, 16] {
            for c in 0..=255u8 {
                let worst =
                    check_solid([c, c / 2, 255 - c], SolidOutput::FullRangeRgb(bits)).unwrap();
                assert!(worst <= 1);
            }
        }
    }

    #[test]
    fn invariant_2_limited_ycbcr_within_two_and_greys_neutral() {
        for g in 0..=255u8 {
            check_solid([g, g, g], SolidOutput::LimitedYcbcr8).unwrap();
        }
        let mut worst = 0;
        for r in (0..=255u8).step_by(5) {
            for g in (0..=255u8).step_by(5) {
                for b in (0..=255u8).step_by(5) {
                    worst = worst.max(check_solid([r, g, b], SolidOutput::LimitedYcbcr8).unwrap());
                }
            }
        }
        assert!(worst <= 2, "{worst}");
    }
}
