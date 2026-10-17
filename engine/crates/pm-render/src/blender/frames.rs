//! The Blender frames of one output frame: today's `blenderFrameRequests`
//! (`legacy/core/frame-preparation.ts`), which mirrors nested output sizes
//! and shutter times before compositing (section 17: surfaces per frame,
//! shutter sample, and nested composition).
//!
//! From the root at its output size, each composition visited at a time
//! and size (at most 8 levels deep, at most 4,096 distinct frames):
//!
//! - when any active layer has motion blur (and the render asks for it),
//!   its times are the frame's time followed by `n` shutter samples at
//!   `t + ((i + 0.5) / n − 0.5) × shutter / fps` (`n` is the render's
//!   sample count, 10 by default);
//! - at each time, a composition with `render3d.enabled` and an active 3D
//!   object (judged at the frame's time) is a Blender frame;
//! - every precomp layer active at that time is visited at its source time
//!   and at `max(2, round((d.w || comp.w || width) × width / (comp.w ||
//!   width)))` by the same for height.
//!
//! Variable speed integrates as the engine does (amendment 16), so nested
//! retimed frames differ from today's only where amendment 16 says.

use std::collections::HashSet;

use pm_blender::snapshot::BlenderFrame;
use pm_eval::node;
use pm_eval::{Comp, Evaluator, JsResult, Scope};
use pm_model::Node;

use crate::js;

/// `prepareFrame`'s options.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameOptions {
    /// The output size (default: the root composition's size).
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// `options.mblur !== false`.
    pub motion_blur: bool,
    /// `options.mbSamples || 10`.
    pub samples: u32,
    /// `options.shutter || project.shutter || 0.5` (fraction of a frame).
    pub shutter: Option<f64>,
}

impl Default for FrameOptions {
    fn default() -> Self {
        FrameOptions {
            width: None,
            height: None,
            motion_blur: true,
            samples: 10,
            shutter: None,
        }
    }
}

/// Why the walk stopped (today's throws).
pub const TOO_DEEP: &str = "Composition nesting is too deep";
pub const TOO_MANY: &str = "Motion blur requires too many nested frames. Reduce shutter samples or composition nesting.";

fn number(n: Option<&Node>) -> f64 {
    node::to_number(n)
}

/// `a || b` over numbers.
fn or(a: f64, b: f64) -> f64 {
    if a == 0.0 || a.is_nan() { b } else { a }
}

struct Walk {
    frames: Vec<BlenderFrame>,
    keys: HashSet<(String, u64, u32, u32)>,
    visited: HashSet<(String, u64, u32, u32)>,
    options: FrameOptions,
    shutter: f64,
}

#[allow(clippy::too_many_arguments)]
fn visit<'d>(
    walk: &mut Walk,
    ev: &mut Evaluator,
    comp: &Comp<'d>,
    project: &Comp<'d>,
    path: &[String],
    t: f64,
    width: u32,
    height: u32,
    depth: u32,
) -> JsResult<Result<(), String>> {
    if depth > 8 {
        return Ok(Err(TOO_DEEP.into()));
    }
    let id = comp.id().to_string_lossy();
    if !walk.visited.insert((id.clone(), t.to_bits(), width, height)) {
        return Ok(Ok(()));
    }
    if walk.visited.len() > 4096 {
        return Ok(Err(TOO_MANY.into()));
    }
    let mut path = path.to_vec();
    path.push(id.clone());
    let root = comp.root();
    let layers: Vec<&'d Node> = comp.layers().iter().collect();
    let mut children = Vec::new();
    let (blurred, has_object, fps) = {
        let mut scope = Scope::new(ev, comp, project);
        let mut blurred = false;
        let mut has_object = false;
        for l in &layers {
            if !scope.active(l, t)? {
                continue;
            }
            if walk.options.motion_blur
                && !blurred
                && scope
                    .evaluated(l, node::member(Some(l), "mblur"), t, "l.mblur")?
                    .truthy()
            {
                blurred = true;
            }
            if node::truthy(node::member(
                node::member(node::member(Some(l), "d"), "data"),
                "object",
            )) {
                has_object = true;
            }
        }
        (blurred, has_object, number(comp.member("fps")))
    };
    let mut times = vec![t];
    if blurred {
        let n = walk.options.samples;
        let shutter = walk.shutter / fps;
        for i in 0..n {
            times.push(t + ((f64::from(i) + 0.5) / f64::from(n) - 0.5) * shutter);
        }
    }
    let enabled = super::blender_enabled(comp);
    let cw = number(comp.member("w"));
    let ch = number(comp.member("h"));
    for (i, &sample) in times.iter().enumerate() {
        if enabled && has_object && walk.keys.insert((id.clone(), sample.to_bits(), width, height)) {
            walk.frames.push(BlenderFrame {
                composition_path: path.clone(),
                time: sample,
                width,
                height,
                shutter: (i > 0).then(|| ((i - 1) as u16, times.len() as u16 - 1)),
                preview: false,
            });
        }
        children.clear();
        {
            let mut scope = Scope::new(ev, comp, project);
            for l in &layers {
                if js::layer_type(l) != "precomp" || !scope.active(l, sample)? {
                    continue;
                }
                let Some((sub, _)) = crate::capability::precomp_target(root, l) else {
                    continue;
                };
                let d = node::member(Some(l), "d");
                let w = js::max(
                    2.0,
                    js::round(or(or(number(node::member(d, "w")), cw), f64::from(width)) * f64::from(width) / or(cw, f64::from(width))),
                );
                let h = js::max(
                    2.0,
                    js::round(or(or(number(node::member(d, "h")), ch), f64::from(height)) * f64::from(height) / or(ch, f64::from(height))),
                );
                let st = scope.source_time(l, sample, None)?;
                children.push((sub, st, w as u32, h as u32));
            }
        }
        for (sub, st, w, h) in std::mem::take(&mut children) {
            let Ok(sub) = Comp::in_root(root, &sub, comp.revision()) else {
                continue;
            };
            if let Err(e) = visit(walk, ev, &sub, project, &path, st, w, h, depth + 1)? {
                return Ok(Err(e));
            }
        }
    }
    Ok(Ok(()))
}

/// The Blender frames of `root_comp` at `time` with `options`.
pub fn frame_requests(
    ev: &mut Evaluator,
    root_comp: &Comp<'_>,
    project: &Comp<'_>,
    time: f64,
    options: FrameOptions,
) -> JsResult<Result<Vec<BlenderFrame>, String>> {
    let width = options
        .width
        .unwrap_or_else(|| number(root_comp.member("w")) as u32);
    let height = options
        .height
        .unwrap_or_else(|| number(root_comp.member("h")) as u32);
    let shutter = options
        .shutter
        .filter(|s| *s != 0.0 && !s.is_nan())
        .unwrap_or_else(|| or(number(root_comp.member("shutter")), 0.5));
    let mut walk = Walk {
        frames: Vec::new(),
        keys: HashSet::new(),
        visited: HashSet::new(),
        options,
        shutter,
    };
    Ok(visit(&mut walk, ev, root_comp, project, &[], time, width, height, 0)?.map(|()| walk.frames))
}
