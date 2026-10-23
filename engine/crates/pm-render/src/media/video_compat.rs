//! Chromium's source reading is distinct from Interpret Footage's defaults.
//! Apply only to unoverridden level 1 SDR video, never to level 2 or a
//! sequence's separately preserved persisted-rendition reading.
use pm_media::probe::VideoProbe;
use pm_protocol::color::{ColorDescription, MatrixCoefficients, Range};

pub(super) fn bt2020_upload(video: &VideoProbe) -> bool {
    let tags = &video.color.tagged;
    video.codec.codec == "h264"
        && video.codec.bit_depth == 8
        && tags.primaries == Some(9)
        && tags.transfer == Some(1)
        && tags.matrix == Some(9)
        && tags.full_range == Some(false)
}

pub(super) fn color(video: &VideoProbe, mut desc: ColorDescription) -> ColorDescription {
    let tags = &video.color.tagged;
    // An incomplete SD H.264 color description is invalid to Chromium's
    // hardware frame upload. Its Skia fallback is limited Rec.601, even
    // when a lone matrix tag says Rec.709. Keep fully specified color tags.
    if video.codec.codec == "h264"
        && video.height <= 576
        && desc.bit_depth == 8
        && desc.range == Range::Limited
        && tags.primaries.is_none()
        && tags.transfer.is_none()
    {
        desc.matrix = MatrixCoefficients::Bt601;
    }
    // Opaque VP9's default Chromium hardware reading is Rec.709 at SD as
    // well as HD. The native source interpretation intentionally defaults
    // untagged SD to Rec.601; that remains the level 2 / Interpret reading.
    if video.codec.codec == "vp9" && tags.matrix.is_none() {
        desc.matrix = MatrixCoefficients::Bt709;
    }
    desc
}
