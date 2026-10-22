//! Whole-clip proxies made directly from original image-sequence frames.
//! The manifest is a machine-local capability, not generated project media.

use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

use pm_media::frame::PlaneFormat;
use pm_media::image::icc::IccClass;
use pm_protocol::color::{AlphaMode, Primaries, Transfer, may_unpremultiply};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tempfile::{NamedTempFile, TempDir};

use crate::generate::{GeneratedProxy, GenerationControl, GenerationRequest, PauseReason};
use crate::{ProxyError, ProxyRecipe, Result, disk_guard};

const MAX_MANIFEST_BYTES: u64 = 32 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SequenceColor {
    Srgb,
    DisplayP3,
    Linear709,
    LinearP3,
    Linear2020,
    /// Mixed/other ICC, arbitrary EXR chromaticities or scene highlights:
    /// linear-light BT.2020, mapped to PQ with 1.0 scene white = 100 nits.
    /// This retains wide colors and highlights to 10,000 nits in a video
    /// format, instead of mislabeling or clipping them as sRGB.
    Bt2020Pq,
}

impl SequenceColor {
    fn tags(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Srgb => ("bt709", "iec61966-2-1", "bt709"),
            Self::DisplayP3 => ("smpte432", "iec61966-2-1", "bt709"),
            Self::Linear709 => ("bt709", "linear", "bt709"),
            Self::LinearP3 => ("smpte432", "linear", "bt709"),
            Self::Linear2020 => ("bt2020", "linear", "bt2020nc"),
            Self::Bt2020Pq => ("bt2020", "smpte2084", "bt2020nc"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SequenceRate {
    pub num: u32,
    pub den: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SequenceFrame {
    pub path: PathBuf,
    pub hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SequenceManifest {
    pub powermove_sequence: u8,
    pub rate: SequenceRate,
    pub frames: Vec<SequenceFrame>,
}

impl SequenceManifest {
    /// Stable across moves and relinks: only rational rate and ordered raw
    /// frame content hashes participate. The input strings are normalized to
    /// lowercase hex before hashing.
    pub fn content_hash(&self) -> Result<String> {
        if self.powermove_sequence != 1
            || self.rate.num == 0
            || self.rate.den == 0
            || self.rate.num > i32::MAX as u32
            || self.frames.is_empty()
            || self.frames.len() > 1_000_000
        {
            return Err(ProxyError::InvalidMedia(
                "invalid image-sequence manifest".into(),
            ));
        }
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"pm-sequence-v1\n");
        hasher.update(format!("{}/{}\n", self.rate.num, self.rate.den).as_bytes());
        for frame in &self.frames {
            if !frame.path.is_absolute()
                || frame.hash.len() != 64
                || !frame.hash.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err(ProxyError::InvalidMedia(
                    "sequence frame requires an absolute path and BLAKE3 hash".into(),
                ));
            }
            hasher.update(frame.hash.to_ascii_lowercase().as_bytes());
            hasher.update(b"\n");
        }
        Ok(hasher.finalize().to_hex().to_string())
    }
}

/// Identifies the explicit marker without treating an ordinary JSON source
/// as a sequence. A bounded manifest avoids untrusted local input allocation.
pub(crate) fn read_manifest(path: &Path) -> Result<Option<SequenceManifest>> {
    if path.extension().is_none_or(|s| s != "json") {
        return Ok(None);
    }
    let file = File::open(path)?;
    if file.metadata()?.len() > MAX_MANIFEST_BYTES {
        return Err(ProxyError::InvalidMedia(
            "image-sequence manifest is too large".into(),
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_MANIFEST_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(ProxyError::InvalidMedia(
            "image-sequence manifest is too large".into(),
        ));
    }
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|e| ProxyError::InvalidMedia(format!("sequence manifest JSON: {e}")))?;
    if value.get("powermoveSequence").is_none() {
        return Ok(None);
    }
    let manifest: SequenceManifest = serde_json::from_value(value)
        .map_err(|e| ProxyError::InvalidMedia(format!("sequence manifest: {e}")))?;
    manifest.content_hash()?;
    Ok(Some(manifest))
}

fn wait_for_idle(control: &GenerationControl<'_>) -> Result<()> {
    let mut reported = false;
    loop {
        if control.cancel.load(Ordering::Relaxed) {
            return Err(ProxyError::Cancelled);
        }
        if !(control.pause)() {
            if reported {
                (control.on_pause)(None);
            }
            return Ok(());
        }
        if !reported {
            (control.on_pause)(Some(PauseReason::InteractiveWork));
            reported = true;
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn verify_frame(frame: &SequenceFrame, control: &GenerationControl<'_>) -> Result<()> {
    let mut file = File::open(&frame.path)?;
    let before = file.metadata()?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = [0_u8; 1 << 20];
    loop {
        wait_for_idle(control)?;
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let after = fs::metadata(&frame.path)?;
    if before.len() != after.len() || before.modified()? != after.modified()? {
        return Err(ProxyError::InvalidMedia(
            "sequence frame changed while hashing".into(),
        ));
    }
    if hasher.finalize().to_hex().as_str() != frame.hash.to_ascii_lowercase() {
        return Err(ProxyError::InvalidMedia(
            "sequence frame content hash changed".into(),
        ));
    }
    Ok(())
}

fn verify_frames(manifest: &SequenceManifest, control: &GenerationControl<'_>) -> Result<()> {
    for frame in &manifest.frames {
        verify_frame(frame, control)?;
    }
    Ok(())
}

fn exr_primaries(ch: [f32; 8]) -> Option<Primaries> {
    for p in [Primaries::Bt709, Primaries::DisplayP3, Primaries::Bt2020] {
        let c = pm_color::primaries::chromaticities(p)?;
        let expected = [
            c.r[0], c.r[1], c.g[0], c.g[1], c.b[0], c.b[1], c.w[0], c.w[1],
        ];
        if ch
            .iter()
            .zip(expected)
            .all(|(a, b)| (f64::from(*a) - b).abs() < 1e-4)
        {
            return Some(p);
        }
    }
    None
}

fn float_outside_sdr(still: &pm_media::image::StillImage) -> Result<bool> {
    if !still.float {
        return Ok(false);
    }
    let format = still.frame.format.planes[0].format;
    let bytes_per_component = match format {
        PlaneFormat::Rgba16Float => 2,
        PlaneFormat::Rgba32Float => 4,
        _ => return Ok(false),
    };
    let width = still.frame.format.width;
    let height = still.frame.format.height;
    still
        .frame
        .with_planes(|planes| {
            let p = &planes[0];
            for y in 0..height {
                let row = unsafe {
                    std::slice::from_raw_parts(
                        p.ptr.add(y * p.stride),
                        width * bytes_per_component * 4,
                    )
                };
                for px in row.chunks_exact(bytes_per_component * 4) {
                    for channel in px.chunks_exact(bytes_per_component).take(3) {
                        let v = if bytes_per_component == 2 {
                            pm_media::frame::half_to_f64(u16::from_le_bytes([
                                channel[0], channel[1],
                            ]))
                        } else {
                            f32::from_le_bytes(channel.try_into().unwrap()) as f64
                        };
                        if !v.is_finite() || !(0.0..=1.0).contains(&v) {
                            return true;
                        }
                    }
                }
            }
            false
        })
        .map_err(|e| ProxyError::InvalidMedia(format!("sequence pixels: {e}")))
}

fn direct_color(still: &pm_media::image::StillImage, path: &Path) -> Result<Option<SequenceColor>> {
    if pm_media::image::png_gamma::imageio_gamma(path)
        .map_err(|e| ProxyError::InvalidMedia(format!("sequence PNG gamma: {e}")))?
        .is_some()
        || matches!(still.color.icc_class, Some(IccClass::Other))
        || float_outside_sdr(still)?
    {
        return Ok(None);
    }
    let primaries = if let Some(ch) = still.color.chromaticities {
        exr_primaries(ch)
    } else if still.color.icc_class == Some(IccClass::DisplayP3) {
        Some(Primaries::DisplayP3)
    } else {
        Some(Primaries::Bt709)
    };
    Ok(match (primaries, still.color.transfer) {
        (Some(Primaries::Bt709), Transfer::DisplayE) => Some(SequenceColor::Srgb),
        (Some(Primaries::DisplayP3), Transfer::DisplayE) => Some(SequenceColor::DisplayP3),
        (Some(Primaries::Bt709), Transfer::Linear) => Some(SequenceColor::Linear709),
        (Some(Primaries::DisplayP3), Transfer::Linear) => Some(SequenceColor::LinearP3),
        (Some(Primaries::Bt2020), Transfer::Linear) => Some(SequenceColor::Linear2020),
        _ => None,
    })
}

fn frame_facts(
    manifest: &SequenceManifest,
    control: &GenerationControl<'_>,
) -> Result<(u32, u32, u8, bool, SequenceColor)> {
    let pool = pm_media::pool::FramePool::new(0);
    let mut facts = None;
    let mut depth = 8;
    let mut alpha = false;
    let mut color = None;
    let mut normalize = false;
    let mut extension = None;
    for frame in &manifest.frames {
        wait_for_idle(control)?;
        let still = pm_media::image::read(&frame.path, &pool)
            .map_err(|e| ProxyError::InvalidMedia(format!("sequence image: {e}")))?;
        let size = (
            still.frame.format.width as u32,
            still.frame.format.height as u32,
        );
        if size.0 == 0 || size.1 == 0 || facts.is_some_and(|previous| previous != size) {
            return Err(ProxyError::InvalidMedia(
                "sequence frame dimensions differ".into(),
            ));
        }
        facts = Some(size);
        depth = depth.max(still.bit_depth);
        alpha |= still.color.alpha != AlphaMode::Opaque;
        let frame_color = direct_color(&still, &frame.path)?;
        if let Some(frame_color) = frame_color {
            if color.is_some_and(|previous| previous != frame_color) {
                normalize = true;
            }
            color = Some(frame_color);
        } else {
            normalize = true;
        }
        let ext = frame.path.extension().and_then(|e| e.to_str());
        if extension.is_some_and(|previous| ext != Some(previous)) {
            normalize = true;
        }
        extension = ext;
    }
    let (width, height) = facts.expect("manifest contains frames");
    Ok((
        width,
        height,
        depth,
        alpha,
        if normalize {
            SequenceColor::Bt2020Pq
        } else {
            color.unwrap_or(SequenceColor::Srgb)
        },
    ))
}

type Mat3 = pm_color::primaries::Mat3;

fn mat_mul(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut out = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            out[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

fn mat_inverse(m: &Mat3) -> Option<Mat3> {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if !det.is_finite() || det.abs() < 1e-12 {
        return None;
    }
    let d = 1.0 / det;
    Some([
        [
            (m[1][1] * m[2][2] - m[1][2] * m[2][1]) * d,
            (m[0][2] * m[2][1] - m[0][1] * m[2][2]) * d,
            (m[0][1] * m[1][2] - m[0][2] * m[1][1]) * d,
        ],
        [
            (m[1][2] * m[2][0] - m[1][0] * m[2][2]) * d,
            (m[0][0] * m[2][2] - m[0][2] * m[2][0]) * d,
            (m[0][2] * m[1][0] - m[0][0] * m[1][2]) * d,
        ],
        [
            (m[1][0] * m[2][1] - m[1][1] * m[2][0]) * d,
            (m[0][1] * m[2][0] - m[0][0] * m[2][1]) * d,
            (m[0][0] * m[1][1] - m[0][1] * m[1][0]) * d,
        ],
    ])
}

fn xyz_white(xy: [f64; 2]) -> Option<[f64; 3]> {
    (xy[0].is_finite() && xy[1].is_finite() && xy[1] > 0.0).then_some([
        xy[0] / xy[1],
        1.0,
        (1.0 - xy[0] - xy[1]) / xy[1],
    ])
}

/// Arbitrary EXR chromaticities, including non-D65 scene spaces such as
/// ACEScg, are adapted relative-colorimetrically through Bradford before
/// the bounded BT.2020/PQ video coding. This is the same white adaptation
/// ColorSync uses for an arbitrary ICC profile.
fn exr_to_bt2020(ch: [f32; 8]) -> Result<Mat3> {
    let c = pm_color::primaries::Chromaticities {
        r: [ch[0] as f64, ch[1] as f64],
        g: [ch[2] as f64, ch[3] as f64],
        b: [ch[4] as f64, ch[5] as f64],
        w: [ch[6] as f64, ch[7] as f64],
    };
    let source = pm_color::primaries::rgb_to_xyz(&c);
    let destination = pm_color::primaries::rgb_to_xyz(
        &pm_color::primaries::chromaticities(Primaries::Bt2020).unwrap(),
    );
    let src_white =
        xyz_white(c.w).ok_or_else(|| ProxyError::InvalidMedia("invalid EXR white point".into()))?;
    let dst_white = xyz_white([0.3127, 0.3290]).unwrap();
    let bradford: Mat3 = [
        [0.8951, 0.2664, -0.1614],
        [-0.7502, 1.7135, 0.0367],
        [0.0389, -0.0685, 1.0296],
    ];
    let src = pm_color::primaries::apply(&bradford, src_white);
    let dst = pm_color::primaries::apply(&bradford, dst_white);
    let mut diagonal = [[0.0; 3]; 3];
    for i in 0..3 {
        if !src[i].is_finite() || src[i].abs() < 1e-12 {
            return Err(ProxyError::InvalidMedia(
                "invalid EXR chromaticities".into(),
            ));
        }
        diagonal[i][i] = dst[i] / src[i];
    }
    let inverse_bradford = mat_inverse(&bradford).unwrap();
    let inverse_destination = mat_inverse(&destination).unwrap();
    let matrix = mat_mul(
        &inverse_destination,
        &mat_mul(
            &inverse_bradford,
            &mat_mul(&diagonal, &mat_mul(&bradford, &source)),
        ),
    );
    if matrix.iter().flatten().any(|x| !x.is_finite()) {
        return Err(ProxyError::InvalidMedia(
            "invalid EXR chromaticities".into(),
        ));
    }
    Ok(matrix)
}

fn source_to_bt2020(still: &pm_media::image::StillImage) -> Result<Mat3> {
    if matches!(still.color.transfer, Transfer::Pq | Transfer::Hlg) {
        return Ok(pm_color::primaries::conversion(Primaries::Bt2020, Primaries::Bt2020).unwrap());
    }
    if let Some(ch) = still.color.chromaticities {
        return exr_to_bt2020(ch);
    }
    let source = if still.color.icc_class == Some(IccClass::DisplayP3) {
        Primaries::DisplayP3
    } else {
        Primaries::Bt709
    };
    Ok(pm_color::primaries::conversion(source, Primaries::Bt2020).unwrap())
}

#[cfg(test)]
mod color_tests {
    use super::*;

    #[test]
    fn arbitrary_d60_exr_white_adapts_to_bt2020_without_a_color_cast() {
        // ACEScg AP1 primaries and D60 white. A neutral scene value remains
        // neutral after Bradford adaptation to BT.2020's D65 white.
        let matrix =
            exr_to_bt2020([0.713, 0.293, 0.165, 0.830, 0.128, 0.044, 0.32168, 0.33767]).unwrap();
        let neutral = pm_color::primaries::apply(&matrix, [1.0; 3]);
        for channel in neutral {
            assert!((channel - 1.0).abs() < 0.002, "{neutral:?}");
        }
        assert!(pq_code(4.0) > pq_code(1.0));
        assert!((source_linear(Transfer::Pq, pq_code(4.0)) - 4.0).abs() < 1e-9);
    }
}

/// ST 2084, treating scene-linear 1.0 as reference white at 100 cd/m².
/// This maps positive EXR highlights up to 100× reference white without the
/// sRGB clip. Values beyond PQ's 10,000 cd/m² ceiling saturate at that
/// display limit, and the original frames remain the source for export.
fn pq_code(linear: f64) -> f64 {
    let y = (linear.max(0.0) / 100.0).min(1.0);
    const M1: f64 = 2610.0 / 16384.0;
    const M2: f64 = 2523.0 / 32.0;
    const C1: f64 = 3424.0 / 4096.0;
    const C2: f64 = 2413.0 / 128.0;
    const C3: f64 = 2392.0 / 128.0;
    let p = y.powf(M1);
    ((C1 + C2 * p) / (1.0 + C3 * p)).powf(M2)
}

fn source_linear(transfer: Transfer, encoded: f64) -> f64 {
    match transfer {
        Transfer::Pq => {
            const M1: f64 = 2610.0 / 16384.0;
            const M2: f64 = 2523.0 / 32.0;
            const C1: f64 = 3424.0 / 4096.0;
            const C2: f64 = 2413.0 / 128.0;
            const C3: f64 = 2392.0 / 128.0;
            let p = encoded.max(0.0).powf(1.0 / M2);
            ((p - C1).max(0.0) / (C2 - C3 * p)).max(0.0).powf(1.0 / M1) * 100.0
        }
        Transfer::Hlg => {
            const A: f64 = 0.178_832_77;
            const B: f64 = 0.284_668_92;
            const C: f64 = 0.559_910_73;
            if encoded <= 0.5 {
                encoded * encoded / 3.0
            } else {
                (((encoded - C) / A).exp() + B) / 12.0
            }
        }
        other => pm_color::transfer::to_linear(other, encoded).expect("SDR transfer"),
    }
}

fn write_normalized_frame(
    source: &Path,
    target: &Path,
    pool: &pm_media::pool::FramePool,
    control: &GenerationControl<'_>,
) -> Result<()> {
    let mut still = pm_media::image::read(source, pool)
        .map_err(|e| ProxyError::InvalidMedia(format!("sequence image: {e}")))?;
    if let Some(gamma) = pm_media::image::png_gamma::imageio_gamma(source)
        .map_err(|e| ProxyError::InvalidMedia(format!("sequence PNG gamma: {e}")))?
    {
        still = pm_media::image::png_gamma::gamma_to_linear(&still, gamma, pool)
            .map_err(|e| ProxyError::InvalidMedia(format!("sequence gamma transform: {e}")))?;
    } else if still.color.icc_class == Some(IccClass::Other) {
        still = pm_media::image::imageio::convert_to_linear_srgb(&still, pool, still.color.alpha)
            .map_err(|e| ProxyError::InvalidMedia(format!("sequence ICC transform: {e}")))?;
    }
    let matrix = source_to_bt2020(&still)?;
    let format = still.frame.format.planes[0].format;
    if !matches!(
        format,
        PlaneFormat::Rgba8
            | PlaneFormat::Rgba16
            | PlaneFormat::Rgba16Float
            | PlaneFormat::Rgba32Float
    ) {
        return Err(ProxyError::InvalidMedia(
            "sequence image is not packed RGBA".into(),
        ));
    }
    let bytes = format.bytes() / 4;
    let max = if still.frame.format.is_float() {
        1.0
    } else {
        ((1u32 << still.frame.format.bits.clamp(1, 16)) - 1) as f64
    };
    let (width, height) = (still.frame.format.width, still.frame.format.height);
    let mut encoder = png::Encoder::new(
        BufWriter::new(File::create(target)?),
        width as u32,
        height as u32,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Sixteen);
    let mut writer = encoder
        .write_header()
        .map_err(|e| ProxyError::InvalidMedia(format!("sequence PNG header: {e}")))?;
    let mut stream = writer
        .stream_writer()
        .map_err(|e| ProxyError::InvalidMedia(format!("sequence PNG stream: {e}")))?;
    let transfer = still.color.transfer;
    let alpha_mode = still.color.alpha;
    let result = still
        .frame
        .with_planes(|planes| -> Result<()> {
            let p = &planes[0];
            let mut row_out = vec![0u8; width * 8];
            for y in 0..height {
                wait_for_idle(control)?;
                let row = unsafe {
                    std::slice::from_raw_parts(p.ptr.add(y * p.stride), width * bytes * 4)
                };
                for (x, pixel) in row.chunks_exact(bytes * 4).enumerate() {
                    let sample = |c: usize| -> f64 {
                        let at = &pixel[c * bytes..(c + 1) * bytes];
                        match format {
                            PlaneFormat::Rgba8 => f64::from(at[0]) / max,
                            PlaneFormat::Rgba16 => {
                                f64::from(u16::from_le_bytes([at[0], at[1]])) / max
                            }
                            PlaneFormat::Rgba16Float => {
                                pm_media::frame::half_to_f64(u16::from_le_bytes([at[0], at[1]]))
                            }
                            PlaneFormat::Rgba32Float => {
                                f32::from_le_bytes(at.try_into().unwrap()) as f64
                            }
                            _ => 0.0,
                        }
                    };
                    let alpha = if alpha_mode == AlphaMode::Opaque {
                        1.0
                    } else {
                        sample(3).clamp(0.0, 1.0)
                    };
                    let mut rgb = [sample(0), sample(1), sample(2)];
                    if let AlphaMode::PremultipliedWithColor { matte } = alpha_mode {
                        for c in 0..3 {
                            rgb[c] -= (1.0 - alpha) * matte[c];
                        }
                    }
                    if matches!(
                        alpha_mode,
                        AlphaMode::Premultiplied | AlphaMode::PremultipliedWithColor { .. }
                    ) && may_unpremultiply(alpha)
                    {
                        for v in &mut rgb {
                            *v /= alpha;
                        }
                    }
                    let linear = rgb.map(|v| source_linear(transfer, v));
                    let wide = pm_color::primaries::apply(&matrix, linear);
                    let out = [pq_code(wide[0]), pq_code(wide[1]), pq_code(wide[2]), alpha];
                    for (c, value) in out.into_iter().enumerate() {
                        let code = (value.clamp(0.0, 1.0) * 65535.0).round() as u16;
                        row_out[x * 8 + c * 2..x * 8 + c * 2 + 2]
                            .copy_from_slice(&code.to_be_bytes());
                    }
                }
                stream.write_all(&row_out)?;
            }
            Ok(())
        })
        .map_err(|e| ProxyError::InvalidMedia(format!("sequence pixels: {e}")))?;
    result?;
    stream
        .finish()
        .map_err(|e| ProxyError::InvalidMedia(format!("sequence PNG: {e}")))?;
    Ok(())
}

fn disk_ok(root: &Path, disk_space: &dyn Fn(&Path) -> Result<(u64, u64)>) -> Result<bool> {
    let (available, capacity) = disk_space(root)?;
    Ok(disk_guard(available, capacity))
}

fn wait_for_disk(
    request: &GenerationRequest<'_>,
    disk_space: &dyn Fn(&Path) -> Result<(u64, u64)>,
) -> Result<()> {
    let mut reported = false;
    while !disk_ok(request.store.root(), disk_space)? {
        if request.control.cancel.load(Ordering::Relaxed) {
            return Err(ProxyError::Cancelled);
        }
        if !reported {
            (request.control.on_pause)(Some(PauseReason::LowDisk));
            reported = true;
        }
        thread::sleep(Duration::from_millis(200));
    }
    if reported {
        (request.control.on_pause)(None);
    }
    Ok(())
}

struct ChildGuard {
    child: Child,
    finished: bool,
}
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        // SIGCONT lets a stopped VideoToolbox process exit on macOS.
        unsafe { libc::kill(self.child.id() as i32, libc::SIGCONT) };
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn transcode(
    request: &GenerationRequest<'_>,
    manifest: &SequenceManifest,
    recipe: &ProxyRecipe,
    color: SequenceColor,
    destination: &Path,
    staged: &TempDir,
    disk_space: &dyn Fn(&Path) -> Result<(u64, u64)>,
) -> Result<()> {
    let source_ext = manifest.frames[0]
        .path
        .extension()
        .and_then(|v| v.to_str())
        .filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_alphanumeric()))
        .ok_or_else(|| {
            ProxyError::InvalidMedia("sequence image extension is unsupported".into())
        })?;
    let ext = if color == SequenceColor::Bt2020Pq {
        "png"
    } else {
        source_ext
    };
    let pool = pm_media::pool::FramePool::new(0);
    for (index, frame) in manifest.frames.iter().enumerate() {
        wait_for_idle(&request.control)?;
        if color != SequenceColor::Bt2020Pq
            && frame.path.extension().and_then(|v| v.to_str()) != Some(ext)
        {
            return Err(ProxyError::InvalidMedia(
                "sequence frame formats differ".into(),
            ));
        }
        let target = staged.path().join(format!("frame-{index:09}.{ext}"));
        if color == SequenceColor::Bt2020Pq {
            write_normalized_frame(&frame.path, &target, &pool, &request.control)?;
            wait_for_disk(request, disk_space)?;
        } else {
            symlink(&frame.path, target)?;
        }
    }
    let pattern = staged.path().join(format!("frame-%09d.{ext}"));
    let rate = format!("{}/{}", manifest.rate.num, manifest.rate.den);
    let (primaries, transfer, matrix) = color.tags();
    let scale = format!(
        "scale={}:{}:flags=lanczos:in_range=full:out_range=tv:out_color_matrix={matrix},setparams=field_mode=prog:range=tv:color_primaries={primaries}:color_trc={transfer}:colorspace={matrix}",
        recipe.width, recipe.height,
    );
    let mut command = Command::new(request.ffmpeg);
    command.args([
        "-hide_banner",
        "-nostdin",
        "-loglevel",
        "error",
        "-y",
        "-framerate",
        &rate,
    ]);
    command.arg("-start_number").arg("0").arg("-i").arg(pattern);
    command.args([
        "-frames:v",
        &manifest.frames.len().to_string(),
        "-an",
        "-vf",
        &scale,
        "-fps_mode",
        "passthrough",
        "-enc_time_base",
        "demux",
        "-pix_fmt",
    ]);
    match recipe.codec() {
        "prores_4444" => {
            command.args([
                "ayuv64le",
                "-c:v",
                "prores_videotoolbox",
                "-allow_sw",
                "1",
                "-profile:v",
                "4444",
            ]);
        }
        "prores_proxy" => {
            command.args([
                "p216le",
                "-c:v",
                "prores_videotoolbox",
                "-allow_sw",
                "1",
                "-profile:v",
                "proxy",
            ]);
        }
        "prores_lt" => {
            command.args([
                "p216le",
                "-c:v",
                "prores_videotoolbox",
                "-allow_sw",
                "1",
                "-profile:v",
                "lt",
            ]);
        }
        "hevc_10" => {
            command.args([
                "p010le",
                "-c:v",
                "hevc_videotoolbox",
                "-allow_sw",
                "1",
                "-profile:v",
                "main10",
                "-tag:v",
                "hvc1",
            ]);
        }
        _ => {
            return Err(ProxyError::InvalidMedia(
                "unknown sequence proxy codec".into(),
            ));
        }
    }
    command.args([
        "-color_primaries",
        primaries,
        "-color_trc",
        transfer,
        "-colorspace",
        matrix,
        "-color_range",
        "tv",
        "-video_track_timescale",
    ]);
    command.arg(manifest.rate.num.to_string());
    command.args(["-movflags", "+use_metadata_tags+write_colr", "-f", "mov"]);
    command.arg(destination);
    let log = NamedTempFile::new_in(request.store.root())?;
    command
        .stdout(Stdio::null())
        .stderr(Stdio::from(log.reopen()?));
    let mut child = ChildGuard {
        child: command.spawn()?,
        finished: false,
    };
    let mut stopped = false;
    let mut last_reason = None;
    loop {
        if request.control.cancel.load(Ordering::Relaxed) {
            return Err(ProxyError::Cancelled);
        }
        if let Some(status) = child.child.try_wait()? {
            child.finished = true;
            if status.success() {
                return Ok(());
            }
            let tail = fs::read_to_string(log.path()).unwrap_or_default();
            let tail = tail
                .chars()
                .rev()
                .take(2048)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>();
            return Err(ProxyError::Codec(format!("{status}: {tail}")));
        }
        let reason = if !disk_ok(request.store.root(), disk_space)? {
            Some(PauseReason::LowDisk)
        } else if (request.control.pause)() {
            Some(PauseReason::InteractiveWork)
        } else {
            None
        };
        if reason != last_reason {
            (request.control.on_pause)(reason);
            last_reason = reason;
        }
        if stopped != reason.is_some() {
            let signal = if reason.is_some() {
                libc::SIGSTOP
            } else {
                libc::SIGCONT
            };
            if unsafe { libc::kill(child.child.id() as i32, signal) } != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            stopped = reason.is_some();
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn probe(ffprobe: &Path, path: &Path) -> Result<Value> {
    let output = Command::new(ffprobe)
        .args(["-v", "error", "-show_streams", "-of", "json"])
        .arg(path)
        .output()?;
    if !output.status.success() {
        return Err(ProxyError::Validation(
            String::from_utf8_lossy(&output.stderr).into(),
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|e| ProxyError::Validation(e.to_string()))
}

fn validate(
    output: &Path,
    ffprobe: &Path,
    manifest: &SequenceManifest,
    recipe: &ProxyRecipe,
    color: SequenceColor,
) -> Result<()> {
    let probe = probe(ffprobe, output)?;
    let streams = probe["streams"]
        .as_array()
        .ok_or_else(|| ProxyError::Validation("sequence proxy streams missing".into()))?;
    if streams.len() != 1 || streams[0]["codec_type"] != "video" {
        return Err(ProxyError::Validation(
            "sequence proxy must contain one video stream".into(),
        ));
    }
    let stream = &streams[0];
    let (primaries, transfer, matrix) = color.tags();
    let expected_tag = match recipe.codec() {
        "prores_4444" => "ap4h",
        "prores_proxy" => "apco",
        "prores_lt" => "apcs",
        "hevc_10" => "hvc1",
        _ => "",
    };
    if stream["codec_tag_string"] != expected_tag
        || stream["width"] != recipe.width
        || stream["height"] != recipe.height
        || stream["color_space"] != matrix
        || stream["color_range"] != "tv"
        || stream["color_primaries"] != primaries
        || stream["color_transfer"] != transfer
    {
        return Err(ProxyError::Validation(format!(
            "sequence proxy codec, raster or color tags differ: {stream}"
        )));
    }
    let native = pm_media::probe::probe(output)
        .map_err(|e| ProxyError::Validation(format!("sequence proxy structure: {e}")))?;
    let video = native
        .probe
        .video
        .as_ref()
        .ok_or_else(|| ProxyError::Validation("sequence proxy video probe missing".into()))?;
    if video.codec.bit_depth < recipe.source_depth.min(10)
        || (recipe.source_depth > 10 && video.codec.bit_depth < 12)
        || (recipe.source_has_alpha && video.alpha.is_none())
    {
        return Err(ProxyError::Validation(
            "sequence proxy lost depth or alpha".into(),
        ));
    }
    let index = native
        .index
        .as_ref()
        .ok_or_else(|| ProxyError::Validation("sequence proxy frame index missing".into()))?;
    if index.frames.len() != manifest.frames.len() {
        return Err(ProxyError::Validation(
            "sequence proxy frame count differs".into(),
        ));
    }
    for (n, frame) in index.frames.iter().enumerate() {
        let left = i128::from(frame.pts)
            * i128::from(index.timebase.num())
            * i128::from(manifest.rate.num);
        let right = n as i128 * i128::from(manifest.rate.den) * i128::from(index.timebase.den());
        if left != right {
            return Err(ProxyError::Validation(
                "sequence proxy presentation timestamps differ".into(),
            ));
        }
    }
    Ok(())
}

pub(crate) fn generate_sequence(
    request: GenerationRequest<'_>,
    disk_space: &dyn Fn(&Path) -> Result<(u64, u64)>,
    manifest: SequenceManifest,
) -> Result<GeneratedProxy> {
    let hash = manifest.content_hash()?;
    if hash != request.source_hash.to_ascii_lowercase() {
        return Err(ProxyError::InvalidMedia(
            "sequence aggregate content hash changed".into(),
        ));
    }
    verify_frames(&manifest, &request.control)?;
    let (width, height, depth, alpha, color) = frame_facts(&manifest, &request.control)?;
    let recipe = ProxyRecipe::new(request.format, request.size, width, height, depth, alpha)?;
    match request.store.lookup(&hash, &recipe) {
        Ok(Some(stored)) => {
            return Ok(GeneratedProxy {
                stored,
                reused: true,
            });
        }
        Err(ProxyError::CorruptStore(_)) => request.store.remove(&hash, &recipe)?,
        Ok(None) => (),
        Err(error) => return Err(error),
    }
    wait_for_disk(&request, disk_space)?;
    let staged = tempfile::Builder::new()
        .prefix("pm-sequence-")
        .tempdir_in(request.store.root())?;
    let destination = NamedTempFile::new_in(request.store.root())?;
    transcode(
        &request,
        &manifest,
        &recipe,
        color,
        destination.path(),
        &staged,
        disk_space,
    )?;
    verify_frames(&manifest, &request.control)?;
    validate(
        destination.path(),
        request.ffprobe,
        &manifest,
        &recipe,
        color,
    )?;
    let stored = request.store.commit(&hash, &recipe, destination)?;
    Ok(GeneratedProxy {
        stored,
        reused: false,
    })
}
