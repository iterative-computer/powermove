//! Plan-aware video startup before the first requested frame. The plan is
//! built with the same document, time and request as rendering, then walked
//! without submitting GPU work. Source and selected decoded frame preparation
//! run asynchronously; the presenter never waits for a hash, index or decoder.

use super::*;
use crate::composite;
use crate::plan::{Content, Draw, Source};

/// Result of preparing media for a planned frame. A source that cannot be
/// prepared is reported here and will still get its usual render failure.
#[derive(Debug, Default)]
pub struct MediaPreparation {
    pub queued_videos: usize,
    pub failures: Vec<String>,
}

fn visit_content(content: &Content, layer: &str, visit: &mut impl FnMut(&str, &Source)) {
    match content {
        Content::Texture { source, .. } => {
            visit(layer, source);
            if let Source::Comp(plan) | Source::Accumulate(plan) = source {
                visit_plan(plan, visit);
            }
        }
        Content::Placed(placed) => visit_content(&placed.content, layer, visit),
        Content::Composite(node) => {
            if let composite::Node::Layer(layer_node) = &**node {
                for draw in &layer_node.content {
                    visit_draw(draw, visit);
                }
                if let Some(matte) = &layer_node.matte {
                    visit_plan(&matte.source, visit);
                }
            }
        }
        Content::Solid(_) => {}
    }
}

fn visit_draw(draw: &Draw, visit: &mut impl FnMut(&str, &Source)) {
    visit_content(&draw.content, &draw.layer, visit);
}

fn visit_plan(plan: &CompPlan, visit: &mut impl FnMut(&str, &Source)) {
    for draw in &plan.draws {
        visit_draw(draw, visit);
    }
}

/// The video draws of a plan, nested compositions included.
pub(super) fn planned_videos(plan: &CompPlan) -> Vec<crate::media::PlannedVideoDraw> {
    let mut out = Vec::new();
    visit_plan(plan, &mut |layer, source| {
        if let Source::Video {
            path,
            time,
            interpret,
            composition_rate,
            ..
        } = source
        {
            out.push(crate::media::PlannedVideoDraw {
                layer: layer.to_owned(),
                path: path.clone(),
                instance: crate::exec::instance_of(layer),
                seconds: *time,
                interpret: interpret.clone(),
                composition: *composition_rate,
            });
        }
    });
    out
}

impl Renderer {
    /// Queue preparation of the videos selected by one planned view. The
    /// worker retains each completed source and matching idle decoder until
    /// the renderer adopts them. It also offers the selected decoded frame to
    /// the shared lease-bound cache. A first render can proceed cold if the
    /// worker is unfinished or the frame does not fit that cache.
    pub fn prepare_media(
        &mut self,
        document: &Document,
        composition: &str,
        time: Ticks,
        request: &RenderRequest,
        assets: &dyn AssetResolver,
    ) -> Result<MediaPreparation, RenderError> {
        request.validate()?;
        let root = document
            .root()
            .ok_or_else(|| RenderError::Composition(composition.to_owned()))?;
        project_settings(root)?;
        let comp = Comp::resolve(document, composition)
            .map_err(|_| RenderError::Composition(composition.to_owned()))?;
        let open = capability::open_composition_id(root);
        let project = open
            .as_deref()
            .and_then(|id| Comp::resolve(document, id).ok())
            .unwrap_or_else(|| comp.clone());
        let seconds = comp.seconds_at(time.get());
        let (w, h) = (request.output_size.width, request.output_size.height);
        let scale = request
            .scale
            .unwrap_or_else(|| plan::derived_scale(&comp, w, h));
        let options3d = self.options3d.for_request(request);
        let blender = crate::blender::PlanState::new(options3d.blender.clone());
        let (plan, build) = plan::build_blender(
            &mut self.evaluator,
            &comp,
            &project,
            seconds,
            w,
            h,
            scale,
            assets,
            effect_host(&self.effect_host, &self.effects),
            options3d,
            Some(&mut self.text),
            build_options(&comp, request, self.shaders.fallback_runner),
            blender,
            None,
            None,
        );
        if !build.unsupported.is_empty() {
            return Err(RenderError::Unsupported(build.unsupported));
        }
        let mut result = MediaPreparation::default();
        for v in planned_videos(&plan) {
            match self.media.prepare_video(
                &v.path,
                v.instance,
                v.seconds,
                request.export,
                v.interpret.as_ref(),
                v.composition,
            ) {
                Ok(true) => result.queued_videos += 1,
                Ok(false) => {}
                Err(error) => {
                    result
                        .failures
                        .push(format!("{} ({}): {error:?}", v.layer, v.path.display()))
                }
            }
        }
        Ok(result)
    }

    /// Wait for queued preparation in headless tests and performance runs.
    /// The interactive presenter must use `prepare_media` alone.
    pub fn finish_media_preparation(&mut self) -> Vec<String> {
        self.media
            .wait_for_preparation()
            .into_iter()
            .map(|error| format!("{error:?}"))
            .collect()
    }
}
