//! Layer bounds and picking for 2D layers (`legacy/gl/compositor.ts`
//! `GL.bounds`, `nativeBounds`, `GL.pick`; `legacy/core/layer-groups.ts`
//! `groupBounds`; `legacy/core/vector-paths.ts` `pathValues` and
//! `groupMatrix`).
//!
//! Bounds are in layer space before the layer transform, relative to the
//! layer's anchor origin, with the anchor point (`ax`, `ay`) inside the
//! raster box. Shapes, text, captions, solids, media, shaders, extensions,
//! nulls, precomps, and groups are answered here; what only today's 3D
//! runtime can answer in Phase 1 (3D object bounds and picks, model groups,
//! extension scenes without a background) is asked of a [`LayerHost`].

use pm_model::{JsString, Node};
use pm_vector::{PathLayer, PathNode, PathValues, Vertex};

use crate::evaluator::Content;
use crate::js_sem::{JsResult, JsThrow, JsValue};
use crate::node::{self, Got};
use crate::parenting::Role3d;
use crate::scope::{Scope, flag, has_paths, is_type, js_max, js_min};
use crate::space3d::{Rect, inverse_plane, project_point};
use crate::text_geom::TextMeasure;
use crate::transform::{apply, invert, mul};

/// `GL.bounds`' answer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    pub w: f64,
    pub h: f64,
    /// The anchor origin inside the raster box.
    pub ax: f64,
    pub ay: f64,
    /// A model group's depth extent (`modelGroupBounds`' `z0` and `z1`).
    pub z: Option<(f64, f64)>,
}

impl Bounds {
    pub fn rect(&self) -> Rect {
        Rect {
            x0: self.x0,
            y0: self.y0,
            x1: self.x1,
            y1: self.y1,
        }
    }

    fn sized(x0: f64, y0: f64, x1: f64, y1: f64, ax: f64, ay: f64) -> Bounds {
        Bounds {
            x0,
            y0,
            x1,
            y1,
            w: x1 - x0,
            h: y1 - y0,
            ax,
            ay,
            z: None,
        }
    }
}

/// The thrown kind for questions only the host's 3D runtime answers in
/// Phase 1.
pub const HOST_ONLY: &str = "HostOnly";

/// What the host supplies for layer questions: registry facts, and the 3D
/// geometry (model assets, the editor camera) that 3D picks and bounds
/// need ([`crate::query3d`]). Without geometry, every 3D question throws a
/// [`HOST_ONLY`] error, so an unanswered question is never silently wrong.
pub trait LayerHost {
    /// The 3D geometry service, or `None` when this host has none.
    fn geometry(&mut self) -> Option<&mut dyn crate::query3d::Geometry3d> {
        None
    }
    /// `PM.layerDefinition(definition)?.renderer`: the asset field of a mesh
    /// renderer definition, `None` for other definitions. The default knows
    /// the built-in OBJ model layer.
    fn mesh_asset_field(&self, definition: &JsString) -> Option<String> {
        (definition.as_str() == Some("powermove.3d.obj-model")).then(|| "assetId".to_owned())
    }
}

/// A host that answers nothing beyond the defaults.
pub struct NoHost;

impl LayerHost for NoHost {}

/// `PM.TYPE_META[type].pickable === false`.
fn unpickable(layer: &Node) -> bool {
    ["audio", "adjustment", "group"]
        .iter()
        .any(|t| is_type(layer, t))
}

fn content_get<'c>(d: &'c Content, key: &str) -> &'c JsValue {
    static UNDEFINED: JsValue = JsValue::Undefined;
    d.get(&JsString::from(key)).unwrap_or(&UNDEFINED)
}

/// `a || b` on JavaScript values, then `Number`.
fn or_number(a: &JsValue, b: &JsValue) -> f64 {
    if a.truthy() {
        a.to_number()
    } else {
        b.to_number()
    }
}

/// Evaluated values of one editable path (`pathValues`).
#[derive(Clone, Debug, PartialEq)]
pub struct EvaluatedPath {
    pub id: JsValue,
    /// `path.p` evaluated (raw JavaScript values).
    pub values: Content,
    /// Each vertex's `p` evaluated (`Number(...) || 0`).
    pub vertices: Vec<Vec<(JsString, f64)>>,
}

impl EvaluatedPath {
    /// The geometry values `pm-vector` draws.
    pub fn path_values(&self) -> PathValues {
        let n = |k: &str| content_get(&self.values, k).to_number();
        let vertex = |v: &Vec<(JsString, f64)>| {
            let get = |k: &str| {
                v.iter()
                    .find(|(key, _)| key.as_str() == Some(k))
                    .map_or(0.0, |(_, x)| *x)
            };
            Vertex {
                x: get("x"),
                y: get("y"),
                in_x: get("inX"),
                in_y: get("inY"),
                out_x: get("outX"),
                out_y: get("outY"),
            }
        };
        PathValues {
            x: n("x"),
            y: n("y"),
            rotation: n("rotation"),
            scale_x: n("scaleX"),
            scale_y: n("scaleY"),
            closed: content_get(&self.values, "closed").truthy(),
            fill_enabled: content_get(&self.values, "fillEnabled").truthy(),
            fill_opacity: n("fillOpacity"),
            stroke_width: n("strokeWidth"),
            stroke_opacity: n("strokeOpacity"),
            trim_start: n("trimStart"),
            trim_end: n("trimEnd"),
            trim_offset: n("trimOffset"),
            copies: n("copies"),
            repeat_x: n("repeatX"),
            repeat_y: n("repeatY"),
            repeat_rotation: n("repeatRotation"),
            vertices: self.vertices.iter().map(vertex).collect(),
        }
    }
}

impl<'d> Scope<'_, 'd> {
    /// `pathValues(PM, layer, path, time, prefix)`.
    pub fn path_values(
        &mut self,
        layer: &'d Node,
        path: &'d Node,
        time: f64,
        prefix: &str,
    ) -> JsResult<EvaluatedPath> {
        let mut values = Content::new();
        let p = node::get_checked(Some(path), b"p")?;
        if let Got::Node(p) = &p {
            if node::is_nullish(Some(p)) {
                return Err(JsThrow::type_error(
                    "Cannot convert undefined or null to object",
                ));
            }
            for (k, prop) in node::entries(p) {
                let key = format!("{prefix}.{}", k.to_string_lossy());
                let v = self.evaluated(layer, prop.node(), time, &key)?;
                values.insert(k, v);
            }
        } else {
            return Err(JsThrow::type_error(
                "Cannot convert undefined or null to object",
            ));
        }
        let mut vertices = Vec::new();
        let list = node::get_checked(Some(path), b"vertices")?;
        for v in node::array(list.node()).unwrap_or(&[]) {
            let id = node::to_js_string(node::member(Some(v), "id")).to_string_lossy();
            let mut out = Vec::new();
            if let Some(vp) = node::member(Some(v), "p") {
                for (k, prop) in node::entries(vp) {
                    let key = format!("{prefix}.v.{id}.{}", k.to_string_lossy());
                    let x = crate::scope::or0(
                        self.evaluated(layer, prop.node(), time, &key)?.to_number(),
                    );
                    out.push((k, x));
                }
            }
            vertices.push(out);
        }
        Ok(EvaluatedPath {
            id: node::js_value_opt(node::member(Some(path), "id")),
            values,
            vertices,
        })
    }

    /// Every editable path of a shape layer with its values (`d.paths`).
    pub fn layer_paths(&mut self, layer: &'d Node, time: f64) -> JsResult<Vec<EvaluatedPath>> {
        let d = node::get_checked(Some(layer), b"d")?;
        let paths = d.node().and_then(|d| node::member(Some(d), "paths"));
        let mut out = Vec::new();
        for path in node::array(paths).unwrap_or(&[]) {
            let id = node::to_js_string(node::member(Some(path), "id")).to_string_lossy();
            out.push(self.path_values(layer, path, time, &format!("g.{id}"))?);
        }
        Ok(out)
    }

    /// `groupMatrix` for each path, and the `rasterPaths` bounds.
    pub fn path_geometry(
        &mut self,
        layer: &'d Node,
        time: f64,
    ) -> JsResult<(Vec<EvaluatedPath>, Vec<[f64; 6]>, PathLayer)> {
        let evaluated = self.layer_paths(layer, time)?;
        let d = node::member(Some(layer), "d");
        let stored = node::array(node::member(d, "paths")).unwrap_or(&[]);
        let nodes: Vec<PathNode> = evaluated
            .iter()
            .zip(stored)
            .map(|(e, s)| PathNode {
                id: e.id.to_js_string().to_string_lossy(),
                parent: {
                    let p = node::member(Some(s), "parent");
                    node::string(p).map(|s| s.to_string_lossy())
                },
                values: e.path_values(),
            })
            .collect();
        let matrices = (0..nodes.len())
            .map(|i| pm_vector::path::group_matrix(&nodes, i).as_coeffs())
            .collect();
        Ok((evaluated, matrices, PathLayer::new(&nodes)))
    }

    /// Whether `sceneRuntime(PM, L, T)` exists for an extension layer and its
    /// scene has no background (`!runtime.scene.background`): a legacy scene
    /// layer whose environment has no background color, or a mesh
    /// definition layer with an asset and a falsy `background` parameter.
    pub fn backgroundless_scene(
        &mut self,
        layer: &'d Node,
        time: f64,
        host: &dyn LayerHost,
    ) -> JsResult<bool> {
        let d = node::member(Some(layer), "d");
        let definition = node::member(d, "definition");
        if node::is_str(definition, "powermove.3d.scene") {
            let scene = node::member(node::member(d, "data"), "scene");
            if !node::truthy(scene) {
                return Ok(false);
            }
            let background = node::member(node::member(scene, "environment"), "background");
            return Ok(!node::truthy(background));
        }
        let Some(field) = node::string(definition).and_then(|id| host.mesh_asset_field(id)) else {
            return Ok(false);
        };
        let params = node::member(d, "params");
        let background = match node::member(params, "background") {
            Some(prop) if node::truthy(Some(prop)) => {
                self.channel_p(layer, Some(prop), time, "x.background")?
            }
            _ => JsValue::from("#0C0D12"),
        };
        let asset = node::member(node::member(d, "data"), &field);
        Ok(node::truthy(asset) && !background.truthy())
    }

    /// `GL.bounds(L, T)` for any layer: `None` where the legacy answer is
    /// `null`.
    pub fn bounds(
        &mut self,
        layer: &'d Node,
        time: f64,
        measure: &mut dyn TextMeasure,
        host: &mut dyn LayerHost,
    ) -> JsResult<Option<Bounds>> {
        let bounds = self.native_bounds(layer, time, measure, host)?;
        if Role3d::of(layer).is_some() {
            return Ok(bounds);
        }
        let Some(b) = bounds else {
            return Ok(None);
        };
        let offset = self.size_anchor_offset(layer, time)?;
        if offset.x == 0.0 && offset.y == 0.0 {
            return Ok(Some(b));
        }
        Ok(Some(Bounds {
            x0: b.x0 + offset.x,
            x1: b.x1 + offset.x,
            y0: b.y0 + offset.y,
            y1: b.y1 + offset.y,
            ax: b.ax - offset.x,
            ay: b.ay - offset.y,
            ..b
        }))
    }

    /// `nativeBounds(L, T)`.
    fn native_bounds(
        &mut self,
        layer: &'d Node,
        time: f64,
        measure: &mut dyn TextMeasure,
        host: &mut dyn LayerHost,
    ) -> JsResult<Option<Bounds>> {
        match Role3d::of(layer) {
            Some(Role3d::Object) => return self.object_bounds3d(layer, time, host),
            Some(_) => return Ok(None),
            None => {}
        }
        if is_type(layer, "group") {
            return self.group_bounds(layer, time, measure, host);
        }
        if is_type(layer, "extension") && self.backgroundless_scene(layer, time, host)? {
            return self.scene_bounds3d(layer, time, host);
        }
        let ty = node::to_js_string(node::member(Some(layer), "type"));
        let ty = ty.as_str().unwrap_or("");
        if ty == "captions" {
            let b = self.caption_bounds(layer, time, measure)?;
            return Ok(Some(Bounds::sized(b.x0, b.y0, b.x1, b.y1, 0.0, 0.0)));
        }
        if ty == "text" {
            let r = self.text_raster(layer, time, measure)?;
            let s = r.selection;
            return Ok(Some(Bounds {
                x0: s.x0,
                y0: s.y0,
                x1: s.x1,
                y1: s.y1,
                w: s.w,
                h: s.h,
                ax: r.anchor_x,
                ay: r.anchor_y,
                z: None,
            }));
        }
        if !matches!(
            ty,
            "solid" | "shader" | "extension" | "null" | "precomp" | "shape" | "image" | "video"
        ) {
            return Ok(None);
        }
        if ty == "shape" && has_paths(layer)? {
            let (_, _, paths) = self.path_geometry(layer, time)?;
            let b = paths.bounds;
            return Ok(Some(Bounds {
                x0: b.x0,
                y0: b.y0,
                x1: b.x1,
                y1: b.y1,
                w: b.w,
                h: b.h,
                ax: -b.x0,
                ay: -b.y0,
                z: None,
            }));
        }
        let d = self.ev.resolve_content(self.comp, layer, time)?;
        let (pw, ph) = (
            node::js_value_opt(self.project.member("w")),
            node::js_value_opt(self.project.member("h")),
        );
        let w_raw = content_get(&d, "w");
        let h_raw = content_get(&d, "h");
        let (w, h, ax, ay) = match ty {
            "shape" => {
                let shape = pm_vector::ShapeContent {
                    w: w_raw.to_number(),
                    h: h_raw.to_number(),
                    stroke: content_get(&d, "stroke").to_number(),
                    ..Default::default()
                };
                let g = pm_vector::ShapeRaster::new(&shape, 1.0);
                let s = g.selection;
                return Ok(Some(Bounds {
                    x0: s.x0,
                    y0: s.y0,
                    x1: s.x1,
                    y1: s.y1,
                    w: s.w,
                    h: s.h,
                    ax: g.w / 2.0,
                    ay: g.h / 2.0,
                    z: None,
                }));
            }
            "null" => {
                let w = if w_raw.truthy() {
                    w_raw.to_number()
                } else {
                    100.0
                };
                let h = if h_raw.truthy() {
                    h_raw.to_number()
                } else {
                    100.0
                };
                (w, h, 0.0, 0.0)
            }
            "image" | "video" => {
                let w = or_number(w_raw, &pw);
                let h = or_number(h_raw, &ph);
                (w, h, w / 2.0, h / 2.0)
            }
            _ => (or_number(w_raw, &pw), or_number(h_raw, &ph), 0.0, 0.0),
        };
        Ok(Some(Bounds {
            x0: -ax,
            y0: -ay,
            x1: w - ax,
            y1: h - ay,
            w,
            h,
            ax,
            ay,
            z: None,
        }))
    }

    /// `PM.groupBounds(group, time)`: the union of the active members' bounds
    /// (or origins) in the group's space.
    pub fn group_bounds(
        &mut self,
        group: &'d Node,
        time: f64,
        measure: &mut dyn TextMeasure,
        host: &mut dyn LayerHost,
    ) -> JsResult<Option<Bounds>> {
        if self.is_model_group(group)? {
            return self.model_group_bounds3d(group, time, host);
        }
        let m = self.world_matrix(group, time)?;
        let Some(inv) = invert(&m) else {
            return Ok(None);
        };
        let gid = node::get(group, b"id").to_js();
        let mut points: Vec<(f64, f64)> = Vec::new();
        for child in self.layers() {
            if is_type(child, "group") {
                continue;
            }
            let inside = self
                .group_ancestors(child)?
                .iter()
                .any(|g| crate::js_sem::strict_eq(&node::get(g, b"id").to_js(), &gid));
            if !inside || !self.active(child, time)? {
                continue;
            }
            let bounds = self.bounds(child, time, measure, host)?;
            let world = self.world_matrix(child, time)?;
            let local = mul(&inv, &world);
            if let Some(b) = bounds {
                for x in [b.x0, b.x1] {
                    for y in [b.y0, b.y1] {
                        points.push(apply(&local, x, y));
                    }
                }
            } else if !is_type(child, "audio") {
                points.push((local[4], local[5]));
            }
        }
        if points.is_empty() {
            return Ok(None);
        }
        let fold = |f: fn(f64, f64) -> f64, start: f64, sel: fn(&(f64, f64)) -> f64| {
            points.iter().map(sel).fold(start, f)
        };
        let x0 = fold(js_min, f64::INFINITY, |p| p.0);
        let x1 = fold(js_max, f64::NEG_INFINITY, |p| p.0);
        let y0 = fold(js_min, f64::INFINITY, |p| p.1);
        let y1 = fold(js_max, f64::NEG_INFINITY, |p| p.1);
        Ok(Some(Bounds::sized(x0, y0, x1, y1, 0.0, 0.0)))
    }

    /// `compositionOrderedLayers(PM, layers, T)` without the 3D camera depth
    /// sort of model blocks: blocks of 3D layers (objects, lights, cameras)
    /// keep their stack order, and other blocks are depth ordered. A pick
    /// does not depend on the order inside a model block (three.js answers
    /// the nearest object of the depth set).
    pub fn pick_order(&mut self, layers: &[&'d Node], time: f64) -> JsResult<Vec<&'d Node>> {
        if !layers.iter().any(|l| Role3d::of(l).is_some()) {
            return self.depth_ordered_layers(layers, time);
        }
        let mut out = Vec::with_capacity(layers.len());
        let mut start = 0;
        while start < layers.len() {
            let model = Role3d::of(layers[start]).is_some();
            let mut end = start + 1;
            while end < layers.len() && Role3d::of(layers[end]).is_some() == model {
                end += 1;
            }
            if model {
                out.extend_from_slice(&layers[start..end]);
            } else {
                out.extend(self.depth_ordered_layers(&layers[start..end], time)?);
            }
            start = end;
        }
        Ok(out)
    }

    /// `GL.pick(x, y, T, options)`: the top-most layer under a composition
    /// point.
    pub fn pick(
        &mut self,
        x: f64,
        y: f64,
        time: f64,
        options: &PickOptions<'_>,
        measure: &mut dyn TextMeasure,
        host: &mut dyn LayerHost,
    ) -> JsResult<Option<&'d Node>> {
        let layers: Vec<&'d Node> = self
            .layers()
            .iter()
            .filter(|l| match options.local_view {
                Some(ids) => {
                    Role3d::of(l) != Some(Role3d::Object)
                        || ids
                            .iter()
                            .any(|id| crate::js_sem::strict_eq(id, &node::get(l, b"id").to_js()))
                }
                None => true,
            })
            .collect();
        let (w, h) = self.project_size();
        for layer in self.pick_order(&layers, time)? {
            if !self.active(layer, time)? || unpickable(layer) {
                continue;
            }
            if !options.include_locked
                && (flag(layer, "lock")
                    || self.group_ancestors(layer)?.iter().any(|g| flag(g, "lock")))
            {
                continue;
            }
            match Role3d::of(layer) {
                Some(Role3d::Object) => {
                    if self.pick_object3d(
                        layer,
                        x / w * 2.0 - 1.0,
                        1.0 - y / h * 2.0,
                        time,
                        host,
                    )? {
                        return Ok(Some(layer));
                    }
                    continue;
                }
                Some(_) => continue,
                None => {}
            }
            let Some(b) = self.bounds(layer, time, measure, host)? else {
                continue;
            };
            if self.plane_contains(layer, time, x, y, &b.rect())? {
                let scene =
                    is_type(layer, "extension") && self.backgroundless_scene(layer, time, host)?;
                if scene {
                    let Some(inverse) = inverse_plane(&self.plane_matrix(layer, time, false)?)
                    else {
                        continue;
                    };
                    let (px, py) = project_point(&inverse, x, y);
                    if !self.pick_scene3d(layer, px, py, time, host)? {
                        continue;
                    }
                }
                return Ok(Some(layer));
            }
        }
        Ok(None)
    }
}

/// `GL.pick` options.
#[derive(Clone, Copy, Debug, Default)]
pub struct PickOptions<'a> {
    /// `includeLocked`: locked layers (and layers in locked groups) can be picked.
    pub include_locked: bool,
    /// The editor's Local View set: object layers outside it are skipped.
    /// `None` outside Local View.
    pub local_view: Option<&'a [JsValue]>,
}
