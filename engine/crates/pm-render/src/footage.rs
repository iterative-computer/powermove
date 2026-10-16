//! Footage as interpreted (plan sections 9 and 13): which source frame and
//! field a time shows (section 5 rule 7, a conformed frame rate, field
//! rate or frame rate for interlaced sources), and the display geometry of
//! a converted frame (clean aperture and display rotation).
//!
//! The reading itself comes from `pm_media::interpret` (the probe, section
//! 9's defaults, and the asset's `interpret` overrides); deinterlacing is
//! `pm_gpu::decode_in::deinterlace` (bwdif on normalized Y′CbCr, before
//! chroma upsampling); the color conversion is the source pass.

use pm_gpu::{Target, working_format};
use pm_media::decode::service::MediaSource;
use pm_media::interlace::{self, DeinterlaceOutput, FieldOverride, FieldPlan};
use pm_media::interpret::Interpretation;
use pm_media::interpret::VideoReading;
use pm_media::probe::FieldInfo;
use pm_protocol::color::Precision;
use pm_time::{Rate, Tick};

/// An asset record's `interpret` member (Interpret Footage overrides), or
/// `None` when it has none. Members that cannot be used read as "as the
/// file says" (the sheet and validation report them).
pub fn interpretation(record: &pm_model::Node) -> Option<Interpretation> {
    let member = pm_eval::node::member(Some(record), "interpret")?;
    let text = pm_model::json::write::to_canonical_string(member);
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let (interp, _) = Interpretation::from_json(&value);
    (!interp.is_empty()).then_some(interp)
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
}

impl Timing {
    pub fn new(src: &MediaSource, reading: &VideoReading, composition: Rate) -> Timing {
        let source_rate = reading.conformed().unwrap_or(src.rate());
        let info = FieldInfo {
            order: reading.field_order.value,
            source: String::new(),
        };
        Timing {
            fields: interlace::plan(&info, FieldOverride::Auto, composition, source_rate),
            conformed: reading.conformed(),
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
                // a uniform grid, so no rounding beyond the tick.
                let grid = if field_rate {
                    Rate::new(rate.num() * 2, rate.den()).ok()?
                } else {
                    rate
                };
                let n = grid.frame_of(t);
                if n < 0 {
                    return None;
                }
                let (frame, second) = if field_rate {
                    ((n / 2) as usize, n % 2 == 1)
                } else {
                    (n as usize, false)
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
                let (frame, second) =
                    interlace::select_field(&src.pts, src.index.timebase, src.rate(), t)?;
                Some(Pick {
                    frame,
                    second: second == 1,
                })
            }
            None => Some(Pick {
                frame: src.select(t)?,
                second: false,
            }),
        }
    }
}

/// The display geometry of a converted frame: a crop in coded pixels
/// (the clean aperture) and quarter turns counterclockwise (the display
/// matrix, as `av_display_rotation_get` reports it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    pub crop: Option<(u32, u32, u32, u32)>,
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
            quarter_turns: reading.quarter_turns(),
        }
    }

    pub fn is_identity(&self) -> bool {
        self.crop.is_none() && self.quarter_turns == 0
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
        (cx + u, cy + v)
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct OrientParams {
    /// crop x, y, w, h.
    crop: [u32; 4],
    /// quarter turns, output w, output h, 0.
    turn: [u32; 4],
}

/// The orientation pass (`orient.wgsl`): crop and quarter turns of a
/// working texture into a new one, exactly (no resampling).
pub struct Orient {
    pipelines: [wgpu::ComputePipeline; 2],
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
            pipelines: [make("rgba16float"), make("rgba32float")],
        }
    }

    pub fn run(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        input: &Target,
        geometry: Geometry,
        precision: Precision,
    ) -> Target {
        let coded = (input.width(), input.height());
        let (w, h) = geometry.output_size(coded);
        let out = Target::new(
            device,
            "pm oriented source",
            w,
            h,
            working_format(precision),
            pm_gpu::target::working_usage(),
        );
        let crop = geometry.crop.unwrap_or((0, 0, coded.0, coded.1));
        let params = OrientParams {
            crop: [crop.0, crop.1, crop.2, crop.3],
            turn: [geometry.quarter_turns % 4, w, h, 0],
        };
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pm orient params"),
            size: size_of::<OrientParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&buffer, 0, crate::pipelines::bytes_of(&params));
        let pipeline = match precision {
            Precision::Float16 => &self.pipelines[0],
            Precision::Float32 => &self.pipelines[1],
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
            quarter_turns: 0,
        };
        assert_eq!(out(crop), [1, 2, 5, 6]);
        assert_eq!(crop.output_size(coded), (2, 2));
    }
}
