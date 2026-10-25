//! Color metadata (section 13 probe item 2) and its interpretation under
//! section 9's rules for sources that are untagged.

use crate::ff::CodecInfo;
use crate::interpret::Origin;
use crate::sys::ffmpeg as ff;
use pm_protocol::color::{
    AlphaMode, ChromaSiting, ColorDescription, MatrixCoefficients, Primaries, Range, Subsampling,
    Transfer,
};
use serde::Serialize;

/// What the file says, before interpretation (H.273 code points).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaggedColor {
    pub primaries: Option<u8>,
    pub transfer: Option<u8>,
    pub matrix: Option<u8>,
    pub full_range: Option<bool>,
    /// H.273 `chroma_sample_loc_type` (0 to 5).
    pub chroma_location: Option<u8>,
    /// Where the tags came from: "bitstream/container (FFmpeg)", "colr nclx", "colr nclc".
    pub source: Vec<String>,
}

/// A complete video colour reading as H.273 code points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoReading {
    pub primaries: u8,
    pub transfer: u8,
    pub matrix: u8,
    pub full_range: bool,
}

/// The one video colour table shared by the source monitor, composition and
/// export, for every codec: tags win field by field; an untagged matrix is
/// BT.601 when the frame is at most 576 lines tall and under 1280 wide,
/// otherwise BT.709 (the mpv/FFmpeg heuristic); untagged primaries follow
/// the BT.601 system an SD frame comes from (owner decision VIDEO-SD-1):
/// 576 active lines are the 625-line system's BT.470 System B/G (EBU)
/// primaries, other SD sizes the 525-line system's SMPTE 170M, HD and above
/// BT.709; an untagged transfer is
/// BT.709, read as the sRGB curve; the range is limited unless tagged full.
pub fn video_reading(tagged: &TaggedColor, width: u32, height: u32) -> VideoReading {
    let sd = height <= 576 && width < 1280;
    let sd_primaries = if height == 576 { 5 } else { 6 };
    VideoReading {
        primaries: tagged
            .primaries
            .unwrap_or(if sd { sd_primaries } else { 1 }),
        transfer: tagged.transfer.unwrap_or(1),
        matrix: tagged.matrix.unwrap_or(if sd { 6 } else { 1 }),
        full_range: tagged.full_range.unwrap_or(false),
    }
}

/// What a decoded frame's code values mean, for the source monitor,
/// composition and export alike: the probe's reading ([`interpret`], whose
/// untagged fields follow [`video_reading`]), except that an image-sequence
/// rendition's untagged matrix is BT.601 at any size and its untagged
/// primaries Rec.709. The importer converts sRGB stills with FFmpeg's BT.601
/// default; renditions written before it tagged them carry no colour tags.
pub fn frame_reading(color: &ColorInfo, sequence: bool) -> ColorDescription {
    let mut desc = color.interpreted;
    if sequence && color.tagged.matrix.is_none() {
        desc.matrix = MatrixCoefficients::Bt601;
    }
    if sequence && color.tagged.primaries.is_none() {
        desc.primaries = Primaries::Bt709;
    }
    desc
}

/// SMPTE ST 2086 mastering display, chromaticities as (x, y).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MasteringDisplay {
    pub red: Option<(f64, f64)>,
    pub green: Option<(f64, f64)>,
    pub blue: Option<(f64, f64)>,
    pub white: Option<(f64, f64)>,
    pub min_luminance: Option<f64>,
    pub max_luminance: Option<f64>,
}

/// CTA-861.3 content light level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentLight {
    pub max_cll: u32,
    pub max_fall: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum HdrFormat {
    Hdr10,
    Hlg,
    /// Dolby Vision; the profile and whether the base layer is
    /// HDR10- or HLG-compatible (section 9 decodes the base layer).
    DolbyVision {
        profile: u8,
        level: u8,
        base_compatibility: u8,
    },
}

/// The color section of a probe.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColorInfo {
    pub tagged: TaggedColor,
    /// Section 9's reading (Interpret Footage can override it).
    pub interpreted: ColorDescription,
    /// One line per rule that filled in an untagged value.
    pub defaults_applied: Vec<String>,
    /// Where each value of `interpreted` came from (Interpret Footage shows
    /// values with their origin).
    pub origins: ColorOrigins,
    pub mastering: Option<MasteringDisplay>,
    pub content_light: Option<ContentLight>,
    /// Size of an embedded ICC profile and its BLAKE3 hash.
    pub icc: Option<IccProfile>,
    pub hdr: Option<HdrFormat>,
}

/// The origin of each value of [`ColorInfo::interpreted`].
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColorOrigins {
    pub primaries: Origin,
    pub transfer: Origin,
    pub matrix: Origin,
    pub range: Origin,
    pub siting: Origin,
    pub alpha: Origin,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IccProfile {
    pub bytes: usize,
    pub blake3: String,
    #[serde(skip)]
    pub data: Vec<u8>,
}

fn tag<T: Into<i64>>(v: T, unspecified: &[i64]) -> Option<u8> {
    let v = v.into();
    (!unspecified.contains(&v) && (0..=255).contains(&v)).then_some(v as u8)
}

/// Reads tags from FFmpeg's codec parameters and the MOV `colr` atom, then
/// applies section 9 ("Per-source interpretation"):
///
/// * untagged matrix, primaries, transfer and range follow [`video_reading`],
///   the table the source monitor, composition and export share;
/// * MJPEG, `yuvj`, and RGB codecs are full range;
/// * untagged chroma siting is left (type 0);
/// * BT.709, BT.601, BT.2020 SDR, and sRGB transfers read as display encoding E.
pub fn interpret(
    info: &CodecInfo,
    colr: Option<&[u8]>,
    side: &SideData,
    alpha: Option<AlphaMode>,
) -> ColorInfo {
    let mut tagged = TaggedColor {
        primaries: tag(info.color_primaries, &[2, 0]),
        transfer: tag(info.color_trc, &[2, 0]),
        matrix: tag(info.color_space, &[2]),
        full_range: match info.color_range {
            ff::AVCOL_RANGE_JPEG => Some(true),
            ff::AVCOL_RANGE_MPEG => Some(false),
            _ => None,
        },
        chroma_location: match info.chroma_location {
            ff::AVCHROMA_LOC_UNSPECIFIED => None,
            // FFmpeg's enum is H.273's value + 1.
            v => Some((v - 1).clamp(0, 5) as u8),
        },
        source: vec![],
    };
    const FFMPEG: &str = "bitstream or container";
    if tagged.primaries.is_some() || tagged.transfer.is_some() || tagged.matrix.is_some() {
        tagged.source.push("bitstream or container (FFmpeg)".into());
    }
    let file = |s: &str| Origin::File { source: s.into() };
    let mut from = [
        tagged.primaries.map(|_| file(FFMPEG)),
        tagged.transfer.map(|_| file(FFMPEG)),
        tagged.matrix.map(|_| file(FFMPEG)),
        tagged.full_range.map(|_| file(FFMPEG)),
    ];
    // FFmpeg reports matrix 0 (RGB) for untagged RGB codecs too; only an
    // RGB pixel format keeps it.
    let rgb_format = info.pix_fmt.starts_with("gbr")
        || info.pix_fmt.starts_with("rgb")
        || info.pix_fmt.starts_with("bgr");
    if tagged.matrix == Some(0) && !rgb_format {
        tagged.matrix = None;
        from[2] = None;
    }
    if let Some(c) = colr
        && c.len() >= 10
    {
        let kind = &c[..4];
        if kind == b"nclx" || kind == b"nclc" {
            let p = u16::from_be_bytes([c[4], c[5]]);
            let t = u16::from_be_bytes([c[6], c[7]]);
            let m = u16::from_be_bytes([c[8], c[9]]);
            let atom = format!("colr {}", String::from_utf8_lossy(kind));
            let mut fill = |slot: &mut Option<u8>, i: usize, v: Option<u8>| {
                if slot.is_none() && v.is_some() {
                    *slot = v;
                    from[i] = Some(file(&atom));
                }
            };
            fill(&mut tagged.primaries, 0, tag(p, &[2, 0]));
            fill(&mut tagged.transfer, 1, tag(t, &[2, 0]));
            fill(
                &mut tagged.matrix,
                2,
                tag(m, &[2]).filter(|&m| m != 0 || rgb_format),
            );
            if kind == b"nclx" && c.len() >= 11 && tagged.full_range.is_none() {
                tagged.full_range = Some(c[10] & 0x80 != 0);
                from[3] = Some(file(&atom));
            }
            tagged
                .source
                .push(format!("colr {}", String::from_utf8_lossy(kind)));
        }
    }

    let mut defaults = Vec::new();
    let (w, h) = (info.width, info.height);
    let size = (w.max(0) as u32, h.max(0) as u32);
    let table = video_reading(&tagged, size.0, size.1);
    // The table's matrix for a matrix it cannot read (an untagged one).
    let sd = video_reading(&TaggedColor::default(), size.0, size.1).matrix == 6;
    let primaries = match tagged.primaries {
        Some(1) => Primaries::Bt709,
        Some(5) => Primaries::Bt601_625,
        Some(6) | Some(7) => Primaries::Bt601_525,
        Some(9) => Primaries::Bt2020,
        Some(12) => Primaries::DisplayP3,
        Some(_) => Primaries::Unspecified,
        None => {
            let p = match table.primaries {
                5 => Primaries::Bt601_625,
                6 => Primaries::Bt601_525,
                _ => Primaries::Bt709,
            };
            defaults.push(format!("untagged primaries: {p:?} ({w}×{h})"));
            from[0] = Some(Origin::Default {
                rule: match p {
                    Primaries::Bt709 => "Untagged video reads as Rec.709 primaries".into(),
                    Primaries::Bt601_625 => {
                        format!("Untagged {w}×{h} (625-line) video reads as BT.470BG primaries")
                    }
                    _ => format!("Untagged {w}×{h} (525-line) video reads as SMPTE 170M primaries"),
                },
            });
            p
        }
    };
    let transfer = match tagged.transfer {
        Some(1) | Some(6) | Some(13) | Some(14) | Some(15) => Transfer::DisplayE,
        Some(4) => Transfer::Gamma22,
        Some(8) => Transfer::Linear,
        Some(16) => Transfer::Pq,
        Some(18) => Transfer::Hlg,
        Some(_) => Transfer::DisplayE,
        None => {
            defaults.push("untagged transfer: display encoding E".into());
            from[1] = Some(Origin::Default {
                rule: "Untagged video reads with the display encoding (sRGB curve)".into(),
            });
            Transfer::DisplayE
        }
    };
    let matrix = if rgb_format {
        from[2] = Some(file("RGB pixel format"));
        MatrixCoefficients::Identity
    } else {
        match tagged.matrix {
            Some(1) => MatrixCoefficients::Bt709,
            Some(5) | Some(6) => MatrixCoefficients::Bt601,
            Some(9) => MatrixCoefficients::Bt2020Ncl,
            Some(10) => MatrixCoefficients::Bt2020Cl,
            _ => {
                let m = if sd {
                    MatrixCoefficients::Bt601
                } else {
                    MatrixCoefficients::Bt709
                };
                defaults.push(format!("untagged matrix: {m:?} ({w}×{h})"));
                from[2] = Some(Origin::Default {
                    rule: if sd {
                        format!("Untagged {w}×{h} video uses BT.601 coefficients")
                    } else {
                        "Untagged HD and larger video uses BT.709 coefficients".into()
                    },
                });
                m
            }
        }
    };
    let jpeg_like =
        info.codec == "mjpeg" || info.codec == "mjpegb" || info.pix_fmt.starts_with("yuvj");
    let range = if rgb_format || jpeg_like {
        if tagged.full_range != Some(true) {
            defaults.push(format!(
                "{} reads as full range",
                if rgb_format { "RGB" } else { "MJPEG/yuvj" }
            ));
            from[3] = Some(Origin::Default {
                rule: if rgb_format {
                    "RGB video is full range".into()
                } else {
                    "MJPEG video is full range".into()
                },
            });
        }
        Range::Full
    } else {
        match tagged.full_range {
            Some(true) => Range::Full,
            Some(false) => Range::Limited,
            None => {
                defaults.push("untagged range: limited".into());
                from[3] = Some(Origin::Default {
                    rule: "Untagged Y′CbCr video is limited range".into(),
                });
                Range::Limited
            }
        }
    };
    let subsampling = match (info.log2_chroma_w, info.log2_chroma_h) {
        (0, 0) => Subsampling::S444,
        (_, 0) => Subsampling::S422,
        _ => Subsampling::S420,
    };
    if info.log2_chroma_w == 2 {
        defaults.push("4:1:1 chroma has no contract 4 subsampling value; reported as 4:2:2 (the frame format carries the exact factors)".into());
    }
    let mut siting_origin = if tagged.chroma_location.is_some() {
        file(FFMPEG)
    } else if subsampling == Subsampling::S444 {
        Origin::Default {
            rule: "4:4:4 has no chroma siting".into(),
        }
    } else {
        Origin::Default {
            rule: "Untagged chroma is sited left (type 0)".into(),
        }
    };
    let siting = match tagged.chroma_location {
        Some(0) => ChromaSiting::Left,
        Some(1) => ChromaSiting::Center,
        Some(2) => ChromaSiting::TopLeft,
        Some(3) => ChromaSiting::Top,
        Some(4) => ChromaSiting::BottomLeft,
        Some(5) => ChromaSiting::Bottom,
        _ => {
            if subsampling != Subsampling::S444 {
                defaults.push("untagged chroma siting: left (type 0)".into());
            } else {
                siting_origin = Origin::Default {
                    rule: "4:4:4 has no chroma siting".into(),
                };
            }
            ChromaSiting::Left
        }
    };
    let alpha_origin = match alpha {
        Some(_) => file("alpha channel"),
        None => Origin::Default {
            rule: "No alpha channel".into(),
        },
    };
    let alpha = alpha.unwrap_or(AlphaMode::Opaque);
    let unknown = || Origin::Default {
        rule: "A code point the engine reads as the nearest supported value".into(),
    };
    let [p0, t0, m0, r0] = from;
    let origins = ColorOrigins {
        primaries: p0.unwrap_or_else(unknown),
        transfer: t0.unwrap_or_else(unknown),
        matrix: m0.unwrap_or_else(unknown),
        range: r0.unwrap_or_else(unknown),
        siting: siting_origin,
        alpha: alpha_origin,
    };
    let hdr = match (&side.dovi, transfer) {
        (Some(d), _) => Some(*d),
        (None, Transfer::Pq) => Some(HdrFormat::Hdr10),
        (None, Transfer::Hlg) => Some(HdrFormat::Hlg),
        _ => None,
    };
    ColorInfo {
        tagged,
        interpreted: ColorDescription {
            primaries,
            transfer,
            matrix,
            range,
            subsampling,
            siting,
            alpha,
            bit_depth: info.bit_depth,
        },
        defaults_applied: defaults,
        origins,
        mastering: side.mastering.clone(),
        content_light: side.content_light,
        icc: side.icc.as_ref().map(|d| IccProfile {
            bytes: d.len(),
            blake3: blake3::hash(d).to_hex().to_string(),
            data: d.clone(),
        }),
        hdr,
    }
}

/// Stream side data FFmpeg reads from the container or bitstream.
#[derive(Clone, Debug, Default)]
pub struct SideData {
    pub mastering: Option<MasteringDisplay>,
    pub content_light: Option<ContentLight>,
    pub icc: Option<Vec<u8>>,
    pub dovi: Option<HdrFormat>,
    /// Display matrix rotation in degrees (counterclockwise), when present.
    pub rotation: Option<f64>,
}

impl SideData {
    pub fn read(input: &crate::ff::Input, stream: usize) -> SideData {
        let mut s = SideData::default();
        unsafe {
            let par = input.par(stream);
            let get = |t: ff::AVPacketSideDataType| -> Option<&[u8]> {
                let sd = ff::av_packet_side_data_get(
                    (*par).coded_side_data,
                    (*par).nb_coded_side_data,
                    t,
                );
                (!sd.is_null() && !(*sd).data.is_null())
                    .then(|| std::slice::from_raw_parts((*sd).data, (*sd).size))
            };
            let q = |r: ff::AVRational| (r.den != 0).then(|| r.num as f64 / r.den as f64);
            if let Some(b) = get(ff::AV_PKT_DATA_MASTERING_DISPLAY_METADATA)
                && b.len() >= std::mem::size_of::<ff::AVMasteringDisplayMetadata>()
            {
                let m = &*(b.as_ptr() as *const ff::AVMasteringDisplayMetadata);
                let xy = |p: [ff::AVRational; 2]| Some((q(p[0])?, q(p[1])?));
                s.mastering = Some(MasteringDisplay {
                    red: if m.has_primaries != 0 {
                        xy(m.display_primaries[0])
                    } else {
                        None
                    },
                    green: if m.has_primaries != 0 {
                        xy(m.display_primaries[1])
                    } else {
                        None
                    },
                    blue: if m.has_primaries != 0 {
                        xy(m.display_primaries[2])
                    } else {
                        None
                    },
                    white: if m.has_primaries != 0 {
                        xy(m.white_point)
                    } else {
                        None
                    },
                    min_luminance: if m.has_luminance != 0 {
                        q(m.min_luminance)
                    } else {
                        None
                    },
                    max_luminance: if m.has_luminance != 0 {
                        q(m.max_luminance)
                    } else {
                        None
                    },
                });
            }
            if let Some(b) = get(ff::AV_PKT_DATA_CONTENT_LIGHT_LEVEL)
                && b.len() >= std::mem::size_of::<ff::AVContentLightMetadata>()
            {
                let c = &*(b.as_ptr() as *const ff::AVContentLightMetadata);
                s.content_light = Some(ContentLight {
                    max_cll: c.MaxCLL,
                    max_fall: c.MaxFALL,
                });
            }
            if let Some(b) = get(ff::AV_PKT_DATA_ICC_PROFILE) {
                s.icc = Some(b.to_vec());
            }
            if let Some(b) = get(ff::AV_PKT_DATA_DOVI_CONF)
                && b.len() >= 8
            {
                // AVDOVIDecoderConfigurationRecord: version major/minor (u8 each),
                // profile (u8), level (u8), rpu/el/bl present (u8 each),
                // dv_bl_signal_compatibility_id (u8).
                s.dovi = Some(HdrFormat::DolbyVision {
                    profile: b[2],
                    level: b[3],
                    base_compatibility: b[7],
                });
            }
            if let Some(b) = get(ff::AV_PKT_DATA_DISPLAYMATRIX)
                && b.len() >= 36
            {
                let r = ff::av_display_rotation_get(b.as_ptr() as *const i32);
                if r.is_finite() {
                    s.rotation = Some(r);
                }
            }
        }
        s
    }
}

#[cfg(test)]
mod reading_tests {
    use super::*;

    #[test]
    fn video_reading_applies_the_shared_table() {
        let untagged = TaggedColor::default();
        // PAL: 625-line BT.470BG primaries; NTSC and other SD: 170M.
        let sd = video_reading(&untagged, 720, 576);
        assert_eq!(
            (sd.primaries, sd.transfer, sd.matrix, sd.full_range),
            (5, 1, 6, false)
        );
        for (w, h) in [(720, 480), (720, 486), (640, 360), (256, 144)] {
            assert_eq!(video_reading(&untagged, w, h).primaries, 6, "{w}×{h}");
        }
        assert_eq!(video_reading(&untagged, 1280, 576).primaries, 1);
        assert_eq!(video_reading(&untagged, 1280, 576).matrix, 1);
        assert_eq!(video_reading(&untagged, 480, 720).matrix, 1);
        let partial = TaggedColor {
            matrix: Some(1),
            full_range: Some(true),
            ..TaggedColor::default()
        };
        let read = video_reading(&partial, 320, 180);
        assert_eq!(
            (read.primaries, read.transfer, read.matrix, read.full_range),
            (6, 1, 1, true)
        );
        let tagged = TaggedColor {
            primaries: Some(6),
            transfer: Some(6),
            matrix: Some(5),
            ..TaggedColor::default()
        };
        let read = video_reading(&tagged, 1920, 1080);
        assert_eq!((read.primaries, read.transfer, read.matrix), (6, 6, 5));
    }
}

#[cfg(test)]
mod interpret_tests {
    use super::*;

    /// A probed stream's codec facts with chosen size and tags (FFmpeg enum
    /// values: 2 is unspecified).
    fn info(codec: &str, w: i32, h: i32, tags: [i32; 3]) -> CodecInfo {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../apps/desktop/e2e/fixtures/h264-aac.mp4");
        let input = crate::ff::Input::open(&path).unwrap();
        let mut info = input.codec_info(input.best_video().unwrap());
        info.codec = codec.into();
        (info.width, info.height) = (w, h);
        info.color_primaries = tags[0] as _;
        info.color_trc = tags[1] as _;
        info.color_space = tags[2] as _;
        info.color_range = ff::AVCOL_RANGE_UNSPECIFIED;
        info
    }

    #[test]
    fn the_probe_reading_is_the_shared_table_for_every_codec() {
        use MatrixCoefficients::{Bt601, Bt709};
        for codec in ["h264", "hevc", "vp9", "prores", "av1"] {
            for (w, h, tags, matrix) in [
                (720, 480, [2, 2, 2], Bt601),
                (1024, 576, [2, 2, 2], Bt601),
                (1280, 576, [2, 2, 2], Bt709),
                (960, 640, [1, 1, 2], Bt709),
                (640, 480, [1, 1, 2], Bt601),
                (256, 144, [2, 2, 1], Bt709),
                (1920, 1080, [2, 2, 2], Bt709),
            ] {
                let c = interpret(&info(codec, w, h, tags), None, &SideData::default(), None);
                let table = video_reading(&c.tagged, w as u32, h as u32);
                let i = c.interpreted;
                assert_eq!(i.matrix, matrix, "{codec} {w}×{h} {tags:?}");
                assert_eq!(table.matrix, if matrix == Bt601 { 6 } else { 1 });
                assert_eq!(i.transfer, Transfer::DisplayE);
                assert_eq!(i.range, Range::Limited);
                let primaries = match (tags[0], h <= 576 && w < 1280, h) {
                    (2, true, 576) => Primaries::Bt601_625,
                    (2, true, _) => Primaries::Bt601_525,
                    _ => Primaries::Bt709,
                };
                assert_eq!(
                    i.primaries, primaries,
                    "VIDEO-SD-1 {codec} {w}×{h} {tags:?}"
                );
                assert_eq!(
                    frame_reading(&c, true).matrix,
                    if tags[2] == 2 { Bt601 } else { matrix }
                );
                assert_eq!(frame_reading(&c, true).primaries, Primaries::Bt709);
            }
        }
    }
}
