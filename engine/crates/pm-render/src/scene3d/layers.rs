//! 3D layers of a composition pass (`compositor.ts` and
//! `core/scene3d/service.ts`): the composition's scene
//! (`compositionRuntime`), the pass order (`compositionOrderedLayers`),
//! each object layer's depth set (`compositionDepthIds`) and whether it
//! draws the background (the bottom of its set), the view override, and
//! the static capability check.

use std::collections::HashMap;
use std::sync::Arc;

use pm_eval::node;
use pm_eval::parenting::Role3d;
use pm_eval::{JsResult, JsValue, Scope};
use pm_geom3d::layering::{self, PassEntry, Role};
use pm_model::json::Value;
use pm_model::{Node, NodeKind};
use pm_protocol::frame::{Projection as ViewProjection, Shading3d, ViewOverride};

use crate::assets::AssetResolver;
use crate::js;
use crate::three::math::{self, Mat4};

use super::model::{
    Camera3d, Evaluated, LayerFacts, Look, NoTextures, Placement, Projection, Scene3d,
    SceneBuilder, TextureSource,
};
use super::{Object3dPlan, Render3d};

/// The 3D state of one compositing pass.
#[derive(Default)]
pub(crate) struct Pass3d {
    /// The composition, time, and canvas the scene was built for.
    key: Option<(String, u64, u32, u32)>,
    /// The composition's scene, or why it cannot be drawn here.
    scene: Option<Result<Arc<Scene3d>, String>>,
    /// Per object layer id: its render.
    layers: HashMap<String, Render3d>,
    /// The pass in drawing order (top first), as indices into the
    /// composition's layers, and the build's inputs: what a motion-blur
    /// sample needs to rebuild the 3D state at its own time
    /// ([`Pass3d::at_time`]).
    ordered: Vec<usize>,
    root_comp: bool,
}

/// The view override as the 3D pass applies it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct View3d {
    /// The editor camera (root composition only).
    pub camera: Option<Camera3d>,
    pub look: Look,
    /// Local View: the object layers shown (root composition only).
    pub local: Option<Vec<String>>,
}

impl View3d {
    /// From contract 2's view override: Wireframe shading draws wireframes;
    /// X-ray applies in the draft shadings (Wireframe and Solid) only.
    pub fn from_override(v: &ViewOverride) -> View3d {
        let draft = matches!(v.shading, Shading3d::Wireframe | Shading3d::Solid);
        View3d {
            camera: v.camera.map(|c| {
                let world = math::invert(&c.view);
                let projection = match c.projection {
                    ViewProjection::Perspective {
                        fov_y_degrees,
                        near,
                        far,
                    } => Projection::Perspective {
                        fov: fov_y_degrees,
                        zoom: 1.0,
                        near,
                        far,
                    },
                    ViewProjection::Orthographic {
                        half_height,
                        near,
                        far,
                    } => Projection::Orthographic {
                        // `resizeCamera` fixes the half-height at 3.
                        zoom: 3.0 / half_height,
                        near,
                        far,
                    },
                };
                Camera3d { world, projection }
            }),
            look: Look {
                wireframe: v.shading == Shading3d::Wireframe,
                xray: v.xray && draft,
            },
            local: (!v.local_view.is_empty()).then(|| {
                v.local_view
                    .iter()
                    .map(|id| id.as_str().to_owned())
                    .collect()
            }),
        }
    }
}

fn role(layer: &Node) -> Option<Role> {
    match Role3d::of(layer)? {
        Role3d::Object => Some(Role::Object),
        Role3d::Light => Some(Role::Light),
        Role3d::Camera => Some(Role::Camera),
    }
}

/// Image assets as the runtime's texture getter sees them: an image that
/// is offline or does not decode has no bitmap, so the material draws
/// without the map and the scene warns (`Missing texture`). Images this
/// pass does not draw keep their path; the capability check routes them.
struct DocumentTextures<'a> {
    root: &'a Node,
    assets: &'a dyn AssetResolver,
}

impl TextureSource for DocumentTextures<'_> {
    fn image(&self, asset: &str) -> Option<std::path::PathBuf> {
        let record = node::member(node::member(Some(self.root), "assets"), asset)?;
        if !node::is_str(node::member(Some(record), "kind"), "image") {
            return None;
        }
        let path = self.assets.path(asset, Some(record))?;
        match super::textures::Textures::probe(&path) {
            Err(super::textures::MapError::Failed(_)) => None,
            _ => Some(path),
        }
    }
}

/// Model assets as the asset store holds them: `kind: 'model'` records
/// with `format` `obj` (parsed meshes), `gltf` or `glb` (imported scenes),
/// or `blend` (no geometry in the 3D pass).
pub(crate) struct DocumentModels<'a> {
    pub root: &'a Node,
    pub assets: &'a dyn AssetResolver,
}

impl super::model::ModelSource for DocumentModels<'_> {
    fn model(&self, asset: &str) -> super::model::ModelLookup {
        use super::model::ModelLookup;
        let Some(record) = node::member(node::member(Some(self.root), "assets"), asset) else {
            return ModelLookup::Missing;
        };
        let format = str_member(Some(record), "format").unwrap_or_default();
        let name = str_member(Some(record), "name")
            .unwrap_or_default()
            .to_ascii_lowercase();
        let gltf = match format.as_str() {
            "gltf" | "glb" => true,
            "obj" => false,
            _ if name.ends_with(".glb") || name.ends_with(".gltf") => true,
            _ if name.ends_with(".obj") => false,
            _ => return ModelLookup::Missing,
        };
        let Some(path) = self.assets.path(asset, Some(record)) else {
            return ModelLookup::Missing;
        };
        if gltf {
            return super::gltf::lookup(&path);
        }
        match super::legacy::obj_mesh_versioned(&path) {
            Ok((key, mesh)) => ModelLookup::Obj { key, mesh },
            Err(_) => ModelLookup::Missing,
        }
    }
}

/// Any enabled mask (`userData.layerOccludes` is its negation).
fn masked<'d>(scope: &mut Scope<'_, 'd>, layer: &'d Node, time: f64) -> JsResult<bool> {
    for m in node::array(node::member(Some(layer), "masks")).unwrap_or(&[]) {
        let id = node::to_js_string(node::member(Some(m), "id")).to_string_lossy();
        let on = scope.evaluated(
            layer,
            node::member(Some(m), "on"),
            time,
            &format!("m.{id}.on"),
        )?;
        if on != JsValue::Bool(false) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The composition's scene at `time` (`compositionRuntime`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn composition_scene<'d>(
    scope: &mut Scope<'_, 'd>,
    time: f64,
    width: u32,
    height: u32,
    assets: &dyn AssetResolver,
    view: Option<&View3d>,
    root_comp: bool,
) -> JsResult<Result<Scene3d, String>> {
    let evaluated = scope.composition_scene(time)?;
    if !evaluated.recipe_parts.is_empty() {
        return Ok(Err("model-group recipe parts".into()));
    }
    let Value::Object(scene) = &evaluated.scene else {
        return Ok(Err("the evaluated 3D scene".into()));
    };
    let layers: Vec<&'d Node> = scope.layers().iter().collect();
    let mut objects: HashMap<String, Mat4> = HashMap::new();
    let mut facts: HashMap<String, LayerFacts> = HashMap::new();
    let mut lights: HashMap<String, ([f64; 3], [f64; 3])> = HashMap::new();
    for l in &layers {
        match Role3d::of(l) {
            Some(Role3d::Object) if scope.active(l, time)? => {
                let id = js::layer_id(l);
                objects.insert(id.clone(), scope.world3d(l, time, false)?);
                facts.insert(
                    id,
                    LayerFacts {
                        opacity: scope.world_opacity(l, time)?,
                        occludes: !masked(scope, l, time)?,
                    },
                );
            }
            Some(Role3d::Light) if scope.active(l, time)? => {
                if let Some(p) = scope.scene_light_aim(l, time)? {
                    lights.insert(js::layer_id(l), p);
                }
            }
            _ => {}
        }
    }
    let pose = scope.scene_camera(time)?;
    let has_camera_layer = {
        let mut found = false;
        for l in &layers {
            if Role3d::of(l) == Some(Role3d::Camera) && scope.active(l, time)? {
                found = true;
                break;
            }
        }
        found
    };
    let placement = Placement::Composition {
        objects: &objects,
        lights: &lights,
        camera: has_camera_layer.then_some((pose.position, pose.target, pose.up)),
    };
    let root = scope.comp.root();
    let textures = DocumentTextures { root, assets };
    let models = DocumentModels { root, assets };
    let mut builder = SceneBuilder::default();
    // Imported models' mixers: `time − layer.from` per layer
    // (`compositionRuntime`'s `animationTime`).
    let starts: HashMap<String, f64> = layers
        .iter()
        .map(|l| {
            let from = node::number(node::member(Some(l), "from"))
                .filter(|f| *f != 0.0 && !f.is_nan())
                .unwrap_or(0.0);
            (js::layer_id(l), time - from)
        })
        .collect();
    builder.set_animation_times(time, starts);
    let built = builder.build(
        scene,
        width,
        height,
        &placement,
        &facts,
        &mut Evaluated,
        &textures,
        &models,
    );
    let mut scene = match built {
        Ok(s) => s,
        Err(e) => return Ok(Err(e)),
    };
    if let Some(v) = view {
        if root_comp && let Some(c) = v.camera {
            scene.camera = c;
        }
        scene.apply_look(v.look);
    }
    Ok(Ok(scene))
}

/// `compositionOrderedLayers` and the depth sets of a compositing pass:
/// returns the pass in drawing order (top first) and the 3D state the
/// pass's object layers draw from. `pass` is in stack order (top first).
#[allow(clippy::too_many_arguments)]
pub(crate) fn order_pass<'d>(
    scope: &mut Scope<'_, 'd>,
    stack: &[Pass3d],
    pass: Vec<&'d Node>,
    time: f64,
    width: u32,
    height: u32,
    assets: &dyn AssetResolver,
    view: Option<&View3d>,
    root_comp: bool,
) -> JsResult<(Vec<&'d Node>, Pass3d)> {
    let mut ctx = Pass3d::default();
    // Local View hides the other models in the editor's root composition.
    let pass: Vec<&'d Node> = match view.and_then(|v| v.local.as_ref()) {
        Some(local) if root_comp => pass
            .into_iter()
            .filter(|l| Role3d::of(l) != Some(Role3d::Object) || local.contains(&js::layer_id(l)))
            .collect(),
        _ => pass,
    };
    if !pass.iter().any(|l| Role3d::of(l).is_some()) {
        // No scene objects, and no 2.5D switch anywhere in this
        // composition (including enclosing groups): timeline order is
        // already the answer. Keep the old path for malformed nullish
        // lists, whose group lookups can throw.
        if !scope
            .layers()
            .iter()
            .any(|l| js::flag(l, "threeD") || node::is_nullish(Some(l)))
        {
            return Ok((pass, ctx));
        }
        // No 3D objects: `depthOrderedLayers` (the 2.5D order).
        let ordered = scope.depth_ordered_layers(&pass, time)?;
        return Ok((ordered, ctx));
    }
    // The composition's scene is shared by its passes (group boundaries).
    let comp_id = scope.comp.id().to_string_lossy();
    let key = (comp_id, time.to_bits(), width, height);
    let shared = stack
        .iter()
        .rev()
        .find(|p| p.key.as_ref() == Some(&key))
        .and_then(|p| p.scene.clone());
    let scene = match shared {
        Some(s) => s,
        None => {
            composition_scene(scope, time, width, height, assets, view, root_comp)?.map(Arc::new)
        }
    };
    ctx.key = Some(key);
    // Ordering: within each run of 3D layers, back to front by the camera
    // depth of the object origin.
    let camera_view = match &scene {
        Ok(s) => s.camera.view(),
        Err(_) => math::IDENTITY,
    };
    let origin = |id: &str| -> Option<[f64; 3]> {
        let s = scene.as_ref().ok()?;
        s.objects.iter().find(|o| o.id == id).map(|o| o.origin)
    };
    let is_3d: Vec<bool> = pass.iter().map(|l| Role3d::of(l).is_some()).collect();
    let mut ordered: Vec<&'d Node> = Vec::with_capacity(pass.len());
    for (range, three) in layering::runs(&is_3d) {
        let block = &pass[range];
        if three {
            let depths: Vec<f64> = block
                .iter()
                .map(|l| match Role3d::of(l) {
                    Some(Role3d::Object) => {
                        origin(&js::layer_id(l)).map_or(0.0, |p| layering::view_z(&camera_view, p))
                    }
                    _ => 0.0,
                })
                .collect();
            ordered.extend(layering::sort_run(&depths).into_iter().map(|i| block[i]));
        } else {
            ordered.extend(scope.depth_ordered_layers(block, time)?);
        }
    }
    // Depth sets over the ordered pass.
    if let Ok(sc) = &scene {
        ctx.layers = depth_sets(scope, &ordered, sc, time)?;
    }
    let layers = scope.layers();
    ctx.ordered = ordered
        .iter()
        .filter_map(|l| layers.iter().position(|n| std::ptr::eq(n, *l)))
        .collect();
    ctx.root_comp = root_comp;
    ctx.scene = Some(scene);
    Ok((ordered, ctx))
}

/// `compositionDepthIds` and the bottom layer of each object layer of a
/// pass in drawing order (top first), with activity, solo, and opacity at
/// `time`.
fn depth_sets<'d>(
    scope: &mut Scope<'_, 'd>,
    ordered: &[&'d Node],
    s: &Scene3d,
    time: f64,
) -> JsResult<HashMap<String, Render3d>> {
    let any_solo = scope.layers().iter().any(|l| js::flag(l, "solo"));
    let mut entries = Vec::with_capacity(ordered.len());
    for l in ordered {
        let active = scope.active(l, time)?;
        let r = role(l);
        let opacity = if r == Some(Role::Object) && active {
            scope.world_opacity(l, time)?
        } else {
            1.0
        };
        entries.push(PassEntry {
            active,
            role: r,
            soloed: js::flag(l, "solo") || soloed_in_pass(l, ordered),
            opacity,
        });
    }
    let index: HashMap<&str, usize> = s
        .objects
        .iter()
        .enumerate()
        .map(|(i, o)| (o.id.as_str(), i))
        .collect();
    let mut out = HashMap::new();
    for (i, l) in ordered.iter().enumerate() {
        if Role3d::of(l) != Some(Role3d::Object) || !entries[i].active {
            continue;
        }
        let id = js::layer_id(l);
        let Some(&object) = index.get(id.as_str()) else {
            continue;
        };
        let set = layering::depth_set(&entries, i, any_solo);
        let bottom = layering::bottom(&set) == Some(i);
        let depth_set: Vec<usize> = set
            .iter()
            .filter_map(|&k| index.get(js::layer_id(ordered[k]).as_str()).copied())
            .collect();
        out.insert(
            id,
            Render3d::Layer {
                object,
                depth_set,
                background: bottom,
            },
        );
    }
    Ok(out)
}

impl Pass3d {
    /// The 3D state of this pass for a motion-blur sample at `time`
    /// (`drawContent(L, T + dt, …)` of a blurred object layer): today's
    /// `compositionRuntime(PM, T + dt)` syncs the whole scene (objects,
    /// lights, camera) at the sample time and `compositionDepthIds` reads
    /// activity and opacity at that time, over the pass as it was ordered
    /// at the center time (`PM._renderComposition3DPass`).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn at_time<'d>(
        &self,
        scope: &mut Scope<'_, 'd>,
        time: f64,
        width: u32,
        height: u32,
        assets: &dyn AssetResolver,
        view: Option<&View3d>,
    ) -> JsResult<Pass3d> {
        let mut ctx = Pass3d {
            ordered: self.ordered.clone(),
            root_comp: self.root_comp,
            ..Pass3d::default()
        };
        if self.scene.is_none() {
            return Ok(ctx);
        }
        let comp_id = scope.comp.id().to_string_lossy();
        ctx.key = Some((comp_id, time.to_bits(), width, height));
        let scene = composition_scene(scope, time, width, height, assets, view, self.root_comp)?
            .map(Arc::new);
        let layers = scope.layers();
        let ordered: Vec<&'d Node> = self.ordered.iter().filter_map(|&i| layers.get(i)).collect();
        if let Ok(sc) = &scene {
            ctx.layers = depth_sets(scope, &ordered, sc, time)?;
        }
        ctx.scene = Some(scene);
        Ok(ctx)
    }
}

/// `PM.groupAncestors(l, passLayers).some(g => g.solo)`: only groups
/// present in the pass are found.
fn soloed_in_pass(layer: &Node, pass: &[&Node]) -> bool {
    let mut seen = vec![js::layer_id(layer)];
    let mut group = node::member(Some(layer), "group")
        .filter(|g| node::truthy(Some(g)))
        .map(|g| node::js_value_opt(Some(g)).to_js_string().to_string_lossy());
    while let Some(id) = group {
        if seen.contains(&id) {
            break;
        }
        seen.push(id.clone());
        let Some(g) = pass
            .iter()
            .find(|l| js::layer_id(l) == id && js::layer_type(l) == "group")
        else {
            break;
        };
        if js::flag(g, "solo") {
            return true;
        }
        group = node::member(Some(g), "group")
            .filter(|g| node::truthy(Some(g)))
            .map(|g| node::js_value_opt(Some(g)).to_js_string().to_string_lossy());
    }
    false
}

/// The render of a 3D object layer in the current pass.
pub(crate) fn plan_layer(pass: Option<&Pass3d>, layer: &Node) -> Result<Object3dPlan, String> {
    let pass = pass.ok_or("a 3D layer outside a compositing pass")?;
    let scene = match &pass.scene {
        Some(Ok(s)) => s.clone(),
        Some(Err(e)) => return Err(e.clone()),
        None => return Err("the 3D scene".into()),
    };
    let id = js::layer_id(layer);
    match pass.layers.get(&id) {
        Some(kind) => Ok(Object3dPlan {
            layer: id,
            scene,
            kind: kind.clone(),
        }),
        None => Err("the 3D object is not in the evaluated scene".into()),
    }
}

/// A static capability issue: the layer (when one is at fault) and why.
pub(crate) type Issue = (Option<String>, String);

fn str_member(n: Option<&Node>, key: &str) -> Option<String> {
    node::string(node::member(n, key)).map(|s| s.to_string_lossy())
}

/// Static checks of a composition's 3D layers against what this pass
/// draws (conservative: anything that could need more at some time is
/// listed). Texture images are checked when `assets` can open them.
pub(crate) fn check(
    root: &Node,
    _comp: &Node,
    layers: &[Node],
    assets: Option<&dyn AssetResolver>,
) -> Vec<Issue> {
    let mut out = Vec::new();
    for l in layers {
        let data = node::member(node::member(Some(l), "d"), "data");
        let mut issues = Vec::new();
        match Role3d::of(l) {
            Some(Role3d::Object) => {
                check_object(root, node::member(data, "object"), assets, &mut issues);
            }
            Some(Role3d::Light) => check_light(node::member(data, "light"), &mut issues),
            _ => {}
        }
        out.extend(issues.into_iter().map(|why| (Some(js::layer_id(l)), why)));
    }
    out
}

/// Static checks of a legacy `scene3d` layer's embedded scene
/// (`d.data.scene`): its objects and lights meet the same checks as 3D
/// object and light layers, so a routing decision holds at every frame.
pub(crate) fn check_legacy_scene(
    root: &Node,
    layer: &Node,
    assets: Option<&dyn AssetResolver>,
) -> Vec<String> {
    let scene = node::member(
        node::member(node::member(Some(layer), "d"), "data"),
        "scene",
    );
    let Some(scene) = scene.filter(|s| matches!(s.kind(), NodeKind::Object(_))) else {
        return vec!["a legacy 3D scene without a scene object".to_owned()];
    };
    let mut out = Vec::new();
    for o in node::array(node::member(Some(scene), "objects")).unwrap_or(&[]) {
        let mut issues = Vec::new();
        check_object(root, Some(o), assets, &mut issues);
        let id = str_member(Some(o), "id").unwrap_or_default();
        out.extend(
            issues
                .into_iter()
                .map(|why| format!("{why} (legacy scene object {id})")),
        );
    }
    for l in node::array(node::member(Some(scene), "lights")).unwrap_or(&[]) {
        let mut issues = Vec::new();
        check_light(Some(l), &mut issues);
        let id = str_member(Some(l), "id").unwrap_or_default();
        out.extend(
            issues
                .into_iter()
                .map(|why| format!("{why} (legacy scene light {id})")),
        );
    }
    out
}

/// The light kinds [`SceneBuilder`] draws (an absent type is a sun).
fn check_light(light: Option<&Node>, out: &mut Vec<String>) {
    if let Some(t) = node::member(light, "type") {
        match node::string(Some(t)).map(|s| s.to_string_lossy()) {
            Some(t) if matches!(t.as_str(), "sun" | "point" | "spot" | "area") => {}
            Some(t) => out.push(format!("{t} lights")),
            None => out.push("a 3D light type that is not a string".to_owned()),
        }
    }
}

/// One scene object (`d.data.object` of a 3D object layer, or an entry of
/// a legacy scene's `objects`): its source and its materials' maps.
fn check_object(
    root: &Node,
    object: Option<&Node>,
    assets: Option<&dyn AssetResolver>,
    out: &mut Vec<String>,
) {
    let asset = |a: &str| node::member(node::member(Some(root), "assets"), a);
    let source = node::member(object, "source");
    if let Some(a) = str_member(source, "assetId")
        && let Some(record) = asset(&a)
    {
        // Imported glTF scenes draw here when this engine loads the file
        // (an offline file is a missing model, as today); OBJ models draw
        // here, other assets as the missing-model box.
        let format = str_member(Some(record), "format").unwrap_or_default();
        let name = str_member(Some(record), "name")
            .unwrap_or_default()
            .to_ascii_lowercase();
        if matches!(format.as_str(), "gltf" | "glb")
            || (format.is_empty() && (name.ends_with(".glb") || name.ends_with(".gltf")))
        {
            match assets.and_then(|r| r.path(&a, Some(record))) {
                Some(path) => {
                    if let super::model::ModelLookup::Unloadable(why) = super::gltf::lookup(&path) {
                        out.push(why);
                    }
                }
                // Without media the file cannot be checked: routed.
                None if assets.is_none() => out.push("imported models".to_owned()),
                None => {}
            }
        }
    }
    if let Some(p) = str_member(source, "primitive")
        && !matches!(
            p.as_str(),
            "box" | "sphere" | "plane" | "cylinder" | "cone" | "torus" | "capsule" | "icosahedron"
        )
    {
        out.push(format!("{p} primitives"));
    }
    if node::truthy(node::member(object, "generation")) {
        out.push("model-group recipe parts".to_owned());
    }
    // Map images outside the drawn still classes.
    let mut materials: Vec<Option<&Node>> = vec![node::member(object, "material")];
    for slot in node::array(node::member(object, "slots")).unwrap_or(&[]) {
        materials.push(node::member(Some(slot), "material"));
    }
    for m in materials {
        let Some(maps) = node::member(m, "maps") else {
            continue;
        };
        let NodeKind::Object(map) = maps.kind() else {
            continue;
        };
        for v in map.values() {
            let Some(a) = node::string(Some(v)).map(|s| s.to_string_lossy()) else {
                continue;
            };
            let Some(record) = asset(&a) else {
                continue;
            };
            if !node::is_str(node::member(Some(record), "kind"), "image") {
                continue;
            }
            let format = str_member(Some(record), "format").unwrap_or_default();
            if format.eq_ignore_ascii_case("svg") {
                out.push("SVG texture maps".to_owned());
                continue;
            }
            match assets {
                Some(r) => {
                    if let Some(path) = r.path(&a, Some(record))
                        && let Err(super::textures::MapError::Unsupported(why)) =
                            super::textures::Textures::probe(&path)
                    {
                        out.push(why);
                    }
                }
                // Without media, the recorded container decides, as for
                // image layers (`capability::check_media`).
                None => {
                    let name = str_member(Some(record), "name").unwrap_or_default();
                    if let Err(f) = crate::stills::static_format(&format, &name) {
                        out.push(format!("3D texture map image ({f})"));
                    }
                }
            }
        }
    }
}

/// A legacy `scene3d` layer's scene (`sceneForLayer` + `sync`), rendered
/// whole at `width` × `height` with its own background.
#[allow(clippy::too_many_arguments)]
pub(crate) fn legacy_scene<'d>(
    scope: &mut Scope<'_, 'd>,
    layer: &'d Node,
    time: f64,
    width: u32,
    height: u32,
    assets: &dyn AssetResolver,
) -> JsResult<Result<Scene3d, String>> {
    let scene_node = node::member(
        node::member(node::member(Some(layer), "d"), "data"),
        "scene",
    );
    let Some(scene_node) = scene_node else {
        return Ok(Err("a legacy 3D scene without a scene".into()));
    };
    let Value::Object(scene) = scene_node.to_value() else {
        return Ok(Err("a legacy 3D scene that is not an object".into()));
    };
    // Channel nodes by path (`o.<id>.<key>`, `o.<id>.m.<key>`,
    // `light.<id>.<key>`, `camera.<key>`, `environment.<key>`).
    let mut channels: HashMap<String, &'d Node> = HashMap::new();
    let mut collect = |p: Option<&'d Node>, prefix: String| {
        if let Some(NodeKind::Object(map)) = p.map(Node::kind) {
            for (k, v) in map {
                channels.insert(format!("{prefix}.{}", k.to_string_lossy()), v);
            }
        }
    };
    for o in node::array(node::member(Some(scene_node), "objects")).unwrap_or(&[]) {
        let id = str_member(Some(o), "id").unwrap_or_default();
        collect(node::member(Some(o), "p"), format!("o.{id}"));
        let material = node::member(Some(o), "material");
        collect(node::member(material, "p"), format!("o.{id}.m"));
    }
    for l in node::array(node::member(Some(scene_node), "lights")).unwrap_or(&[]) {
        let id = str_member(Some(l), "id").unwrap_or_default();
        collect(node::member(Some(l), "p"), format!("light.{id}"));
    }
    collect(
        node::member(node::member(Some(scene_node), "camera"), "p"),
        "camera".into(),
    );
    collect(
        node::member(node::member(Some(scene_node), "environment"), "p"),
        "environment".into(),
    );
    struct Layered<'s, 'a, 'd> {
        scope: &'s mut Scope<'a, 'd>,
        layer: &'d Node,
        time: f64,
        channels: HashMap<String, &'d Node>,
        error: Option<pm_eval::JsThrow>,
    }
    impl super::model::Channels for Layered<'_, '_, '_> {
        fn evaluate(&mut self, channel: &Value, path: &str) -> Value {
            let stored = match channel {
                Value::Object(m) => m.get("v").cloned().unwrap_or(Value::Null),
                _ => Value::Null,
            };
            let Some(&prop) = self.channels.get(path) else {
                return stored;
            };
            match self
                .scope
                .channel_p(self.layer, Some(prop), self.time, path)
            {
                Ok(v) => v.to_json().unwrap_or(stored),
                Err(e) => {
                    self.error.get_or_insert(e);
                    stored
                }
            }
        }
    }
    let root = scope.comp.root();
    let textures = DocumentTextures { root, assets };
    let models = DocumentModels { root, assets };
    let mut evaluator = Layered {
        scope,
        layer,
        time,
        channels,
        error: None,
    };
    // A legacy scene syncs at `time − layer.from` (every model's mixer).
    let mut builder = SceneBuilder::default();
    let from = node::number(node::member(Some(layer), "from")).unwrap_or(0.0);
    builder.set_animation_times(time - from, HashMap::new());
    let built = builder.build(
        &scene,
        width,
        height,
        &Placement::Channels,
        &HashMap::new(),
        &mut evaluator,
        &textures,
        &models,
    );
    if let Some(e) = evaluator.error {
        return Err(e);
    }
    Ok(built)
}

/// A scene with no textures and channels as stored (tests and tools).
pub fn scene_from_json(
    scene: &pm_model::json::Map,
    width: u32,
    height: u32,
    objects: &HashMap<String, Mat4>,
) -> Result<Scene3d, String> {
    let lights = HashMap::new();
    let placement = if objects.is_empty() {
        Placement::Channels
    } else {
        Placement::Composition {
            objects,
            lights: &lights,
            camera: None,
        }
    };
    SceneBuilder::default().build(
        scene,
        width,
        height,
        &placement,
        &HashMap::new(),
        &mut Evaluated,
        &NoTextures,
        &super::model::NoModels,
    )
}
