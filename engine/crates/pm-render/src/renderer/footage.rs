//! What a frame's plan says about the footage in its picture (section 9,
//! "Pass-through"): the candidate when the frame is one untouched video
//! source, and every video file the picture draws from, so an exporter can
//! tell one continuous piece of picture from a cut.

use std::path::PathBuf;

use super::*;
use crate::composite;
use crate::plan::{Content, Draw, Source};

/// The footage in a frame's picture, from the plan the frame is drawn with.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FrameFootage {
    /// The pass-through candidate, when the frame is one video source and
    /// nothing else ([`Renderer::pass_through`]).
    pub candidate: Option<PassThrough>,
    /// Every video file the frame draws from, nested compositions, mattes
    /// and layers in their own targets included.
    pub videos: Vec<PathBuf>,
    /// Part of the picture comes from something the plan does not name the
    /// pixels of (an effect or transition, an adjustment layer, a 3D scene,
    /// a shader or extension layer, an extension's raster): it may show any
    /// footage, so the frame cannot be told apart from the ones around it.
    pub unnamed: bool,
}

impl FrameFootage {
    /// Whether the frame may show `path`'s picture.
    pub fn may_show(&self, path: &std::path::Path) -> bool {
        self.unnamed || self.videos.iter().any(|v| v == path)
    }
}

fn walk_content(content: &Content, out: &mut FrameFootage) {
    match content {
        Content::Solid(_) => {}
        Content::Placed(placed) => walk_content(&placed.content, out),
        Content::Texture { source, .. } => match source {
            Source::Video { path, .. } => {
                if !out.videos.contains(path) {
                    out.videos.push(path.clone());
                }
            }
            Source::Comp(plan) | Source::Accumulate(plan) => walk_plan(plan, out),
            Source::Image { .. } | Source::Vector(_) | Source::Text(_) => {}
            Source::Object3d(_) | Source::Shader(_) | Source::Compat(_) => out.unnamed = true,
        },
        Content::Composite(node) => match &**node {
            composite::Node::Adjustment(_) => out.unnamed = true,
            composite::Node::Layer(layer) => {
                out.unnamed |= layer.effects.is_some() || layer.transition.is_some();
                for draw in &layer.content {
                    walk_content(&draw.content, out);
                }
                if let Some(matte) = &layer.matte {
                    walk_plan(&matte.source, out);
                }
            }
        },
    }
}

fn walk_plan(plan: &CompPlan, out: &mut FrameFootage) {
    for draw in &plan.draws {
        walk_content(&draw.content, out);
    }
}

/// The one video source a plan draws and nothing else, with its draw.
fn sole_video(plan: &CompPlan) -> Option<(&Draw, &std::path::Path, f64, [f64; 4], f64)> {
    let [draw] = plan.draws.as_slice() else {
        return None;
    };
    // Raster content reaches the sampler as a placed draw; only a flat
    // (non-2.5D) placement can be a 1:1 copy.
    let content = match &draw.content {
        Content::Placed(p) if p.plane.is_none() => &p.content,
        other => other,
    };
    let Content::Texture { source, uv, alpha } = content else {
        return None;
    };
    // An interpreted source is read differently from its file (color,
    // geometry, or frame timing), so its planes are never copied.
    let Source::Video {
        sequence: false,
        path,
        time,
        interpret: None,
        ..
    } = source
    else {
        return None;
    };
    Some((draw, path, *time, *uv, *alpha))
}

impl Renderer {
    /// The footage facts of a plan built for a `width` × `height` output.
    pub(super) fn footage_of(&mut self, plan: &CompPlan, width: u32, height: u32) -> FrameFootage {
        let mut out = FrameFootage::default();
        walk_plan(plan, &mut out);
        if let Some((draw, path, seconds, uv, alpha)) = sole_video(plan) {
            let full = [f64::from(width), 0.0, 0.0, f64::from(height), 0.0, 0.0];
            let untransformed = draw.matrix == full
                && uv == [0.0, 0.0, 1.0, 1.0]
                && self.media.video_size(path).ok() == Some((width, height));
            out.candidate = Some(PassThrough {
                path: path.to_path_buf(),
                seconds,
                facts: pm_color::chroma::FrameFacts {
                    single_source: true,
                    untransformed,
                    effect_free: alpha == 1.0,
                },
            });
        }
        out
    }

    /// The footage facts of a frame without rendering it: what
    /// [`Frame::footage`] reports for the same request (an exporter looking
    /// ahead along a clip).
    pub fn footage(
        &mut self,
        document: &Document,
        composition: &str,
        time: Ticks,
        request: &RenderRequest,
        assets: &dyn AssetResolver,
    ) -> Result<FrameFootage, RenderError> {
        request.validate()?;
        let (plan, _) = self.plan(document, composition, time, request, assets)?;
        let (w, h) = (request.output_size.width, request.output_size.height);
        Ok(self.footage_of(&plan, w, h))
    }
}
