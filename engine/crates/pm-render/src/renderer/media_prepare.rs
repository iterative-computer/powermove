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
    /// The video draws the plan selected: (layer, instance).
    pub videos: Vec<(String, u64)>,
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

/// The still draws of a plan, nested compositions included: each file, its
/// asset's interpretation, and the edge its draw reads it at
/// (`crate::exec::source_edge`, from the same matrix, UV window and plane
/// the draw resolves with).
type Interpret<'a> = Option<&'a pm_media::interpret::Interpretation>;

fn visit_stills(plan: &CompPlan, visit: &mut impl FnMut(&std::path::Path, Interpret, Option<u32>)) {
    fn content(
        c: &Content,
        matrix: &[f64; 6],
        plane: bool,
        visit: &mut impl FnMut(&std::path::Path, Interpret, Option<u32>),
    ) {
        match c {
            Content::Texture { source, uv, .. } => match source {
                Source::Image {
                    path, interpret, ..
                } => visit(
                    path,
                    interpret.as_ref(),
                    crate::exec::source_edge(matrix, *uv, plane),
                ),
                Source::Comp(plan) | Source::Accumulate(plan) => visit_stills(plan, visit),
                _ => {}
            },
            Content::Placed(placed) => content(
                &placed.content,
                matrix,
                plane || placed.plane.is_some(),
                visit,
            ),
            Content::Composite(node) => {
                if let composite::Node::Layer(layer) = &**node {
                    for draw in &layer.content {
                        content(&draw.content, &draw.matrix, false, visit);
                    }
                    if let Some(matte) = &layer.matte {
                        visit_stills(&matte.source, visit);
                    }
                }
            }
            Content::Solid(_) => {}
        }
    }
    for draw in &plan.draws {
        content(&draw.content, &draw.matrix, false, visit);
    }
}

/// The video draws of a plan, nested compositions included.
/// Whether `held` says yes for a still `plan` draws.
pub(super) fn any_still(plan: &CompPlan, held: &mut impl FnMut(&std::path::Path) -> bool) -> bool {
    let mut any = false;
    visit_stills(plan, &mut |path, _, _| any = any || held(path));
    any
}

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
        let (plan, _) = plan::build_reusing(
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
            build_options(
                &comp,
                request,
                self.shaders.fallback_runner,
                self.isolate_layers,
            ),
            None,
            None,
        );
        self.evaluator.close_channel_memo();
        // A layer the renderer cannot draw is reported by the frame itself;
        // the media of every other layer is still prepared (refusing here
        // left a project with one such layer without any look-ahead).
        let mut result = MediaPreparation::default();
        if !request.export {
            visit_stills(&plan, &mut |path, interpret, edge| {
                self.media.prepare_image(path, edge, interpret);
            });
        }
        for v in planned_videos(&plan) {
            result.videos.push((v.layer.clone(), v.instance));
            match self.media.prepare_video(
                &v.path,
                v.instance,
                v.seconds,
                request.export,
                request.read_ahead,
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
        self.media.wait_for_stills();
        self.media
            .wait_for_preparation()
            .into_iter()
            .map(|error| format!("{error:?}"))
            .collect()
    }
}
