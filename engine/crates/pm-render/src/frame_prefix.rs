//! The bottom of the viewer's frame kept on the GPU across frames
//! (docs/engine/perf.md, "Frame prefix").
//!
//! The top-level pass draws a composition's layers bottom to top into one
//! target. When a frame's draws begin with the same draws as the last
//! frame's (a drag moves one layer; everything below it is unchanged), the
//! target after those draws is the same pixels as last time. The viewer's
//! renderer keeps a copy of it: the next frame copies it back and draws only
//! from the first draw that changed. Splitting a pass and copying an
//! `rgba16f`/`rgba32f` target are exact, so the frame is the pixels one pass
//! over every draw makes.
//!
//! The copy is kept only when every draw below the split is content whose
//! pixels are a function of the draw alone and whose sources resolved
//! without a failure: solids, stills, vector scenes, and nested
//! compositions and groups of those (no effects or other compositing
//! nodes, video, text, shaders, 3D, or motion-blur samples). The
//! depth and id surface, exports, and plans with compositing nodes at the
//! top level always draw in one pass.

use pm_gpu::Target;

use crate::exec::{Exec, Region, Resolved};
use crate::pipelines::PassBlend;
use crate::plan::{Background, CompPlan, Content, Draw, Source};

/// A split is made only above this many unchanged draws.
const MIN_KEPT: usize = 8;
/// A kept copy is moved up only when this many more draws are unchanged.
const MIN_RAISE: usize = 32;

/// What the kept pixels were drawn for.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PrefixKey {
    pub composition: String,
    pub width: u32,
    pub height: u32,
    pub format: wgpu::TextureFormat,
    pub region: Option<Region>,
    pub precision: pm_protocol::color::Precision,
    pub working: pm_protocol::color::WorkingEncoding,
    pub blend_domain: pm_protocol::color::BlendDomain,
    pub background: Background,
    pub plan_size: (u32, u32),
}

/// A copy of the target after the first draws of a frame, with those draws
/// and the stills they read as they were on disk (the media cache reads a
/// still again when its size or modification time changes).
struct Kept {
    draws: Vec<Draw>,
    stills: Vec<(std::path::PathBuf, Option<(u64, u128)>)>,
    target: Target,
}

/// The stills `draws` read, once each, with their size and modification
/// time now.
fn stills(draws: &[Draw]) -> Vec<(std::path::PathBuf, Option<(u64, u128)>)> {
    fn walk(draws: &[Draw], out: &mut Vec<std::path::PathBuf>) {
        for d in draws {
            let mut c = &d.content;
            if let Content::Placed(p) = c {
                c = &p.content;
            }
            match c {
                Content::Texture {
                    source: Source::Image { path, .. },
                    ..
                } if !out.contains(path) => out.push(path.clone()),
                Content::Texture {
                    source: Source::Comp(child),
                    ..
                } => walk(&child.draws, out),
                _ => {}
            }
        }
    }
    let mut paths = Vec::new();
    walk(draws, &mut paths);
    paths
        .into_iter()
        .map(|p| {
            let s = crate::media::stamp(&p);
            (p, s)
        })
        .collect()
}

/// The last viewer frame's top-level draws (where the next frame's split
/// goes) and the kept copy (valid while a frame starts with its draws).
#[derive(Default)]
pub struct FramePrefix {
    key: Option<PrefixKey>,
    draws: Vec<Draw>,
    kept: Option<Kept>,
    /// Frames drawn from a kept copy, and draws they did not draw again.
    pub(crate) reused_frames: u64,
    pub(crate) reused_draws: u64,
}

impl FramePrefix {
    /// Forgets the last frame (and gives the copy back to `pool`).
    pub(crate) fn clear(&mut self, pool: &mut pm_gpu::TargetPool) {
        if let Some(k) = self.kept.take() {
            pool.give(k.target);
        }
        self.key = None;
        self.draws.clear();
    }

    /// Remembers this frame's draws (after a complete frame).
    pub(crate) fn remember(&mut self, draws: Vec<Draw>) {
        self.draws = draws;
    }
}

/// Whether a draw's pixels depend on nothing but the draw (and its sources
/// resolve from caches keyed by what the draw names).
fn stable(draw: &Draw) -> bool {
    fn content(c: &Content) -> bool {
        match c {
            Content::Solid(_) => true,
            Content::Placed(p) => content(&p.content),
            Content::Texture { source, .. } => match source {
                Source::Image { .. } => true,
                Source::Vector(v) => matches!(**v, crate::vector::VectorSource::Scene { .. }),
                Source::Comp(child) => child.draws.iter().all(stable),
                _ => false,
            },
            Content::Composite(_) => false,
        }
    }
    content(&draw.content)
}

fn copy(encoder: &mut wgpu::CommandEncoder, from: &Target, to: &Target) {
    encoder.copy_texture_to_texture(
        from.texture.as_image_copy(),
        to.texture.as_image_copy(),
        from.texture.size(),
    );
}

/// Draws `plan`'s top-level pass into `target` (cleared to its
/// background), from the kept copy when the frame starts like the last
/// one. Returns the resolved draws it drew and the layers of the draws the
/// copy stood for (the legend of the frame is those, then the resolved).
pub(crate) fn render(
    exec: &mut Exec<'_>,
    encoder: &mut wgpu::CommandEncoder,
    plan: &CompPlan,
    region: Option<Region>,
    target: &Target,
    prefix: &mut FramePrefix,
    key: PrefixKey,
) -> (Vec<Resolved>, Vec<String>) {
    if prefix.key.as_ref() != Some(&key) {
        prefix.clear(exec.pool);
        prefix.key = Some(key);
    }
    let draws = &plan.draws;
    // The first draw that differs from the last frame's.
    let same = prefix
        .draws
        .iter()
        .zip(draws)
        .take_while(|(a, b)| a == b)
        .count();
    // Draw from the kept copy when the frame starts with its draws.
    let start = match &prefix.kept {
        Some(kept)
            if kept.draws.len() <= draws.len()
                && kept.draws[..] == draws[..kept.draws.len()]
                && kept
                    .stills
                    .iter()
                    .all(|(p, s)| crate::media::stamp(p) == *s) =>
        {
            copy(encoder, &kept.target, target);
            prefix.reused_frames += 1;
            prefix.reused_draws += kept.draws.len() as u64;
            pm_perf::bump(pm_perf::Counter::FramePrefixHits);
            kept.draws.len()
        }
        _ => {
            pm_perf::bump(pm_perf::Counter::FramePrefixMisses);
            0
        }
    };
    let legend: Vec<String> = draws[..start].iter().map(|d| d.layer.clone()).collect();
    let clear = start == 0;
    let raise = same >= MIN_KEPT
        && (start == 0 || same >= start + MIN_RAISE)
        && draws[start..same].iter().all(stable);
    if !raise {
        if start == 0
            && let Some(k) = prefix.kept.take()
        {
            exec.pool.give(k.target);
        }
        let resolved = exec.render_draws(
            encoder,
            plan,
            &draws[start..],
            region,
            target,
            PassBlend::Over,
            clear,
        );
        return (resolved, legend);
    }
    // Draw up to the first change, keep a copy, then the rest. The stills'
    // stamps are read before they are drawn: a file that changes meanwhile
    // invalidates the copy.
    let stamps = stills(&draws[..same]);
    let reported = (exec.failures.len(), exec.unsupported.len());
    let mut resolved = exec.render_draws(
        encoder,
        plan,
        &draws[start..same],
        region,
        target,
        PassBlend::Over,
        clear,
    );
    let clean = (exec.failures.len(), exec.unsupported.len()) == reported
        && resolved
            .iter()
            .all(|r| r.solid.is_some() || r.view.is_some());
    if clean {
        let kept = match prefix.kept.take() {
            Some(k) => k.target,
            None => exec.pool.take(
                exec.device,
                target.width(),
                target.height(),
                target.format(),
                pm_gpu::target::working_usage(),
            ),
        };
        copy(encoder, target, &kept);
        prefix.kept = Some(Kept {
            draws: draws[..same].to_vec(),
            stills: stamps,
            target: kept,
        });
    } else if let Some(k) = prefix.kept.take() {
        exec.pool.give(k.target);
    }
    resolved.extend(exec.render_draws(
        encoder,
        plan,
        &draws[same..],
        region,
        target,
        PassBlend::Over,
        false,
    ));
    (resolved, legend)
}
