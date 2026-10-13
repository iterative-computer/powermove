//! The 3D scene pass (plan section 8, "3D layers"; amendment 11).

use pm_eval::{JsResult, Scope};
use pm_gpu::Target;
use pm_model::Node;

/// One 3D object layer's render.
#[derive(Debug, Clone, PartialEq)]
pub struct Object3dPlan {
    pub layer: String,
    pub width: u32,
    pub height: u32,
}

/// Static checks of a composition's 3D layers: (layer, reason) pairs.
pub(crate) fn check(
    _root: &Node,
    _comp: &Node,
    layers: &[Node],
    _objects: usize,
) -> Vec<(Option<String>, String)> {
    layers
        .iter()
        .filter(|l| pm_eval::parenting::Role3d::of(l).is_some())
        .map(|l| (Some(crate::js::layer_id(l)), "the 3D scene pass".to_owned()))
        .collect()
}

pub(crate) fn plan_object<'d>(
    _scope: &mut Scope<'_, 'd>,
    _layer: &'d Node,
    _time: f64,
    _width: u32,
    _height: u32,
) -> JsResult<Result<Object3dPlan, String>> {
    Ok(Err("the 3D scene pass".to_owned()))
}

/// The 3D pass output: premultiplied 8-bit color and linear view depth.
pub struct Rendered3d {
    pub color: Target,
    pub depth: Target,
}

#[derive(Default)]
pub struct Scene3dRenderer {}

impl Scene3dRenderer {
    pub fn render(
        &mut self,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _encoder: &mut wgpu::CommandEncoder,
        _plan: &Object3dPlan,
    ) -> Result<Rendered3d, String> {
        Err("the 3D scene pass".to_owned())
    }
}
