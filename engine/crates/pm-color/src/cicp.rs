//! Output tags and H.273 code points (section 9, "Output tags"; amendment 7).
//!
//! Every output writes ITU-T H.273 code points (VUI, `colr nclx`, PNG
//! `cICP`, WebM Colour). Amendment 7 settles "Rec.709 (QuickTime and
//! Safari)" as CICP 1-13-1, limited range: S2 measured 1-13-1 passing bar
//! (3) and 1-2-1 with a gamma 2.2 tag failing it, so the losing candidate
//! the draft contract still carries is refused here rather than written.

use pm_protocol::color::{
    Cicp, ColorDescription, MatrixCoefficients, OutputColor, Primaries, QuickTimeTag, Range,
    Transfer,
};

/// The tag amendment 7 chose for "Rec.709 (QuickTime and Safari)".
pub const QUICKTIME_TAG: QuickTimeTag = QuickTimeTag::Cicp1_13_1;

/// Why an output has no tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagError {
    /// The rejected S2 candidate (CICP 1-2-1 with a gamma 2.2 tag).
    RejectedQuickTimeCandidate,
}

impl std::fmt::Display for TagError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "CICP 1-2-1 with gamma 2.2 failed S2's QuickTime bar; amendment 7 uses 1-13-1"
        )
    }
}

impl std::error::Error for TagError {}

/// The CICP an output writes, or `None` for the viewer (no tag; working
/// values are encoded for the canvas). The QuickTime output always resolves
/// to amendment 7's 1-13-1.
pub fn output_tag(output: OutputColor) -> Result<Option<Cicp>, TagError> {
    match output {
        OutputColor::Rec709QuickTime {
            candidate: QuickTimeTag::Cicp1_2_1Gamma22,
        } => Err(TagError::RejectedQuickTimeCandidate),
        other => Ok(other.cicp()),
    }
}

/// The QuickTime output as amendment 7 defines it.
pub const fn quicktime_output() -> OutputColor {
    OutputColor::Rec709QuickTime {
        candidate: QUICKTIME_TAG,
    }
}

/// How an output's codes relate to the working values: the transfer the
/// codes are in and the range. Every Standard output before cutover is
/// E-encoded (sRGB and Rec.709 share E by default), whatever its tag says;
/// "Rec.709 (broadcast)" is E-encoded and tagged 1-1-1, which is why it
/// looks darker than the viewer on a BT.1886 monitor, by design.
pub fn output_encoding(output: OutputColor) -> (Transfer, Range) {
    let range = match output.cicp() {
        Some(c) if !c.full_range => Range::Limited,
        _ => Range::Full,
    };
    (Transfer::DisplayE, range)
}

/// H.273 `ColourPrimaries` to the contract's primaries.
pub fn primaries(code: u8) -> Primaries {
    match code {
        1 => Primaries::Bt709,
        5 => Primaries::Bt601_625,
        6 | 7 => Primaries::Bt601_525,
        9 => Primaries::Bt2020,
        12 => Primaries::DisplayP3,
        _ => Primaries::Unspecified,
    }
}

/// H.273 `TransferCharacteristics` to how Standard reads it. The Rec.709
/// family (1, 6, 14, 15) and sRGB (13) read as E, the section 9 default for
/// sRGB and Rec.709 sources; `None` means unspecified (interpretation rules
/// decide).
pub fn transfer(code: u8) -> Option<Transfer> {
    match code {
        1 | 6 | 13 | 14 | 15 => Some(Transfer::DisplayE),
        4 => Some(Transfer::Gamma22),
        8 => Some(Transfer::Linear),
        16 => Some(Transfer::Pq),
        18 => Some(Transfer::Hlg),
        _ => None,
    }
}

/// H.273 `MatrixCoefficients`; `None` is unspecified.
pub fn matrix(code: u8) -> Option<MatrixCoefficients> {
    match code {
        0 => Some(MatrixCoefficients::Identity),
        1 => Some(MatrixCoefficients::Bt709),
        5 | 6 => Some(MatrixCoefficients::Bt601),
        9 => Some(MatrixCoefficients::Bt2020Ncl),
        10 => Some(MatrixCoefficients::Bt2020Cl),
        _ => None,
    }
}

/// The H.273 transfer code a description writes (E writes 13 for full-range
/// RGB images and 1 for Rec.709 video unless the output says otherwise).
pub fn transfer_code(transfer: Transfer) -> u8 {
    match transfer {
        Transfer::DisplayE => 13,
        Transfer::Bt1886 | Transfer::SceneReferredBt709 => 1,
        Transfer::Linear => 8,
        Transfer::Gamma22 => 4,
        Transfer::Pq => 16,
        Transfer::Hlg => 18,
    }
}

/// The H.273 matrix code of a description.
pub fn matrix_code(matrix: MatrixCoefficients) -> u8 {
    match matrix {
        MatrixCoefficients::Identity => 0,
        MatrixCoefficients::Bt709 => 1,
        MatrixCoefficients::Bt601 => 6,
        MatrixCoefficients::Bt2020Ncl => 9,
        MatrixCoefficients::Bt2020Cl => 10,
    }
}

/// The CICP of a source description, for diagnostics and pass-through
/// checks against an output's tag.
pub fn of_description(desc: &ColorDescription) -> Cicp {
    Cicp {
        primaries: desc.primaries.h273(),
        transfer: transfer_code(desc.transfer),
        matrix: matrix_code(desc.matrix),
        full_range: desc.range == Range::Full,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_protocol::color::ViewerColorSpace;

    fn tuple(c: Cicp) -> (u8, u8, u8, bool) {
        (c.primaries, c.transfer, c.matrix, c.full_range)
    }

    #[test]
    fn section_9_table_with_amendment_7() {
        let tag = |o| tuple(output_tag(o).unwrap().unwrap());
        assert_eq!(tag(OutputColor::SrgbImage), (1, 13, 0, true));
        assert_eq!(tag(OutputColor::Rec709Broadcast), (1, 1, 1, false));
        assert_eq!(tag(OutputColor::Rec709Image), (1, 1, 0, true));
        assert_eq!(tag(quicktime_output()), (1, 13, 1, false));
        assert_eq!(
            output_tag(OutputColor::Rec709QuickTime {
                candidate: QuickTimeTag::Cicp1_2_1Gamma22
            }),
            Err(TagError::RejectedQuickTimeCandidate)
        );
        assert_eq!(
            output_tag(OutputColor::Viewer {
                space: ViewerColorSpace::Srgb
            }),
            Ok(None)
        );
    }

    #[test]
    fn outputs_are_e_encoded_with_their_range() {
        assert_eq!(
            output_encoding(OutputColor::Rec709Broadcast),
            (Transfer::DisplayE, Range::Limited)
        );
        assert_eq!(
            output_encoding(OutputColor::SrgbImage),
            (Transfer::DisplayE, Range::Full)
        );
        assert_eq!(
            output_encoding(quicktime_output()),
            (Transfer::DisplayE, Range::Limited)
        );
    }

    #[test]
    fn reading_code_points() {
        assert_eq!(primaries(1), Primaries::Bt709);
        assert_eq!(primaries(2), Primaries::Unspecified);
        assert_eq!(transfer(1), Some(Transfer::DisplayE));
        assert_eq!(transfer(13), Some(Transfer::DisplayE));
        assert_eq!(transfer(16), Some(Transfer::Pq));
        assert_eq!(transfer(2), None);
        assert_eq!(matrix(6), Some(MatrixCoefficients::Bt601));
        assert_eq!(matrix(10), Some(MatrixCoefficients::Bt2020Cl));
        assert_eq!(matrix_code(MatrixCoefficients::Bt2020Cl), 10);
        assert_eq!(matrix(2), None);
        let d = ColorDescription::SRGB_RGBA8;
        assert_eq!(tuple(of_description(&d)), (1, 13, 0, true));
    }
}
