//! Media sources into working textures (plan sections 9, 10, 13).
//!
//! Images come from `pm-media`'s readers at their native depth; video
//! frames from the decode service, selected by section 5 rule 7. Both reach
//! the GPU without copies through `pm-gpu`'s decode-in import, are
//! normalized there (section 13), and are converted here by the source pass
//! (`shaders/source.wgsl`): chroma upsampling, matrix, transfer and
//! primaries into the working encoding, and premultiplication.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use pm_color::alpha::{shader_code, source_plan};
use pm_color::chroma::{Factors, Filter, filters, sample_origin};
use pm_color::ycbcr::Matrix;
use pm_gpu::decode_in::{DecodeImporter, Normalized};
use pm_gpu::{Target, working_format};
use pm_media::decode::service::{DecodeService, MediaSource, ServiceConfig};
use pm_media::image::icc::IccClass;
use pm_media::pool::FramePool;
use pm_media::probe::FieldOrder;
use pm_media::vt::DecodePurpose;
use pm_protocol::color::{
    AlphaMode, ChromaSiting, ColorDescription, MatrixCoefficients, Precision, Primaries, Transfer,
    WorkingEncoding,
};

use crate::capability::Feature;
use crate::pipelines::{Pipelines, SourceUniform, bytes_of};

/// Why a media file cannot be drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeIssue {
    Unsupported(Feature),
    Unreadable(String),
}

/// What the source pass needs to know about a source.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceColor {
    pub transfer: Transfer,
    pub primaries: Primaries,
    pub matrix: MatrixCoefficients,
    pub siting: ChromaSiting,
    pub alpha: AlphaMode,
}

fn stamp(path: &Path) -> Option<(u64, u128)> {
    let m = std::fs::metadata(path).ok()?;
    let modified = m
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some((m.len(), modified))
}

/// The color reading of a still, or why it cannot be read.
pub fn image_color(still: &pm_media::image::StillImage) -> Result<SourceColor, ProbeIssue> {
    let c = &still.color;
    let primaries = match c.icc_class {
        None | Some(IccClass::Srgb) | Some(IccClass::GraySrgb) => Primaries::Bt709,
        Some(IccClass::DisplayP3) => Primaries::DisplayP3,
        Some(IccClass::Other) => {
            return Err(ProbeIssue::Unsupported(Feature::Media(
                "an ICC profile other than sRGB or Display P3".into(),
            )));
        }
    };
    if let Some(ch) = c.chromaticities {
        let rec709 = [0.64f32, 0.33, 0.30, 0.60, 0.15, 0.06, 0.3127, 0.3290];
        if ch.iter().zip(rec709).any(|(a, b)| (a - b).abs() > 1e-4) {
            return Err(ProbeIssue::Unsupported(Feature::Media(
                "OpenEXR chromaticities other than Rec.709".into(),
            )));
        }
    }
    if matches!(c.transfer, Transfer::Pq | Transfer::Hlg) {
        return Err(ProbeIssue::Unsupported(Feature::Media("HDR stills".into())));
    }
    Ok(SourceColor {
        transfer: c.transfer,
        primaries,
        matrix: MatrixCoefficients::Identity,
        siting: ChromaSiting::Center,
        alpha: c.alpha,
    })
}

type ProbeCache = Mutex<BTreeMap<(PathBuf, u64, u128), Result<(), ProbeIssue>>>;

fn probe_cache() -> &'static ProbeCache {
    static CACHE: OnceLock<ProbeCache> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// Whether an image file can be drawn: readable, not animated, in a tested
/// still class ([`crate::stills`]), a color profile the engine converts.
pub fn probe_image(path: &Path) -> Result<(), ProbeIssue> {
    let Some((len, modified)) = stamp(path) else {
        return Err(ProbeIssue::Unreadable(format!(
            "{} is offline",
            path.display()
        )));
    };
    let key = (path.to_path_buf(), len, modified);
    if let Some(r) = probe_cache().lock().unwrap().get(&key) {
        return r.clone();
    }
    let result = (|| {
        if let Ok(Some(a)) = pm_media::image::animation(path)
            && a.frames > 1
        {
            return Err(ProbeIssue::Unsupported(Feature::ImageSequence));
        }
        let pool = FramePool::new(0);
        let still = pm_media::image::read(path, &pool)
            .map_err(|e| ProbeIssue::Unreadable(format!("{}: {e}", path.display())))?;
        crate::stills::check(path, &still).map_err(ProbeIssue::Unsupported)?;
        image_color(&still).map(|_| ())
    })();
    probe_cache().lock().unwrap().insert(key, result.clone());
    result
}

/// Whether a video file can be drawn: it opens and indexes, is progressive,
/// unrotated, square-pixel, without a cropping clean aperture, SDR, with
/// Rec.709 or BT.601 primaries.
///
/// Answers are kept per file, size, and modification time: capability
/// routing asks at every document revision, and opening a source indexes
/// it.
pub fn probe_video(path: &Path) -> Result<(), String> {
    type VideoProbes = Mutex<HashMap<(PathBuf, u64, u128), Result<(), String>>>;
    static CACHE: OnceLock<VideoProbes> = OnceLock::new();
    let key = stamp(path).map(|(len, modified)| (path.to_path_buf(), len, modified));
    if let Some(key) = &key
        && let Some(r) = CACHE.get_or_init(Default::default).lock().unwrap().get(key)
    {
        return r.clone();
    }
    let result = MediaSource::open(path)
        .map_err(|e| format!("{}: {e}", path.display()))
        .and_then(|src| video_supported(&src));
    if let Some(key) = key {
        CACHE
            .get_or_init(Default::default)
            .lock()
            .unwrap()
            .insert(key, result.clone());
    }
    result
}

fn video_supported(src: &MediaSource) -> Result<(), String> {
    let Some(v) = src.probe.video.as_ref() else {
        return Err("no video stream".into());
    };
    if matches!(
        v.fields.order,
        FieldOrder::TopFirst | FieldOrder::BottomFirst
    ) {
        return Err("interlaced video (deinterlacing is not wired yet)".into());
    }
    if v.rotation.is_some_and(|r| r % 360.0 != 0.0) {
        return Err("rotated video".into());
    }
    if v.pixel_aspect.0 != v.pixel_aspect.1 {
        return Err("non-square pixels".into());
    }
    if let Some(clap) = v.clean_aperture
        && (clap.width != f64::from(v.width)
            || clap.height != f64::from(v.height)
            || clap.horizontal_offset != 0.0
            || clap.vertical_offset != 0.0)
    {
        return Err("a cropping clean aperture".into());
    }
    let c = &src.color;
    if matches!(c.transfer, Transfer::Pq | Transfer::Hlg) {
        return Err("HDR video".into());
    }
    // Rec.709 is the identity; BT.601 primaries read as Rec.709 in Standard
    // (`standard_video_primaries`). Wide gamut sources wait for Interpret
    // and wide-gamut projects.
    if !matches!(
        c.primaries,
        Primaries::Bt709 | Primaries::Bt601_525 | Primaries::Bt601_625
    ) {
        return Err(format!("{:?} primaries", c.primaries));
    }
    Ok(())
}

/// The primaries a video source is read with in Standard. BT.601 525- and
/// 625-line primaries (section 9's reading of untagged SD, and SD tags)
/// pass through as Rec.709: Standard keeps today's look of footage exactly
/// (section 9, "The rule"), and today's viewer (Chromium) does not convert
/// them (measured on the e2e fixtures: a BT.601 red decodes to code 254, 0,
/// 0 there; converting SMPTE C primaries in linear light would show 248, 36,
/// 0). Converting them is an Interpret option for later phases.
pub fn standard_video_primaries(p: Primaries) -> Primaries {
    match p {
        Primaries::Bt601_525 | Primaries::Bt601_625 | Primaries::Unspecified => Primaries::Bt709,
        other => other,
    }
}

/// The source pass parameters for a source.
fn source_uniform(
    color: &SourceColor,
    working: WorkingEncoding,
    precision: Precision,
    chroma: Option<(Factors, u32, u32)>,
) -> SourceUniform {
    let transfer = match color.transfer {
        Transfer::DisplayE => 0,
        Transfer::Bt1886 => 1,
        Transfer::SceneReferredBt709 => 2,
        Transfer::Linear => 3,
        Transfer::Gamma22 => 4,
        // Refused before this point.
        Transfer::Pq | Transfer::Hlg => 0,
    };
    let primaries = pm_color::primaries::conversion(
        if color.primaries == Primaries::Unspecified {
            Primaries::Bt709
        } else {
            color.primaries
        },
        Primaries::Bt709,
    )
    .unwrap_or([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
    let convert_primaries =
        color.primaries != Primaries::Bt709 && color.primaries != Primaries::Unspecified;
    let identity = transfer == 0 && working == WorkingEncoding::Standard && !convert_primaries;
    let plan = source_plan(color.alpha, identity);
    let matte = match color.alpha {
        AlphaMode::PremultipliedWithColor { matte } => {
            [matte[0] as f32, matte[1] as f32, matte[2] as f32, 0.0]
        }
        _ => [0.0; 4],
    };
    let mut u = SourceUniform {
        a: [
            u32::from(chroma.is_some()),
            transfer,
            u32::from(working == WorkingEncoding::LinearLight),
            shader_code(plan),
        ],
        c: [
            0,
            0,
            u32::from(precision == Precision::Float16),
            u32::from(convert_primaries),
        ],
        matte,
        ..Default::default()
    };
    // Primaries columns (the matrix is row-major).
    for (j, column) in u.p.iter_mut().enumerate() {
        *column = [
            primaries[0][j] as f32,
            primaries[1][j] as f32,
            primaries[2][j] as f32,
            0.0,
        ];
    }
    if let Some((factors, cw, ch)) = chroma {
        let (fh, fv) = filters(factors);
        let code = |f: Filter| match f {
            Filter::None => 0,
            Filter::Linear => 1,
            Filter::CatmullRom => 2,
        };
        let (ox, oy) = sample_origin(color.siting, factors);
        u.b = [factors.x, factors.y, code(fh), code(fv)];
        u.c[0] = cw;
        u.c[1] = ch;
        u.origin = [ox as f32, oy as f32, 0.0, 0.0];
        let m = Matrix::of(color.matrix).unwrap_or(Matrix::BT709);
        for (column, c) in u.m.iter_mut().zip(m.to_rgb_columns()) {
            *column = [c[0], c[1], c[2], 0.0];
        }
    }
    u
}

/// Records the source pass from normalized textures into a new working
/// texture of the source's size.
#[allow(clippy::too_many_arguments)]
pub(crate) fn convert(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipelines: &Pipelines,
    encoder: &mut wgpu::CommandEncoder,
    normalized: &Normalized,
    color: &SourceColor,
    working: WorkingEncoding,
    precision: Precision,
    factors: Factors,
) -> Target {
    let (luma, chroma, chroma_info) = match normalized {
        Normalized::Rgb { rgba } => (rgba, None, None),
        Normalized::YCbCr { luma, chroma } => (
            luma,
            Some(chroma),
            Some((factors, chroma.width(), chroma.height())),
        ),
    };
    let (w, h) = (luma.width(), luma.height());
    let out = Target::new(
        device,
        "pm source",
        w,
        h,
        working_format(precision),
        pm_gpu::target::working_usage(),
    );
    let uniform = source_uniform(color, working, precision, chroma_info);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("pm source params"),
        size: size_of::<SourceUniform>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&buffer, 0, bytes_of(&uniform));
    let luma_view = luma.create_view(&Default::default());
    // A 1×1 stand-in when there is no chroma plane.
    let dummy;
    let chroma_view = match chroma {
        Some(c) => c.create_view(&Default::default()),
        None => {
            dummy = Target::new(
                device,
                "pm no chroma",
                1,
                1,
                wgpu::TextureFormat::Rg32Float,
                wgpu::TextureUsages::TEXTURE_BINDING,
            );
            dummy.view.clone()
        }
    };
    let layout = pipelines.source_layout(precision);
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("pm source"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&luma_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&chroma_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&out.view),
            },
        ],
    });
    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("pm source"),
        timestamp_writes: None,
    });
    pass.set_pipeline(pipelines.source(precision));
    pass.set_bind_group(0, &bind, &[]);
    pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
    drop(pass);
    out
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ImageKey {
    path: PathBuf,
    len: u64,
    modified: u128,
    precision: u8,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct FrameKey {
    path: PathBuf,
    frame: usize,
    precision: u8,
    export: bool,
}

/// Media state kept across frames: decoded and converted stills, video
/// sources and sessions, and the last converted frame of each source.
pub struct MediaCache {
    pool: FramePool,
    service: DecodeService,
    importer: Option<DecodeImporter>,
    images: BTreeMap<ImageKey, Target>,
    /// Open video sources with the file's size and modification time when
    /// opened: a file replaced at the same path is reopened.
    sources: BTreeMap<PathBuf, ((u64, u128), Arc<MediaSource>)>,
    frames: BTreeMap<FrameKey, Target>,
    /// "Interpret Footage" overrides by file (section 9, per-source
    /// interpretation): the color reading replaces the probe's.
    interpretations: BTreeMap<PathBuf, ColorDescription>,
    /// Bytes of converted textures held (stills and frames).
    pub bytes: u64,
}

/// A media failure while preparing a frame.
#[derive(Debug, Clone)]
pub enum MediaFailure {
    Unsupported(Feature),
    Failed(String),
}

impl MediaCache {
    pub fn new(device: &wgpu::Device) -> MediaCache {
        MediaCache {
            pool: FramePool::new(64 << 20),
            service: DecodeService::new(ServiceConfig {
                cache_bytes: 64 << 20,
                ..ServiceConfig::default()
            }),
            importer: DecodeImporter::new(device).ok(),
            images: BTreeMap::new(),
            sources: BTreeMap::new(),
            frames: BTreeMap::new(),
            interpretations: BTreeMap::new(),
            bytes: 0,
        }
    }

    fn importer(&self) -> Result<&DecodeImporter, MediaFailure> {
        self.importer
            .as_ref()
            .ok_or_else(|| MediaFailure::Failed("the device cannot import decoded frames".into()))
    }

    /// The working texture of a still.
    #[allow(clippy::too_many_arguments)]
    pub fn image(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipelines: &Pipelines,
        encoder: &mut wgpu::CommandEncoder,
        path: &Path,
        working: WorkingEncoding,
        precision: Precision,
    ) -> Result<Target, MediaFailure> {
        let (len, modified) = stamp(path)
            .ok_or_else(|| MediaFailure::Failed(format!("{} is offline", path.display())))?;
        let key = ImageKey {
            path: path.to_path_buf(),
            len,
            modified,
            precision: precision as u8,
        };
        if let Some(t) = self.images.get(&key) {
            return Ok(t.clone());
        }
        match probe_image(path) {
            Ok(()) => {}
            Err(ProbeIssue::Unsupported(f)) => return Err(MediaFailure::Unsupported(f)),
            Err(ProbeIssue::Unreadable(m)) => return Err(MediaFailure::Failed(m)),
        }
        let still = pm_media::image::read(path, &self.pool)
            .map_err(|e| MediaFailure::Failed(format!("{}: {e}", path.display())))?;
        let color = image_color(&still).map_err(|i| match i {
            ProbeIssue::Unsupported(f) => MediaFailure::Unsupported(f),
            ProbeIssue::Unreadable(m) => MediaFailure::Failed(m),
        })?;
        let importer = self.importer()?;
        let imported = importer
            .import(&still.frame)
            .map_err(|e| MediaFailure::Failed(e.to_string()))?;
        let normalized = importer
            .normalize(queue, encoder, &imported)
            .map_err(|e| MediaFailure::Failed(e.to_string()))?;
        let target = convert(
            device,
            queue,
            pipelines,
            encoder,
            &normalized,
            &color,
            working,
            precision,
            Factors { x: 1, y: 1 },
        );
        // The decoded pixels must outlive the commands reading them; the
        // imported textures hold the frame until wgpu releases them.
        drop(imported);
        // Older versions of the same file are dropped.
        // (Dropped, not destroyed: a frame being recorded may still use it.)
        self.images
            .retain(|k, _| k.path != key.path || k.precision != key.precision);
        self.images.insert(key, target.clone());
        self.recount();
        Ok(target)
    }

    fn source(&mut self, path: &Path) -> Result<Arc<MediaSource>, MediaFailure> {
        let stamp = stamp(path)
            .ok_or_else(|| MediaFailure::Failed(format!("{} is offline", path.display())))?;
        match self.sources.get(path) {
            Some((s, src)) if *s == stamp => return Ok(src.clone()),
            Some(_) => {
                // Replaced on disk: drop the stale index and frames.
                self.sources.remove(path);
                self.frames.retain(|k, _| k.path != path);
                self.recount();
            }
            None => {}
        }
        let src = MediaSource::open(path)
            .map_err(|e| MediaFailure::Failed(format!("{}: {e}", path.display())))?;
        video_supported(&src).map_err(|m| MediaFailure::Unsupported(Feature::Media(m)))?;
        self.sources
            .insert(path.to_path_buf(), (stamp, src.clone()));
        Ok(src)
    }

    /// The working texture of the video frame showing at `seconds` (source
    /// time), or `None` before the first frame.
    #[allow(clippy::too_many_arguments)]
    pub fn video(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipelines: &Pipelines,
        encoder: &mut wgpu::CommandEncoder,
        path: &Path,
        instance: u64,
        seconds: f64,
        export: bool,
        working: WorkingEncoding,
        precision: Precision,
    ) -> Result<Option<Target>, MediaFailure> {
        let src = self.source(path)?;
        let ticks = (seconds * pm_time::TICKS_PER_SECOND as f64).round() as i64;
        let Some(n) = src.select(ticks) else {
            return Ok(None);
        };
        let key = FrameKey {
            path: path.to_path_buf(),
            frame: n,
            precision: precision as u8,
            export,
        };
        if let Some(t) = self.frames.get(&key) {
            return Ok(Some(t.clone()));
        }
        let purpose = if export {
            DecodePurpose::Export
        } else {
            DecodePurpose::Display
        };
        let frame = {
            let mut session = self
                .service
                .session(&src, instance, purpose)
                .map_err(|e| MediaFailure::Failed(e.to_string()))?;
            session
                .frame(n)
                .map_err(|e| MediaFailure::Failed(e.to_string()))?
        };
        let importer = self.importer()?;
        let imported = importer
            .import(&frame)
            .map_err(|e| MediaFailure::Failed(e.to_string()))?;
        let normalized = importer
            .normalize(queue, encoder, &imported)
            .map_err(|e| MediaFailure::Failed(e.to_string()))?;
        let desc: ColorDescription = match self.interpretations.get(path) {
            Some(d) => *d,
            None => frame.color.unwrap_or(src.color),
        };
        let color = SourceColor {
            transfer: desc.transfer,
            primaries: standard_video_primaries(desc.primaries),
            matrix: desc.matrix,
            siting: desc.siting,
            alpha: if frame.format.has_alpha() {
                match desc.alpha {
                    AlphaMode::Opaque => AlphaMode::Straight,
                    other => other,
                }
            } else {
                AlphaMode::Opaque
            },
        };
        let factors = Factors::from_log2(frame.format.log2_chroma_w, frame.format.log2_chroma_h);
        let target = convert(
            device,
            queue,
            pipelines,
            encoder,
            &normalized,
            &color,
            working,
            precision,
            factors,
        );
        drop(imported);
        // Keep only the newest frame per source and precision.
        self.frames.retain(|k, _| {
            k.path != key.path || k.precision != key.precision || k.export != key.export
        });
        self.frames.insert(key, target.clone());
        self.recount();
        Ok(Some(target))
    }

    /// Reads `path` with `color` instead of the probe's interpretation (or
    /// restores the probe's with `None`). Converted frames of the file are
    /// dropped.
    pub fn interpret(&mut self, path: &Path, color: Option<ColorDescription>) {
        match color {
            Some(c) => {
                self.interpretations.insert(path.to_path_buf(), c);
            }
            None => {
                self.interpretations.remove(path);
            }
        }
        self.frames.retain(|k, _| k.path != path);
        self.recount();
    }

    /// The coded size of a video file's frames.
    pub fn video_size(&mut self, path: &Path) -> Result<(u32, u32), MediaFailure> {
        let src = self.source(path)?;
        let v = src
            .probe
            .video
            .as_ref()
            .ok_or_else(|| MediaFailure::Failed("no video stream".into()))?;
        Ok((v.width, v.height))
    }

    /// The color reading of a video file: an override, or the probe's.
    pub fn video_color(&mut self, path: &Path) -> Result<ColorDescription, MediaFailure> {
        if let Some(c) = self.interpretations.get(path) {
            return Ok(*c);
        }
        Ok(self.source(path)?.color)
    }

    /// Bytes of converted textures held.
    fn recount(&mut self) {
        self.bytes = self
            .images
            .values()
            .chain(self.frames.values())
            .map(Target::bytes)
            .sum();
    }

    /// Releases CoreVideo texture cache entries no longer in use.
    pub fn flush(&self) {
        if let Some(i) = &self.importer {
            i.flush_cache();
        }
    }
}
