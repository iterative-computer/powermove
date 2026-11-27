//! SVG filter primitives in f64, from W3C Filter Effects Module Level 1
//! (§15 "Filter primitives", §17 `color-interpolation-filters`).
//!
//! An [`Image`] is a grid of premultiplied pixels, each constant over its
//! square (the box reconstruction of [`crate::gaussian`]), transparent
//! outside. Primitives work on premultiplied colour in their colour space:
//! [`into_space`] converts sRGB-encoded storage to linearRGB through
//! IEC 61966-2-1 on unpremultiplied colour, [`from_space`] converts back.
//! Clamping follows each primitive's definition; nothing here quantizes.

use crate::color::Transfer;
use crate::composite::{self, Blend, Operator, Rgba};

/// A premultiplied image, `width × height`, top-down rows.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<Rgba>,
}

impl Image {
    pub fn new(width: usize, height: usize) -> Image {
        Image {
            width,
            height,
            pixels: vec![[0.0; 4]; width * height],
        }
    }

    /// The pixel at `(x, y)`, transparent outside.
    pub fn at(&self, x: i64, y: i64) -> Rgba {
        if x < 0 || y < 0 || x >= self.width as i64 || y >= self.height as i64 {
            return [0.0; 4];
        }
        self.pixels[y as usize * self.width + x as usize]
    }

    /// Every pixel through `f`.
    pub fn map(&self, f: impl Fn(Rgba) -> Rgba) -> Image {
        Image {
            width: self.width,
            height: self.height,
            pixels: self.pixels.iter().map(|p| f(*p)).collect(),
        }
    }

    /// Pixel pairs of two images of one size through `f`.
    pub fn zip(&self, other: &Image, f: impl Fn(Rgba, Rgba) -> Rgba) -> Image {
        assert_eq!((self.width, self.height), (other.width, other.height));
        Image {
            width: self.width,
            height: self.height,
            pixels: self
                .pixels
                .iter()
                .zip(&other.pixels)
                .map(|(a, b)| f(*a, *b))
                .collect(),
        }
    }
}

/// `color-interpolation-filters`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Space {
    Srgb,
    LinearRgb,
}

/// Below this alpha a premultiplied colour is transparent: its straight
/// colour would be the ratio of two rounding residues (an exact coverage
/// computation leaves ~1e-17 far outside a shape).
const TRANSPARENT: f64 = 1e-12;

fn convert(c: Rgba, f: impl Fn(f64) -> f64) -> Rgba {
    if c[3] <= TRANSPARENT {
        return c;
    }
    let a = c[3];
    [f(c[0] / a) * a, f(c[1] / a) * a, f(c[2] / a) * a, a]
}

/// sRGB-encoded premultiplied colour in `space`.
pub fn into_space(c: Rgba, space: Space) -> Rgba {
    match space {
        Space::Srgb => c,
        Space::LinearRgb => convert(c, |v| Transfer::Srgb.to_linear(v)),
    }
}

/// Premultiplied colour in `space` back to sRGB encoding.
pub fn from_space(c: Rgba, space: Space) -> Rgba {
    match space {
        Space::Srgb => c,
        Space::LinearRgb => convert(c, |v| Transfer::Srgb.from_linear(v)),
    }
}

/// `SourceAlpha` (§15.3.2): the alpha channel alone.
pub fn alpha(c: Rgba) -> Rgba {
    [0.0, 0.0, 0.0, c[3]]
}

/// `feFlood` (§15.14): `flood-color` (straight sRGB) at `flood-opacity`.
pub fn flood(rgb: [f64; 3], opacity: f64) -> Rgba {
    [
        rgb[0] * opacity,
        rgb[1] * opacity,
        rgb[2] * opacity,
        opacity,
    ]
}

/// `feColorMatrix` (§15.10) types.
#[derive(Clone, Debug, PartialEq)]
pub enum Matrix {
    /// 4 × 5 values, row-major.
    Values([f64; 20]),
    Saturate(f64),
    /// Degrees.
    HueRotate(f64),
    LuminanceToAlpha,
}

impl Matrix {
    /// The 4 × 5 matrix the type defines.
    pub fn values(&self) -> [f64; 20] {
        let rgb = |m: [f64; 9]| {
            [
                m[0], m[1], m[2], 0.0, 0.0, m[3], m[4], m[5], 0.0, 0.0, m[6], m[7], m[8], 0.0, 0.0,
                0.0, 0.0, 0.0, 1.0, 0.0,
            ]
        };
        match self {
            Matrix::Values(v) => *v,
            Matrix::Saturate(s) => rgb([
                0.213 + 0.787 * s,
                0.715 - 0.715 * s,
                0.072 - 0.072 * s,
                0.213 - 0.213 * s,
                0.715 + 0.285 * s,
                0.072 - 0.072 * s,
                0.213 - 0.213 * s,
                0.715 - 0.715 * s,
                0.072 + 0.928 * s,
            ]),
            Matrix::HueRotate(degrees) => {
                // [a00 a01 a02; …] = base + cos·C + sin·S (§15.10).
                let (s, c) = degrees.to_radians().sin_cos();
                let base = [0.213, 0.715, 0.072];
                let cm = [
                    [0.787, -0.715, -0.072],
                    [-0.213, 0.285, -0.072],
                    [-0.213, -0.715, 0.928],
                ];
                let sm = [
                    [-0.213, -0.715, 0.928],
                    [0.143, 0.140, -0.283],
                    [-0.787, 0.715, 0.072],
                ];
                rgb(std::array::from_fn(|i| {
                    let (r, k) = (i / 3, i % 3);
                    base[k] + c * cm[r][k] + s * sm[r][k]
                }))
            }
            Matrix::LuminanceToAlpha => {
                let mut m = [0.0; 20];
                m[15] = 0.2125;
                m[16] = 0.7154;
                m[17] = 0.0721;
                m
            }
        }
    }
}

fn unpremultiply(c: Rgba) -> Rgba {
    if c[3] <= TRANSPARENT {
        return [0.0; 4];
    }
    [c[0] / c[3], c[1] / c[3], c[2] / c[3], c[3]]
}

fn premultiply(c: Rgba) -> Rgba {
    let c = c.map(|v| v.clamp(0.0, 1.0));
    [c[0] * c[3], c[1] * c[3], c[2] * c[3], c[3]]
}

/// `feColorMatrix` on premultiplied colour: the matrix on unpremultiplied
/// values, each result clamped to [0, 1].
pub fn color_matrix(m: &Matrix, c: Rgba) -> Rgba {
    let v = m.values();
    let s = unpremultiply(c);
    let x = [s[0], s[1], s[2], s[3], 1.0];
    premultiply(std::array::from_fn(|row| {
        (0..5).map(|col| v[row * 5 + col] * x[col]).sum()
    }))
}

/// One `feFuncX` of `feComponentTransfer` (§15.11).
#[derive(Clone, Debug, PartialEq)]
pub enum Function {
    Identity,
    Table(Vec<f64>),
    Discrete(Vec<f64>),
    Linear {
        slope: f64,
        intercept: f64,
    },
    Gamma {
        amplitude: f64,
        exponent: f64,
        offset: f64,
    },
}

impl Function {
    /// The function of one unpremultiplied component.
    pub fn apply(&self, c: f64) -> f64 {
        match self {
            Function::Identity => c,
            Function::Table(v) if v.is_empty() => c,
            Function::Table(v) => {
                let n = v.len() - 1;
                if n == 0 || c >= 1.0 {
                    return v[n];
                }
                let k = ((c * n as f64).floor() as usize).min(n - 1);
                v[k] + (c - k as f64 / n as f64) * n as f64 * (v[k + 1] - v[k])
            }
            Function::Discrete(v) if v.is_empty() => c,
            Function::Discrete(v) => {
                let n = v.len();
                v[((c * n as f64).floor() as usize).min(n - 1)]
            }
            Function::Linear { slope, intercept } => slope * c + intercept,
            Function::Gamma {
                amplitude,
                exponent,
                offset,
            } => amplitude * c.powf(*exponent) + offset,
        }
    }
}

/// `feComponentTransfer` on premultiplied colour: `[r, g, b, a]`
/// functions on unpremultiplied values, clamped to [0, 1].
pub fn component_transfer(f: &[Function; 4], c: Rgba) -> Rgba {
    let s = unpremultiply(c);
    premultiply(std::array::from_fn(|i| f[i].apply(s[i])))
}

/// `feComposite` operators (§15.12).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CompositeOp {
    Over,
    In,
    Out,
    Atop,
    Xor,
    /// `k1·i1·i2 + k2·i1 + k3·i2 + k4` per premultiplied channel.
    Arithmetic([f64; 4]),
}

/// `feComposite` of `in` (`i1`) with `in2` (`i2`). Arithmetic results are
/// clamped to [0, 1] and colour to at most alpha (a premultiplied colour).
pub fn composite(op: CompositeOp, i1: Rgba, i2: Rgba) -> Rgba {
    let pd = |o| composite::porter_duff(o, i1, i2);
    match op {
        CompositeOp::Over => pd(Operator::SourceOver),
        CompositeOp::In => pd(Operator::SourceIn),
        CompositeOp::Out => pd(Operator::SourceOut),
        CompositeOp::Atop => pd(Operator::SourceAtop),
        CompositeOp::Xor => pd(Operator::Xor),
        CompositeOp::Arithmetic([k1, k2, k3, k4]) => {
            let v: Rgba =
                std::array::from_fn(|i| k1 * i1[i] * i2[i] + k2 * i1[i] + k3 * i2[i] + k4);
            let a = v[3].clamp(0.0, 1.0);
            [
                v[0].clamp(0.0, a),
                v[1].clamp(0.0, a),
                v[2].clamp(0.0, a),
                a,
            ]
        }
    }
}

/// `feBlend` (§15.6): `in` blended over `in2` with `mode`.
pub fn blend(mode: Blend, i1: Rgba, i2: Rgba) -> Rgba {
    composite::blend(mode, i1, i2)
}

/// `feMerge` (§15.18): each input over the ones before it.
pub fn merge(inputs: &[Rgba]) -> Rgba {
    inputs.iter().fold([0.0; 4], |acc, c| {
        composite::porter_duff(Operator::SourceOver, *c, acc)
    })
}

/// `feOffset` (§15.19) by whole pixels.
pub fn offset(image: &Image, dx: i64, dy: i64) -> Image {
    let mut out = Image::new(image.width, image.height);
    for y in 0..image.height {
        for x in 0..image.width {
            out.pixels[y * image.width + x] = image.at(x as i64 - dx, y as i64 - dy);
        }
    }
    out
}

/// The primitive subregion clip: transparent outside the pixel rectangle
/// `[x0, x1) × [y0, y1)`.
pub fn clip(image: &Image, [x0, y0, x1, y1]: [i64; 4]) -> Image {
    let mut out = image.clone();
    for y in 0..image.height {
        for x in 0..image.width {
            let (xi, yi) = (x as i64, y as i64);
            if xi < x0 || yi < y0 || xi >= x1 || yi >= y1 {
                out.pixels[y * image.width + x] = [0.0; 4];
            }
        }
    }
    out
}

/// `feGaussianBlur` (§15.17) with transparent edges: the exact Gaussian of
/// the box reconstruction ([`crate::gaussian::blur`]), σ in pixels.
pub fn gaussian(image: &Image, sigma: (f64, f64)) -> Image {
    let flat: Vec<f64> = image.pixels.iter().flatten().copied().collect();
    let blurred = crate::gaussian::blur(
        &flat,
        image.width,
        image.height,
        4,
        sigma,
        crate::gaussian::Edge::Transparent,
    );
    Image {
        width: image.width,
        height: image.height,
        pixels: blurred
            .chunks_exact(4)
            .map(|c| [c[0], c[1], c[2], c[3]])
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities_and_definitions() {
        let c = [0.2, 0.3, 0.1, 0.5];
        let near = |a: Rgba, b: Rgba| a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-12);
        assert!(near(color_matrix(&Matrix::Saturate(1.0), c), c));
        assert!(near(color_matrix(&Matrix::HueRotate(0.0), c), c));
        assert!(near(
            color_matrix(&Matrix::LuminanceToAlpha, [1.0; 4]),
            [0.0, 0.0, 0.0, 1.0]
        ));
        // Saturate 0 gives the luminance in every channel.
        let gray = color_matrix(&Matrix::Saturate(0.0), [1.0, 0.0, 0.0, 1.0]);
        assert!(near(gray, [0.213, 0.213, 0.213, 1.0]));
        // A table interpolates; discrete steps; linear and gamma are formulas.
        assert!((Function::Table(vec![0.0, 1.0, 0.0]).apply(0.25) - 0.5).abs() < 1e-12);
        assert_eq!(Function::Discrete(vec![0.2, 0.8]).apply(0.6), 0.8);
        assert_eq!(Function::Table(vec![0.3]).apply(0.9), 0.3);
        let g = Function::Gamma {
            amplitude: 2.0,
            exponent: 2.0,
            offset: 0.1,
        };
        assert!((g.apply(0.5) - 0.6).abs() < 1e-12);
        // Arithmetic clamps alpha to [0, 1] and colour to alpha.
        let a = composite(
            CompositeOp::Arithmetic([0.0, 1.0, 1.0, 0.0]),
            [0.6, 0.6, 0.6, 0.6],
            [0.8, 0.1, 0.1, 0.8],
        );
        assert_eq!(a, [1.0, 0.7, 0.7, 1.0]);
        let m = merge(&[[0.0, 0.0, 0.5, 0.5], [0.5, 0.0, 0.0, 0.5]]);
        assert!(near(m, [0.5, 0.0, 0.25, 0.75]));
        // linearRGB round-trips.
        let l = from_space(into_space(c, Space::LinearRgb), Space::LinearRgb);
        assert!(near(l, c));
    }
}
