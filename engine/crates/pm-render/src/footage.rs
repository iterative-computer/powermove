//! Footage as interpreted (plan sections 9 and 13): which source frame and
//! field a time shows (section 5 rule 7, a conformed frame rate, field
//! rate or frame rate for interlaced sources), and the display geometry of
//! a converted frame (clean aperture and display rotation).
//!
//! The reading itself comes from `pm_media::interpret` (the probe, section
//! 9's defaults, and the asset's `interpret` overrides); deinterlacing is
//! `pm_gpu::decode_in::deinterlace` (bwdif on normalized Y′CbCr, before
//! chroma upsampling); the color conversion is the source pass.

use pm_gpu::Target;
use pm_media::clip::ClipTiming;
use pm_media::decode::service::MediaSource;
use pm_media::interlace::{self, DeinterlaceOutput, FieldOverride, FieldPlan};
use pm_media::interpret::VideoReading;
use pm_media::interpret::{Interpretation, OriginalReading};
use pm_media::probe::FieldInfo;
use pm_time::{Rate, Tick};

/// How an asset record says its file is read: the `interpret` member
/// (Interpret Footage overrides; members that cannot be used read as "as
/// the file says", and the sheet and validation report them) and the
/// record's `timing` member ([`clip_timing`]). `None` when the record
/// overrides nothing and its times are an older project's.
pub fn interpretation(record: &pm_model::Node) -> Option<Interpretation> {
    let mut interp = pm_eval::node::member(Some(record), "interpret")
        .and_then(|member| {
            let text = pm_model::json::write::to_canonical_string(member);
            let value: serde_json::Value = serde_json::from_str(&text).ok()?;
            Some(Interpretation::from_json(&value).0)
        })
        .unwrap_or_default();
    interp.electron_color = pm_eval::node::is_str(
        pm_eval::node::member(Some(record), "videoColor"),
        "electron",
    );
    interp.clip = clip_timing(Some(record));
    (!interp.is_empty()).then_some(interp)
}

/// Where a clip drawn from a converted copy of its original (a proxy or
/// optimized media) reads its frames, and with which interpretation.
#[derive(Clone, Debug, PartialEq)]
pub enum CopyReading {
    /// The copy, read with this interpretation.
    Copy(Option<Interpretation>),
    /// The original: the copy cannot carry this interpretation.
    Original(Interpretation),
}

/// An asset's interpretation as it applies to a converted copy of its
/// original. Interpret Footage describes the original file; the copy's
/// maker (`pm_proxy`) tags the copy with the original's reading and writes
/// limited-range Y′CbCr.
/// - A copy of a limited-range Y′CbCr original holds the original's codes,
///   so every member applies to it as it stands.
/// - A copy of a full-range Y′CbCr original holds its codes rescaled to
///   limited range: matrix, siting and the rest apply, but a corrected range
///   says the original's codes were never full-range ones, which the
///   rescaled copy cannot express, so the clip is drawn from its original.
/// - A copy of an R′G′B′ original was converted to Y′CbCr by its maker,
///   which chose the matrix and range and tagged them: the asset's matrix,
///   range and chroma siting do nothing on the original and would misread
///   the copy, so the copy keeps its own.
///
/// The copy is read with its original's reading ([`OriginalReading`]): the
/// file's, or Electron's for an asset that keeps a v1 project's reading
/// (`videoColor`), so a copy tagged by another recipe or with the native
/// reading never changes the picture. Interpret Footage choices apply on top.
///
/// An original that can't be read is taken as limited-range Y′CbCr, what
/// cameras and editors write, and the copy keeps its own tags.
pub fn for_copy(
    mut interp: Interpretation,
    original: &std::path::Path,
    sequence: bool,
) -> CopyReading {
    use crate::media::StoredCodes;
    use pm_protocol::color::Range;
    let facts = crate::media::original_facts(original);
    if interp.matrix.is_some() || interp.range.is_some() || interp.siting.is_some() {
        match facts.as_ref().map(|f| f.codes) {
            Some(StoredCodes::Rgb) => {
                interp.matrix = None;
                interp.range = None;
                interp.siting = None;
            }
            Some(StoredCodes::YCbCr(Range::Full)) if interp.range.is_some() => {
                return CopyReading::Original(interp);
            }
            _ => (),
        }
    }
    if let Some(facts) = facts {
        use pm_media::probe::color::{electron_frame_reading, frame_reading};
        let reading = if interp.electron_color {
            electron_frame_reading(&facts.color, &facts.codec, sequence)
        } else {
            frame_reading(&facts.color, sequence)
        };
        interp.original = Some(OriginalReading {
            primaries: reading.primaries,
            transfer: reading.transfer,
            matrix: (facts.codes != StoredCodes::Rgb).then_some(reading.matrix),
        });
    }
    CopyReading::Copy((!interp.is_empty()).then_some(interp))
}

/// Which reading of its file's timestamps an asset's stored times address
/// (`pm_media::clip`): the record's `timing` member.
pub fn clip_timing(record: Option<&pm_model::Node>) -> ClipTiming {
    ClipTiming::of_asset(pm_eval::node::number(pm_eval::node::member(
        record, "timing",
    )))
}

/// Persisted animated imports can identify their sequence by format even
/// when older asset records have no `imageSequence` slot metadata.
pub fn is_sequence(record: &pm_model::Node) -> bool {
    use pm_eval::node;
    let member = |name| node::member(Some(record), name);
    node::is_str(member("kind"), "video")
        && (node::truthy(member("imageSequence"))
            || ["gif", "webp", "png", "apng", "avif"]
                .iter()
                .any(|format| node::is_str(member("format"), format)))
}

/// Source-only consumers use the same persisted image-sequence slots as
/// composition. A conformed rate overrides those slots, as in the planner.
pub fn source_time(
    record: Option<&pm_model::Node>,
    time: pm_protocol::common::Ticks,
    interpretation: Option<&Interpretation>,
) -> pm_protocol::common::Ticks {
    if interpretation.is_some_and(|i| i.frame_rate.is_some()) {
        return time;
    }
    let seconds = time.get() as f64 / pm_time::TICKS_PER_SECOND as f64;
    record
        .filter(|record| is_sequence(record))
        .and_then(|record| crate::media::sequence_playback_time(record, seconds))
        .and_then(|seconds| {
            pm_protocol::common::Ticks::new(
                (seconds * pm_time::TICKS_PER_SECOND as f64).round() as i64
            )
            .ok()
        })
        .unwrap_or(time)
}

/// What a composition time shows of a source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pick {
    /// Presented frame index.
    pub frame: usize,
    /// The second field in display order (field-rate output only).
    pub second: bool,
}

/// How a source's frames become pictures for a composition.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Timing {
    pub fields: FieldPlan,
    /// A conformed frame rate (Interpret Footage): frame n shows from n / rate.
    pub conformed: Option<Rate>,
    /// The asset's reading of the file's timestamps.
    pub clip: ClipTiming,
}

impl Timing {
    /// A source monitor has no composition cadence: retain every field at
    /// its source time, using the resolved field order and conformed grid.
    pub fn source(reading: &VideoReading) -> Timing {
        let order = reading.field_order.value;
        Timing {
            fields: FieldPlan {
                order,
                output: match order {
                    pm_media::probe::FieldOrder::TopFirst
                    | pm_media::probe::FieldOrder::BottomFirst => DeinterlaceOutput::FieldRate,
                    _ => DeinterlaceOutput::Off,
                },
            },
            conformed: reading.conformed(),
            clip: reading.clip,
        }
    }

    pub fn new(src: &MediaSource, reading: &VideoReading, composition: Rate) -> Timing {
        let source_rate = reading.conformed().unwrap_or(src.rate());
        let info = FieldInfo {
            order: reading.field_order.value,
            source: String::new(),
        };
        Timing {
            fields: interlace::plan(&info, FieldOverride::Auto, composition, source_rate),
            conformed: reading.conformed(),
            clip: reading.clip,
        }
    }

    pub fn deinterlaced(&self) -> bool {
        self.fields.output != DeinterlaceOutput::Off
    }

    /// The frame (and field) showing at source time `t` (ticks).
    pub fn pick(&self, src: &MediaSource, t: Tick) -> Option<Pick> {
        let frames = src.pts.len();
        if frames == 0 {
            return None;
        }
        let field_rate = self.fields.output == DeinterlaceOutput::FieldRate;
        match self.conformed {
            Some(rate) => {
                // Frame n (or field 2n + j at twice the rate) from n / rate:
                // section 5 rule 7 on the uniform grid (the last picture
                // starting at or before t + q/2, `crate::retime`).
                let grid = if field_rate {
                    Rate::new(rate.num() * 2, rate.den()).ok()?
                } else {
                    rate
                };
                let n = crate::retime::override_frame_at(t, grid)?;
                let (frame, second) = if field_rate {
                    (n / 2, n % 2 == 1)
                } else {
                    (n, false)
                };
                if frame >= frames {
                    // Past the end: the last picture holds.
                    return Some(Pick {
                        frame: frames - 1,
                        second: field_rate,
                    });
                }
                Some(Pick { frame, second })
            }
            None if field_rate => {
                let pts = src.clip_pts(self.clip);
                let picked = interlace::select_field(pts, src.index.timebase, src.rate(), t);
                let (frame, second) = match self.clip {
                    ClipTiming::Presentation => picked?,
                    // The first shown frame holds before its own time.
                    ClipTiming::Legacy => picked
                        .filter(|(frame, _)| *frame >= src.legacy.head)
                        .unwrap_or((src.legacy.head, 0)),
                };
                Some(Pick {
                    frame,
                    second: second == 1,
                })
            }
            None => Some(Pick {
                frame: src.select_as(self.clip, t)?,
                second: false,
            }),
        }
    }
}

/// The display geometry of a converted frame: a crop in coded pixels
/// (the clean aperture), a horizontal mirror of the cropped input, and
/// quarter turns counterclockwise (the display matrix, as
/// `av_display_rotation_get` reports it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    pub crop: Option<(u32, u32, u32, u32)>,
    pub mirror: bool,
    pub quarter_turns: u32,
}

impl Geometry {
    pub fn of(reading: &VideoReading, coded: (u32, u32)) -> Geometry {
        let crop = reading.clean_aperture.value.map(|c| {
            let (x, y, w, h) = c.rect;
            let x = x.min(coded.0.saturating_sub(1));
            let y = y.min(coded.1.saturating_sub(1));
            (x, y, w.clamp(1, coded.0 - x), h.clamp(1, coded.1 - y))
        });
        Geometry {
            crop,
            mirror: false,
            quarter_turns: reading.quarter_turns(),
        }
    }

    /// An EXIF orientation (1 to 8) as a mirror and counterclockwise turns.
    pub fn exif(orientation: u8) -> Geometry {
        let (mirror, quarter_turns) = match orientation {
            2 => (true, 0),
            3 => (false, 2),
            4 => (true, 2),
            5 => (true, 1),
            6 => (false, 3),
            7 => (true, 3),
            8 => (false, 1),
            _ => (false, 0),
        };
        Geometry {
            crop: None,
            mirror,
            quarter_turns,
        }
    }

    pub fn is_identity(&self) -> bool {
        self.crop.is_none() && !self.mirror && self.quarter_turns == 0
    }

    /// The output size for a coded size.
    pub fn output_size(&self, coded: (u32, u32)) -> (u32, u32) {
        let (w, h) = self.crop.map_or(coded, |c| (c.2, c.3));
        if self.quarter_turns % 2 == 1 {
            (h, w)
        } else {
            (w, h)
        }
    }

    /// The input pixel an output pixel shows (exact: a crop and a
    /// permutation). The same mapping runs in `orient.wgsl`.
    pub fn source_of(&self, x: u32, y: u32, coded: (u32, u32)) -> (u32, u32) {
        let (cx, cy, cw, ch) = self.crop.unwrap_or((0, 0, coded.0, coded.1));
        let (u, v) = match self.quarter_turns % 4 {
            // Content turned 90° counterclockwise: the input's right column
            // becomes the top row.
            1 => (cw - 1 - y, x),
            2 => (cw - 1 - x, ch - 1 - y),
            3 => (y, ch - 1 - x),
            _ => (x, y),
        };
        let u = if self.mirror { cw - 1 - u } else { u };
        (cx + u, cy + v)
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct OrientParams {
    /// crop x, y, w, h.
    crop: [u32; 4],
    /// quarter turns, output w, output h, mirror.
    turn: [u32; 4],
}

/// The orientation pass (`orient.wgsl`): crop and quarter turns of a
/// working texture into a new one, exactly (no resampling).
pub struct Orient {
    pipelines: [wgpu::ComputePipeline; 3],
}

impl Orient {
    pub fn new(device: &wgpu::Device) -> Orient {
        let make = |name: &str| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("pm-render orient"),
                source: wgpu::ShaderSource::Wgsl(
                    include_str!("shaders/orient.wgsl")
                        .replace("OUT_FORMAT", name)
                        .into(),
                ),
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("pm-render orient"),
                layout: None,
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        Orient {
            pipelines: [make("rgba16float"), make("rgba32float"), make("rgba8unorm")],
        }
    }

    /// The oriented copy of `input` in `format` storage (one of the
    /// working formats).
    pub fn run(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        input: &Target,
        geometry: Geometry,
        format: wgpu::TextureFormat,
    ) -> Target {
        let coded = (input.width(), input.height());
        let (w, h) = geometry.output_size(coded);
        let out = Target::new(
            device,
            "pm oriented source",
            w,
            h,
            format,
            pm_gpu::target::working_usage(),
        );
        let crop = geometry.crop.unwrap_or((0, 0, coded.0, coded.1));
        let params = OrientParams {
            crop: [crop.0, crop.1, crop.2, crop.3],
            turn: [geometry.quarter_turns % 4, w, h, u32::from(geometry.mirror)],
        };
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pm orient params"),
            size: size_of::<OrientParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&buffer, 0, crate::pipelines::bytes_of(&params));
        let pipeline = match format {
            wgpu::TextureFormat::Rgba8Unorm => &self.pipelines[2],
            wgpu::TextureFormat::Rgba32Float => &self.pipelines[1],
            _ => &self.pipelines[0],
        };
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pm orient"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&input.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&out.view),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("pm orient"),
            timestamp_writes: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        drop(pass);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quarter_turns_permute_exactly() {
        let coded = (4, 2);
        let g = |t| Geometry {
            crop: None,
            mirror: false,
            quarter_turns: t,
        };
        // 0 1 2 3 / 4 5 6 7, as (x, y) of the input.
        let idx = |(x, y): (u32, u32)| y * 4 + x;
        let out = |gm: Geometry| {
            let (w, h) = gm.output_size(coded);
            (0..h)
                .flat_map(|y| (0..w).map(move |x| (x, y)))
                .map(|(x, y)| idx(gm.source_of(x, y, coded)))
                .collect::<Vec<_>>()
        };
        assert_eq!(out(g(0)), [0, 1, 2, 3, 4, 5, 6, 7]);
        // 90° counterclockwise: right column on top.
        assert_eq!(out(g(1)), [3, 7, 2, 6, 1, 5, 0, 4]);
        assert_eq!(out(g(2)), [7, 6, 5, 4, 3, 2, 1, 0]);
        assert_eq!(out(g(3)), [4, 0, 5, 1, 6, 2, 7, 3]);
        let crop = Geometry {
            crop: Some((1, 0, 2, 2)),
            mirror: false,
            quarter_turns: 0,
        };
        assert_eq!(out(crop), [1, 2, 5, 6]);
        assert_eq!(crop.output_size(coded), (2, 2));
        // EXIF orientations are the permutations pm-media applies to packed
        // pixels (`pm_media::image::orient`).
        let input: Vec<u32> = (0..8).collect();
        for o in 1..=8u8 {
            let (want, ..) = pm_media::image::orient(&input, 4, 2, 1, o);
            assert_eq!(out(Geometry::exif(o)), want, "orientation {o}");
        }
    }
}
