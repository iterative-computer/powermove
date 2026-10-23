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

/// The digits `hexDigits` leaves (the first `#` dropped, 3- and 4-digit
/// forms expanded): the first eight UTF-16 units, which are all the channels
/// read, and how many there are. Parsed on the stack: every solid and fill
/// color of every frame goes through here.
struct Digits {
    units: [u16; 8],
    len: usize,
}

impl Digits {
    /// Up to two units from `from` (`s.slice(from, from + 2)`, `from` ≤ 6).
    fn pair(&self, from: usize) -> &[u16] {
        let end = (from + 2).min(self.len);
        if from < end {
            &self.units[from..end]
        } else {
            &[]
        }
    }
}

/// `hexDigits`: drop the first `#`, expand 3- and 4-digit forms.
fn hex_digits(hex: &str) -> Digits {
    let mut raw = [0u16; 8];
    let mut n = 0usize;
    let mut dropped = false;
    for u in hex.encode_utf16() {
        if !dropped && u == u16::from(b'#') {
            dropped = true;
            continue;
        }
        if n < raw.len() {
            raw[n] = u;
        }
        n += 1;
    }
    if n == 3 || n == 4 {
        let mut units = [0u16; 8];
        for i in 0..n {
            units[2 * i] = raw[i];
            units[2 * i + 1] = raw[i];
        }
        Digits { units, len: 2 * n }
    } else {
        Digits { units: raw, len: n }
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
fn channel(s: &Digits, from: usize) -> f64 {
    match parse_int_16(s.pair(from)) {
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
    let alpha = if s.len == 8 {
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

    /// The parser before it read digits on the stack (a `Vec<u16>` copy),
    /// kept to prove the two agree.
    fn parse_hex_vec(hex: &str) -> HexColor {
        let mut s: Vec<u16> = hex.encode_utf16().collect();
        if let Some(i) = s.iter().position(|&u| u == u16::from(b'#')) {
            s.remove(i);
        }
        if s.len() == 3 || s.len() == 4 {
            s = s.iter().flat_map(|&u| [u, u]).collect();
        }
        let channel = |from: usize| {
            let end = (from + 2).min(s.len());
            let slice: &[u16] = if from < s.len() { &s[from..end] } else { &[] };
            match parse_int_16(slice) {
                Some(v) if v != 0.0 => v,
                _ => 0.0,
            }
        };
        let rgb = [0, 2, 4].map(|i| channel(i) / 255.0);
        let alpha = if s.len() == 8 {
            channel(6) / 255.0
        } else {
            1.0
        };
        HexColor { rgb, alpha }
    }

    #[test]
    fn the_stack_parser_reads_every_string_as_the_vec_parser_did() {
        let pieces = [
            "#", "0", "9", "a", "F", "g", " ", "\u{a0}", "-", "+", "x", "0x", "é", "😀", "\t",
        ];
        let mut cases: Vec<String> = vec![
            String::new(),
            "#4C8DFF".into(),
            "#3fcf8e80".into(),
            "#f80".into(),
            "#f80c".into(),
            "4C8DFF".into(),
            "a#bc".into(),
            "##123456".into(),
            "#12345678901".into(),
            "#-1-2-3".into(),
            "# 1 2 3".into(),
        ];
        // Every string of up to four pieces, and longer seeded ones.
        let n = pieces.len();
        for a in 0..n {
            for b in 0..n {
                for c in 0..n {
                    cases.push(format!("{}{}{}", pieces[a], pieces[b], pieces[c]));
                    cases.push(format!(
                        "#{}{}{}{}",
                        pieces[a],
                        pieces[b],
                        pieces[c],
                        pieces[(a + b + c) % n]
                    ));
                }
            }
        }
        let mut seed = 7u64;
        for _ in 0..20_000 {
            let mut s = String::new();
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            for k in 0..(seed >> 60) as usize + 1 {
                s.push_str(pieces[((seed >> (k * 4)) % n as u64) as usize]);
            }
            cases.push(s);
        }
        for c in &cases {
            let (a, b) = (parse_hex(c), parse_hex_vec(c));
            assert!(
                a.rgb
                    .iter()
                    .zip(&b.rgb)
                    .all(|(x, y)| x.to_bits() == y.to_bits())
                    && a.alpha.to_bits() == b.alpha.to_bits(),
                "{c:?}"
            );
        }
    }

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
