//! The color contract (contract 4, section 9).
//!
//! Types describe how a source is read, how the working space is encoded, and
//! what an output is tagged as. The functions here are the normative reference
//! (an oracle in f64 through `libm`): pm-color, the GPU shaders, and the
//! encoder-input shader must agree with them within each bar's tolerance.

use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

/// Color primaries (ITU-T H.273 `ColourPrimaries`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum Primaries {
    /// BT.709 and sRGB (H.273 code 1).
    Bt709,
    /// BT.601 625-line, BT.470 BG (code 5).
    Bt601_625,
    /// BT.601 525-line, SMPTE 170M (code 6).
    Bt601_525,
    /// BT.2020 (code 9). After cutover except for HDR input tone mapping.
    Bt2020,
    /// Display P3, D65 white (code 12).
    DisplayP3,
    /// Not tagged; the interpretation rules in section 9 choose.
    Unspecified,
}

impl Primaries {
    pub const fn h273(self) -> u8 {
        match self {
            Primaries::Bt709 => 1,
            Primaries::Bt601_625 => 5,
            Primaries::Bt601_525 => 6,
            Primaries::Bt2020 => 9,
            Primaries::DisplayP3 => 12,
            Primaries::Unspecified => 2,
        }
    }
}

/// How code values map to light.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum Transfer {
    /// Display encoding E: the sRGB piecewise curve extended to all reals.
    /// The default reading of sRGB and Rec.709 sources.
    DisplayE,
    /// "BT.1886 (gamma 2.4)" interpretation.
    Bt1886,
    /// "Scene-referred (inverse BT.709 OETF)" interpretation.
    SceneReferredBt709,
    /// Linear light.
    Linear,
    /// Pure gamma 2.2 (QuickTime tag candidate, decoded as written).
    Gamma22,
    /// SMPTE ST 2084 (HDR input; HDR projects after cutover).
    Pq,
    /// ARIB STD-B67 (HDR input; HDR projects after cutover).
    Hlg,
}

/// Y′CbCr matrix coefficients.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum MatrixCoefficients {
    /// RGB, no matrix (H.273 code 0).
    Identity,
    /// BT.709 (code 1).
    Bt709,
    /// BT.601 (codes 5 and 6).
    Bt601,
    /// BT.2020 non-constant luminance (code 9).
    Bt2020Ncl,
}

impl MatrixCoefficients {
    /// Luma coefficients (Kr, Kb); Kg = 1 − Kr − Kb.
    pub const fn kr_kb(self) -> Option<(f64, f64)> {
        match self {
            MatrixCoefficients::Identity => None,
            MatrixCoefficients::Bt709 => Some((0.2126, 0.0722)),
            MatrixCoefficients::Bt601 => Some((0.299, 0.114)),
            MatrixCoefficients::Bt2020Ncl => Some((0.2627, 0.0593)),
        }
    }
}

/// Quantization range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum Range {
    /// Narrow, "video" range (luma 16..235 at 8-bit).
    Limited,
    /// Full, "data" range.
    Full,
}

/// Chroma subsampling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
pub enum Subsampling {
    #[serde(rename = "444")]
    #[cfg_attr(feature = "ts", ts(rename = "444"))]
    S444,
    #[serde(rename = "422")]
    #[cfg_attr(feature = "ts", ts(rename = "422"))]
    S422,
    #[serde(rename = "420")]
    #[cfg_attr(feature = "ts", ts(rename = "420"))]
    S420,
}

/// Chroma sample location (H.273 `chroma_sample_loc_type`, 0 to 5).
/// Untagged 4:2:0 is `Left` (type 0), section 9.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ChromaSiting {
    /// Type 0: horizontally co-sited with the left luma sample, vertically centered.
    Left,
    /// Type 1: centered both ways.
    Center,
    /// Type 2: co-sited with the top-left luma sample (BT.2020, HEVC 4:2:2 default).
    TopLeft,
    /// Type 3.
    Top,
    /// Type 4.
    BottomLeft,
    /// Type 5.
    Bottom,
}

impl ChromaSiting {
    pub const fn h273(self) -> u8 {
        match self {
            ChromaSiting::Left => 0,
            ChromaSiting::Center => 1,
            ChromaSiting::TopLeft => 2,
            ChromaSiting::Top => 3,
            ChromaSiting::BottomLeft => 4,
            ChromaSiting::Bottom => 5,
        }
    }

    /// Whether chroma samples are horizontally co-sited with luma samples.
    /// Co-sited samples are kept exact by upsampling and by point decimation
    /// in the encoder-input shader.
    pub const fn horizontally_cosited(self) -> bool {
        matches!(
            self,
            ChromaSiting::Left | ChromaSiting::TopLeft | ChromaSiting::BottomLeft
        )
    }
}

/// How a source's alpha relates to its color.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum AlphaMode {
    /// No alpha, or alpha ignored.
    Opaque,
    /// Straight (unassociated) alpha.
    Straight,
    /// Premultiplied (associated) alpha.
    Premultiplied,
    /// "Premultiplied with color": premultiplied against a matte color.
    PremultipliedWithColor {
        /// Matte color in the source encoding.
        matte: [f64; 3],
    },
}

/// A full description of how to read or write pixel values.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ColorDescription {
    pub primaries: Primaries,
    pub transfer: Transfer,
    pub matrix: MatrixCoefficients,
    pub range: Range,
    pub subsampling: Subsampling,
    pub siting: ChromaSiting,
    pub alpha: AlphaMode,
    /// Code value bit depth of the stored samples (8, 10, 12, 16, or 32 for float).
    pub bit_depth: u8,
}

impl ColorDescription {
    /// The Standard reading of an sRGB still or a full-range RGB frame.
    pub const SRGB_RGBA8: Self = Self {
        primaries: Primaries::Bt709,
        transfer: Transfer::DisplayE,
        matrix: MatrixCoefficients::Identity,
        range: Range::Full,
        subsampling: Subsampling::S444,
        siting: ChromaSiting::Center,
        alpha: AlphaMode::Straight,
        bit_depth: 8,
    };

    /// Whether reading this source into the working space is the identity:
    /// BT.709 primaries and E transfer in Standard color. Premultiplied
    /// identity sources are used as they are; others unpremultiply first.
    pub fn input_is_identity(&self, working: WorkingEncoding) -> bool {
        working == WorkingEncoding::Standard
            && self.primaries == Primaries::Bt709
            && self.transfer == Transfer::DisplayE
    }
}

/// The encoding of intermediates (section 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum WorkingEncoding {
    /// Linear blending off: premultiplied, display encoding E, BT.709 primaries.
    Standard,
    /// Linear blending on: premultiplied, linear light, BT.709 primaries.
    LinearLight,
}

/// Intermediate storage precision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum Precision {
    /// `rgba16float` (default).
    Float16,
    /// `rgba32float`.
    Float32,
}

/// Where screen, overlay, and soft light run when linear blending is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum BlendDomain {
    /// HDR-safe formulas in the active encoding (default).
    HdrSafe,
    /// The per-project option: run them in E.
    DisplayEncoded,
}

/// Project color mode. Only `Standard` exists before cutover.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ColorMode {
    Standard,
}

/// The project's `color` field as the engine sees it (Settings › Project › Color).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProjectColor {
    pub mode: ColorMode,
    pub linear_blending: bool,
    pub precision: Precision,
    pub blend_domain: BlendDomain,
}

impl ProjectColor {
    /// Decision 1: Standard color, linear blending off. Migrated projects too.
    pub const DEFAULT: Self = Self {
        mode: ColorMode::Standard,
        linear_blending: false,
        precision: Precision::Float16,
        blend_domain: BlendDomain::HdrSafe,
    };

    pub const fn working(&self) -> WorkingEncoding {
        if self.linear_blending {
            WorkingEncoding::LinearLight
        } else {
            WorkingEncoding::Standard
        }
    }

    /// Non-default color settings require the native engine (section 6 table).
    pub fn requires_native(&self) -> bool {
        *self != Self::DEFAULT
    }
}

/// The nine blend modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum BlendMode {
    Normal,
    Add,
    Screen,
    Multiply,
    Overlay,
    SoftLight,
    Difference,
    Lighten,
    Darken,
}

impl BlendMode {
    /// Modes that use HDR-safe formulas, or E with the per-project option,
    /// when linear blending is on. The rest always run in the active encoding.
    pub const fn uses_blend_domain(self) -> bool {
        matches!(
            self,
            BlendMode::Screen | BlendMode::Overlay | BlendMode::SoftLight
        )
    }
}

/// 3D tone mapping (decision 17): curves differ by design at cutover.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum ToneMapping3d {
    /// Native 3D: three.js r180 `ACESFilmicToneMapping` with its
    /// `exposure / 0.6` pre-scale, then three's sRGB OETF, per fragment.
    AcesFilmicR180 { exposure: f64 },
    /// Blender: AgX view transform to piecewise-sRGB PNG codes (pinned worker settings).
    AgxBlender,
}

/// An output's color tag choice (section 9, output tags).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum OutputColor {
    /// sRGB images: CICP 1-13-0, full range (PNG `cICP` or ICC).
    SrgbImage,
    /// "Rec.709 (broadcast)", video: CICP 1-1-1, limited range.
    Rec709Broadcast,
    /// Rec.709 images: CICP 1-1-0, full range.
    Rec709Image,
    /// "Rec.709 (QuickTime and Safari)": the tag spike S2 selects.
    Rec709QuickTime { candidate: QuickTimeTag },
    /// The viewer: E-encoded working values for the display canvas.
    Viewer { space: ViewerColorSpace },
}

/// The two tags S2 tests for "Rec.709 (QuickTime and Safari)" (section 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum QuickTimeTag {
    /// CICP 1-13-1 (sRGB transfer).
    Cicp1_13_1,
    /// CICP 1-2-1 with a gamma 2.2 tag.
    Cicp1_2_1Gamma22,
}

/// The canvas color space the viewer draws into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ViewerColorSpace {
    /// `srgb` canvas: working values encoded with E.
    Srgb,
    /// `display-p3` canvas: BT.709 primaries converted to P3 in linear, then E.
    DisplayP3,
}

/// ITU-T H.273 code points written to VUI, `colr nclx`, `cICP`, or WebM Colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Cicp {
    pub primaries: u8,
    pub transfer: u8,
    pub matrix: u8,
    pub full_range: bool,
}

impl OutputColor {
    /// The tag written for this output, or `None` for the viewer.
    pub const fn cicp(self) -> Option<Cicp> {
        let (primaries, transfer, matrix, full_range) = match self {
            OutputColor::SrgbImage => (1, 13, 0, true),
            OutputColor::Rec709Broadcast => (1, 1, 1, false),
            OutputColor::Rec709Image => (1, 1, 0, true),
            OutputColor::Rec709QuickTime {
                candidate: QuickTimeTag::Cicp1_13_1,
            } => (1, 13, 1, false),
            OutputColor::Rec709QuickTime {
                candidate: QuickTimeTag::Cicp1_2_1Gamma22,
            } => (1, 2, 1, false),
            OutputColor::Viewer { .. } => return None,
        };
        Some(Cicp {
            primaries,
            transfer,
            matrix,
            full_range,
        })
    }
}

/// The color-related components of a render cache key (section 8). The
/// render graph hashes [`ColorKey::key_bytes`] together with its other inputs.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ColorKey {
    pub working: WorkingEncoding,
    pub precision: Precision,
    pub blend_domain: BlendDomain,
    pub output: OutputColor,
    pub dither: bool,
}

impl ColorKey {
    /// A stable byte encoding (positional MessagePack, no maps), identical on
    /// every target.
    pub fn key_bytes(&self) -> Vec<u8> {
        rmp_serde::to_vec(self).expect("color keys always encode")
    }
}

// ---------------------------------------------------------------------------
// Normative reference functions
// ---------------------------------------------------------------------------

const E_LINEAR_BREAK: f64 = 0.003_130_8;
const E_ENCODED_BREAK: f64 = 0.040_45;

/// E: linear light to display encoding, the sRGB piecewise curve extended to
/// all reals (odd-symmetric below 0, the power segment continued above 1).
pub fn display_encode(linear: f64) -> f64 {
    let magnitude = linear.abs();
    let encoded = if magnitude <= E_LINEAR_BREAK {
        12.92 * magnitude
    } else {
        1.055 * libm::pow(magnitude, 1.0 / 2.4) - 0.055
    };
    encoded.copysign(linear)
}

/// E⁻¹: display encoding to linear light, extended to all reals.
pub fn display_decode(encoded: f64) -> f64 {
    let magnitude = encoded.abs();
    let linear = if magnitude <= E_ENCODED_BREAK {
        magnitude / 12.92
    } else {
        libm::pow((magnitude + 0.055) / 1.055, 2.4)
    };
    linear.copysign(encoded)
}

/// three.js r180's `sRGBTransferOETF`, which the native 3D pass reproduces
/// for parity: exponent 0.41666 (not 1/2.4) and no extension below 0.
/// Only the 3D scene pass uses this; compositing uses [`display_encode`].
pub fn three_r180_srgb_oetf(linear: f64) -> f64 {
    if linear <= E_LINEAR_BREAK {
        linear * 12.92
    } else {
        libm::pow(linear, 0.41666) * 1.055 - 0.055
    }
}

/// BT.1886 EOTF with zero black level (pure gamma 2.4), extended odd-symmetric.
pub fn bt1886_decode(encoded: f64) -> f64 {
    libm::pow(encoded.abs(), 2.4).copysign(encoded)
}

/// Inverse BT.709 OETF ("scene-referred"), extended odd-symmetric.
pub fn bt709_inverse_oetf(encoded: f64) -> f64 {
    let magnitude = encoded.abs();
    let linear = if magnitude < 0.081 {
        magnitude / 4.5
    } else {
        libm::pow((magnitude + 0.099) / 1.099, 1.0 / 0.45)
    };
    linear.copysign(encoded)
}

/// three.js r180 `ACESFilmicToneMapping` on linear BT.709 RGB, including
/// the `exposure / 0.6` pre-scale and the final saturate. The result is
/// linear; the 3D pass then applies [`three_r180_srgb_oetf`].
pub fn aces_filmic_r180(rgb: [f64; 3], exposure: f64) -> [f64; 3] {
    // GLSL mat3(vec3, vec3, vec3) takes columns; these are r180's columns.
    const INPUT: [[f64; 3]; 3] = [
        [0.59719, 0.07600, 0.02840],
        [0.35458, 0.90834, 0.13383],
        [0.04823, 0.01566, 0.83777],
    ];
    const OUTPUT: [[f64; 3]; 3] = [
        [1.60475, -0.10208, -0.00327],
        [-0.53108, 1.10813, -0.07276],
        [-0.07367, -0.00605, 1.07602],
    ];
    fn mul(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
        let mut out = [0.0; 3];
        for (row, value) in out.iter_mut().enumerate() {
            *value = m[0][row] * v[0] + m[1][row] * v[1] + m[2][row] * v[2];
        }
        out
    }
    let scale = exposure / 0.6;
    let mut color = mul(&INPUT, [rgb[0] * scale, rgb[1] * scale, rgb[2] * scale]);
    for c in &mut color {
        let a = *c * (*c + 0.024_578_6) - 0.000_090_537;
        let b = *c * (0.983_729 * *c + 0.432_951_0) + 0.238_081;
        *c = a / b;
    }
    let color = mul(&OUTPUT, color);
    [
        color[0].clamp(0.0, 1.0),
        color[1].clamp(0.0, 1.0),
        color[2].clamp(0.0, 1.0),
    ]
}

/// BT.709 luma of E-encoded values, used by luma mattes (section 9).
pub fn luma_709(r: f64, g: f64, b: f64) -> f64 {
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// Below this alpha, premultiplied values are not unpremultiplied at half
/// float (section 9, alpha).
pub const UNPREMULTIPLY_ALPHA_FLOOR: f64 = 1.0 / 4096.0;

/// Whether a premultiplied pixel may be unpremultiplied for a color
/// transform. Additive pixels (α = 0, RGB > 0) never are.
pub fn may_unpremultiply(alpha: f64) -> bool {
    alpha >= UNPREMULTIPLY_ALPHA_FLOOR
}

/// The codable range Y′CbCr outputs clip to (section 9), inclusive.
/// Superwhites and sub-blacks survive the pipeline and clip only here.
pub const fn ycbcr_codable_range(bit_depth: u8) -> Option<(u32, u32)> {
    match bit_depth {
        8 => Some((1, 254)),
        10 => Some((4, 1019)),
        12 => Some((16, 4079)),
        _ => None,
    }
}

/// Offsets and scales that normalize integer code values to Y′ in 0..1 and
/// Cb, Cr in −0.5..0.5 (section 13, normalization).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Normalization {
    pub luma_offset: f64,
    pub luma_scale: f64,
    pub chroma_offset: f64,
    pub chroma_scale: f64,
}

impl Normalization {
    pub fn luma(&self, code: f64) -> f64 {
        (code - self.luma_offset) / self.luma_scale
    }

    pub fn chroma(&self, code: f64) -> f64 {
        (code - self.chroma_offset) / self.chroma_scale
    }
}

/// How sample bits sit in their container word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum SampleAlignment {
    /// VideoToolbox `x420`/`x422` and P010: n-bit value in the high bits of 16.
    MsbIn16,
    /// FFmpeg planar n-bit: value in the low bits.
    Lsb,
    /// `y416`: full 16-bit samples.
    Full16,
}

/// Normalization for a sample format. `raw` is the integer read from memory;
/// the returned constants apply to it directly (MSB-aligned samples fold the
/// shift into the constants, so `x420` uses c = v / 64 as in section 13).
pub fn normalization(bits: u8, alignment: SampleAlignment, range: Range) -> Normalization {
    let n = bits as i32;
    let unit = match alignment {
        SampleAlignment::MsbIn16 => libm::pow(2.0, (16 - n) as f64),
        SampleAlignment::Lsb | SampleAlignment::Full16 => 1.0,
    };
    let step = libm::pow(2.0, (n - 8) as f64);
    match range {
        Range::Limited => Normalization {
            luma_offset: 16.0 * step * unit,
            luma_scale: 219.0 * step * unit,
            chroma_offset: 128.0 * step * unit,
            chroma_scale: 224.0 * step * unit,
        },
        Range::Full => {
            let max = libm::pow(2.0, n as f64) - 1.0;
            Normalization {
                luma_offset: 0.0,
                luma_scale: max * unit,
                chroma_offset: libm::pow(2.0, (n - 1) as f64) * unit,
                chroma_scale: max * unit,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tolerance: f64) -> bool {
        (a - b).abs() <= tolerance
    }

    #[test]
    fn e_round_trips_over_all_reals() {
        let mut x = -4.0;
        while x <= 4.0 {
            assert!(
                close(
                    display_decode(display_encode(x)),
                    x,
                    1e-12 * x.abs().max(1.0)
                ),
                "{x}"
            );
            x += 0.000_731;
        }
    }

    #[test]
    fn e_is_odd_and_continuous_at_the_break() {
        for x in [0.001, 0.5, 1.0, 2.5] {
            assert_eq!(display_encode(-x), -display_encode(x));
            assert_eq!(display_decode(-x), -display_decode(x));
        }
        let below = display_encode(E_LINEAR_BREAK);
        let above = 1.055 * libm::pow(E_LINEAR_BREAK, 1.0 / 2.4) - 0.055;
        assert!(close(below, above, 1e-6));
        assert!(
            display_encode(2.0) > 1.0,
            "the power segment continues above 1"
        );
    }

    #[test]
    fn linear_blending_bar_value() {
        // Section 2: 50% white over black is 188 (0.735357; boundary 187.5).
        let e = display_encode(0.5);
        assert!(close(e, 0.735_357, 5e-7), "{e}");
        assert_eq!((e * 255.0 + 0.5).floor() as u32, 188);
    }

    #[test]
    fn output_tags_match_section_9() {
        let tag = |o: OutputColor| {
            let c = o.cicp().unwrap();
            (c.primaries, c.transfer, c.matrix, c.full_range)
        };
        assert_eq!(tag(OutputColor::SrgbImage), (1, 13, 0, true));
        assert_eq!(tag(OutputColor::Rec709Broadcast), (1, 1, 1, false));
        assert_eq!(tag(OutputColor::Rec709Image), (1, 1, 0, true));
        assert!(
            OutputColor::Viewer {
                space: ViewerColorSpace::Srgb
            }
            .cicp()
            .is_none()
        );
    }

    #[test]
    fn normalization_matches_section_13_constants() {
        // x420 / x422 / P010 (MSB-aligned 10-bit): c = v / 64; Y' = (c − 64) / 876; C = (c − 512) / 896.
        let x420 = normalization(10, SampleAlignment::MsbIn16, Range::Limited);
        for c in [64.0, 512.0, 940.0, 1019.0] {
            let v = c * 64.0;
            assert!(close(x420.luma(v), (c - 64.0) / 876.0, 1e-12));
            assert!(close(x420.chroma(v), (c - 512.0) / 896.0, 1e-12));
        }
        // y416: Y' = (v − 4096) / 56064, C = (v − 32768) / 57344.
        let y416 = normalization(16, SampleAlignment::Full16, Range::Limited);
        assert_eq!(y416.luma_offset, 4096.0);
        assert_eq!(y416.luma_scale, 56064.0);
        assert_eq!(y416.chroma_offset, 32768.0);
        assert_eq!(y416.chroma_scale, 57344.0);
        // FFmpeg LSB planar n-bit: 16·2ⁿ⁻⁸, 219·2ⁿ⁻⁸, 128·2ⁿ⁻⁸, 224·2ⁿ⁻⁸.
        let lsb10 = normalization(10, SampleAlignment::Lsb, Range::Limited);
        assert_eq!((lsb10.luma_offset, lsb10.luma_scale), (64.0, 876.0));
        assert_eq!((lsb10.chroma_offset, lsb10.chroma_scale), (512.0, 896.0));
        let lsb8 = normalization(8, SampleAlignment::Lsb, Range::Limited);
        assert_eq!((lsb8.luma_offset, lsb8.luma_scale), (16.0, 219.0));
        let full8 = normalization(8, SampleAlignment::Lsb, Range::Full);
        assert_eq!(full8.luma(255.0), 1.0);
        assert_eq!(full8.chroma(128.0), 0.0);
    }

    #[test]
    fn codable_ranges() {
        assert_eq!(ycbcr_codable_range(8), Some((1, 254)));
        assert_eq!(ycbcr_codable_range(10), Some((4, 1019)));
        assert_eq!(ycbcr_codable_range(12), Some((16, 4079)));
        assert_eq!(ycbcr_codable_range(16), None);
    }

    #[test]
    fn aces_r180_reference_points() {
        assert_eq!(aces_filmic_r180([0.0; 3], 1.0), {
            // a(0) / b(0) = −0.000090537 / 0.238081, then output matrix, then saturate.
            [0.0, 0.0, 0.0]
        });
        let white = aces_filmic_r180([1.0; 3], 1.0);
        assert!(white.iter().all(|c| *c > 0.7 && *c < 0.95), "{white:?}");
        let hot = aces_filmic_r180([100.0; 3], 1.0);
        assert!(hot.iter().all(|c| close(*c, 1.0, 0.05)), "{hot:?}");
        // Monotonic in exposure.
        let a = aces_filmic_r180([0.18; 3], 1.0)[1];
        let b = aces_filmic_r180([0.18; 3], 2.0)[1];
        assert!(b > a);
    }

    #[test]
    fn three_oetf_differs_from_e_only_slightly() {
        // The exponents differ by 1/2.4 − 0.41666 ≈ 6.7e-6, so the curves
        // differ inside (break, 1), meet at 1, and diverge again above 1
        // (three's curve is not extended, but the 3D pass saturates first).
        for x in [0.01, 0.18, 0.5, 0.9] {
            let difference = (three_r180_srgb_oetf(x) - display_encode(x)).abs();
            assert!(difference > 0.0 && difference < 2e-5, "{x}: {difference}");
        }
        assert!((three_r180_srgb_oetf(1.0) - display_encode(1.0)).abs() < 1e-15);
        assert_eq!(three_r180_srgb_oetf(0.002), display_encode(0.002));
    }

    #[test]
    fn identity_input_rule() {
        let srgb = ColorDescription::SRGB_RGBA8;
        assert!(srgb.input_is_identity(WorkingEncoding::Standard));
        assert!(!srgb.input_is_identity(WorkingEncoding::LinearLight));
        let bt1886 = ColorDescription {
            transfer: Transfer::Bt1886,
            ..srgb
        };
        assert!(!bt1886.input_is_identity(WorkingEncoding::Standard));
        assert!(may_unpremultiply(1.0 / 4096.0));
        assert!(!may_unpremultiply(0.0));
    }

    #[test]
    fn project_color_default_is_decision_1() {
        assert_eq!(ProjectColor::DEFAULT.working(), WorkingEncoding::Standard);
        assert!(!ProjectColor::DEFAULT.requires_native());
        let linear = ProjectColor {
            linear_blending: true,
            ..ProjectColor::DEFAULT
        };
        assert!(linear.requires_native());
    }

    #[test]
    fn color_key_bytes_are_stable() {
        let key = ColorKey {
            working: WorkingEncoding::Standard,
            precision: Precision::Float16,
            blend_domain: BlendDomain::HdrSafe,
            output: OutputColor::Rec709Broadcast,
            dither: true,
        };
        assert_eq!(key.key_bytes(), key.key_bytes());
        let other = ColorKey {
            dither: false,
            ..key
        };
        assert_ne!(key.key_bytes(), other.key_bytes());
    }
}
