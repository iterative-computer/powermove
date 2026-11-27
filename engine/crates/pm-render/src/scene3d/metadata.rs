//! Metadata of the scene that supplied a frame's pixels, without another
//! scene evaluation. Kept plan sources also retain it through draw reuse.
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use pm_model::Document;
use pm_protocol::frame::Scene3dMeta;

use super::Scene3dRenderer;
use crate::LayerFailure;
use crate::plan::{CompPlan, Content, Draw, Source};

type Diagnostics = (Arc<[String]>, Option<String>);

#[derive(Default)]
struct Layers {
    entries: BTreeMap<(String, String), Diagnostics>,
    warnings: HashMap<usize, Arc<[String]>>,
}
impl Layers {
    fn new() -> Self {
        Self::default()
    }
    fn insert(&mut self, key: (String, String), warnings: &[String], error: Option<String>) {
        let address = warnings.as_ptr() as usize;
        let cached = self.warnings.entry(address).or_default();
        if cached.as_ref() != warnings {
            *cached = warnings.into();
        }
        self.entries.insert(key, (cached.clone(), error));
    }
}

fn content(
    content: &Content,
    composition: &str,
    layer: &str,
    renderer: &Scene3dRenderer,
    layers: &mut Layers,
) {
    match content {
        Content::Placed(placed) => {
            content_walk(&placed.content, composition, layer, renderer, layers)
        }
        Content::Composite(node) => {
            if let crate::composite::Node::Layer(node) = &**node {
                for draw in &node.content {
                    draw_walk(draw, composition, renderer, layers);
                }
                if let Some(matte) = &node.matte {
                    plan_walk(&matte.source, renderer, layers);
                }
            }
        }
        Content::Texture { source, .. } => match source {
            Source::Object3d(object) => {
                layers.insert(
                    (composition.into(), layer.into()),
                    renderer.scene_warnings(&object.scene),
                    None,
                );
            }
            Source::Shader(source) => {
                if let Some(error) = &source.scene_error {
                    layers.insert((composition.into(), layer.into()), &[], Some(error.clone()));
                }
            }
            Source::Comp(plan) | Source::Accumulate(plan) => plan_walk(plan, renderer, layers),
            _ => {}
        },
        Content::Solid(_) => {}
    }
}

fn content_walk(
    value: &Content,
    composition: &str,
    layer: &str,
    renderer: &Scene3dRenderer,
    layers: &mut Layers,
) {
    content(value, composition, layer, renderer, layers);
}
fn draw_walk(draw: &Draw, composition: &str, renderer: &Scene3dRenderer, layers: &mut Layers) {
    content(&draw.content, composition, &draw.layer, renderer, layers);
}
fn plan_walk(plan: &CompPlan, renderer: &Scene3dRenderer, layers: &mut Layers) {
    for draw in &plan.draws {
        draw_walk(draw, &plan.composition, renderer, layers);
    }
}

/// Last draw wins per layer, as SceneRuntime's metadata writes do. Layers
/// sharing a warning/error value share one wire record (large scenes do not
/// multiply a composition's complete warning list by its object count).
pub(crate) fn collect(plan: &CompPlan, renderer: &Scene3dRenderer) -> Vec<Scene3dMeta> {
    let mut layers = Layers::new();
    plan_walk(plan, renderer, &mut layers);
    group(layers)
}
fn group(layers: Layers) -> Vec<Scene3dMeta> {
    let mut out: Vec<Scene3dMeta> = Vec::new();
    for ((composition, layer), (warnings, error)) in layers.entries {
        if let Some(existing) = out.iter_mut().find(|m| {
            m.composition.as_str() == composition
                && m.warnings.as_slice() == warnings.as_ref()
                && m.error == error
        }) {
            existing.layers.push(layer.as_str().into());
        } else {
            out.push(Scene3dMeta {
                composition: composition.as_str().into(),
                layers: vec![layer.as_str().into()],
                warnings: warnings.to_vec(),
                error,
            });
        }
    }
    out
}

/// Composition-scene construction can fail before an object has a source
/// in the plan. Its existing failure still supplies today's missing badge.
pub(crate) fn failures(
    document: &Document,
    failures: &[LayerFailure],
    reports: &mut Vec<Scene3dMeta>,
) {
    let mut layers = Layers::new();
    for report in reports.drain(..) {
        for layer in report.layers {
            layers.insert(
                (report.composition.as_str().into(), layer.as_str().into()),
                &report.warnings,
                report.error.clone(),
            );
        }
    }
    for failure in failures {
        let Ok(comp) = pm_eval::Comp::resolve(document, &failure.composition) else {
            continue;
        };
        let Some(layer) = comp
            .layers()
            .iter()
            .find(|l| crate::js::layer_id(l) == failure.layer)
        else {
            continue;
        };
        if pm_eval::parenting::Role3d::of(layer) == Some(pm_eval::parenting::Role3d::Object) {
            layers.insert(
                (failure.composition.clone(), failure.layer.clone()),
                &[],
                Some(failure.message.clone()),
            );
        }
    }
    *reports = group(layers);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn common_scene_warnings_are_not_repeated_per_object_on_the_wire() {
        let warning = vec![
            "Missing texture: image".into(),
            "Missing model: model".into(),
        ];
        let mut layers = Layers::new();
        for i in 0..600 {
            layers.insert(("comp".into(), format!("object{i}")), &warning, None);
        }
        let grouped = group(layers);
        assert_eq!(grouped.len(), 1);
        assert_eq!(grouped[0].layers.len(), 600);
        assert_eq!(grouped[0].warnings, warning);
    }
}
