//! Color components of render cache keys (section 8 "Content keys";
//! contract 4 `ColorKey`, invariant 10) and the precision default
//! (section 8, amendment 8).
//!
//! The render graph hashes these bytes with its other inputs. Every color
//! input that changes pixels is in a key: the output-side key
//! ([`output_key`], contract 4's `ColorKey`) for final frames, and the
//! source-side key ([`source_key_bytes`]) for source conversion nodes (how a
//! source is read: its full color description and the working encoding).

use pm_protocol::color::{
    AlphaMode, ChromaSiting, ColorDescription, ColorKey, MatrixCoefficients, OutputColor,
    Precision, Primaries, ProjectColor, Range, Subsampling, Transfer,
};

/// The output-side color key of a frame: the project's working encoding
/// and blend domain, the request's precision and output, and dither.
pub fn output_key(
    project: &ProjectColor,
    precision: Precision,
    output: OutputColor,
    dither: bool,
) -> ColorKey {
    ColorKey {
        working: project.working(),
        precision,
        blend_domain: project.blend_domain,
        output,
        dither,
    }
}

/// Version of the source key layout; bump when a field is added.
pub const SOURCE_KEY_VERSION: u8 = 1;

fn transfer_id(t: Transfer) -> u8 {
    match t {
        Transfer::DisplayE => 0,
        Transfer::Bt1886 => 1,
        Transfer::SceneReferredBt709 => 2,
        Transfer::Linear => 3,
        Transfer::Gamma22 => 4,
        Transfer::Pq => 5,
        Transfer::Hlg => 6,
    }
}

fn matrix_id(m: MatrixCoefficients) -> u8 {
    match m {
        MatrixCoefficients::Identity => 0,
        MatrixCoefficients::Bt709 => 1,
        MatrixCoefficients::Bt601 => 2,
        MatrixCoefficients::Bt2020Ncl => 3,
        MatrixCoefficients::Bt2020Cl => 4,
    }
}

fn subsampling_id(s: Subsampling) -> u8 {
    match s {
        Subsampling::S444 => 0,
        Subsampling::S422 => 1,
        Subsampling::S420 => 2,
    }
}

fn primaries_id(p: Primaries) -> u8 {
    p.h273()
}

fn siting_id(s: ChromaSiting) -> u8 {
    s.h273()
}

/// A stable positional encoding of how a source is read: every field of
/// its color description plus the project's working encoding and
/// precision. Identical on every target; changes when any field changes.
pub fn source_key_bytes(
    desc: &ColorDescription,
    project: &ProjectColor,
    precision: Precision,
) -> Vec<u8> {
    let mut out = vec![
        SOURCE_KEY_VERSION,
        primaries_id(desc.primaries),
        transfer_id(desc.transfer),
        matrix_id(desc.matrix),
        u8::from(desc.range == Range::Full),
        subsampling_id(desc.subsampling),
        siting_id(desc.siting),
        desc.bit_depth,
        u8::from(project.working() == pm_protocol::color::WorkingEncoding::LinearLight),
        match precision { Precision::Float16 => 0, Precision::Float32 => 1, Precision::Unorm8 => 2 },
    ];
    match desc.alpha {
        AlphaMode::Opaque => out.push(0),
        AlphaMode::Straight => out.push(1),
        AlphaMode::Premultiplied => out.push(2),
        AlphaMode::PremultipliedWithColor { matte } => {
            out.push(3);
            for c in matte {
                out.extend_from_slice(&c.to_bits().to_le_bytes());
            }
        }
    }
    out
}

/// Section 8 and amendment 8: exports default to 32-bit precision when the
/// output or any source exceeds 10 bits (16-bit Blender, PNG, and TIFF
/// surfaces included); otherwise the project's precision applies.
pub fn default_export_precision(
    project: Precision,
    output_bits: u8,
    deepest_source_bits: u8,
) -> Precision {
    if output_bits > 10 || deepest_source_bits > 10 {
        Precision::Float32
    } else {
        project
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_protocol::color::{BlendDomain, QuickTimeTag};

    #[test]
    fn output_keys_change_with_every_field() {
        let p = ProjectColor::DEFAULT;
        let base = output_key(&p, Precision::Float16, OutputColor::Rec709Broadcast, false);
        let variants = [
            output_key(&p, Precision::Float32, OutputColor::Rec709Broadcast, false),
            output_key(&p, Precision::Float16, OutputColor::SrgbImage, false),
            output_key(&p, Precision::Float16, OutputColor::Rec709Broadcast, true),
            output_key(
                &ProjectColor {
                    linear_blending: true,
                    ..p
                },
                Precision::Float16,
                OutputColor::Rec709Broadcast,
                false,
            ),
            output_key(
                &ProjectColor {
                    blend_domain: BlendDomain::DisplayEncoded,
                    ..p
                },
                Precision::Float16,
                OutputColor::Rec709Broadcast,
                false,
            ),
            output_key(
                &p,
                Precision::Float16,
                OutputColor::Rec709QuickTime {
                    candidate: QuickTimeTag::Cicp1_13_1,
                },
                false,
            ),
        ];
        for v in variants {
            assert_ne!(base.key_bytes(), v.key_bytes(), "{v:?}");
        }
        assert_eq!(
            base.key_bytes(),
            output_key(&p, Precision::Float16, OutputColor::Rec709Broadcast, false).key_bytes()
        );
    }

    #[test]
    fn source_keys_change_with_every_field() {
        let p = ProjectColor::DEFAULT;
        let d = ColorDescription::SRGB_RGBA8;
        let base = source_key_bytes(&d, &p, Precision::Float16);
        let changed = [
            ColorDescription {
                primaries: Primaries::DisplayP3,
                ..d
            },
            ColorDescription {
                transfer: Transfer::Bt1886,
                ..d
            },
            ColorDescription {
                matrix: MatrixCoefficients::Bt709,
                ..d
            },
            ColorDescription {
                range: Range::Limited,
                ..d
            },
            ColorDescription {
                subsampling: Subsampling::S420,
                ..d
            },
            ColorDescription {
                siting: ChromaSiting::Left,
                ..d
            },
            ColorDescription {
                alpha: AlphaMode::Premultiplied,
                ..d
            },
            ColorDescription {
                alpha: AlphaMode::PremultipliedWithColor { matte: [1.0; 3] },
                ..d
            },
            ColorDescription { bit_depth: 16, ..d },
        ];
        for c in changed {
            assert_ne!(base, source_key_bytes(&c, &p, Precision::Float16), "{c:?}");
        }
        assert_ne!(base, source_key_bytes(&d, &p, Precision::Float32));
        assert_ne!(base, source_key_bytes(&d, &p, Precision::Unorm8));
        assert_ne!(source_key_bytes(&d, &p, Precision::Float32), source_key_bytes(&d, &p, Precision::Unorm8));
        assert_eq!(base, source_key_bytes(&d, &p, Precision::Float16));
    }

    #[test]
    fn precision_default_rule() {
        assert_eq!(
            default_export_precision(Precision::Float16, 10, 8),
            Precision::Float16
        );
        assert_eq!(
            default_export_precision(Precision::Float16, 12, 8),
            Precision::Float32
        );
        assert_eq!(
            default_export_precision(Precision::Float16, 8, 16),
            Precision::Float32
        );
        assert_eq!(
            default_export_precision(Precision::Float32, 8, 8),
            Precision::Float32
        );
    }
}
