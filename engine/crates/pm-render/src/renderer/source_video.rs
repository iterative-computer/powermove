//! Coded source planes for the source monitor's video presenter. Chromium's
//! compositor scales them and applies the colour tags given here, which are
//! the shared reading (`pm_media::probe::color::frame_reading`) composition
//! and export use. A reading Chromium cannot reproduce exactly is converted
//! by the engine instead (`None`).

use std::path::Path;

use pm_media::frame::{DecodedFrame, Layout, PixelFormat, PlaneFormat, PlaneView};
use pm_media::probe::color::frame_reading;
use pm_protocol::color::{
    ColorDescription, MatrixCoefficients, Primaries, Range, SampleAlignment, Transfer,
};
use pm_protocol::common::{Size, Ticks};
use pm_protocol::frame::MAX_OUTPUT_DIMENSION;
use pm_protocol::snapshot::SourceVideoFrame;

use super::Renderer;

/// The main-process source-surface pool's aggregate byte ceiling. Reject a
/// single coded frame before allocating its packed planes.
const MAX_PACKED_SOURCE_VIDEO_BYTES: usize = 128 << 20;

impl Renderer {
    /// Return the actual coded dimensions, tight YUV(A) planes, and the
    /// frame's colour tags. `outputSize`/fit are presentation geometry and
    /// deliberately do not resample these samples. `Ok(None)`: Chromium cannot
    /// apply this frame's reading exactly; draw it with `source_rgba8`.
    pub fn source_video_planes(
        &mut self,
        path: &Path,
        time: Ticks,
        sequence: bool,
    ) -> Result<Option<(Size, Vec<u8>, SourceVideoFrame)>, String> {
        let tags = |source: &pm_media::decode::service::MediaSource| {
            source
                .probe
                .video
                .as_ref()
                .and_then(|video| chromium_tags(&frame_reading(&video.color, sequence)))
        };
        let Some((source, frame)) = self
            .media
            .source_exact_decoded(path, time, |source| tags(source).is_some())
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
        if Some(bytes.len() as u64) != tags.byte_len(size.width, size.height) {
            return Err("The decoded source planes have an unexpected length".into());
        }
        Ok(Some((size, bytes, tags)))
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
        return Err("The VP9 coded row exceeds the source-surface dimension limit".into());
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
            Some("nv16")
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
            Some("nv16")
        }
        (Layout::Biplanar, 10, SampleAlignment::MsbIn16, 1)
            if f.planes.len() == 2
                && f.planes[0].format == PlaneFormat::R16
                && f.planes[1].format == PlaneFormat::Rg16 =>
        {
            Some("p010le")
        }
        (Layout::Planar, 10, SampleAlignment::Lsb, 1)
            if f.planes.len() == 3 && f.planes.iter().all(|p| p.format == PlaneFormat::R16) =>
        {
            Some("p010le")
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
    let chroma_height = if format == "nv16" {
        height
    } else {
        height.div_ceil(2)
    };
    let sample_bytes = if format == "p010le" { 2 } else { 1 };
    let y_bytes = width
        .checked_mul(height)
        .and_then(|n| n.checked_mul(sample_bytes))
        .ok_or("Source video is too large")?;
    let uv_bytes = chroma_width
        .checked_mul(chroma_height)
        .and_then(|n| n.checked_mul(2 * sample_bytes))
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
        .checked_add(uv_bytes)
        .and_then(|n| n.checked_add(alpha_bytes))
        .ok_or("Source video is too large")?;
    if total > MAX_PACKED_SOURCE_VIDEO_BYTES {
        return Err("The coded source video exceeds the source-surface byte limit".into());
    }
    if source.has_alpha() && format != "i420a" && !alpha_is_opaque(planes, width, height)? {
        return Err("The source video has translucent alpha; it needs an I420A presenter".into());
    }
    let mut out = vec![0; total];
    copy_rows_padded(
        &planes[0],
        source.width * sample_bytes,
        width * sample_bytes,
        height,
        &mut out[..y_bytes],
    )?;
    match source.layout {
        Layout::Biplanar => copy_rows_padded(
            &planes[1],
            visible_chroma_width * 2 * sample_bytes,
            chroma_width * 2 * sample_bytes,
            chroma_height,
            &mut out[y_bytes..],
        )?,
        Layout::Planar if format == "i420a" => {
            let chroma_plane_bytes = chroma_width * chroma_height;
            copy_rows_padded(
                &planes[1],
                visible_chroma_width,
                chroma_width,
                chroma_height,
                &mut out[y_bytes..y_bytes + chroma_plane_bytes],
            )?;
            copy_rows_padded(
                &planes[2],
                visible_chroma_width,
                chroma_width,
                chroma_height,
                &mut out[y_bytes + chroma_plane_bytes..y_bytes + uv_bytes],
            )?;
            copy_rows_padded(
                &planes[3],
                source.width,
                width,
                height,
                &mut out[y_bytes + uv_bytes..],
            )?;
        }
        Layout::Planar => {
            for row in 0..chroma_height {
                let u = visible_row(&planes[1], row, visible_chroma_width * sample_bytes)?;
                let v = visible_row(&planes[2], row, visible_chroma_width * sample_bytes)?;
                for col in 0..visible_chroma_width {
                    let dst = y_bytes + (row * chroma_width + col) * 2 * sample_bytes;
                    let src = col * sample_bytes;
                    if sample_bytes == 1 {
                        out[dst] = u[src];
                        out[dst + 1] = v[src];
                    } else {
                        let uc = u16::from_le_bytes([u[src], u[src + 1]]) << 6;
                        let vc = u16::from_le_bytes([v[src], v[src + 1]]) << 6;
                        out[dst..dst + 2].copy_from_slice(&uc.to_le_bytes());
                        out[dst + 2..dst + 4].copy_from_slice(&vc.to_le_bytes());
                    }
                }
            }
        }
        _ => unreachable!("packed_format checked the plane layout"),
    }
    if sample_bytes == 2 && source.layout == Layout::Planar {
        for sample in out[..y_bytes].chunks_exact_mut(2) {
            let code = u16::from_le_bytes([sample[0], sample[1]]) << 6;
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
            pack_planes(&source, &planes, "nv12", source.width).unwrap(),
            [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 20, 11, 21, 12, 22, 13, 23]
        );
    }

    #[test]
    fn planar_10_bit_samples_are_shifted_losslessly_into_p010() {
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
        let bytes = pack_planes(&source, &planes, "p010le", source.width).unwrap();
        let codes: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(codes, [64, 128, 192, 65472, 32768, 57600]);
    }

    #[test]
    fn planar_422_interleaves_every_chroma_row_as_nv16() {
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
            pack_planes(&source, &planes, "nv16", source.width).unwrap(),
            [1, 2, 3, 4, 10, 20, 11, 21]
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
        let error = pack_planes(&source, &planes, "p010le", source.width).unwrap_err();
        assert!(error.contains("byte limit"), "{error}");
    }
}
