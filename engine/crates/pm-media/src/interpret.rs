//! Per-source interpretation (plan section 9, "Per-source interpretation";
//! appendix D "Interpret Footage…").
//!
//! An asset's `interpret` member (an additive, native-only document field,
//! section 6) holds the overrides a user chose. Every absent member means
//! "as the file says, or section 9's default when it says nothing". This
//! module parses and validates that member, and resolves the reading the
//! renderer uses together with the origin of every value, so the sheet can
//! show each value with where it came from.
//!
//! The stored shape (all members optional):
//!
//! ```json
//! {
//!   "primaries": "bt709" | "bt601_625" | "bt601_525" | "bt2020" | "displayP3",
//!   "transfer": "displayE" | "bt1886" | "sceneReferredBt709" | "linear" | "gamma22" | "pq" | "hlg",
//!   "matrix": "bt709" | "bt601" | "bt2020Ncl" | "identity",
//!   "range": "limited" | "full",
//!   "siting": "left" | "center" | "topLeft" | "top" | "bottomLeft" | "bottom",
//!   "alpha": "ignore" | "straight" | "premultiplied" | "premultipliedWithColor",
//!   "alphaMatte": "#RRGGBB",
//!   "frameRate": 23.976, "frameRateRational": [24000, 1001],  (or "frameRate": [24000, 1001])
//!   "fieldOrder": "topFirst" | "bottomFirst" | "progressive",
//!   "pixelAspect": [10, 11],
//!   "cleanAperture": true
//! }
//! ```

use crate::interlace::FieldOverride;
use crate::probe::color::{ColorInfo, HdrFormat, MasteringDisplay};
use crate::probe::{CleanAperture, FieldOrder, VideoProbe};
use pm_protocol::color::{
    AlphaMode, ChromaSiting, ColorDescription, MatrixCoefficients, Primaries, Range, Transfer,
};
use pm_time::Rate;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Where a value of the reading came from.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum Origin {
    /// Read from the file; `source` names the tag ("colr nclx", "pasp",
    /// "fiel", "bitstream or container", "ICC profile", ...).
    File { source: String },
    /// The file says nothing: the default rule that applied (section 9).
    Default { rule: String },
    /// Set in Interpret Footage.
    Override,
}

impl Origin {
    pub fn is_override(&self) -> bool {
        matches!(self, Origin::Override)
    }
}

/// Interpret Footage's alpha choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AlphaChoice {
    /// Treat the source as opaque.
    Ignore,
    Straight,
    Premultiplied,
    /// Premultiplied against `alphaMatte`.
    PremultipliedWithColor,
}

/// The parsed `interpret` member. `None` (or `Auto`) is "as the file says".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Interpretation {
    pub primaries: Option<Primaries>,
    pub transfer: Option<Transfer>,
    pub matrix: Option<MatrixCoefficients>,
    pub range: Option<Range>,
    pub siting: Option<ChromaSiting>,
    pub alpha: Option<AlphaChoice>,
    /// The matte of "premultiplied with color", in the source encoding
    /// (default black).
    pub alpha_matte: [f64; 3],
    /// Conform the source to this rate: presented frame n shows at n / rate.
    pub frame_rate: Option<Rate>,
    pub field_order: FieldOverride,
    /// Pixel aspect ratio (width : height of one pixel).
    pub pixel_aspect: Option<(i64, i64)>,
    /// Crop to the file's clean aperture (`clap`). The default is the full
    /// encoded frame, as today's viewer shows it.
    pub clean_aperture: Option<bool>,
}

/// A member of `interpret` that cannot be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InterpretIssue {
    pub member: String,
    pub message: String,
}

impl std::fmt::Display for InterpretIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "interpret.{}: {}", self.member, self.message)
    }
}

fn parse_enum<T: for<'de> Deserialize<'de>>(
    obj: &Map<String, Value>,
    key: &str,
    issues: &mut Vec<InterpretIssue>,
) -> Option<T> {
    let v = obj.get(key)?;
    if v.is_null() || v.as_str() == Some("auto") {
        return None;
    }
    match serde_json::from_value::<T>(v.clone()) {
        Ok(t) => Some(t),
        Err(_) => {
            issues.push(InterpretIssue {
                member: key.into(),
                message: format!("{v} is not a known value"),
            });
            None
        }
    }
}

/// Parses `#RRGGBB` into source-encoded values 0..1.
pub fn parse_matte(s: &str) -> Option<[f64; 3]> {
    let h = s.strip_prefix('#')?;
    if h.len() != 6 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let c = |i: usize| {
        u8::from_str_radix(&h[i..i + 2], 16)
            .ok()
            .map(|v| f64::from(v) / 255.0)
    };
    Some([c(0)?, c(2)?, c(4)?])
}

impl Interpretation {
    /// Parses an asset's `interpret` member. Unusable members are reported
    /// and read as "as the file says"; the rest still apply.
    pub fn from_json(value: &Value) -> (Interpretation, Vec<InterpretIssue>) {
        let mut issues = Vec::new();
        let mut out = Interpretation::default();
        let Some(obj) = value.as_object() else {
            if !value.is_null() {
                issues.push(InterpretIssue {
                    member: String::new(),
                    message: "is not an object".into(),
                });
            }
            return (out, issues);
        };
        out.primaries = parse_enum::<Primaries>(obj, "primaries", &mut issues)
            .filter(|p| *p != Primaries::Unspecified);
        out.transfer = parse_enum(obj, "transfer", &mut issues);
        out.matrix = parse_enum(obj, "matrix", &mut issues);
        out.range = parse_enum(obj, "range", &mut issues);
        out.siting = parse_enum(obj, "siting", &mut issues);
        out.alpha = parse_enum(obj, "alpha", &mut issues);
        if let Some(m) = obj.get("alphaMatte").filter(|v| !v.is_null()) {
            match m.as_str().and_then(parse_matte) {
                Some(c) => out.alpha_matte = c,
                None => issues.push(InterpretIssue {
                    member: "alphaMatte".into(),
                    message: format!("{m} is not a #RRGGBB color"),
                }),
            }
        }
        let rational = obj.get("frameRateRational").and_then(|v| {
            let a = v.as_array()?;
            Some([a.first()?.as_i64()?, a.get(1)?.as_i64()?])
        });
        if let Some(f) = obj.get("frameRate").filter(|v| !v.is_null()) {
            // A number (with `frameRateRational` beside it, as the sheet
            // writes) or an exact `[num, den]` of positive integers.
            let rate = match f.as_array() {
                Some(pair) => match pair.as_slice() {
                    [num, den] => num
                        .as_u64()
                        .zip(den.as_u64())
                        .filter(|&(n, d)| n >= 1 && d >= 1)
                        .and_then(|(n, d)| Some((u32::try_from(n).ok()?, u32::try_from(d).ok()?)))
                        .and_then(|(n, d)| Rate::new(i64::from(n), i64::from(d)).ok()),
                    _ => None,
                },
                None => f
                    .as_f64()
                    .and_then(|fps| pm_time::rate_from_document(rational, fps).ok())
                    .map(|r| r.rate),
            };
            match rate {
                Some(r) => out.frame_rate = Some(r),
                None => issues.push(InterpretIssue {
                    member: "frameRate".into(),
                    message: format!("{f} is not a frame rate"),
                }),
            }
        }
        out.field_order = match obj.get("fieldOrder").and_then(Value::as_str) {
            None | Some("auto") => FieldOverride::Auto,
            Some("topFirst") => FieldOverride::TopFirst,
            Some("bottomFirst") => FieldOverride::BottomFirst,
            Some("progressive") => FieldOverride::Progressive,
            Some(other) => {
                issues.push(InterpretIssue {
                    member: "fieldOrder".into(),
                    message: format!("{other:?} is not a field order"),
                });
                FieldOverride::Auto
            }
        };
        if let Some(p) = obj.get("pixelAspect").filter(|v| !v.is_null()) {
            let pair = p.as_array().and_then(|a| {
                let (n, d) = (a.first()?.as_i64()?, a.get(1)?.as_i64()?);
                (a.len() == 2 && n > 0 && d > 0 && n <= 1 << 20 && d <= 1 << 20).then_some((n, d))
            });
            match pair {
                Some((n, d)) => {
                    let g = gcd(n, d);
                    out.pixel_aspect = Some((n / g, d / g));
                }
                None => issues.push(InterpretIssue {
                    member: "pixelAspect".into(),
                    message: format!("{p} is not [width, height] of positive integers"),
                }),
            }
        }
        if let Some(c) = obj.get("cleanAperture").filter(|v| !v.is_null()) {
            match c.as_bool() {
                Some(b) => out.clean_aperture = Some(b),
                None => issues.push(InterpretIssue {
                    member: "cleanAperture".into(),
                    message: format!("{c} is not true or false"),
                }),
            }
        }
        for key in obj.keys() {
            if !KNOWN.contains(&key.as_str()) {
                issues.push(InterpretIssue {
                    member: key.clone(),
                    message: "is not an interpretation setting".into(),
                });
            }
        }
        (out, issues)
    }

    /// The stored form (only overridden members).
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        let mut put = |k: &str, v: Value| {
            m.insert(k.into(), v);
        };
        if let Some(p) = self.primaries {
            put("primaries", serde_json::to_value(p).unwrap());
        }
        if let Some(t) = self.transfer {
            put("transfer", serde_json::to_value(t).unwrap());
        }
        if let Some(x) = self.matrix {
            put("matrix", serde_json::to_value(x).unwrap());
        }
        if let Some(r) = self.range {
            put("range", serde_json::to_value(r).unwrap());
        }
        if let Some(s) = self.siting {
            put("siting", serde_json::to_value(s).unwrap());
        }
        if let Some(a) = self.alpha {
            put("alpha", serde_json::to_value(a).unwrap());
            if a == AlphaChoice::PremultipliedWithColor {
                let c = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                let [r, g, b] = self.alpha_matte;
                put(
                    "alphaMatte",
                    Value::String(format!("#{:02X}{:02X}{:02X}", c(r), c(g), c(b))),
                );
            }
        }
        if let Some(r) = self.frame_rate {
            put("frameRate", serde_json::json!(r.to_f64()));
            put("frameRateRational", serde_json::json!([r.num(), r.den()]));
        }
        match self.field_order {
            FieldOverride::Auto => {}
            FieldOverride::TopFirst => put("fieldOrder", "topFirst".into()),
            FieldOverride::BottomFirst => put("fieldOrder", "bottomFirst".into()),
            FieldOverride::Progressive => put("fieldOrder", "progressive".into()),
        }
        if let Some((n, d)) = self.pixel_aspect {
            put("pixelAspect", serde_json::json!([n, d]));
        }
        if let Some(c) = self.clean_aperture {
            put("cleanAperture", Value::Bool(c));
        }
        Value::Object(m)
    }

    pub fn is_empty(&self) -> bool {
        *self == Interpretation::default()
    }
}

const KNOWN: [&str; 12] = [
    "primaries",
    "transfer",
    "matrix",
    "range",
    "siting",
    "alpha",
    "alphaMatte",
    "frameRate",
    "frameRateRational",
    "fieldOrder",
    "pixelAspect",
    "cleanAperture",
];

fn gcd(a: i64, b: i64) -> i64 {
    if b == 0 {
        a.abs().max(1)
    } else {
        gcd(b, a % b)
    }
}

/// One value of the reading with its origin.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Field<T> {
    pub value: T,
    pub origin: Origin,
    /// The value without the override (what the file or the default rule
    /// gives), so the sheet can show what "Automatic" means.
    pub automatic: T,
}

fn field<T: Clone>(automatic: T, origin: Origin, over: Option<T>) -> Field<T> {
    match over {
        Some(v) => Field {
            value: v,
            origin: Origin::Override,
            automatic,
        },
        None => Field {
            value: automatic.clone(),
            origin,
            automatic,
        },
    }
}

/// HDR input tone mapping for Standard projects (section 9): BT.2446
/// Method A from the source's HDR signal to SDR.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToneMap {
    /// The HDR signal (PQ or HLG).
    pub transfer: Transfer,
    /// Peak luminance of the HDR content in cd/m² (L_HDR): 1000 for HLG;
    /// for PQ the mastering display's maximum when tagged, else 1000.
    pub peak_nits: f64,
}

/// The reading of a video source: its color, timing, fields, and geometry.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoReading {
    pub primaries: Field<Primaries>,
    pub transfer: Field<Transfer>,
    pub matrix: Field<MatrixCoefficients>,
    pub range: Field<Range>,
    pub siting: Field<ChromaSiting>,
    pub alpha: Field<AlphaMode>,
    #[serde(serialize_with = "rate_field")]
    pub frame_rate: Field<Rate>,
    pub field_order: Field<FieldOrder>,
    pub pixel_aspect: Field<(i64, i64)>,
    /// The clean aperture in use (None: the full encoded frame).
    pub clean_aperture: Field<Option<CleanAperture>>,
    /// Display rotation in degrees, counterclockwise (from the display
    /// matrix), applied as today's viewer applies it.
    pub rotation: f64,
    pub hdr: Option<HdrFormat>,
    pub tone_map: Option<ToneMap>,
    /// The color description the source pass reads with.
    pub color: ColorDescription,
}

fn rate_field<S: serde::Serializer>(f: &Field<Rate>, s: S) -> Result<S::Ok, S::Error> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct R<'a> {
        value: [i64; 2],
        origin: &'a Origin,
        automatic: [i64; 2],
    }
    R {
        value: [f.value.num(), f.value.den()],
        origin: &f.origin,
        automatic: [f.automatic.num(), f.automatic.den()],
    }
    .serialize(s)
}

impl VideoReading {
    /// Encoded width and height after pixel aspect, clean aperture, and
    /// rotation: the size the frame is shown at (width rounded to pixels).
    pub fn display_size(&self, coded: (u32, u32)) -> (u32, u32) {
        let (w, h) = match self.clean_aperture.value {
            Some(c) => (c.rect.2.max(1), c.rect.3.max(1)),
            None => coded,
        };
        let (n, d) = self.pixel_aspect.value;
        let dw = ((f64::from(w) * n as f64 / d as f64).round() as u32).max(1);
        if self.quarter_turns() % 2 == 1 {
            (h, dw)
        } else {
            (dw, h)
        }
    }

    /// Rotation in quarter turns counterclockwise (0 to 3). Display
    /// matrices other than multiples of 90° are read to the nearest.
    pub fn quarter_turns(&self) -> u32 {
        ((self.rotation / 90.0).round() as i64).rem_euclid(4) as u32
    }

    /// The frame time grid when the rate is conformed (an override):
    /// presented frame n shows from n / rate.
    pub fn conformed(&self) -> Option<Rate> {
        self.frame_rate
            .origin
            .is_override()
            .then_some(self.frame_rate.value)
    }
}

fn hdr_tone_map(transfer: Transfer, mastering: Option<&MasteringDisplay>) -> Option<ToneMap> {
    match transfer {
        Transfer::Hlg => Some(ToneMap {
            transfer,
            peak_nits: 1000.0,
        }),
        Transfer::Pq => Some(ToneMap {
            transfer,
            peak_nits: mastering
                .and_then(|m| m.max_luminance)
                .filter(|l| l.is_finite() && *l >= 100.0)
                .map_or(1000.0, |l| l.min(10000.0)),
        }),
        _ => None,
    }
}

/// Resolves a video source's reading: the probe's (file tags and section 9
/// defaults) with `interp`'s overrides.
pub fn resolve_video(v: &VideoProbe, interp: &Interpretation) -> VideoReading {
    let c: &ColorInfo = &v.color;
    let o = &c.origins;
    let auto = c.interpreted;
    let primaries = field(auto.primaries, o.primaries.clone(), interp.primaries);
    let transfer = field(auto.transfer, o.transfer.clone(), interp.transfer);
    let matrix = field(auto.matrix, o.matrix.clone(), interp.matrix);
    let range = field(auto.range, o.range.clone(), interp.range);
    let siting = field(auto.siting, o.siting.clone(), interp.siting);
    let has_alpha = v.alpha.is_some();
    let alpha_over = interp.alpha.map(|a| match a {
        AlphaChoice::Ignore => AlphaMode::Opaque,
        AlphaChoice::Straight => AlphaMode::Straight,
        AlphaChoice::Premultiplied => AlphaMode::Premultiplied,
        AlphaChoice::PremultipliedWithColor => AlphaMode::PremultipliedWithColor {
            matte: interp.alpha_matte,
        },
    });
    // A source without alpha stays opaque whatever the override says.
    let alpha = field(
        auto.alpha,
        o.alpha.clone(),
        alpha_over.filter(|_| has_alpha),
    );
    let (rate_auto, rate_origin) = match &v.nominal_rate {
        Some(n) => (
            n.rate,
            Origin::File {
                source: if v.variable_rate {
                    "timestamps (variable frame rate)".into()
                } else {
                    "timestamps".into()
                },
            },
        ),
        None => (
            Rate::FPS_30,
            Origin::Default {
                rule: "A single frame has no rate".into(),
            },
        ),
    };
    let frame_rate = field(rate_auto, rate_origin, interp.frame_rate);
    let field_origin = match v.fields.source.as_str() {
        "unknown" => Origin::Default {
            rule: "No field order in the file: progressive".into(),
        },
        s => Origin::File { source: s.into() },
    };
    let field_auto = match v.fields.order {
        FieldOrder::Unknown => FieldOrder::Progressive,
        o => o,
    };
    let field_order = field(
        field_auto,
        field_origin,
        match interp.field_order {
            FieldOverride::Auto => None,
            FieldOverride::TopFirst => Some(FieldOrder::TopFirst),
            FieldOverride::BottomFirst => Some(FieldOrder::BottomFirst),
            FieldOverride::Progressive => Some(FieldOrder::Progressive),
        },
    );
    let pasp_origin = match &v.pixel_aspect_source {
        Some(s) => Origin::File { source: s.clone() },
        None => Origin::Default {
            rule: "No pixel aspect in the file: square pixels".into(),
        },
    };
    let pixel_aspect = field(v.pixel_aspect, pasp_origin, interp.pixel_aspect);
    let clean_aperture = match (interp.clean_aperture, v.clean_aperture) {
        (Some(true), Some(c)) => Field {
            value: Some(c),
            origin: Origin::Override,
            automatic: None,
        },
        (Some(false), _) | (Some(true), None) => Field {
            value: None,
            origin: Origin::Override,
            automatic: None,
        },
        (None, Some(_)) => Field {
            value: None,
            origin: Origin::Default {
                rule: "The full encoded frame is shown; the clean aperture is a choice".into(),
            },
            automatic: None,
        },
        (None, None) => Field {
            value: None,
            origin: Origin::Default {
                rule: "No clean aperture in the file".into(),
            },
            automatic: None,
        },
    };
    let color = ColorDescription {
        primaries: primaries.value,
        transfer: transfer.value,
        matrix: matrix.value,
        range: range.value,
        subsampling: auto.subsampling,
        siting: siting.value,
        alpha: alpha.value,
        bit_depth: auto.bit_depth,
    };
    VideoReading {
        tone_map: hdr_tone_map(transfer.value, c.mastering.as_ref()),
        primaries,
        transfer,
        matrix,
        range,
        siting,
        alpha,
        frame_rate,
        field_order,
        pixel_aspect,
        clean_aperture,
        rotation: v.rotation.unwrap_or(0.0),
        hdr: c.hdr,
        color,
    }
}

/// The reading of a still image (or image sequence or animation frame).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StillReading {
    pub primaries: Field<Primaries>,
    pub transfer: Field<Transfer>,
    pub alpha: Field<AlphaMode>,
    pub tone_map: Option<ToneMap>,
}

/// Resolves a still's reading. `primaries` and `transfer` come from the
/// reader (ICC profile class, EXR chromaticities, PNG chunks); stills
/// without any read as sRGB, as today.
pub fn resolve_still(
    primaries: (Primaries, Origin),
    transfer: (Transfer, Origin),
    alpha: (AlphaMode, Origin),
    has_alpha: bool,
    interp: &Interpretation,
) -> StillReading {
    let alpha_over = interp.alpha.map(|a| match a {
        AlphaChoice::Ignore => AlphaMode::Opaque,
        AlphaChoice::Straight => AlphaMode::Straight,
        AlphaChoice::Premultiplied => AlphaMode::Premultiplied,
        AlphaChoice::PremultipliedWithColor => AlphaMode::PremultipliedWithColor {
            matte: interp.alpha_matte,
        },
    });
    let transfer = field(transfer.0, transfer.1, interp.transfer);
    StillReading {
        primaries: field(primaries.0, primaries.1, interp.primaries),
        tone_map: hdr_tone_map(transfer.value, None),
        transfer,
        alpha: field(alpha.0, alpha.1, alpha_over.filter(|_| has_alpha)),
    }
}

/// How the engine reads a still's H.273 code points (PNG `cICP`, AVIF
/// `colr nclx`): primaries, transfer and matrix, or `None` for a code point
/// outside the still classes. Unspecified primaries read as BT.709 and an
/// unspecified transfer as sRGB (MIAF's defaults); the BT.709 transfer family
/// reads as the display encoding, as video's does; an unspecified matrix is
/// BT.601 (MIAF).
pub fn still_cicp(cicp: [u8; 4]) -> Option<(Primaries, Transfer, MatrixCoefficients)> {
    let primaries = match cicp[0] {
        1 | 2 => Primaries::Bt709,
        9 => Primaries::Bt2020,
        12 => Primaries::DisplayP3,
        _ => return None,
    };
    let transfer = match cicp[1] {
        1 | 2 | 6 | 13 | 14 | 15 => Transfer::DisplayE,
        16 => Transfer::Pq,
        18 => Transfer::Hlg,
        _ => return None,
    };
    let matrix = match cicp[2] {
        0 => MatrixCoefficients::Identity,
        1 => MatrixCoefficients::Bt709,
        2 | 5 | 6 => MatrixCoefficients::Bt601,
        9 => MatrixCoefficients::Bt2020Ncl,
        10 => MatrixCoefficients::Bt2020Cl,
        _ => return None,
    };
    Some((primaries, transfer, matrix))
}

/// A still's reading under an interpretation, and how its samples get there.
#[derive(Clone, Debug, PartialEq)]
pub struct StillInterpretation {
    pub reading: StillReading,
    /// The embedded ICC profile (neither sRGB nor Display P3) is the
    /// reading: ColorSync converts the samples, read with `reading.alpha`,
    /// to premultiplied linear Rec.709
    /// ([`crate::image::imageio::convert_to_linear_srgb`]) before the source
    /// pass.
    pub colorsync: bool,
}

/// Resolves a still's reading from its reader's color facts and the
/// asset's overrides.
///
/// A primaries or transfer override replaces the embedded profile's reading
/// of that value, for every still class. A profile that is neither sRGB nor
/// Display P3 describes primaries and curve together and cannot be split, so
/// either override sets it aside as a whole: the samples are read as stored
/// with the overrides, and the value not overridden takes the default of a
/// still without a profile (Rec.709 primaries; the display encoding for
/// integer stills, linear for float). A still ImageIO already converted to
/// sRGB (CMYK, Lab, indexed: [`IccClass::ConvertedToSrgb`]) reads as sRGB,
/// and its overrides reinterpret those converted samples; its profile is
/// never applied again. Alpha overrides replace the file's alpha reading of
/// the stored samples in every case.
///
/// [`IccClass::ConvertedToSrgb`]: crate::image::icc::IccClass::ConvertedToSrgb
pub fn interpret_still(
    color: &crate::image::ImageColor,
    float: bool,
    interp: &Interpretation,
) -> StillInterpretation {
    use crate::image::icc::IccClass;
    let other = color.icc_class == Some(IccClass::Other);
    let colorsync = other && interp.primaries.is_none() && interp.transfer.is_none();
    let set_aside = other && !colorsync;
    let file = |source: &str| Origin::File {
        source: source.into(),
    };
    let default = |rule: &str| Origin::Default { rule: rule.into() };
    // H.273 code points when no profile overrides them.
    let cicp = color
        .cicp
        .filter(|_| color.icc_class.is_none())
        .and_then(still_cicp);
    let primaries = match color.icc_class {
        None if cicp.is_some() => (
            cicp.map(|c| c.0).unwrap(),
            file("H.273 colour primaries (PNG cICP, AVIF colr)"),
        ),
        None if color.chromaticities.is_some() => (
            Primaries::Bt709,
            file("Chromaticities (PNG cHRM, OpenEXR), converted to Rec.709"),
        ),
        None => (
            Primaries::Bt709,
            default("Stills without a profile read as sRGB"),
        ),
        Some(IccClass::Srgb | IccClass::GraySrgb) => (
            Primaries::Bt709,
            file("ICC profile equivalent to sRGB (passed through)"),
        ),
        Some(IccClass::DisplayP3) => (Primaries::DisplayP3, file("ICC profile (Display P3)")),
        Some(IccClass::Other) if colorsync => (
            Primaries::Bt709,
            file("ICC profile, converted by ColorSync to Rec.709"),
        ),
        Some(IccClass::Other) => (
            Primaries::Bt709,
            default(
                "The ICC profile is set aside by the override; stills without a profile read as sRGB",
            ),
        ),
        Some(IccClass::ConvertedToSrgb) => (
            Primaries::Bt709,
            file("Converted to sRGB by ImageIO from the file's color model and profile"),
        ),
    };
    let transfer = if colorsync {
        (
            Transfer::Linear,
            file("ICC profile, converted by ColorSync to linear light"),
        )
    } else if let (false, Some(class)) = (float, color.icc_class.filter(|c| *c != IccClass::Other))
    {
        // Integer stills whose profile (or conversion) defines the sRGB curve.
        let source = match class {
            IccClass::ConvertedToSrgb => {
                "Converted to sRGB by ImageIO from the file's color model and profile (sRGB curve)"
            }
            IccClass::DisplayP3 => "ICC profile (Display P3, sRGB curve)",
            _ => "ICC profile equivalent to sRGB (sRGB curve)",
        };
        (color.transfer, file(source))
    } else if let Some((_, transfer, _)) = cicp {
        (
            transfer,
            file("H.273 transfer characteristics (PNG cICP, AVIF colr)"),
        )
    } else if color.gamma.is_some() {
        (
            color.transfer,
            file("PNG gAMA: a pure power curve, decoded with 1 / gamma"),
        )
    } else {
        let rule = match (set_aside, float) {
            (false, true) => "Floating-point stills are linear",
            (false, false) => "Integer stills read with the display encoding (sRGB curve)",
            (true, true) => {
                "The ICC profile is set aside by the override; floating-point stills are linear"
            }
            (true, false) => {
                "The ICC profile is set aside by the override; integer stills read with the display encoding (sRGB curve)"
            }
        };
        (color.transfer, default(rule))
    };
    let has_alpha = color.alpha != AlphaMode::Opaque;
    let alpha_origin = if has_alpha {
        file("alpha channel")
    } else {
        default("No alpha channel")
    };
    let mut reading = resolve_still(
        primaries,
        transfer,
        (color.alpha, alpha_origin),
        has_alpha,
        interp,
    );
    // A PQ still's content peak is its mastering display's, as video's.
    if let (Some(t), Some(peak)) = (reading.tone_map.as_mut(), color.mastering_peak)
        && t.transfer == Transfer::Pq
    {
        t.peak_nits = peak;
    }
    StillInterpretation { reading, colorsync }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_every_member_and_round_trips() {
        let v = json!({
            "primaries": "displayP3",
            "transfer": "bt1886",
            "matrix": "bt601",
            "range": "full",
            "siting": "topLeft",
            "alpha": "premultipliedWithColor",
            "alphaMatte": "#FF8000",
            "frameRate": 23.976,
            "fieldOrder": "bottomFirst",
            "pixelAspect": [20, 22],
            "cleanAperture": true
        });
        let (i, issues) = Interpretation::from_json(&v);
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(i.primaries, Some(Primaries::DisplayP3));
        assert_eq!(i.transfer, Some(Transfer::Bt1886));
        assert_eq!(i.matrix, Some(MatrixCoefficients::Bt601));
        assert_eq!(i.range, Some(Range::Full));
        assert_eq!(i.siting, Some(ChromaSiting::TopLeft));
        assert_eq!(i.alpha, Some(AlphaChoice::PremultipliedWithColor));
        assert_eq!(i.alpha_matte, [1.0, 128.0 / 255.0, 0.0]);
        assert_eq!(i.frame_rate, Some(Rate::FPS_23_976));
        assert_eq!(i.field_order, FieldOverride::BottomFirst);
        assert_eq!(i.pixel_aspect, Some((10, 11)));
        assert_eq!(i.clean_aperture, Some(true));
        let (back, issues) = Interpretation::from_json(&i.to_json());
        assert!(issues.is_empty());
        assert_eq!(back, i);
    }

    #[test]
    fn auto_and_null_mean_as_the_file_says() {
        let (i, issues) = Interpretation::from_json(&json!({
            "primaries": "auto", "transfer": null, "fieldOrder": "auto"
        }));
        assert!(issues.is_empty());
        assert!(i.is_empty());
        assert_eq!(i.to_json(), json!({}));
        let (i, issues) = Interpretation::from_json(&Value::Null);
        assert!(issues.is_empty() && i.is_empty());
    }

    #[test]
    fn bad_members_are_reported_and_ignored() {
        let (i, issues) = Interpretation::from_json(&json!({
            "primaries": "bt9999",
            "range": "full",
            "frameRate": -3,
            "pixelAspect": [0, 1],
            "alphaMatte": "red",
            "fieldOrder": "sideways",
            "cleanAperture": "yes",
            "colour": 1
        }));
        assert_eq!(i.range, Some(Range::Full));
        assert_eq!(i.primaries, None);
        let members: Vec<&str> = issues.iter().map(|i| i.member.as_str()).collect();
        assert_eq!(
            members,
            [
                "primaries",
                "alphaMatte",
                "frameRate",
                "fieldOrder",
                "pixelAspect",
                "cleanAperture",
                "colour"
            ]
        );
    }

    #[test]
    fn exact_rates_win_over_the_float() {
        let (i, _) = Interpretation::from_json(&json!({
            "frameRate": 29.97, "frameRateRational": [30000, 1001]
        }));
        assert_eq!(i.frame_rate, Some(Rate::FPS_29_97));
        let (i, _) = Interpretation::from_json(&json!({ "frameRate": 25 }));
        assert_eq!(i.frame_rate, Some(Rate::FPS_25));
        // The exact pair form (`[num, den]` of positive integers).
        let (i, issues) = Interpretation::from_json(&json!({ "frameRate": [24000, 1001] }));
        assert_eq!(i.frame_rate, Some(Rate::FPS_23_976));
        assert!(issues.is_empty());
        for bad in [json!([0, 1]), json!([24.5, 1]), json!([24]), json!("24")] {
            let (i, issues) = Interpretation::from_json(&json!({ "frameRate": bad }));
            assert_eq!(i.frame_rate, None, "{bad}");
            assert_eq!(issues.len(), 1, "{bad}");
        }
    }

    #[test]
    fn pq_peak_follows_mastering() {
        let m = MasteringDisplay {
            red: None,
            green: None,
            blue: None,
            white: None,
            min_luminance: Some(0.005),
            max_luminance: Some(4000.0),
        };
        assert_eq!(
            hdr_tone_map(Transfer::Pq, Some(&m)).unwrap().peak_nits,
            4000.0
        );
        assert_eq!(hdr_tone_map(Transfer::Pq, None).unwrap().peak_nits, 1000.0);
        assert_eq!(
            hdr_tone_map(Transfer::Hlg, Some(&m)).unwrap().peak_nits,
            1000.0
        );
        assert_eq!(hdr_tone_map(Transfer::DisplayE, None), None);
    }

    fn other_profile(alpha: AlphaMode) -> crate::image::ImageColor {
        crate::image::ImageColor {
            icc: Some(vec![0; 4]),
            icc_bytes: Some(4),
            icc_class: Some(crate::image::icc::IccClass::Other),
            ..crate::image::ImageColor::plain(Transfer::DisplayE, alpha)
        }
    }

    #[test]
    fn other_profiles_convert_through_colorsync_with_the_alpha_reading() {
        let color = other_profile(AlphaMode::Straight);
        let si = interpret_still(&color, false, &Interpretation::default());
        assert!(si.colorsync);
        assert_eq!(si.reading.transfer.value, Transfer::Linear);
        assert!(matches!(si.reading.transfer.origin, Origin::File { .. }));
        assert_eq!(si.reading.primaries.value, Primaries::Bt709);
        assert_eq!(si.reading.alpha.value, AlphaMode::Straight);
        // An alpha override keeps the conversion and changes how its input
        // is read.
        let (i, _) = Interpretation::from_json(&serde_json::json!({ "alpha": "premultiplied" }));
        let si = interpret_still(&color, false, &i);
        assert!(si.colorsync);
        assert_eq!(si.reading.alpha.value, AlphaMode::Premultiplied);
        assert_eq!(si.reading.alpha.origin, Origin::Override);
        assert_eq!(si.reading.alpha.automatic, AlphaMode::Straight);
    }

    #[test]
    fn primaries_or_transfer_overrides_set_other_profiles_aside() {
        let color = other_profile(AlphaMode::Opaque);
        let (i, _) = Interpretation::from_json(&serde_json::json!({ "transfer": "gamma22" }));
        let si = interpret_still(&color, false, &i);
        assert!(!si.colorsync);
        assert_eq!(si.reading.transfer.value, Transfer::Gamma22);
        assert_eq!(si.reading.transfer.origin, Origin::Override);
        // What Automatic means: the profile, through ColorSync.
        assert_eq!(si.reading.transfer.automatic, Transfer::DisplayE);
        assert_eq!(si.reading.primaries.value, Primaries::Bt709);
        let Origin::Default { rule } = &si.reading.primaries.origin else {
            panic!("{:?}", si.reading.primaries.origin);
        };
        assert!(rule.contains("set aside"), "{rule}");
        let (i, _) = Interpretation::from_json(&serde_json::json!({ "primaries": "displayP3" }));
        let si = interpret_still(&color, true, &i);
        assert!(!si.colorsync);
        assert_eq!(si.reading.primaries.value, Primaries::DisplayP3);
        assert_eq!(si.reading.transfer.value, Transfer::DisplayE);
        let Origin::Default { rule } = &si.reading.transfer.origin else {
            panic!("{:?}", si.reading.transfer.origin);
        };
        assert!(
            rule.contains("set aside") && rule.contains("floating-point"),
            "{rule}"
        );
        // sRGB and Display P3 profiles: an override replaces that value and
        // the profile keeps the other.
        let mut p3 = other_profile(AlphaMode::Opaque);
        p3.icc_class = Some(crate::image::icc::IccClass::DisplayP3);
        let (i, _) = Interpretation::from_json(&serde_json::json!({ "transfer": "gamma22" }));
        let si = interpret_still(&p3, false, &i);
        assert!(!si.colorsync);
        assert_eq!(si.reading.primaries.value, Primaries::DisplayP3);
        assert_eq!(si.reading.transfer.value, Transfer::Gamma22);
    }

    #[test]
    fn converted_stills_read_as_srgb_and_never_reapply_their_profile() {
        let mut cmyk = other_profile(AlphaMode::Opaque);
        cmyk.icc_class = Some(crate::image::icc::IccClass::ConvertedToSrgb);
        let source = |o: &Origin| match o {
            Origin::File { source } => source.clone(),
            o => panic!("{o:?}"),
        };
        for json in [
            serde_json::json!({}),
            serde_json::json!({ "alpha": "ignore" }),
            serde_json::json!({ "range": "full" }),
        ] {
            let (i, _) = Interpretation::from_json(&json);
            let si = interpret_still(&cmyk, false, &i);
            assert!(!si.colorsync, "{json}");
            assert_eq!(si.reading.primaries.value, Primaries::Bt709);
            assert_eq!(si.reading.transfer.value, Transfer::DisplayE);
            assert!(source(&si.reading.primaries.origin).contains("Converted to sRGB by ImageIO"));
            assert!(source(&si.reading.transfer.origin).contains("sRGB curve"));
        }
        // Overrides reinterpret the converted samples.
        let (i, _) = Interpretation::from_json(&serde_json::json!({ "primaries": "displayP3" }));
        let si = interpret_still(&cmyk, false, &i);
        assert!(!si.colorsync);
        assert_eq!(si.reading.primaries.value, Primaries::DisplayP3);
        assert_eq!(si.reading.primaries.automatic, Primaries::Bt709);
        assert_eq!(si.reading.transfer.value, Transfer::DisplayE);
        assert!(matches!(si.reading.transfer.origin, Origin::File { .. }));
    }

    #[test]
    fn srgb_and_p3_profiles_define_the_transfer_of_integer_stills() {
        use crate::image::icc::IccClass;
        let none = |o: &Origin| matches!(o, Origin::Default { .. });
        for (class, want) in [
            (IccClass::Srgb, "equivalent to sRGB (sRGB curve)"),
            (IccClass::GraySrgb, "equivalent to sRGB (sRGB curve)"),
            (IccClass::DisplayP3, "Display P3, sRGB curve"),
        ] {
            let mut c = other_profile(AlphaMode::Opaque);
            c.icc_class = Some(class);
            let si = interpret_still(&c, false, &Interpretation::default());
            let Origin::File { source } = &si.reading.transfer.origin else {
                panic!("{class:?}: {:?}", si.reading.transfer.origin);
            };
            assert!(source.contains(want), "{class:?}: {source}");
            // Float stills are linear by convention whatever the profile.
            c.transfer = Transfer::Linear;
            let si = interpret_still(&c, true, &Interpretation::default());
            assert!(none(&si.reading.transfer.origin), "{class:?}");
        }
        let mut c = other_profile(AlphaMode::Opaque);
        c.icc_class = None;
        let si = interpret_still(&c, false, &Interpretation::default());
        assert!(none(&si.reading.transfer.origin));
    }
}
