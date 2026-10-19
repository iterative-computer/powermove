//! Blender snapshots built from `pm-eval` and `pm-geom3d` (plan section 17,
//! contract 11): the port of today's `renderSnapshot`
//! (`core/scene3d/rendering.ts`), verified equal to it on the corpus
//! (`tests/blender_snapshots.rs`, section 25's Blender snapshots class).
//!
//! The scene comes from `compositionScene` (`Scope::composition_scene`),
//! its meshes and world matrices from the 3D pass's scene builder (the same
//! geometry and placement `compositionRuntime` gives three.js), and the
//! rest from the composition's layers:
//!
//! - **objects**: every scene object whose layer is in the composition and
//!   whose world opacity is above 0.001, with one mesh per geometry group
//!   (`groups`, or one group over the whole geometry): its vertices
//!   de-indexed (`getVertexPosition`), its normals normalized, its UVs
//!   (`uv.getX(i) || 0`), the group's material index, the slot's material
//!   or the object's (`slot?.material || node.material`, as evaluated), the
//!   mesh's world matrix, and `smooth: true`. `castShadow` is
//!   `node.castShadow && environment.shadows`; `receiveShadow` as stored.
//!   An invisible object keeps no meshes. Source materials
//!   (`useSourceMaterials` without a slot) are written as
//!   `sourceMaterialSnapshot` writes the runtime's material: a fresh
//!   `MeshStandardMaterial`, or the missing model's `MeshBasicMaterial` as
//!   an emissive surface (amendment 14's fix).
//! - **lights** in scene order at their runtime positions and targets
//!   (points and area lights have none: `[0, 0, 0]`).
//! - **blocks**: each object's depth set over the composition's non-group
//!   layers in stack order (`compositionDepthIds`), in its insertion order
//!   (the layer, then outward up the stack, then down);
//!   **backgroundFor**: the objects that are the bottom of their set in
//!   `compositionOrderedLayers`.
//! - **camera**: the composition's camera (or the editor camera of a
//!   preview in the editor view): `matrixWorld`, orthographic, `top −
//!   bottom` (6 after `resizeCamera`, 4 for a perspective camera), field of
//!   view (45 for an orthographic camera), zoom, near, far.
//! - **size**: the frame size times the preview scale, `Math.round`, at
//!   least 2; **settings**: `render3d` parsed, the frame's samples, EEVEE
//!   for Material Preview; **environment** values; **background**.
//! - **assets**: every texture and shader source the scene's materials
//!   name, PNG, JPEG, and WebP images as they are (`<id>.png|jpg|webp`),
//!   other images converted to 8-bit PNG (`<id>.png`; today's Canvas2D
//!   conversion, appendix A item 3), other assets by name.
//!
//! Imported glTF models and model-group recipe parts are not built here
//! (the glTF lane's loader and `pm-geom3d`'s recipe geometry); the builder
//! answers why, and the caller routes the frame.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use pm_blender::snapshot::{AssetBytes, BlenderFrame, Render3dSettings, Snapshot, SnapshotAsset};
use pm_eval::node;
use pm_eval::parenting::Role3d;
use pm_eval::{JsResult, Scope};
use pm_geom3d::layering;
use pm_model::Node;
use pm_model::json::{JsString, Map, Value};

use crate::assets::AssetResolver;
use crate::js;
use crate::scene3d::{Camera3d, LightKind, MaterialKind, Projection, Scene3d};

/// How a snapshot is built.
#[derive(Debug, Clone, Default)]
pub struct SnapshotOptions {
    /// Material Preview: EEVEE whatever the composition's engine.
    pub force_eevee: bool,
    /// The editor camera of a preview in the editor view (root composition).
    pub camera: Option<Camera3d>,
    /// Settings to use instead of the composition's `render3d` (the corpus
    /// capture turned Blender on in memory with these).
    pub settings: Option<Render3dSettings>,
}

fn obj<'a>(v: &'a Value, key: &str) -> Option<&'a Map> {
    v.get(key).and_then(Value::as_object)
}

fn num(v: f64) -> Value {
    Value::Number(v)
}

fn numbers(list: impl IntoIterator<Item = f64>) -> Value {
    Value::Array(list.into_iter().map(Value::Number).collect())
}

/// JavaScript `a && b` over JSON values (`None` is `undefined`).
fn js_and(a: Option<&Value>, b: Option<&Value>) -> Option<Value> {
    match a {
        Some(v) if v.is_truthy() => b.cloned(),
        other => other.cloned(),
    }
}

/// `Math.round`.
fn round(x: f64) -> f64 {
    js::round(x)
}

/// `new THREE.MeshStandardMaterial()` and the missing model's
/// `MeshBasicMaterial({color: 0xff6600})` as `sourceMaterialSnapshot`
/// writes them (no maps: neither has textures).
fn source_material(basic: bool) -> Value {
    let channel = |v: Value| {
        let mut m = Map::new();
        m.insert("v", v);
        Value::Object(m)
    };
    let s = |t: &str| Value::String(JsString::from(t));
    let mut p = Map::new();
    if basic {
        p.insert("color", channel(s("#000000")));
        p.insert("roughness", channel(num(1.0)));
        p.insert("metalness", channel(num(0.0)));
        p.insert("emissive", channel(s("#ff6600")));
        p.insert("emissiveIntensity", channel(num(1.0)));
        p.insert("opacity", channel(num(1.0)));
    } else {
        p.insert("color", channel(s("#ffffff")));
        p.insert("roughness", channel(num(1.0)));
        p.insert("metalness", channel(num(0.0)));
        p.insert("emissive", channel(s("#000000")));
        p.insert("emissiveIntensity", channel(num(1.0)));
        p.insert("opacity", channel(num(1.0)));
    }
    let mut m = Map::new();
    m.insert("p", Value::Object(p));
    m.insert("maps", Value::Object(Map::new()));
    Value::Object(m)
}

/// `Vector3.normalize()`: `divideScalar(length() || 1)`, i.e. a multiply
/// by the reciprocal.
fn normalize(x: f64, y: f64, z: f64) -> [f64; 3] {
    let len = (x * x + y * y + z * z).sqrt();
    let s = 1.0 / if len == 0.0 || len.is_nan() { 1.0 } else { len };
    [x * s, y * s, z * s]
}

/// `value || 0` of an f32 attribute component.
fn or_zero(v: f32) -> f64 {
    let v = f64::from(v);
    if v == 0.0 || v.is_nan() { 0.0 } else { v }
}

/// The meshes of one object (`root.traverse`).
fn object_meshes(scene_node: &Value, object: &crate::scene3d::Object3d) -> Vec<Value> {
    let mut out = Vec::new();
    if !object.visible {
        return out;
    }
    let slots = scene_node
        .get("slots")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let use_source = scene_node
        .get("useSourceMaterials")
        .is_some_and(Value::is_truthy);
    for mesh in &object.meshes {
        let g = &mesh.geometry.geometry;
        let vertices = g.vertex_count();
        let index_count = if g.indexed { g.indices.len() } else { vertices };
        let index = |i: usize| -> usize { if g.indexed { g.indices[i] as usize } else { i } };
        let groups: Vec<(usize, usize, u32)> = if g.groups.is_empty() {
            vec![(0, index_count, 0)]
        } else {
            g.groups
                .iter()
                .map(|gr| (gr.start as usize, gr.count as usize, gr.material_index))
                .collect()
        };
        let has_normals = !g.normals.is_empty();
        let has_uvs = !g.uvs.is_empty();
        for (start, count, material_index) in groups {
            let end = (start + count).min(index_count);
            let n = end.saturating_sub(start);
            let mut positions = Vec::with_capacity(n * 3);
            let mut normals = Vec::with_capacity(if has_normals { n * 3 } else { 0 });
            let mut uvs = Vec::with_capacity(n * 2);
            for i in start..end {
                let v = index(i);
                positions.extend(g.positions[v * 3..v * 3 + 3].iter().map(|p| f64::from(*p)));
                if has_normals {
                    let q = &g.normals[v * 3..v * 3 + 3];
                    normals.extend(normalize(f64::from(q[0]), f64::from(q[1]), f64::from(q[2])));
                }
                if has_uvs {
                    uvs.push(or_zero(g.uvs[v * 2]));
                    uvs.push(or_zero(g.uvs[v * 2 + 1]));
                } else {
                    uvs.extend([0.0, 0.0]);
                }
            }
            let slot = slots.get(material_index as usize).filter(|s| !s.is_null());
            let material = if use_source && slot.is_none() {
                source_material(
                    object.missing
                        && matches!(
                            mesh.materials.iter().next().map(|m| m.kind),
                            Some(MaterialKind::Basic)
                        ),
                )
            } else {
                slot.and_then(|s| s.get("material"))
                    .filter(|m| m.is_truthy())
                    .or_else(|| scene_node.get("material"))
                    .cloned()
                    .unwrap_or(Value::Null)
            };
            let mut m = Map::new();
            m.insert("positions", numbers(positions));
            m.insert("normals", numbers(normals));
            m.insert("uvs", numbers(uvs));
            m.insert("materialIndex", num(f64::from(material_index)));
            m.insert("matrix", numbers(mesh.world));
            m.insert("material", material);
            m.insert("smooth", Value::Bool(true));
            out.push(Value::Object(m));
        }
    }
    out
}

/// A composition's non-group layers in stack order.
fn non_group_layers<'d>(scope: &Scope<'_, 'd>) -> Vec<&'d Node> {
    scope
        .layers()
        .iter()
        .filter(|l| js::layer_type(l) != "group")
        .collect()
}

/// `blocks` and `backgroundFor`.
fn depth_sets<'d>(
    scope: &mut Scope<'_, 'd>,
    time: f64,
    scene: &Scene3d,
    objects: &[String],
) -> JsResult<(Map, Vec<Value>)> {
    let layers = non_group_layers(scope);
    let solo = scope.layers().iter().any(|l| js::flag(l, "solo"));
    let mut active = Vec::with_capacity(layers.len());
    let mut joins = Vec::with_capacity(layers.len());
    for l in &layers {
        let a = scope.active(l, time)?;
        active.push(a);
        let object = Role3d::of(l) == Some(Role3d::Object);
        // `!solo || l.solo` (group ancestors are looked up in a list
        // without groups, so none are found) and opacity above 0.001.
        joins.push(
            a && object && (!solo || js::flag(l, "solo")) && scope.world_opacity(l, time)? > 0.001,
        );
    }
    // `compositionOrderedLayers` over the same list: runs of 3D layers
    // sorted back to front by the camera depth of their object origin.
    let view = scene.camera.view();
    let is_3d: Vec<bool> = layers.iter().map(|l| Role3d::of(l).is_some()).collect();
    let mut order = vec![0usize; layers.len()];
    let mut next = 0usize;
    for (range, three) in layering::runs(&is_3d) {
        let block: Vec<usize> = range.clone().collect();
        let sorted: Vec<usize> = if three {
            let depths: Vec<f64> = block
                .iter()
                .map(|&i| {
                    let id = js::layer_id(layers[i]);
                    let origin = scene
                        .objects
                        .iter()
                        .find(|o| o.id == id)
                        .map(|o| o.origin)
                        .or_else(|| {
                            scene
                                .lights
                                .iter()
                                .find(|l| l.id == id)
                                .map(|l| l.position())
                        });
                    origin.map_or(0.0, |p| layering::view_z(&view, p))
                })
                .collect();
            layering::sort_run(&depths)
                .into_iter()
                .map(|k| block[k])
                .collect()
        } else {
            block
        };
        for i in sorted {
            order[i] = next;
            next += 1;
        }
    }
    let mut blocks = Map::new();
    let mut background = Vec::new();
    for id in objects {
        let Some(index) = layers.iter().position(|l| &js::layer_id(l) == id) else {
            continue;
        };
        // The walk: the layer, then up the stack (towards index 0), then
        // down, stopping at an active layer without a 3D role.
        let mut ids: Vec<usize> = Vec::new();
        let mut visit = |i: usize| -> bool {
            if !active[i] {
                return true;
            }
            if Role3d::of(layers[i]).is_none() {
                return false;
            }
            if joins[i] {
                ids.push(i);
            }
            true
        };
        let mut i = index as isize;
        while i >= 0 && visit(i as usize) {
            i -= 1;
        }
        let mut i = index + 1;
        while i < layers.len() && visit(i) {
            i += 1;
        }
        blocks.insert(
            id.as_str(),
            Value::Array(
                ids.iter()
                    .map(|&i| Value::String(JsString::from(js::layer_id(layers[i]).as_str())))
                    .collect(),
            ),
        );
        if ids.iter().max_by_key(|&&i| order[i]) == Some(&index) {
            background.push(Value::String(JsString::from(id.as_str())));
        }
    }
    Ok((blocks, background))
}

fn camera_json(camera: &Camera3d) -> Value {
    let mut m = Map::new();
    m.insert("matrix", numbers(camera.world));
    let (ortho, fov, zoom, near, far) = match camera.projection {
        Projection::Perspective {
            fov,
            zoom,
            near,
            far,
        } => (false, fov, zoom, near, far),
        Projection::Orthographic { zoom, near, far } => (true, 45.0, zoom, near, far),
    };
    m.insert("orthographic", Value::Bool(ortho));
    // `c.top − c.bottom`: ±3 after `resizeCamera`.
    m.insert("orthoHeight", num(if ortho { 6.0 } else { 4.0 }));
    m.insert("fov", num(fov));
    m.insert("zoom", num(zoom));
    m.insert("near", num(near));
    m.insert("far", num(far));
    Value::Object(m)
}

fn light_json(node: &Value, scene: &Scene3d, environment_shadows: Option<&Value>) -> Value {
    let id = node
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let p = obj(node, "p");
    let v = |k: &str| p.and_then(|p| p.get(k)).and_then(|c| c.get("v")).cloned();
    let light = scene.lights.iter().find(|l| l.id == id);
    let position = match light {
        Some(l) => l.position(),
        None => [
            v("x").and_then(|x| x.as_f64()).unwrap_or(0.0),
            v("y").and_then(|x| x.as_f64()).unwrap_or(0.0),
            v("z").and_then(|x| x.as_f64()).unwrap_or(0.0),
        ],
    };
    let target = match light {
        Some(l) if matches!(l.kind, LightKind::Directional | LightKind::Spot) => l.target,
        _ => [0.0; 3],
    };
    let mut m = Map::new();
    m.insert("id", Value::String(JsString::from(id.as_str())));
    if let Some(t) = node.get("type") {
        m.insert("type", t.clone());
    }
    m.insert("position", numbers(position));
    m.insert("target", numbers(target));
    for k in ["color", "intensity", "distance", "angle", "penumbra"] {
        if let Some(x) = v(k) {
            m.insert(k, x);
        }
    }
    for k in ["width", "height"] {
        m.insert(k, v(k).filter(|x| !x.is_null()).unwrap_or(num(1.0)));
    }
    if let Some(c) = js_and(node.get("castShadow"), environment_shadows) {
        m.insert("castShadow", c);
    }
    Value::Object(m)
}

/// Sniffs PNG, JPEG, and WebP (the formats the worker loads as they are;
/// today's `blob.type` of the stored media).
fn image_extension(path: &Path) -> Option<&'static str> {
    use std::io::Read;
    let mut head = [0u8; 12];
    let mut f = std::fs::File::open(path).ok()?;
    let n = f.read(&mut head).ok()?;
    let head = &head[..n];
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if head.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("jpg")
    } else if head.len() >= 12 && &head[0..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

/// An image Blender should not load as it is, as an 8-bit straight-alpha
/// PNG (today: drawn into a canvas and `toBlob('image/png')`).
fn convert_to_png(path: &Path) -> Result<Vec<u8>, String> {
    use pm_media::frame::{Layout, PlaneFormat};
    let pool = pm_media::pool::FramePool::new(0);
    let still =
        pm_media::image::read(path, &pool).map_err(|e| format!("{}: {e}", path.display()))?;
    let f = &still.frame.format;
    if f.layout != Layout::Rgba {
        return Err("a texture image layout this engine cannot convert".into());
    }
    let (w, h) = (f.width, f.height);
    let premultiplied = f.rgba_premultiplied;
    let rgba = still
        .frame
        .with_planes(|planes| {
            let p = &planes[0];
            let mut out = Vec::with_capacity(w * h * 4);
            for y in 0..h {
                for x in 0..w {
                    let mut px = [0f64; 4];
                    for (c, v) in px.iter_mut().enumerate() {
                        // SAFETY: inside the plane (`with_planes` views).
                        *v = unsafe {
                            let b = p.desc.format.bytes();
                            let at = p.ptr.add(y * p.stride + x * b);
                            match p.desc.format {
                                PlaneFormat::Rgba8 => f64::from(*at.add(c)) / 255.0,
                                PlaneFormat::Rgba16 => {
                                    f64::from(u16::from_le_bytes([
                                        *at.add(c * 2),
                                        *at.add(c * 2 + 1),
                                    ])) / 65535.0
                                }
                                PlaneFormat::Rgba32Float => f64::from(f32::from_le_bytes([
                                    *at.add(c * 4),
                                    *at.add(c * 4 + 1),
                                    *at.add(c * 4 + 2),
                                    *at.add(c * 4 + 3),
                                ])),
                                _ => 0.0,
                            }
                        };
                    }
                    if premultiplied && px[3] > 0.0 {
                        for c in 0..3 {
                            px[c] /= px[3];
                        }
                    }
                    out.extend(px.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8));
                }
            }
            out
        })
        .map_err(|e| e.to_string())?;
    if !matches!(
        f.planes.first().map(|p| p.format),
        Some(PlaneFormat::Rgba8 | PlaneFormat::Rgba16 | PlaneFormat::Rgba32Float)
    ) {
        return Err("a texture image sample format this engine cannot convert".into());
    }
    let mut png = Vec::new();
    {
        let mut e = png::Encoder::new(&mut png, w as u32, h as u32);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        e.write_header()
            .and_then(|mut wr| wr.write_image_data(&rgba))
            .map_err(|e| e.to_string())?;
    }
    Ok(png)
}

/// The assets the scene's materials name (`assetBytes` of each id).
fn scene_assets(
    root: &Node,
    scene: &Value,
    assets: &dyn AssetResolver,
) -> Result<Vec<SnapshotAsset>, String> {
    let mut ids: Vec<String> = Vec::new();
    let mut add = |id: Option<&str>| {
        if let Some(id) = id.filter(|s| !s.is_empty())
            && !ids.iter().any(|x| x == id)
        {
            ids.push(id.to_owned());
        }
    };
    for node in scene
        .get("objects")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let slots = node.get("slots").and_then(Value::as_array);
        let materials: Vec<&Value> = match slots {
            Some(s) if !s.is_empty() => s.iter().filter_map(|s| s.get("material")).collect(),
            _ => node.get("material").into_iter().collect(),
        };
        for m in materials {
            if let Some(maps) = obj(m, "maps") {
                for v in maps.values() {
                    add(v.as_str());
                }
            }
            if let Some(shader) = m.get("shader") {
                add(shader
                    .get("source")
                    .and_then(|s| s.get("assetId"))
                    .and_then(Value::as_str));
                for n in shader
                    .get("graph")
                    .and_then(|g| g.get("nodes"))
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    add(n.get("image").and_then(Value::as_str));
                }
            }
        }
    }
    let mut out = Vec::new();
    for id in ids {
        let record = node::member(node::member(Some(root), "assets"), &id)
            .ok_or_else(|| format!("Missing 3D asset: {id}"))?;
        let name = js::member_string(record, "name").unwrap_or_default();
        let path = assets
            .path(&id, Some(record))
            .filter(|p| p.exists())
            .ok_or_else(|| format!("Missing media for {name}"))?;
        let io = |e: std::io::Error| format!("{}: {e}", path.display());
        let asset = if node::is_str(node::member(Some(record), "kind"), "image") {
            match image_extension(&path) {
                Some(ext) => SnapshotAsset::new(
                    id.clone(),
                    format!("{id}.{ext}"),
                    AssetBytes::File(path.clone()),
                )
                .map_err(io)?,
                None => {
                    let png = convert_to_png(&path)
                        .map_err(|e| format!("Could not convert texture {name}: {e}"))?;
                    SnapshotAsset::new(
                        id.clone(),
                        format!("{id}.png"),
                        AssetBytes::Bytes(Arc::new(png)),
                    )
                    .map_err(io)?
                }
            }
        } else {
            SnapshotAsset::new(id.clone(), name, AssetBytes::File(path.clone())).map_err(io)?
        };
        out.push(asset);
    }
    Ok(out)
}

/// Builds the snapshot of `frame` (`scope` is the frame's composition).
/// Whether a model source names an imported glTF scene (`format` `gltf`
/// or `glb`, or a `.gltf`/`.glb` name without a format), as
/// `DocumentModels` reads it.
fn names_gltf(root: &Node, object: Option<&Node>) -> bool {
    let text = |n: Option<&Node>, k: &str| {
        node::string(node::member(n, k))
            .map(|s| s.to_string_lossy())
            .unwrap_or_default()
    };
    let asset = text(node::member(object, "source"), "assetId");
    if asset.is_empty() {
        return false;
    }
    let Some(record) = node::member(node::member(Some(root), "assets"), &asset) else {
        return false;
    };
    let format = text(Some(record), "format");
    let name = text(Some(record), "name").to_ascii_lowercase();
    matches!(format.as_str(), "gltf" | "glb")
        || (format.is_empty() && (name.ends_with(".glb") || name.ends_with(".gltf")))
}

/// Whether the composition draws an imported glTF model: 3D object layers
/// and legacy scenes' objects.
fn imports_gltf(scope: &Scope<'_, '_>) -> bool {
    let root = scope.comp.root();
    scope.layers().iter().any(|l| {
        let data = node::member(node::member(Some(l), "d"), "data");
        if names_gltf(root, node::member(data, "object")) {
            return true;
        }
        node::array(node::member(node::member(data, "scene"), "objects"))
            .unwrap_or(&[])
            .iter()
            .any(|o| names_gltf(root, Some(o)))
    })
}

pub fn build<'d>(
    scope: &mut Scope<'_, 'd>,
    frame: &BlenderFrame,
    assets: &dyn AssetResolver,
    options: &SnapshotOptions,
) -> JsResult<Result<Snapshot, String>> {
    let time = frame.time;
    let settings = match &options.settings {
        Some(s) => s.clone(),
        None => {
            let stored = node::member(Some(scope.comp.node()), "render3d").map(Node::to_value);
            match Render3dSettings::parse(stored.as_ref().filter(|v| !v.is_null())) {
                Ok(s) => s,
                Err(e) => return Ok(Err(e)),
            }
        }
    };
    // The 3D pass now loads imported glTF scenes, but this snapshot does
    // not yet match today's glTF meshes and embedded textures: such
    // frames stay routed until it does.
    if imports_gltf(scope) {
        return Ok(Err("imported models".into()));
    }
    let evaluated = scope.composition_scene(time)?;
    if !evaluated.recipe_parts.is_empty() {
        return Ok(Err("model-group recipe parts".into()));
    }
    let scene_json = evaluated.scene;
    let scene = match crate::scene3d::layers::composition_scene(
        scope,
        time,
        frame.width,
        frame.height,
        assets,
        None,
        false,
    )? {
        Ok(s) => s,
        Err(e) => return Ok(Err(e)),
    };
    let environment = scene_json.get("environment");
    let environment_shadows = environment.and_then(|e| e.get("shadows"));
    let layers: HashMap<String, &'d Node> = scope
        .layers()
        .iter()
        .map(|l| (js::layer_id(l), l))
        .collect();
    let mut objects = Vec::new();
    let mut ids = Vec::new();
    for node in scene_json
        .get("objects")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let id = node
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let Some(object) = scene.objects.iter().find(|o| o.id == id) else {
            continue;
        };
        let Some(layer) = layers.get(&id) else {
            continue;
        };
        if scope.world_opacity(layer, time)? <= 0.001 {
            continue;
        }
        let mut m = Map::new();
        m.insert("id", Value::String(JsString::from(id.as_str())));
        m.insert("meshes", Value::Array(object_meshes(node, object)));
        if let Some(c) = js_and(node.get("castShadow"), environment_shadows) {
            m.insert("castShadow", c);
        }
        if let Some(r) = node.get("receiveShadow") {
            m.insert("receiveShadow", r.clone());
        }
        objects.push(Value::Object(m));
        ids.push(id);
    }
    let lights: Vec<Value> = scene_json
        .get("lights")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|l| light_json(l, &scene, environment_shadows))
        .collect();
    let (blocks, background_for) = depth_sets(scope, time, &scene, &ids)?;
    let scale = if frame.preview {
        settings.preview_scale
    } else {
        1.0
    };
    let size = |v: u32| js::max(2.0, round(f64::from(v) * scale));
    let mut s = settings.to_json();
    if let Value::Object(m) = &mut s {
        if frame.preview && options.force_eevee {
            m.insert("engine", Value::String(JsString::from("eevee")));
        }
        m.insert(
            "samples",
            num(f64::from(if frame.preview {
                settings.preview_samples
            } else {
                settings.samples
            })),
        );
    }
    let mut env = Map::new();
    if let Some(p) = environment.and_then(|e| obj(e, "p")) {
        for (k, c) in p.iter() {
            env.insert(k.clone(), c.get("v").cloned().unwrap_or(Value::Null));
        }
    }
    let camera = options.camera.unwrap_or(scene.camera);
    let mut snapshot = Map::new();
    snapshot.insert("width", num(size(frame.width)));
    snapshot.insert("height", num(size(frame.height)));
    snapshot.insert("settings", s);
    // Output compatibility is per layer. It is hashed with the snapshot,
    // so an 8-bit legacy surface can never be reused for a level 2 render.
    let compatibility_layers: Vec<Value> = ids
        .iter()
        .filter(|id| {
            layers.get(*id).is_some_and(|layer| {
                let level2 = node::to_number(node::member(Some(layer), "compat")) >= 2.0;
                !level2
            })
        })
        .map(|id| Value::String(JsString::from(id.as_str())))
        .collect();
    if !compatibility_layers.is_empty() {
        snapshot.insert("compatibilityLayers", Value::Array(compatibility_layers));
    }
    snapshot.insert("objects", Value::Array(objects));
    snapshot.insert("lights", Value::Array(lights));
    snapshot.insert("blocks", Value::Object(blocks));
    snapshot.insert("backgroundFor", Value::Array(background_for));
    snapshot.insert("environment", Value::Object(env));
    if let Some(b) = environment.and_then(|e| e.get("background")) {
        snapshot.insert("background", b.clone());
    }
    snapshot.insert("camera", camera_json(&camera));
    let assets = match scene_assets(scope.comp.root(), &scene_json, assets) {
        Ok(a) => a,
        Err(e) => return Ok(Err(e)),
    };
    Ok(Ok(Snapshot {
        frame: frame.clone(),
        json: Value::Object(snapshot),
        assets,
    }))
}
