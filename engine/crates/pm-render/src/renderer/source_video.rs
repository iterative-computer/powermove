//! Coded source planes for the source monitor's video presenter. Chromium's
//! compositor scales them and applies the colour tags given here, which are
//! the shared reading (`pm_media::probe::color::frame_reading`) composition
//! and export use. A reading Chromium cannot reproduce exactly is converted
//! by the engine instead (`None`).

use std::path::Path;

use pm_media::frame::{Layout, PixelFormat, PlaneFormat, PlaneView};
use pm_media::probe::color::frame_reading;
use pm_protocol::color::{
    ColorDescription, MatrixCoefficients, Primaries, Range, SampleAlignment, Transfer,
};
use pm_protocol::common::{Size, Ticks};
use pm_protocol::frame::MAX_OUTPUT_DIMENSION;
use pm_protocol::snapshot::SourceVideoFrame;

use super::Renderer;

/// Rejects a single coded frame before allocating its packed planes.
const MAX_PACKED_SOURCE_VIDEO_BYTES: usize = 128 << 20;

impl Renderer {
    /// Return the actual coded dimensions, tight YUV(A) planes, and the
    /// frame's colour tags. `outputSize`/fit are presentation geometry and
    /// deliberately do not resample these samples. `Ok(None)`: Chromium cannot
    /// apply this frame's reading exactly; draw it with `source_rgba8_composed`.
    pub fn source_video_planes(
        &mut self,
        path: &Path,
        time: Ticks,
        sequence: bool,
    ) -> Result<Option<(Size, Vec<u8>, SourceVideoFrame)>, String> {
        self.source_video_planes_with(path, time, sequence, u64::MAX, |length, pack| {
            let mut bytes = vec![0; length];
            pack(&mut bytes).map(|()| bytes)
        })
    }

    /// [`Self::source_video_planes`], packing in place: `sink` is given the
    /// packed length and a packer that writes it into zeroed storage the sink
    /// owns (for example a shared-memory segment), so the decoded frame is
    /// copied once. The packer runs while the decoded frame is locked. Nothing
    /// fails after a successful sink, so whatever it published is returned.
    /// A stream with more than `max_pixels` coded pixels is `Ok(None)` before
    /// decoding: the engine resamples it for the monitor instead of sending
    /// planes many times larger than what is shown.
    pub fn source_video_planes_with<T>(
        &mut self,
        path: &Path,
        time: Ticks,
        sequence: bool,
        max_pixels: u64,
        sink: impl FnOnce(usize, &mut dyn FnMut(&mut [u8]) -> Result<(), String>) -> Result<T, String>,
    ) -> Result<Option<(Size, T, SourceVideoFrame)>, String> {
        let tags = |source: &pm_media::decode::service::MediaSource| {
            source
                .probe
                .video
                .as_ref()
                .and_then(|video| chromium_tags(&frame_reading(&video.color, sequence)))
        };
        // A per-file colour override is composition's reading, not the probe's.
        if self.media.overrides_color(path) {
            return Ok(None);
        }
        let Some((source, frame)) = self
            .media
            .source_exact_decoded(path, time, |source| {
                tags(source).is_some()
                    && source.probe.video.as_ref().is_some_and(|video| {
                        u64::from(video.width) * u64::from(video.height) <= max_pixels
                    })
            })
            .map_err(|e| match e {
                crate::media::MediaFailure::Unsupported(feature) => feature.to_string(),
                crate::media::MediaFailure::Failed(message) => message,
            })?
        else {
            return Ok(None);
        };
        let video = source
            .probe
            .video
            .as_ref()
            .ok_or("The source has no video stream")?;
        let (primaries, transfer, matrix) = tags(&source).ok_or("The source reading changed")?;
        let visible_width =
            u32::try_from(frame.format.width).map_err(|_| "The visible width is too large")?;
        let coded_width = if video.codec.codec == "vp9" {
            vp9_coded_width(visible_width)?
        } else {
            visible_width
        };
        let size = Size {
            width: coded_width,
            height: u32::try_from(frame.format.height)
                .map_err(|_| "The coded height is too large")?,
        };
        if size.width > MAX_OUTPUT_DIMENSION || size.height > MAX_OUTPUT_DIMENSION {
            return Err("The coded source video exceeds the snapshot dimension limit".into());
        }
        let Some(input_format) = packed_format(&frame.format)? else {
            // Original production formats (for example ProRes v216/y416)
            // retain their precision through the native conversion path.
            return Ok(None);
        };
        frame
            .with_planes(|planes| {
                let format = if input_format == "i420a"
                    && video.codec.codec != "vp9"
                    && alpha_is_opaque(planes, frame.format.width, frame.format.height)?
                {
                    "nv12"
                } else {
                    input_format
                };
                let tags = SourceVideoFrame {
                    format: format.into(),
                    primaries: primaries.into(),
                    transfer: transfer.into(),
                    matrix: matrix.into(),
                    // The decoded planes' own range: what these bytes mean.
                    range: if frame.format.range == Range::Full {
                        "full"
                    } else {
                        "limited"
                    }
                    .into(),
                    visible_width: (coded_width != visible_width).then_some(visible_width),
                };
                let length = packed_length(&frame.format, planes, format, coded_width as usize)?;
                if Some(length as u64) != tags.byte_len(size.width, size.height) {
                    return Err("The decoded source planes have an unexpected length".into());
                }
                let mut pack = |out: &mut [u8]| {
                    pack_planes(&frame.format, planes, format, coded_width as usize, out)
                };
                sink(length, &mut pack).map(|packed| Some((size, packed, tags)))
            })
            .map_err(|e| e.to_string())?
    }
}

/// libvpx's VP9 Y stride: round the visible width to eight, add its 32-pixel
/// border on each side, then align the row to 32 bytes. Chrome uses this
/// stride as VideoFrame.codedWidth while retaining the visible rectangle.
/// https://chromium.googlesource.com/chromium/src/+/refs/tags/152.0.7977.54/media/filters/vpx_video_decoder.cc#531
fn vp9_coded_width(width: u32) -> Result<u32, String> {
    let aligned = width.checked_add(7).ok_or("VP9 coded width overflow")? & !7;
    let bordered = aligned.checked_add(64).ok_or("VP9 coded width overflow")?;
    let coded = bordered.checked_add(31).ok_or("VP9 coded width overflow")? & !31;
    if coded > MAX_OUTPUT_DIMENSION {
        return Err("The VP9 coded row exceeds the snapshot dimension limit".into());
    }
    Ok(coded)
}

/// Check descriptors before deciding whether presentation needs native RGBA.
/// An unsupported layout is a fallback; a broken layout is a decoder error.
fn validate_source_planes(f: &PixelFormat) -> Result<(), String> {
    let invalid = || format!("The decoded source {} has malformed planes", f.label);
    if f.width == 0 || f.height == 0 || f.bits == 0 {
        return Err(invalid());
    }
    let count = match f.layout {
        Layout::Biplanar => 2,
        Layout::BiplanarAlpha => 3,
        Layout::Planar | Layout::PlanarRgb => 3 + usize::from(f.has_alpha()),
        Layout::Ayuv16 | Layout::Cbycry16 | Layout::Rgba => 1,
    };
    if f.planes.len() != count {
        return Err(invalid());
    }
    let chroma = || -> Result<(usize, usize), String> {
        let x = 1usize
            .checked_shl(u32::from(f.log2_chroma_w))
            .ok_or_else(invalid)?;
        let y = 1usize
            .checked_shl(u32::from(f.log2_chroma_h))
            .ok_or_else(invalid)?;
        Ok((f.width.div_ceil(x), f.height.div_ceil(y)))
    };
    let scalar = if f.bits <= 8 {
        PlaneFormat::R8
    } else {
        PlaneFormat::R16
    };
    for (i, plane) in f.planes.iter().enumerate() {
        let expected = match f.layout {
            Layout::Biplanar | Layout::BiplanarAlpha if i == 1 => {
                let (w, h) = chroma()?;
                (
                    if f.bits <= 8 {
                        PlaneFormat::Rg8
                    } else {
                        PlaneFormat::Rg16
                    },
                    w,
                    h,
                )
            }
            Layout::BiplanarAlpha if i == 2 => (PlaneFormat::R8, f.width, f.height),
            Layout::Planar if i == 1 || i == 2 => {
                let (w, h) = chroma()?;
                (scalar, w, h)
            }
            Layout::Ayuv16 => (PlaneFormat::Rgba16, f.width, f.height),
            Layout::Cbycry16 => (PlaneFormat::Rgba16, f.width.div_ceil(2), f.height),
            Layout::Rgba => {
                let valid = matches!(
                    (plane.format, f.bits),
                    (PlaneFormat::Rgba8, 1..=8)
                        | (PlaneFormat::Rgba16, 9..=16)
                        | (PlaneFormat::Rgba16Float, 16)
                        | (PlaneFormat::Rgba32Float, 32)
                );
                if !valid {
                    return Err(invalid());
                }
                (plane.format, f.width, f.height)
            }
            _ => (scalar, f.width, f.height),
        };
        if (plane.format, plane.width, plane.height) != expected {
            return Err(invalid());
        }
    }
    Ok(())
}

fn packed_format(f: &PixelFormat) -> Result<Option<&'static str>, String> {
    validate_source_planes(f)?;
    if f.log2_chroma_w != 1 {
        return Ok(None);
    }
    let supported = match (f.layout, f.bits, f.alignment, f.log2_chroma_h) {
        (Layout::Biplanar, 8, _, 1) => Some("nv12"),
        (Layout::Biplanar, 8, _, 0) => Some("i422"),
        (Layout::Planar, 8, _, 1) if !f.has_alpha() => Some("nv12"),
        (Layout::Planar, 8, _, 1) if f.alpha_bits == 8 => Some("i420a"),
        (Layout::Planar, 8, _, 0) if !f.has_alpha() => Some("i422"),
        (Layout::Biplanar, 10, SampleAlignment::MsbIn16, 1) => Some("i420p10"),
        (Layout::Planar, 10, SampleAlignment::Lsb, 1) if !f.has_alpha() => Some("i420p10"),
        _ => None,
    };
    Ok(supported)
}

/// Plane byte lengths of the packed `format`: (Y, one chroma plane, total).
fn packed_lengths(
    source: &PixelFormat,
    format: &str,
    coded_width: usize,
) -> Result<(usize, usize, usize), String> {
    let width = coded_width;
    if width < source.width {
        return Err("The coded source row is narrower than its visible pixels".into());
    }
    let height = source.height;
    let chroma_height = if format == "i422" {
        height
    } else {
        height.div_ceil(2)
    };
    let sample_bytes = if format == "i420p10" { 2 } else { 1 };
    let y_bytes = width
        .checked_mul(height)
        .and_then(|n| n.checked_mul(sample_bytes))
        .ok_or("Source video is too large")?;
    let chroma_bytes = width
        .div_ceil(2)
        .checked_mul(chroma_height)
        .and_then(|n| n.checked_mul(sample_bytes))
        .ok_or("Source video is too large")?;
    let alpha_bytes = if format == "i420a" {
        if source.layout != Layout::Planar || source.alpha_bits != 8 {
            return Err("I420A needs 8-bit planar source alpha".into());
        }
        y_bytes
    } else {
        0
    };
    let total = chroma_bytes
        .checked_mul(2)
        .and_then(|n| n.checked_add(y_bytes))
        .and_then(|n| n.checked_add(alpha_bytes))
        .ok_or("Source video is too large")?;
    if total > MAX_PACKED_SOURCE_VIDEO_BYTES {
        return Err("The coded source video exceeds the source byte limit".into());
    }
    Ok((y_bytes, chroma_bytes, total))
}

/// The packed length, once the frame is known to be presentable as `format`.
fn packed_length(
    source: &PixelFormat,
    planes: &[PlaneView<'_>],
    format: &str,
    coded_width: usize,
) -> Result<usize, String> {
    let (.., total) = packed_lengths(source, format, coded_width)?;
    if source.has_alpha()
        && format != "i420a"
        && !alpha_is_opaque(planes, source.width, source.height)?
    {
        return Err("The source video has translucent alpha; it needs an I420A presenter".into());
    }
    Ok(total)
}

/// Packs the decoded planes into `out` (zeroed, `packed_length` bytes) in a
/// WebCodecs buffer layout, rows padded to the coded width: `nv12` (Y, then
/// interleaved CbCr), or planar `i420a`, `i422` and `i420p10` (16-bit
/// little-endian samples holding the low ten bits).
fn pack_planes(
    source: &PixelFormat,
    planes: &[PlaneView<'_>],
    format: &str,
    coded_width: usize,
    out: &mut [u8],
) -> Result<(), String> {
    let (y_bytes, chroma_bytes, total) = packed_lengths(source, format, coded_width)?;
    if out.len() != total {
        return Err("The packed source planes have an unexpected length".into());
    }
    let width = coded_width;
    let height = source.height;
    let chroma_height = if format == "i422" {
        height
    } else {
        height.div_ceil(2)
    };
    let sample_bytes = if format == "i420p10" { 2 } else { 1 };
    let visible_bytes = source.width.div_ceil(2) * sample_bytes;
    let row_bytes = width.div_ceil(2) * sample_bytes;
    let (luma, rest) = out.split_at_mut(y_bytes);
    copy_rows_padded(
        &planes[0],
        source.width * sample_bytes,
        width * sample_bytes,
        height,
        luma,
    )?;
    let (chroma, alpha) = rest.split_at_mut(2 * chroma_bytes);
    match (format, source.layout) {
        ("nv12", Layout::Biplanar) => copy_rows_padded(
            &planes[1],
            2 * visible_bytes,
            2 * row_bytes,
            chroma_height,
            chroma,
        )?,
        ("nv12", Layout::Planar) => {
            for row in 0..chroma_height {
                let u = visible_row(&planes[1], row, visible_bytes)?;
                let v = visible_row(&planes[2], row, visible_bytes)?;
                let dst = &mut chroma[row * 2 * row_bytes..];
                for (col, (u, v)) in u.iter().zip(v).enumerate() {
                    dst[2 * col] = *u;
                    dst[2 * col + 1] = *v;
                }
            }
        }
        (_, Layout::Planar) => {
            let (u, v) = chroma.split_at_mut(chroma_bytes);
            copy_rows_padded(&planes[1], visible_bytes, row_bytes, chroma_height, u)?;
            copy_rows_padded(&planes[2], visible_bytes, row_bytes, chroma_height, v)?;
        }
        (_, Layout::Biplanar) => {
            let (u, v) = chroma.split_at_mut(chroma_bytes);
            for row in 0..chroma_height {
                let uv = visible_row(&planes[1], row, 2 * visible_bytes)?;
                for (col, pair) in uv.chunks_exact(2 * sample_bytes).enumerate() {
                    let at = row * row_bytes + col * sample_bytes;
                    u[at..at + sample_bytes].copy_from_slice(&pair[..sample_bytes]);
                    v[at..at + sample_bytes].copy_from_slice(&pair[sample_bytes..]);
                }
            }
        }
        _ => unreachable!("packed_format checked the plane layout"),
    }
    if format == "i420a" {
        copy_rows_padded(&planes[3], source.width, width, height, alpha)?;
    }
    if sample_bytes == 2 && source.alignment == SampleAlignment::MsbIn16 {
        for sample in out.chunks_exact_mut(2) {
            let code = u16::from_le_bytes([sample[0], sample[1]]) >> 6;
            sample.copy_from_slice(&code.to_le_bytes());
        }
    }
    Ok(())
}

fn alpha_is_opaque(planes: &[PlaneView<'_>], width: usize, height: usize) -> Result<bool, String> {
    let alpha = planes
        .get(3)
        .ok_or("The decoded source alpha plane is unavailable")?;
    for row in 0..height {
        if visible_row(alpha, row, width)?
            .iter()
            .any(|sample| *sample != 255)
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn copy_rows_padded(
    plane: &PlaneView<'_>,
    visible_row_bytes: usize,
    row_bytes: usize,
    rows: usize,
    dst: &mut [u8],
) -> Result<(), String> {
    if plane.desc.height != rows
        || plane.desc.width * plane.desc.format.bytes() != visible_row_bytes
        || visible_row_bytes > row_bytes
        || dst.len() != row_bytes * rows
    {
        return Err("The decoded source plane has unexpected dimensions".into());
    }
    for row in 0..rows {
        dst[row * row_bytes..row * row_bytes + visible_row_bytes].copy_from_slice(visible_row(
            plane,
            row,
            visible_row_bytes,
        )?);
    }
    Ok(())
}

fn visible_row<'a>(plane: &'a PlaneView<'_>, row: usize, bytes: usize) -> Result<&'a [u8], String> {
    if row >= plane.desc.height || bytes > plane.stride || plane.ptr.is_null() {
        return Err("The decoded source plane is unavailable".into());
    }
    // with_planes holds a read-only lock for the entire packing operation.
    Ok(unsafe { std::slice::from_raw_parts(plane.ptr.add(row * plane.stride), bytes) })
}

/// The WebCodecs/gfx tags under which Chromium computes exactly what the
/// engine does for `desc`, or `None`. SDR video transfers the engine reads
/// as the display encoding E are what Chromium renders for `bt709`. Wide
/// gamut, HDR and other curves convert in the engine like composition, so
/// the monitor clips and tone-maps as the project does.
fn chromium_tags(desc: &ColorDescription) -> Option<(&'static str, &'static str, &'static str)> {
    let primaries = match desc.primaries {
        Primaries::Bt709 => "bt709",
        Primaries::Bt601_525 => "smpte170m",
        Primaries::Bt601_625 => "bt470bg",
        Primaries::Bt2020 | Primaries::DisplayP3 | Primaries::Unspecified => return None,
    };
    let transfer = match desc.transfer {
        Transfer::DisplayE => "bt709",
        Transfer::Linear => "linear",
        Transfer::Bt1886
        | Transfer::SceneReferredBt709
        | Transfer::Gamma22
        | Transfer::Pq
        | Transfer::Hlg => return None,
    };
    let matrix = match desc.matrix {
        MatrixCoefficients::Bt709 => "bt709",
        MatrixCoefficients::Bt601 => "smpte170m",
        MatrixCoefficients::Bt2020Ncl => "bt2020-ncl",
        MatrixCoefficients::Identity | MatrixCoefficients::Bt2020Cl => return None,
    };
    Some((primaries, transfer, matrix))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a sink sees: the checked length, then the frame packed into zeroed storage.
    fn packed(
        source: &PixelFormat,
        planes: &[PlaneView<'_>],
        format: &str,
        coded_width: usize,
    ) -> Result<Vec<u8>, String> {
        let mut out = vec![0; packed_length(source, planes, format, coded_width)?];
        pack_planes(source, planes, format, coded_width, &mut out).map(|()| out)
    }
    use pm_media::frame::PlaneDesc;

    fn format(
        layout: Layout,
        bits: u8,
        width: usize,
        height: usize,
        planes: Vec<PlaneDesc>,
    ) -> PixelFormat {
        PixelFormat {
            layout,
            bits,
            alignment: SampleAlignment::Lsb,
            range: Range::Limited,
            log2_chroma_w: 1,
            log2_chroma_h: 1,
            alpha_bits: 0,
            planes,
            width,
            height,
            label: "unit".into(),
            rgba_premultiplied: false,
        }
    }

    fn plane(format: PlaneFormat, width: usize, height: usize) -> PlaneDesc {
        PlaneDesc {
            format,
            width,
            height,
        }
    }

    #[test]
    fn valid_production_layouts_need_native_conversion() {
        for (layout, bits, cw, ch, alpha, planes) in [
            (
                Layout::Cbycry16,
                16,
                1,
                0,
                0,
                vec![plane(PlaneFormat::Rgba16, 3, 3)],
            ),
            (
                Layout::Ayuv16,
                16,
                0,
                0,
                16,
                vec![plane(PlaneFormat::Rgba16, 5, 3)],
            ),
            (
                Layout::Planar,
                12,
                0,
                0,
                0,
                vec![plane(PlaneFormat::R16, 5, 3); 3],
            ),
            (
                Layout::Biplanar,
                10,
                1,
                0,
                0,
                vec![
                    plane(PlaneFormat::R16, 5, 3),
                    plane(PlaneFormat::Rg16, 3, 3),
                ],
            ),
            (
                Layout::BiplanarAlpha,
                8,
                1,
                1,
                8,
                vec![
                    plane(PlaneFormat::R8, 5, 3),
                    plane(PlaneFormat::Rg8, 3, 2),
                    plane(PlaneFormat::R8, 5, 3),
                ],
            ),
        ] {
            let mut source = format(layout, bits, 5, 3, planes);
            source.log2_chroma_w = cw;
            source.log2_chroma_h = ch;
            source.alpha_bits = alpha;
            source.alignment = match layout {
                Layout::Cbycry16 | Layout::Ayuv16 => SampleAlignment::Full16,
                Layout::Biplanar if bits == 10 => SampleAlignment::MsbIn16,
                _ => SampleAlignment::Lsb,
            };
            assert_eq!(packed_format(&source).unwrap(), None, "{layout:?}");
        }
    }

    #[test]
    fn malformed_presenter_planes_are_errors_instead_of_fallbacks() {
        let source = format(
            Layout::Biplanar,
            8,
            5,
            3,
            vec![plane(PlaneFormat::R8, 5, 3), plane(PlaneFormat::Rg8, 3, 2)],
        );
        assert_eq!(packed_format(&source).unwrap(), Some("nv12"));
        let mut malformed = Vec::new();
        let mut missing = source.clone();
        missing.planes.pop();
        malformed.push(missing);
        let mut extra = source.clone();
        extra.planes.push(plane(PlaneFormat::R8, 5, 3));
        malformed.push(extra);
        let mut wrong_type = source.clone();
        wrong_type.planes[1].format = PlaneFormat::R8;
        malformed.push(wrong_type);
        let mut truncated_chroma = source.clone();
        truncated_chroma.planes[1].width = 2;
        malformed.push(truncated_chroma);
        let mut zero_size = source.clone();
        zero_size.height = 0;
        malformed.push(zero_size);
        let mut missing_alpha = format(
            Layout::Planar,
            8,
            5,
            3,
            vec![
                plane(PlaneFormat::R8, 5, 3),
                plane(PlaneFormat::R8, 3, 2),
                plane(PlaneFormat::R8, 3, 2),
            ],
        );
        missing_alpha.alpha_bits = 8;
        malformed.push(missing_alpha);
        for source in malformed {
            assert!(
                packed_format(&source)
                    .unwrap_err()
                    .contains("malformed planes")
            );
        }
    }

    #[test]
    fn native_fallback_does_not_hide_malformed_packed_or_444_planes() {
        let mut packed = format(
            Layout::Cbycry16,
            16,
            5,
            3,
            vec![plane(PlaneFormat::Rgba16, 2, 3)],
        );
        packed.log2_chroma_h = 0;
        assert!(
            packed_format(&packed).is_err(),
            "v216 needs ceil(width / 2) texels"
        );
        packed.planes[0] = plane(PlaneFormat::Rgba8, 3, 3);
        assert!(
            packed_format(&packed).is_err(),
            "v216 needs 16-bit components"
        );
        let mut planar = format(
            Layout::Planar,
            12,
            5,
            3,
            vec![plane(PlaneFormat::R16, 5, 3); 2],
        );
        planar.log2_chroma_w = 0;
        planar.log2_chroma_h = 0;
        assert!(
            packed_format(&planar).is_err(),
            "4:4:4 still needs all three planes"
        );
    }

    #[test]
    fn chromium_presents_only_readings_it_reproduces_exactly() {
        let desc = |primaries, transfer, matrix| ColorDescription {
            primaries,
            transfer,
            matrix,
            range: Range::Limited,
            subsampling: pm_protocol::color::Subsampling::S420,
            siting: pm_protocol::color::ChromaSiting::Left,
            alpha: pm_protocol::color::AlphaMode::Opaque,
            bit_depth: 8,
        };
        use MatrixCoefficients as M;
        use Primaries as P;
        use Transfer as T;
        assert_eq!(
            chromium_tags(&desc(P::Bt601_525, T::DisplayE, M::Bt601)),
            Some(("smpte170m", "bt709", "smpte170m"))
        );
        assert_eq!(
            chromium_tags(&desc(P::Bt601_625, T::DisplayE, M::Bt601)),
            Some(("bt470bg", "bt709", "smpte170m"))
        );
        for (p, t, m) in [
            (P::Bt2020, T::DisplayE, M::Bt2020Ncl),
            (P::Bt709, T::Pq, M::Bt709),
            (P::Bt709, T::Hlg, M::Bt709),
            (P::Bt709, T::Gamma22, M::Bt709),
            (P::DisplayP3, T::DisplayE, M::Bt709),
        ] {
            assert_eq!(chromium_tags(&desc(p, t, m)), None, "{p:?} {t:?} {m:?}");
        }
    }

    #[test]
    fn packs_padded_nv12_without_padding_or_chroma_swap() {
        let descriptions = vec![
            PlaneDesc {
                format: PlaneFormat::R8,
                width: 3,
                height: 3,
            },
            PlaneDesc {
                format: PlaneFormat::Rg8,
                width: 2,
                height: 2,
            },
        ];
        let source = format(Layout::Biplanar, 8, 3, 3, descriptions);
        let y = [1, 2, 3, 99, 4, 5, 6, 99, 7, 8, 9, 99];
        let uv = [10, 20, 11, 21, 99, 99, 12, 22, 13, 23, 99, 99];
        let planes = [
            PlaneView {
                ptr: y.as_ptr(),
                stride: 4,
                desc: &source.planes[0],
            },
            PlaneView {
                ptr: uv.as_ptr(),
                stride: 6,
                desc: &source.planes[1],
            },
        ];
        assert_eq!(
            packed(&source, &planes, "nv12", source.width).unwrap(),
            [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 20, 11, 21, 12, 22, 13, 23]
        );
    }

    #[test]
    fn planar_10_bit_samples_stay_planar_i420p10() {
        let descriptions = vec![
            PlaneDesc {
                format: PlaneFormat::R16,
                width: 2,
                height: 2,
            },
            PlaneDesc {
                format: PlaneFormat::R16,
                width: 1,
                height: 1,
            },
            PlaneDesc {
                format: PlaneFormat::R16,
                width: 1,
                height: 1,
            },
        ];
        let source = format(Layout::Planar, 10, 2, 2, descriptions);
        let y = [1u16, 2, 3, 1023];
        let u = [512u16];
        let v = [900u16];
        let planes = [
            PlaneView {
                ptr: y.as_ptr().cast(),
                stride: 4,
                desc: &source.planes[0],
            },
            PlaneView {
                ptr: u.as_ptr().cast(),
                stride: 2,
                desc: &source.planes[1],
            },
            PlaneView {
                ptr: v.as_ptr().cast(),
                stride: 2,
                desc: &source.planes[2],
            },
        ];
        let bytes = packed(&source, &planes, "i420p10", source.width).unwrap();
        let codes: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(codes, [1, 2, 3, 1023, 512, 900]);
    }

    #[test]
    fn biplanar_p010_deinterleaves_and_shifts_into_i420p10() {
        let descriptions = vec![
            PlaneDesc {
                format: PlaneFormat::R16,
                width: 2,
                height: 2,
            },
            PlaneDesc {
                format: PlaneFormat::Rg16,
                width: 1,
                height: 1,
            },
        ];
        let mut source = format(Layout::Biplanar, 10, 2, 2, descriptions);
        source.alignment = SampleAlignment::MsbIn16;
        let y = [1u16 << 6, 2 << 6, 3 << 6, 1023 << 6];
        let uv = [512u16 << 6, 900 << 6];
        let planes = [
            PlaneView {
                ptr: y.as_ptr().cast(),
                stride: 4,
                desc: &source.planes[0],
            },
            PlaneView {
                ptr: uv.as_ptr().cast(),
                stride: 4,
                desc: &source.planes[1],
            },
        ];
        let bytes = packed(&source, &planes, "i420p10", source.width).unwrap();
        let codes: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(codes, [1, 2, 3, 1023, 512, 900]);
    }

    #[test]
    fn planar_422_keeps_every_chroma_row_as_i422() {
        let descriptions = vec![
            PlaneDesc {
                format: PlaneFormat::R8,
                width: 2,
                height: 2,
            },
            PlaneDesc {
                format: PlaneFormat::R8,
                width: 1,
                height: 2,
            },
            PlaneDesc {
                format: PlaneFormat::R8,
                width: 1,
                height: 2,
            },
        ];
        let mut source = format(Layout::Planar, 8, 2, 2, descriptions);
        source.log2_chroma_h = 0;
        let y = [1, 2, 3, 4];
        let u = [10, 11];
        let v = [20, 21];
        let planes = [
            PlaneView {
                ptr: y.as_ptr(),
                stride: 2,
                desc: &source.planes[0],
            },
            PlaneView {
                ptr: u.as_ptr(),
                stride: 1,
                desc: &source.planes[1],
            },
            PlaneView {
                ptr: v.as_ptr(),
                stride: 1,
                desc: &source.planes[2],
            },
        ];
        assert_eq!(
            packed(&source, &planes, "i422", source.width).unwrap(),
            [1, 2, 3, 4, 10, 11, 20, 21]
        );
    }

    #[test]
    fn opaque_yuva420p_drops_only_the_verified_alpha_plane() {
        let descriptions = vec![
            PlaneDesc {
                format: PlaneFormat::R8,
                width: 2,
                height: 2,
            },
            PlaneDesc {
                format: PlaneFormat::R8,
                width: 1,
                height: 1,
            },
            PlaneDesc {
                format: PlaneFormat::R8,
                width: 1,
                height: 1,
            },
            PlaneDesc {
                format: PlaneFormat::R8,
                width: 2,
                height: 2,
            },
        ];
        let mut source = format(Layout::Planar, 8, 2, 2, descriptions);
        source.alpha_bits = 8;
        let y = [1, 2, 3, 4];
        let u = [10];
        let v = [20];
        let opaque = [255, 255, 255, 255];
        let mut planes = [
            PlaneView {
                ptr: y.as_ptr(),
                stride: 2,
                desc: &source.planes[0],
            },
            PlaneView {
                ptr: u.as_ptr(),
                stride: 1,
                desc: &source.planes[1],
            },
            PlaneView {
                ptr: v.as_ptr(),
                stride: 1,
                desc: &source.planes[2],
            },
            PlaneView {
                ptr: opaque.as_ptr(),
                stride: 2,
                desc: &source.planes[3],
            },
        ];
        assert!(alpha_is_opaque(&planes, 2, 2).unwrap());
        assert_eq!(
            packed(&source, &planes, "nv12", source.width).unwrap(),
            [1, 2, 3, 4, 10, 20]
        );
        let translucent = [255, 254, 255, 255];
        planes[3].ptr = translucent.as_ptr();
        assert!(!alpha_is_opaque(&planes, 2, 2).unwrap());
        assert!(
            packed(&source, &planes, "nv12", source.width)
                .unwrap_err()
                .contains("I420A")
        );
        assert_eq!(
            packed(&source, &planes, "i420a", source.width).unwrap(),
            [1, 2, 3, 4, 10, 20, 255, 254, 255, 255]
        );
        assert_eq!(
            packed(&source, &planes, "i420a", 4).unwrap(),
            [
                1, 2, 0, 0, 3, 4, 0, 0, // Y, zero-coded border
                10, 0, 20, 0, // U and V each have one visible chroma sample
                255, 254, 0, 0, 255, 255, 0, 0 // A, same visible rectangle
            ]
        );
    }

    #[test]
    fn vp9_coded_stride_and_nv12_border_match_chromium_layout() {
        assert_eq!(vp9_coded_width(160).unwrap(), 224);
        assert_eq!(vp9_coded_width(320).unwrap(), 384);
        assert_eq!(vp9_coded_width(161).unwrap(), 256);
        let descriptions = vec![
            PlaneDesc {
                format: PlaneFormat::R8,
                width: 2,
                height: 2,
            },
            PlaneDesc {
                format: PlaneFormat::Rg8,
                width: 1,
                height: 1,
            },
        ];
        let source = format(Layout::Biplanar, 8, 2, 2, descriptions);
        let y = [1, 2, 3, 4];
        let uv = [10, 20];
        let planes = [
            PlaneView {
                ptr: y.as_ptr(),
                stride: 2,
                desc: &source.planes[0],
            },
            PlaneView {
                ptr: uv.as_ptr(),
                stride: 2,
                desc: &source.planes[1],
            },
        ];
        assert_eq!(
            packed(&source, &planes, "nv12", 4).unwrap(),
            [1, 2, 0, 0, 3, 4, 0, 0, 10, 20, 0, 0]
        );
    }

    #[test]
    fn oversized_coded_planes_fail_before_read_or_allocation() {
        let descriptions = vec![
            PlaneDesc {
                format: PlaneFormat::R16,
                width: 8192,
                height: 8192,
            },
            PlaneDesc {
                format: PlaneFormat::Rg16,
                width: 4096,
                height: 4096,
            },
        ];
        let source = format(Layout::Biplanar, 10, 8192, 8192, descriptions);
        let planes = [
            PlaneView {
                ptr: std::ptr::null(),
                stride: 0,
                desc: &source.planes[0],
            },
            PlaneView {
                ptr: std::ptr::null(),
                stride: 0,
                desc: &source.planes[1],
            },
        ];
        let error = packed(&source, &planes, "i420p10", source.width).unwrap_err();
        assert!(error.contains("byte limit"), "{error}");
    }
}
