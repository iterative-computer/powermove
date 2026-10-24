//! Coded source planes for Chromium's source-only video presenter. No GPU
//! colour conversion or project interpretation happens on this path.

use std::path::Path;

use pm_media::frame::{DecodedFrame, Layout, PixelFormat, PlaneFormat, PlaneView};
use pm_media::probe::{VideoProbe, color::TaggedColor};
use pm_protocol::color::{Range, SampleAlignment};
use pm_protocol::common::{Size, Ticks};
use pm_protocol::frame::MAX_OUTPUT_DIMENSION;
use pm_protocol::snapshot::SourceVideoFrame;

use super::Renderer;
use crate::media::source_platform_priority;

/// Rejects a single coded frame before allocating its packed planes.
const MAX_PACKED_SOURCE_VIDEO_BYTES: usize = 128 << 20;

impl Renderer {
    /// Return the actual coded dimensions, tight YUV(A) planes, and the
    /// frame's colour tags. `outputSize`/fit are presentation geometry and
    /// deliberately do not resample these samples.
    pub fn source_video_planes(
        &mut self,
        path: &Path,
        time: Ticks,
    ) -> Result<(Size, Vec<u8>, SourceVideoFrame), String> {
        let (source, frame) = self
            .media
            .source_exact_decoded(path, time)
            .map_err(|e| match e {
                crate::media::MediaFailure::Unsupported(feature) => feature.to_string(),
                crate::media::MediaFailure::Failed(message) => message,
            })?;
        let video = source
            .probe
            .video
            .as_ref()
            .ok_or("The source has no video stream")?;
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
        let input_format = packed_format(&frame)?;
        let (format, bytes) = frame
            .with_planes(|planes| {
                let format = if input_format == "i420a"
                    && video.codec.codec != "vp9"
                    && alpha_is_opaque(planes, frame.format.width, frame.format.height)?
                {
                    "nv12"
                } else {
                    input_format
                };
                pack_planes(&frame.format, planes, format, coded_width as usize)
                    .map(|bytes| (format, bytes))
            })
            .map_err(|e| e.to_string())??;
        let mut tags = source_video_color(&frame, video, size, format)?;
        if coded_width != visible_width {
            tags.visible_width = Some(visible_width);
        }
        if Some(bytes.len() as u64) != tags.byte_len(size.width, size.height) {
            return Err("The decoded source planes have an unexpected length".into());
        }
        Ok((size, bytes, tags))
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

fn packed_format(frame: &DecodedFrame) -> Result<&'static str, String> {
    let f = &frame.format;
    if f.log2_chroma_w != 1 || f.width == 0 || f.height == 0 {
        return Err(format!(
            "Chromium source video cannot import {} plane layout",
            f.label
        ));
    }
    let supported = match (f.layout, f.bits, f.alignment, f.log2_chroma_h) {
        (Layout::Biplanar, 8, _, 1)
            if f.planes.len() == 2
                && f.planes[0].format == PlaneFormat::R8
                && f.planes[1].format == PlaneFormat::Rg8 =>
        {
            Some("nv12")
        }
        (Layout::Biplanar, 8, _, 0)
            if f.planes.len() == 2
                && f.planes[0].format == PlaneFormat::R8
                && f.planes[1].format == PlaneFormat::Rg8 =>
        {
            Some("i422")
        }
        (Layout::Planar, 8, _, 1)
            if f.planes.len() == 3
                && !f.has_alpha()
                && f.planes.iter().all(|p| p.format == PlaneFormat::R8) =>
        {
            Some("nv12")
        }
        (Layout::Planar, 8, _, 1)
            if f.planes.len() == 4
                && f.alpha_bits == 8
                && f.planes.iter().all(|p| p.format == PlaneFormat::R8) =>
        {
            Some("i420a")
        }
        (Layout::Planar, 8, _, 0)
            if f.planes.len() == 3
                && !f.has_alpha()
                && f.planes.iter().all(|p| p.format == PlaneFormat::R8) =>
        {
            Some("i422")
        }
        (Layout::Biplanar, 10, SampleAlignment::MsbIn16, 1)
            if f.planes.len() == 2
                && f.planes[0].format == PlaneFormat::R16
                && f.planes[1].format == PlaneFormat::Rg16 =>
        {
            Some("i420p10")
        }
        (Layout::Planar, 10, SampleAlignment::Lsb, 1)
            if f.planes.len() == 3 && f.planes.iter().all(|p| p.format == PlaneFormat::R16) =>
        {
            Some("i420p10")
        }
        _ => None,
    };
    supported.ok_or_else(|| {
        format!(
            "Chromium source video cannot import {} plane layout",
            f.label
        )
    })
}

/// Packs the decoded planes tightly in a WebCodecs buffer layout: `nv12`
/// (Y, interleaved CbCr), or planar `i420a`, `i422` and `i420p10` (16-bit
/// little-endian samples holding the low ten bits).
fn pack_planes(
    source: &PixelFormat,
    planes: &[PlaneView<'_>],
    format: &str,
    coded_width: usize,
) -> Result<Vec<u8>, String> {
    let width = coded_width;
    if width < source.width {
        return Err("The coded source row is narrower than its visible pixels".into());
    }
    let height = source.height;
    let chroma_width = width.div_ceil(2);
    let visible_chroma_width = source.width.div_ceil(2);
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
    let chroma_bytes = chroma_width
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
    let total = y_bytes
        .checked_add(2 * chroma_bytes)
        .and_then(|n| n.checked_add(alpha_bytes))
        .ok_or("Source video is too large")?;
    if total > MAX_PACKED_SOURCE_VIDEO_BYTES {
        return Err("The coded source video exceeds the source byte limit".into());
    }
    if source.has_alpha() && format != "i420a" && !alpha_is_opaque(planes, width, height)? {
        return Err("The source video has translucent alpha; it needs an I420A presenter".into());
    }
    let mut out = vec![0; total];
    let (luma, rest) = out.split_at_mut(y_bytes);
    copy_rows_padded(&planes[0], source.width * sample_bytes, width * sample_bytes, height, luma)?;
    let (chroma, alpha) = rest.split_at_mut(2 * chroma_bytes);
    let visible_bytes = visible_chroma_width * sample_bytes;
    let row_bytes = chroma_width * sample_bytes;
    match (format, source.layout) {
        ("nv12", Layout::Biplanar) => {
            copy_rows_padded(&planes[1], 2 * visible_bytes, 2 * row_bytes, chroma_height, chroma)?
        }
        ("nv12", Layout::Planar) => {
            for row in 0..chroma_height {
                let u = visible_row(&planes[1], row, visible_bytes)?;
                let v = visible_row(&planes[2], row, visible_bytes)?;
                let dst = &mut chroma[row * 2 * row_bytes..];
                for col in 0..visible_chroma_width {
                    dst[2 * col] = u[col];
                    dst[2 * col + 1] = v[col];
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
                for col in 0..visible_chroma_width {
                    let (src, dst) = (2 * col * sample_bytes, row * row_bytes + col * sample_bytes);
                    u[dst..dst + sample_bytes].copy_from_slice(&uv[src..src + sample_bytes]);
                    v[dst..dst + sample_bytes]
                        .copy_from_slice(&uv[src + sample_bytes..src + 2 * sample_bytes]);
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
    Ok(out)
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

fn source_video_color(
    frame: &DecodedFrame,
    video: &VideoProbe,
    size: Size,
    format: &str,
) -> Result<SourceVideoFrame, String> {
    let attachments = frame.core_video_color_codes();
    if std::env::var_os("PM_SOURCE_PROFILE").is_some() {
        eprintln!(
            "pm source coded color cv={attachments:?} stream={:?}/{:?}/{:?} range={:?}",
            video.color.tagged.primaries,
            video.color.tagged.transfer,
            video.color.tagged.matrix,
            frame.format.range
        );
    }
    let visible_height = video
        .clean_aperture
        .as_ref()
        .map(|aperture| aperture.rect.3)
        .unwrap_or(video.height);
    source_video_color_codes(
        &video.color.tagged,
        size,
        format,
        frame.format.range,
        &video.codec.codec,
        visible_height,
        attachments,
    )
}

fn source_video_color_codes(
    tagged: &TaggedColor,
    size: Size,
    format: &str,
    decoded_range: Range,
    codec: &str,
    visible_height: u32,
    attachments: Option<[Option<i32>; 3]>,
) -> Result<SourceVideoFrame, String> {
    if matches!(codec, "vp8" | "vp9") {
        // Chromium leaves unspecified VPx fields unspecified. The VPx
        // decoder, unlike FFmpeg H.264, does not receive the demuxer's
        // resolution-based REC601/REC709 inference. In particular, a VP9
        // alpha proxy with no CICP tags has an invalid P/T/M color space;
        // Chromium's video resource updater applies its Rec601 YUV default
        // while preserving those unspecified output tags.
        // https://chromium.googlesource.com/chromium/src/+/refs/tags/152.0.7977.54/media/ffmpeg/ffmpeg_common.cc#723
        // https://chromium.googlesource.com/chromium/src/+/refs/tags/152.0.7977.54/media/filters/vpx_video_decoder.cc#363
        return Ok(SourceVideoFrame {
            format: format.into(),
            primaries: tagged
                .primaries
                .map(|code| primaries(i32::from(code)))
                .transpose()?
                .unwrap_or("unspecified")
                .into(),
            transfer: tagged
                .transfer
                .map(|code| transfer(i32::from(code)))
                .transpose()?
                .unwrap_or("unspecified")
                .into(),
            matrix: tagged
                .matrix
                .map(|code| matrix(i32::from(code)))
                .transpose()?
                .unwrap_or("unspecified")
                .into(),
            range: if decoded_range == Range::Full {
                "full".into()
            } else {
                "limited".into()
            },
            visible_width: None,
        });
    }

    // Chromium prioritizes its platform decoder for src= video with visible
    // height >=360; below that it prioritizes FFmpeg. That policy is source-
    // only here. FFmpeg frames use the stream's CICP or demux default, even
    // when our independent decoder happened to use VideoToolbox. Its 160x90
    // untagged H.264 CV buffer, for example, carries 170M/709/170M while
    // Chromium's actual software VideoFrame is 170M/170M/170M.
    // https://chromium.googlesource.com/chromium/src/+/refs/tags/152.0.7977.54/media/filters/decoder_selector.cc#59
    let platform_priority = source_platform_priority(codec, visible_height);
    if platform_priority {
        // Chromium's VideoToolbox frame converter uses the CV attachments
        // only when the entire color space is valid. Its requested output is
        // limited-range.
        // https://chromium.googlesource.com/chromium/src/+/refs/tags/152.0.7977.54/media/gpu/mac/video_toolbox_frame_converter.cc#187
        // Chromium reads the CoreVideo 709 transfer key as BT709_APPLE (a
        // 1.961 gamma) for CSS video only; its Canvas2D/WebGL uploads, our
        // composition and our sRGB-tagged export all use the sRGB curve that
        // section 9 specifies for BT.709. The monitor uses that curve too
        // (docs/engine/quality-ledger.md), so the CICP 709 code is sent.
        if let Some([Some(p), Some(t), Some(m)]) = attachments {
            if let (Ok(primaries), Ok(transfer), Ok(matrix)) =
                (primaries(p), transfer(t), matrix(m))
            {
                return Ok(SourceVideoFrame {
                    format: format.into(),
                    primaries: primaries.into(),
                    transfer: transfer.into(),
                    matrix: matrix.into(),
                    range: "limited".into(),
                    visible_width: None,
                });
            }
        }
    }

    // The demuxer supplies REC601 for untagged SD H.264 and REC709 for HD.
    // Incomplete source CICP never mixes with that default field by field.
    let sd = size.height < 720;
    let fallback = if sd { 6 } else { 1 };
    let [primaries_code, transfer_code, matrix_code] =
        match (tagged.primaries, tagged.transfer, tagged.matrix) {
            (Some(p), Some(t), Some(m)) => [i32::from(p), i32::from(t), i32::from(m)],
            // VideoColorSpace::IsSpecified() requires all three fields; the
            // demuxer's resolution inference replaces an incomplete set.
            _ => [fallback; 3],
        };
    let primaries = primaries(primaries_code)?;
    let transfer = transfer(transfer_code)?;
    let matrix = matrix(matrix_code)?;
    let range = if decoded_range == Range::Full {
        "full"
    } else {
        "limited"
    };
    Ok(SourceVideoFrame {
        format: format.into(),
        primaries: primaries.into(),
        transfer: transfer.into(),
        matrix: matrix.into(),
        range: range.into(),
        visible_width: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
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
            pack_planes(&source, &planes, "nv12", source.width).unwrap(),
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
        let bytes = pack_planes(&source, &planes, "i420p10", source.width).unwrap();
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
        let y = [1u16 << 6, 2 << 6, 3 << 6, 1023 << 6, 0xffff];
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
        let bytes = pack_planes(&source, &planes, "i420p10", source.width).unwrap();
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
            pack_planes(&source, &planes, "i422", source.width).unwrap(),
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
            pack_planes(&source, &planes, "nv12", source.width).unwrap(),
            [1, 2, 3, 4, 10, 20]
        );
        let translucent = [255, 254, 255, 255];
        planes[3].ptr = translucent.as_ptr();
        assert!(!alpha_is_opaque(&planes, 2, 2).unwrap());
        assert!(
            pack_planes(&source, &planes, "nv12", source.width)
                .unwrap_err()
                .contains("I420A")
        );
        assert_eq!(
            pack_planes(&source, &planes, "i420a", source.width).unwrap(),
            [1, 2, 3, 4, 10, 20, 255, 254, 255, 255]
        );
        assert_eq!(
            pack_planes(&source, &planes, "i420a", 4).unwrap(),
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
            pack_planes(&source, &planes, "nv12", 4).unwrap(),
            [1, 2, 0, 0, 3, 4, 0, 0, 10, 20, 0, 0]
        );
    }

    #[test]
    fn coded_color_uses_browser_stream_tags_and_sd_default() {
        let size = Size {
            width: 160,
            height: 90,
        };
        let untagged = TaggedColor::default();
        let sd =
            source_video_color_codes(&untagged, size, "nv12", Range::Limited, "h264", 90, None)
                .unwrap();
        assert_eq!(
            (&*sd.primaries, &*sd.transfer, &*sd.matrix),
            ("smpte170m", "smpte170m", "smpte170m")
        );
        let partial = TaggedColor {
            transfer: Some(1),
            ..TaggedColor::default()
        };
        let guessed =
            source_video_color_codes(&partial, size, "nv12", Range::Limited, "h264", 90, None)
                .unwrap();
        assert_eq!(
            (&*guessed.primaries, &*guessed.transfer, &*guessed.matrix),
            ("smpte170m", "smpte170m", "smpte170m")
        );
        let tagged = TaggedColor {
            primaries: Some(1),
            transfer: Some(1),
            matrix: Some(1),
            ..TaggedColor::default()
        };
        let vt = source_video_color_codes(&tagged, size, "nv12", Range::Limited, "h264", 90, None)
            .unwrap();
        assert_eq!(
            (&*vt.primaries, &*vt.transfer, &*vt.matrix),
            ("bt709", "bt709", "bt709")
        );
    }

    #[test]
    fn platform_source_uses_only_complete_cv_tags() {
        let size = Size {
            width: 480,
            height: 360,
        };
        let untagged = TaggedColor::default();
        let cv = Some([Some(1), Some(1), Some(1)]);
        let platform =
            source_video_color_codes(&untagged, size, "nv12", Range::Full, "h264", 360, cv)
                .unwrap();
        assert_eq!(platform.primaries, "bt709");
        assert_eq!(platform.transfer, "bt709");
        assert_eq!(platform.matrix, "bt709");
        assert_eq!(platform.range, "limited");

        let software =
            source_video_color_codes(&untagged, size, "nv12", Range::Limited, "h264", 359, cv)
                .unwrap();
        assert_eq!(software.transfer, "smpte170m");
        let partial = source_video_color_codes(
            &untagged,
            size,
            "nv12",
            Range::Limited,
            "h264",
            360,
            Some([Some(1), None, Some(1)]),
        )
        .unwrap();
        assert_eq!(partial.transfer, "smpte170m");
    }

    #[test]
    fn vp9_preserves_unspecified_and_partial_stream_colour() {
        let size = Size {
            width: 320,
            height: 180,
        };
        let untagged = source_video_color_codes(
            &TaggedColor::default(),
            size,
            "i420a",
            Range::Limited,
            "vp9",
            180,
            None,
        )
        .unwrap();
        assert_eq!(untagged.primaries, "unspecified");
        assert_eq!(untagged.transfer, "unspecified");
        assert_eq!(untagged.matrix, "unspecified");
        let partial = source_video_color_codes(
            &TaggedColor {
                matrix: Some(1),
                ..TaggedColor::default()
            },
            size,
            "nv12",
            Range::Limited,
            "vp9",
            180,
            None,
        )
        .unwrap();
        assert_eq!(partial.primaries, "unspecified");
        assert_eq!(partial.transfer, "unspecified");
        assert_eq!(partial.matrix, "bt709");
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
        let error = pack_planes(&source, &planes, "i420p10", source.width).unwrap_err();
        assert!(error.contains("byte limit"), "{error}");
    }
}

fn primaries(code: i32) -> Result<&'static str, String> {
    match code {
        1 => Ok("bt709"),
        4 => Ok("bt470m"),
        5 => Ok("bt470bg"),
        6 => Ok("smpte170m"),
        7 => Ok("smpte240m"),
        8 => Ok("film"),
        9 => Ok("bt2020"),
        10 => Ok("smptest428-1"),
        11 => Ok("smptest431-2"),
        12 => Ok("p3"),
        _ => Err(format!(
            "Chromium source video does not support primaries code {code}"
        )),
    }
}

fn transfer(code: i32) -> Result<&'static str, String> {
    match code {
        1 => Ok("bt709"),
        4 => Ok("gamma22"),
        5 => Ok("gamma28"),
        6 => Ok("smpte170m"),
        7 => Ok("smpte240m"),
        8 => Ok("linear"),
        13 => Ok("srgb"),
        16 => Ok("pq"),
        18 => Ok("hlg"),
        _ => Err(format!(
            "Chromium source video does not support transfer code {code}"
        )),
    }
}

fn matrix(code: i32) -> Result<&'static str, String> {
    match code {
        1 => Ok("bt709"),
        4 => Ok("fcc"),
        5 => Ok("bt470bg"),
        6 => Ok("smpte170m"),
        7 => Ok("smpte240m"),
        8 => Ok("ycocg"),
        9 => Ok("bt2020-ncl"),
        _ => Err(format!(
            "Chromium source video does not support matrix code {code}"
        )),
    }
}
