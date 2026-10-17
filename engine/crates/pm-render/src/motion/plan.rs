//! Motion-blur samples in the render graph (`renderProject`'s slow path for
//! `mb`).

use pm_eval::{JsResult, Scope};
use pm_model::Node;

use super::{Blur, level_fps, sample_offset};
use crate::js;
use crate::plan::{Background, Build, CompPlan, Content, Draw, Source, comp_pass, layer_draw};

fn full(width: u32, height: u32) -> [f64; 6] {
    [f64::from(width), 0.0, 0.0, f64::from(height), 0.0, 0.0]
}

/// The accumulation target of one blurred layer: transparent, in the
/// parent's pixel space.
fn accumulation(scope: &Scope<'_, '_>, width: u32, height: u32, draws: Vec<Draw>) -> CompPlan {
    CompPlan {
        composition: scope.comp.id().to_string_lossy(),
        group: true,
        width,
        height,
        background: Background::Transparent,
        draws,
    }
}

/// A non-group layer with motion blur: `drawContent(L, T + dt, W, H,
/// alpha / n, undefined, true)` for every sample into the layer's target,
/// then the target over the stack at opacity 1. `None` when no sample
/// draws anything.
#[allow(clippy::too_many_arguments)]
pub(crate) fn blur_layer<'d>(
    scope: &mut Scope<'_, 'd>,
    b: &mut Build<'_>,
    layer: &'d Node,
    time: f64,
    width: u32,
    height: u32,
    scale: (f64, f64),
    alpha: f64,
    blur: Blur,
) -> JsResult<Option<Draw>> {
    let n = blur.samples.max(1);
    let fps = level_fps(scope.comp);
    let sample_alpha = alpha / f64::from(n);
    let outer_center = b.video_center;
    b.video_center = Some(outer_center.unwrap_or(time));
    let mut draws = Vec::with_capacity(n as usize);
    let mut result = Ok(());
    for s in 0..n {
        let t = time + sample_offset(s, n, blur.shutter_frames, fps);
        match layer_draw(scope, b, layer, t, width, height, scale, sample_alpha) {
            Ok(Some(d)) => draws.push(d),
            Ok(None) => {}
            Err(e) => {
                result = Err(e);
                break;
            }
        }
    }
    b.video_center = outer_center;
    result?;
    if draws.is_empty() {
        return Ok(None);
    }
    Ok(Some(Draw {
        layer: js::layer_id(layer),
        matrix: full(width, height),
        content: Content::Texture {
            source: Source::Accumulate(Box::new(accumulation(scope, width, height, draws))),
            uv: [0.0, 0.0, 1.0, 1.0],
            alpha: 1.0,
        },
    }))
}

/// A group with motion blur: its members' compositing pass at every sample
/// time with motion blur off inside (`renderProject(proj, T + dt, …,
/// {groupParent, mblur: false})`), each accumulated at `1/n`, the sum drawn
/// at the group's opacity.
#[allow(clippy::too_many_arguments)]
pub(crate) fn blur_group<'d>(
    scope: &mut Scope<'_, 'd>,
    b: &mut Build<'_>,
    group: &'d Node,
    time: f64,
    width: u32,
    height: u32,
    scale: (f64, f64),
    alpha: f64,
    blur: Blur,
) -> Draw {
    let n = blur.samples.max(1);
    let fps = level_fps(scope.comp);
    let outer_blur = b.blur.take();
    let outer_center = b.video_center;
    b.video_center = Some(outer_center.unwrap_or(time));
    let mut draws = Vec::with_capacity(n as usize);
    for s in 0..n {
        let t = time + sample_offset(s, n, blur.shutter_frames, fps);
        let pass = comp_pass(scope, b, t, width, height, scale, true, Some(group));
        draws.push(Draw {
            layer: js::layer_id(group),
            matrix: full(width, height),
            content: Content::Texture {
                source: Source::Comp(Box::new(pass)),
                uv: [0.0, 0.0, 1.0, 1.0],
                alpha: 1.0 / f64::from(n),
            },
        });
    }
    b.video_center = outer_center;
    b.blur = outer_blur;
    Draw {
        layer: js::layer_id(group),
        matrix: full(width, height),
        content: Content::Texture {
            source: Source::Accumulate(Box::new(accumulation(scope, width, height, draws))),
            uv: [0.0, 0.0, 1.0, 1.0],
            alpha,
        },
    }
}
