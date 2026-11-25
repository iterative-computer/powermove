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
//! The copy is kept only for self-contained draws. Compositing nodes
//! such as adjustments can read that copy as their accumulator.

use std::sync::{Arc, Weak};

use pm_gpu::Target;

use crate::exec::{Exec, Region, Resolved};
use crate::pipelines::PassBlend;
use crate::plan::{Background, CompPlan, Content, Draw, OutputView, Source};
use crate::shader_layers::ProgramRef;

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
    pub view: Option<OutputView>,
    /// A preview (playback-resampled) frame's pixels never seed a final one.
    pub sampling: crate::sampling::Sampling,
    /// Final output plans levels at the composition's density where their
    /// effects need it.
    pub final_output: bool,
}

/// A copy of the target after the first draws of a frame, with those draws
/// and the stills they read as they were on disk (the media cache reads a
/// still again when its size or modification time changes).
struct Kept {
    draws: Vec<Draw>,
    stills: Vec<(std::path::PathBuf, Option<(u64, u128)>)>,
    shaders: Vec<pm_glsl::Key>,
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

    pub(crate) fn bytes(&self) -> u64 {
        self.kept.as_ref().map_or(0, |k| k.target.bytes())
    }

    /// Remembers this frame's draws (after a complete frame).
    pub(crate) fn remember(&mut self, draws: Vec<Draw>) {
        self.draws = draws;
    }
}

/// Whether a draw's pixels depend on nothing but the draw (and its sources
/// resolve from caches keyed by what the draw names).
pub(crate) fn stable(content: &Content, stills: bool) -> bool {
    match content {
        Content::Solid(_) => true,
        Content::Placed(p) => stable(&p.content, stills),
        Content::Texture { source, .. } => match source {
            Source::Image { .. } => stills,
            Source::Text(_) => true,
            Source::Vector(v) => matches!(**v, crate::vector::VectorSource::Scene { .. }),
            Source::Shader(s) => {
                s.scene_error.is_none() && matches!(&s.program, ProgramRef::Code { .. })
            }
            Source::Comp(child) => child.draws.iter().all(|d| stable(&d.content, stills)),
            _ => false,
        },
        Content::Composite(_) => false,
    }
}

pub(crate) fn shader_keys(draws: &[Draw]) -> Vec<pm_glsl::Key> {
    fn walk(content: &Content, keys: &mut Vec<pm_glsl::Key>) {
        match content {
            Content::Placed(p) => walk(&p.content, keys),
            Content::Texture {
                source: Source::Comp(plan),
                ..
            } => {
                for draw in &plan.draws {
                    walk(&draw.content, keys);
                }
            }
            Content::Texture {
                source: Source::Shader(s),
                ..
            } => {
                if let ProgramRef::Code { key, .. } = &s.program
                    && !keys.contains(key)
                {
                    keys.push(*key);
                }
            }
            _ => {}
        }
    }
    let mut shaders = Vec::new();
    for draw in draws {
        walk(&draw.content, &mut shaders);
    }
    shaders
}

fn equal_draw(a: &Draw, b: &Draw) -> bool {
    fn equal_content(a: &Content, b: &Content) -> bool {
        match (a, b) {
            (Content::Placed(a), Content::Placed(b)) => {
                a.plane == b.plane && a.level2 == b.level2 && equal_content(&a.content, &b.content)
            }
            (
                Content::Texture {
                    source: a,
                    uv: a_uv,
                    alpha: a_alpha,
                },
                Content::Texture {
                    source: b,
                    uv: b_uv,
                    alpha: b_alpha,
                },
            ) => {
                a_uv == b_uv
                    && a_alpha == b_alpha
                    && match (a, b) {
                        (Source::Comp(a), Source::Comp(b)) if Arc::ptr_eq(a, b) => true,
                        (Source::Vector(a), Source::Vector(b)) if Arc::ptr_eq(a, b) => true,
                        _ => a == b,
                    }
            }
            _ => a == b,
        }
    }
    a.layer == b.layer && a.matrix == b.matrix && equal_content(&a.content, &b.content)
}

fn copy(encoder: &mut wgpu::CommandEncoder, from: &Target, to: &Target) {
    encoder.copy_texture_to_texture(
        from.texture.as_image_copy(),
        to.texture.as_image_copy(),
        from.texture.size(),
    );
}

/// Seeds the accumulator with its stable bottom, before either renderer
/// processes the rest of the plan. A zero count leaves the target alone.
pub(crate) fn seed(
    exec: &mut Exec<'_>,
    encoder: &mut wgpu::CommandEncoder,
    plan: &CompPlan,
    region: Option<Region>,
    target: &Target,
    prefix: &mut FramePrefix,
    mut key: PrefixKey,
) -> (usize, Vec<Resolved>, Vec<String>) {
    // An adjustment can expand the accumulator beyond the requested ROI.
    key.width = target.width();
    key.height = target.height();
    key.region = region;
    if prefix.key.as_ref() != Some(&key) {
        prefix.clear(exec.pool);
        prefix.key = Some(key);
    }
    let draws = &plan.draws;
    let same = prefix
        .draws
        .iter()
        .zip(draws)
        .take_while(|(a, b)| !matches!(b.content, Content::Composite(_)) && equal_draw(a, b))
        .count();
    let start = match &prefix.kept {
        Some(kept)
            if kept.draws.len() <= draws.len()
                && kept.draws.iter().zip(draws).all(|(a, b)| equal_draw(a, b))
                && kept
                    .stills
                    .iter()
                    .all(|(p, s)| crate::media::stamp(p) == *s)
                && kept
                    .shaders
                    .iter()
                    .all(|key| exec.shaders.can_reuse_pixels(key, &exec.effects.watchdog)) =>
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
    let legend = draws[..start].iter().map(|d| d.layer.clone()).collect();
    let same = (start..same)
        .take_while(|&i| stable(&draws[i].content, true))
        .last()
        .map_or(start, |i| i + 1);
    if same < MIN_KEPT
        || (start != 0 && same < start + MIN_RAISE)
        || target.bytes() + exec.groups.as_ref().map_or(0, |groups| groups.bytes())
            > MAX_RETAINED_BYTES
    {
        if start == 0
            && let Some(k) = prefix.kept.take()
        {
            exec.pool.give(k.target);
        }
        return (start, Vec::new(), legend);
    }
    // Draw up to the first change, keep a copy, then the rest. The stills'
    // stamps are read before they are drawn: a file that changes meanwhile
    // invalidates the copy.
    if let Some(groups) = &mut exec.groups {
        groups.prefix_bytes = target.bytes();
    }
    let stamps = stills(&draws[..same]);
    let reported = (exec.failures.len(), exec.unsupported.len());
    let resolved = exec.render_draws(
        encoder,
        plan,
        &draws[start..same],
        region,
        target,
        PassBlend::Over,
        start == 0,
    );
    let clean = (exec.failures.len(), exec.unsupported.len()) == reported
        && resolved
            .iter()
            .all(|r| r.solid.is_some() || r.view.is_some());
    let shaders = shader_keys(&draws[..same]);
    if clean
        && shaders
            .iter()
            .all(|key| exec.shaders.can_reuse_pixels(key, &exec.effects.watchdog))
    {
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
            shaders,
            target: kept,
        });
    } else if let Some(k) = prefix.kept.take() {
        exec.pool.give(k.target);
    }
    if let Some(groups) = &mut exec.groups {
        groups.prefix_bytes = prefix.bytes();
    }
    (same, resolved, legend)
}

const MAX_RETAINED_BYTES: u64 = 64 * 1024 * 1024;

struct GroupTarget {
    plan: Weak<CompPlan>,
    region: Option<Region>,
    target: Target,
    revision: u64,
    used: bool,
    shaders: Vec<pm_glsl::Key>,
}

/// Boundary groups can sit above the edited layer as well as below it.
#[derive(Default)]
pub(crate) struct GroupCache {
    key: Option<PrefixKey>,
    targets: Vec<GroupTarget>,
    prefix_bytes: u64,
    pub(crate) hits: u64,
}

impl GroupCache {
    pub(crate) fn begin(&mut self, key: &PrefixKey, prefix_bytes: u64) {
        if self.key.as_ref() != Some(key) {
            self.clear();
            self.key = Some(key.clone());
        }
        self.prefix_bytes = prefix_bytes;
        for entry in &mut self.targets {
            entry.used = false;
        }
    }

    pub(crate) fn finish(&mut self) {
        self.targets.retain(|t| t.used);
    }

    pub(crate) fn clear(&mut self) {
        self.targets.clear();
        self.key = None;
    }

    pub(crate) fn bytes(&self) -> u64 {
        self.targets.iter().map(|t| t.target.bytes()).sum()
    }

    pub(crate) fn get(
        &mut self,
        plan: &Arc<CompPlan>,
        region: Option<Region>,
        shader_valid: impl Fn(&pm_glsl::Key) -> bool,
    ) -> Option<(Target, u64)> {
        let entry = self.targets.iter_mut().find(|t| {
            t.plan.as_ptr() == Arc::as_ptr(plan)
                && t.region == region
                && t.shaders.iter().all(&shader_valid)
        })?;
        self.hits += 1;
        entry.used = true;
        Some((entry.target.clone(), entry.revision))
    }

    pub(crate) fn keep(
        &mut self,
        plan: &Arc<CompPlan>,
        region: Option<Region>,
        target: &Target,
        revision: u64,
        shader_valid: impl Fn(&pm_glsl::Key) -> bool,
    ) -> bool {
        if Arc::strong_count(plan) <= 1
            || self.targets.len() >= 8
            || self.prefix_bytes + self.bytes() + target.bytes() > MAX_RETAINED_BYTES
            || !plan.draws.iter().all(|d| stable(&d.content, false))
        {
            return false;
        }
        let shaders = shader_keys(&plan.draws);
        if !shaders.iter().all(shader_valid) {
            return false;
        }
        self.targets.push(GroupTarget {
            plan: Arc::downgrade(plan),
            region,
            target: target.clone(),
            revision,
            used: true,
            shaders,
        });
        true
    }
}
