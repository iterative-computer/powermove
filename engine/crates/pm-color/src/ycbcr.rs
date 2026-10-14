//! Y′CbCr matrices and quantization (sections 9 and 13).
//!
//! Y′ is 0..1 and Cb, Cr are −0.5..0.5 between the matrix and the codes.
//! Codes follow section 13's normalization (contract 4 `normalization`);
//! encoding rounds half to even, as the encoder-input shader's WGSL `round`
//! does, and clips to the codable range: clipping happens only at output, so
//! superwhites and sub-blacks survive everything before it.

use pm_protocol::color::{
    MatrixCoefficients, Range, SampleAlignment, normalization, ycbcr_codable_range,
};

/// Luma coefficients of a Y′CbCr matrix.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Matrix {
    pub kr: f64,
    pub kb: f64,
}

impl Matrix {
    pub const BT709: Matrix = Matrix {
        kr: 0.2126,
        kb: 0.0722,
    };
    pub const BT601: Matrix = Matrix {
        kr: 0.299,
        kb: 0.114,
    };
    pub const BT2020_NCL: Matrix = Matrix {
        kr: 0.2627,
        kb: 0.0593,
    };

    /// The matrix for a coefficient tag; `None` for RGB (identity).
    pub fn of(coefficients: MatrixCoefficients) -> Option<Matrix> {
        coefficients.kr_kb().map(|(kr, kb)| Matrix { kr, kb })
    }

    pub fn kg(&self) -> f64 {
        1.0 - self.kr - self.kb
    }

    /// R′G′B′ to Y′CbCr (Y′ 0..1, C −0.5..0.5), unclipped.
    pub fn rgb_to_ycbcr(&self, rgb: [f64; 3]) -> [f64; 3] {
        let y = self.kr * rgb[0] + self.kg() * rgb[1] + self.kb * rgb[2];
        let cb = (rgb[2] - y) / (2.0 * (1.0 - self.kb));
        let cr = (rgb[0] - y) / (2.0 * (1.0 - self.kr));
        [y, cb, cr]
    }

    /// Y′CbCr to R′G′B′, unclipped.
    pub fn ycbcr_to_rgb(&self, ycc: [f64; 3]) -> [f64; 3] {
        let [y, cb, cr] = ycc;
        let r = y + 2.0 * (1.0 - self.kr) * cr;
        let b = y + 2.0 * (1.0 - self.kb) * cb;
        let g = (y - self.kr * r - self.kb * b) / self.kg();
        [r, g, b]
    }

    /// The Y′CbCr → R′G′B′ matrix as WGSL `mat3x3<f32>` columns (column j is
    /// the contribution of Y′, Cb, Cr respectively).
    pub fn to_rgb_columns(&self) -> [[f32; 3]; 3] {
        let (kr, kb, kg) = (self.kr, self.kb, self.kg());
        let cr_r = 2.0 * (1.0 - kr);
        let cb_b = 2.0 * (1.0 - kb);
        let cb_g = -kb * cb_b / kg;
        let cr_g = -kr * cr_r / kg;
        [
            [1.0, 1.0, 1.0],
            [0.0, cb_g as f32, cb_b as f32],
            [cr_r as f32, cr_g as f32, 0.0],
        ]
    }

    /// The R′G′B′ → Y′CbCr matrix as WGSL columns (column j is the
    /// contribution of R′, G′, B′).
    pub fn to_ycbcr_columns(&self) -> [[f32; 3]; 3] {
        let (kr, kb, kg) = (self.kr, self.kb, self.kg());
        let sb = 1.0 / (2.0 * (1.0 - kb));
        let sr = 1.0 / (2.0 * (1.0 - kr));
        [
            [kr as f32, (-kr * sb) as f32, ((1.0 - kr) * sr) as f32],
            [kg as f32, (-kg * sb) as f32, (-kg * sr) as f32],
            [kb as f32, ((1.0 - kb) * sb) as f32, (-kb * sr) as f32],
        ]
    }
}

/// Code values of one Y′CbCr output or source: bit depth and range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quantizer {
    pub bits: u8,
    pub range: Range,
}

impl Quantizer {
    pub const fn new(bits: u8, range: Range) -> Quantizer {
        Quantizer { bits, range }
    }

    fn norm(&self) -> pm_protocol::color::Normalization {
        normalization(self.bits, SampleAlignment::Lsb, self.range)
    }

    /// The inclusive codable range Y′CbCr samples are clipped to (section
    /// 9): 8-bit 1..254, 10-bit 4..1019, 12-bit 16..4079; other depths the
    /// full code range.
    pub fn codable(&self) -> (u32, u32) {
        ycbcr_codable_range(self.bits).unwrap_or((0, crate::transfer::code_max(self.bits)))
    }

    /// The unrounded luma code of Y′.
    pub fn luma_exact(&self, y: f64) -> f64 {
        let n = self.norm();
        n.luma_offset + n.luma_scale * y
    }

    /// The unrounded chroma code of Cb or Cr.
    pub fn chroma_exact(&self, c: f64) -> f64 {
        let n = self.norm();
        n.chroma_offset + n.chroma_scale * c
    }

    /// Rounds half to even and clips to the codable range.
    pub fn round_clip(&self, code: f64) -> u32 {
        let (lo, hi) = self.codable();
        if code.is_nan() {
            return lo;
        }
        (code.round_ties_even()).clamp(f64::from(lo), f64::from(hi)) as u32
    }

    pub fn luma_code(&self, y: f64) -> u32 {
        self.round_clip(self.luma_exact(y))
    }

    pub fn chroma_code(&self, c: f64) -> u32 {
        self.round_clip(self.chroma_exact(c))
    }

    /// Y′ of a luma code.
    pub fn luma_value(&self, code: u32) -> f64 {
        self.norm().luma(f64::from(code))
    }

    /// Cb or Cr of a chroma code.
    pub fn chroma_value(&self, code: u32) -> f64 {
        self.norm().chroma(f64::from(code))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrices_invert() {
        for m in [Matrix::BT709, Matrix::BT601, Matrix::BT2020_NCL] {
            for rgb in [
                [0.0, 0.0, 0.0],
                [1.0, 1.0, 1.0],
                [1.0, 0.0, 0.0],
                [0.2, 0.7, 0.4],
                [1.2, -0.1, 0.5],
            ] {
                let back = m.ycbcr_to_rgb(m.rgb_to_ycbcr(rgb));
                for i in 0..3 {
                    assert!((back[i] - rgb[i]).abs() < 1e-14, "{m:?} {rgb:?} {back:?}");
                }
            }
        }
    }

    #[test]
    fn column_matrices_match_the_f64_functions() {
        let m = Matrix::BT709;
        let rgb = [0.3f64, 0.6, 0.9];
        let ycc = m.rgb_to_ycbcr(rgb);
        let c = m.to_ycbcr_columns();
        for row in 0..3 {
            let v: f64 = (0..3).map(|j| f64::from(c[j][row]) * rgb[j]).sum();
            assert!((v - ycc[row]).abs() < 1e-6);
        }
        let c = m.to_rgb_columns();
        for row in 0..3 {
            let v: f64 = (0..3).map(|j| f64::from(c[j][row]) * ycc[j]).sum();
            assert!((v - rgb[row]).abs() < 1e-6);
        }
    }

    #[test]
    fn greys_have_zero_chroma() {
        for g in 0..=255 {
            let v = f64::from(g) / 255.0;
            let ycc = Matrix::BT709.rgb_to_ycbcr([v, v, v]);
            assert!(ycc[1].abs() < 1e-15 && ycc[2].abs() < 1e-15);
            let q = Quantizer::new(8, Range::Limited);
            assert_eq!(q.chroma_code(ycc[1]), 128);
        }
    }

    #[test]
    fn codable_range_clips_only_at_output() {
        let q = Quantizer::new(10, Range::Limited);
        assert_eq!(q.codable(), (4, 1019));
        assert_eq!(q.luma_code(1.2), 1019);
        assert_eq!(q.luma_code(-0.2), 4);
        assert_eq!(q.luma_code(0.0), 64);
        assert_eq!(q.luma_code(1.0), 940);
        assert_eq!(q.chroma_code(0.5), 960);
        assert_eq!(Quantizer::new(8, Range::Limited).codable(), (1, 254));
        assert_eq!(Quantizer::new(12, Range::Limited).codable(), (16, 4079));
        assert_eq!(Quantizer::new(16, Range::Full).codable(), (0, 65535));
        // Superwhite survives until the code is written: Y′ above 1 maps above 940.
        assert!(q.luma_exact(1.05) > 940.0);
    }

    #[test]
    fn rounding_is_half_to_even() {
        let q = Quantizer::new(8, Range::Full);
        assert_eq!(q.round_clip(126.5), 126);
        assert_eq!(q.round_clip(127.5), 128);
    }
}
