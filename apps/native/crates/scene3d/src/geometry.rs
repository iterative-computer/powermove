//! Geometry preflight runs on a worker at an immutable document snapshot. The
//! actor verifies the snapshot before accepting any geometry-derived edits.
use crate::{Error, Result, fail, navigation::*, schema};
use pm_geom3d::{
    gltf::{self, math, pose},
    primitives::Parameters,
    source::{MeshSource, Source},
};
use serde_json::{Value, json};
use std::collections::HashMap;
#[derive(Clone, Debug, PartialEq)]
pub struct EvaluatedGeometry {
    pub source: pm_model::Node,
    pub comp: Option<String>,
    pub time: f64,
    pub bounds: Value,
    pub parent_inverse: Value,
}
/// The imported-model part a ray meets first: its material slot (the
/// original glTF material index Properties' part sections and edit_3d's
/// `materialIndex` use), the world point and the distance along the ray.
#[derive(Clone, Debug, PartialEq)]
pub struct PartHit {
    pub slot: usize,
    /// The slot's name as Properties titles it (`import::material_slots`).
    pub name: String,
    pub point: Vec3,
    pub distance: f64,
}
/// A part is a node in the model, with hierarchy metadata and an optional
/// source material slot.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelPart {
    pub key: String,
    pub name: String,
    pub raw_name: String,
    pub parent: Option<String>,
    pub depth: usize,
    pub material: Option<usize>,
    pub material_name: Option<String>,
    pub swatch: Option<String>,
}

fn srgb_channel(linear: f64) -> u8 {
    let c = linear.clamp(0., 1.);
    let s = if c <= 0.003_130_8 {
        12.92 * c
    } else {
        1.055 * c.powf(1. / 2.4) - 0.055
    };
    (s * 255.).round().clamp(0., 255.) as u8
}

fn material_swatch(model: &gltf::Model, material_index: usize) -> (Option<String>, Option<String>) {
    let Some(mat) = model.materials.get(material_index) else {
        return (None, None);
    };
    let name = mat.name.trim();
    let material_name = (!name.is_empty()).then(|| name.to_owned());
    let [r, g, b] = mat.color;
    let swatch = Some(format!(
        "#{:02X}{:02X}{:02X}",
        srgb_channel(r),
        srgb_channel(g),
        srgb_channel(b)
    ));
    (material_name, swatch)
}

fn middle_truncate(text: &str, max_chars: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars || max_chars < 6 {
        return text.to_owned();
    }
    let tail = (max_chars / 2).min(18);
    let head = max_chars.saturating_sub(tail + 1);
    let mut out = String::with_capacity(text.len());
    for &c in &chars[..head] {
        out.push(c);
    }
    out.push('…');
    for &c in &chars[chars.len() - tail..] {
        out.push(c);
    }
    out
}

/// Finds a common delimiter-ended prefix shared by at least `min_matches` names
/// (e.g. `flow100_obj_JoyConL_00__Mt_JoyconL_` on game/CAD glTF exports).
fn dominant_token_prefix(names: &[&str]) -> Option<String> {
    if names.len() < 3 {
        return None;
    }
    let min_matches = ((names.len() * 6) / 10).max(3);
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for &name in names {
        let mut seen_for_name = std::collections::HashSet::new();
        for (idx, ch) in name.char_indices() {
            if matches!(ch, '_' | '-' | '.' | '/' | ':' | ' ') {
                let end = idx + ch.len_utf8();
                if end >= 4 && end < name.len() {
                    let prefix = &name[..end];
                    if seen_for_name.insert(prefix) {
                        *counts.entry(prefix).or_insert(0) += 1;
                    }
                }
            }
        }
    }
    counts
        .into_iter()
        .filter(|(_, count)| *count >= min_matches)
        .max_by_key(|(prefix, count)| (prefix.len(), *count))
        .map(|(prefix, _)| prefix.to_owned())
}

fn strip_prefix_tokens<'a>(name: &'a str, primary: Option<&str>, secondary: Option<&str>) -> &'a str {
    for prefix in [primary, secondary].into_iter().flatten() {
        if let Some(rest) = name.strip_prefix(prefix) {
            let trimmed = rest.trim_start_matches(['_', '-', '.', '/', ':', ' ']);
            if !trimmed.is_empty() {
                return trimmed;
            }
        }
    }
    name
}

/// Clean display labels for a model's parts: strip redundant exporter prefixes,
/// middle-truncate instead of cutting off unique suffixes, and disambiguate
/// identical labels with `#1`, `#2`, ...
pub fn clean_part_names(parts: &mut [ModelPart]) {
    if parts.is_empty() {
        return;
    }
    let raw_names: Vec<&str> = parts
        .iter()
        .map(|p| p.raw_name.as_str())
        .filter(|n| !n.is_empty() && !n.starts_with("Part "))
        .collect();
    let primary = dominant_token_prefix(&raw_names);
    // Also detect a broader root prefix (e.g. `flow100_obj_JoyConL_00__`) that
    // matches strictly more nodes so outliers like `flow100_obj_JoyConL_00__lamp_mt`
    // get trimmed cleanly too.
    let secondary = primary.as_deref().and_then(|p| {
        let primary_matches = raw_names.iter().filter(|n| n.starts_with(p)).count();
        let mut cursor = p.trim_end_matches(['_', '-', '.', '/', ':', ' ']);
        while let Some(cut) = cursor.rfind(['_', '-', '.', '/', ':', ' ']) {
            let candidate = &p[..=cut];
            if candidate.len() < 4 {
                break;
            }
            let matches = raw_names.iter().filter(|n| n.starts_with(candidate)).count();
            if matches > primary_matches {
                return Some(candidate.to_owned());
            }
            cursor = cursor[..cut].trim_end_matches(['_', '-', '.', '/', ':', ' ']);
        }
        None
    });

    for part in parts.iter_mut() {
        let base = strip_prefix_tokens(&part.raw_name, primary.as_deref(), secondary.as_deref());
        part.name = middle_truncate(base, 36);
        if let Some(mat) = part.material_name.as_deref() {
            let clean_mat = strip_prefix_tokens(mat, primary.as_deref(), secondary.as_deref());
            part.material_name = Some(middle_truncate(clean_mat, 32));
        }
    }

    // Disambiguate duplicates cleanly (#1, #2, ...) without losing order.
    let mut totals: HashMap<String, usize> = HashMap::new();
    for part in parts.iter() {
        *totals.entry(part.name.clone()).or_insert(0) += 1;
    }
    let mut seen: HashMap<String, usize> = HashMap::new();
    for part in parts.iter_mut() {
        if totals.get(&part.name).copied().unwrap_or(0) > 1 {
            let idx = seen.entry(part.name.clone()).or_insert(0);
            *idx += 1;
            part.name = format!("{} #{idx}", part.name);
        }
    }
}

fn part_info(model: &gltf::Model, index: usize) -> Option<ModelPart> {
    let key = pose::part_key(model, index)?;
    let object = &model.objects[index];
    let (material, material_name, swatch) = match &object.kind {
        gltf::ObjectKind::Drawable(d) => {
            let (mat_name, swatch) = material_swatch(model, d.material);
            (model.material_sources[d.material], mat_name, swatch)
        }
        _ => (None, None, None),
    };
    let raw_name = if object.name.trim().is_empty() || object.name.starts_with("mesh_") {
        material_name.clone()
    } else {
        Some(object.name.trim().to_owned())
    }
    .unwrap_or_else(|| format!("Part {}", key.replace('_', ".")));
    let name = middle_truncate(&raw_name, 36);
    Some(ModelPart {
        key,
        name,
        raw_name,
        parent: None,
        depth: 0,
        material,
        material_name,
        swatch,
    })
}

pub fn model_parts(model: &crate::import::Model) -> Vec<ModelPart> {
    let crate::import::Model::Gltf(model) = model else {
        return vec![];
    };
    let nodes = model.traversal();
    let mut has_surface = vec![false; model.objects.len()];
    for &index in nodes.iter().rev() {
        if matches!(model.objects[index].kind, gltf::ObjectKind::Drawable(_)) {
            has_surface[index] = true;
        }
        if has_surface[index]
            && let Some(parent) = model.objects[index].parent
        {
            has_surface[parent] = true;
        }
    }
    let mut key_by_index: HashMap<usize, String> = HashMap::new();
    let mut depth_by_index: HashMap<usize, usize> = HashMap::new();
    let mut parts = Vec::new();
    for index in nodes.into_iter().filter(|&index| has_surface[index]) {
        if parts.len() >= 4096 {
            break;
        }
        let Some(mut part) = part_info(model, index) else {
            continue;
        };
        // Walk up to nearest included ancestor so parent/depth reflect the tree.
        let mut cursor = model.objects[index].parent;
        while let Some(parent_idx) = cursor {
            if let Some(parent_key) = key_by_index.get(&parent_idx) {
                part.parent = Some(parent_key.clone());
                part.depth = depth_by_index.get(&parent_idx).copied().unwrap_or(0) + 1;
                break;
            }
            cursor = model.objects[parent_idx].parent;
        }
        key_by_index.insert(index, part.key.clone());
        depth_by_index.insert(index, part.depth);
        parts.push(part);
    }
    clean_part_names(&mut parts);
    parts
}

#[derive(Clone, Debug, PartialEq)]
pub struct PartPlacement {
    pub part: ModelPart,
    pub transform: pose::PartTransform,
    pub pivot: Vec3,
    pub bounds: Option<Bounds>,
    pub layer_world: math::Mat4,
}

pub trait AssetGeometry: Send + Sync {
    fn bounds(&self, id: &str, record: &Value, time: f64) -> Result<Bounds>;
    /// The part of the model asset `id`, placed by the layer matrix `world`
    /// at model time `time`, that the world ray `origin` + t·`direction`
    /// meets first. `None`: a miss, or a format without slots.
    fn part_at(
        &self,
        id: &str,
        _record: &Value,
        _world: [f64; 16],
        _time: f64,
        _origin: Vec3,
        _direction: Vec3,
    ) -> Result<Option<PartHit>> {
        fail(format!("3D model geometry is not loaded: {id}"))
    }
    #[allow(clippy::too_many_arguments)]
    fn pick_model_part(
        &self,
        _id: &str,
        _record: &Value,
        _world: math::Mat4,
        _time: f64,
        _parts: &Value,
        _origin: Vec3,
        _direction: Vec3,
    ) -> Result<Option<PartPlacement>> {
        Ok(None)
    }
    fn model_part(
        &self,
        _id: &str,
        _record: &Value,
        _world: math::Mat4,
        _time: f64,
        _parts: &Value,
        _key: &str,
    ) -> Result<Option<PartPlacement>> {
        Ok(None)
    }
    /// The glTF source uses metres. Its fit is inverted when a command opts
    /// into original size/origin; the renderer's normalized resource is kept.
    fn source_fit(&self, _id: &str, _record: &Value) -> Result<gltf::Fit> {
        fail("Real-world import scale requires decoded glTF/GLB geometry")
    }
}
pub struct NoAssets;
impl AssetGeometry for NoAssets {
    fn bounds(&self, id: &str, _: &Value, _: f64) -> Result<Bounds> {
        fail(format!("3D model geometry is not loaded: {id}"))
    }
}
/// Import preflight's normalized geometry box, available before mesh IO.
/// Legacy records without a retained measurement keep their unit envelope.
pub struct RetainedAssets;
impl AssetGeometry for RetainedAssets {
    fn bounds(&self, _: &str, record: &Value, _: f64) -> Result<Bounds> {
        if record["kind"] != "model" {
            return fail("Model geometry is not retained");
        }
        if record["native3dBounds"]["version"] == 1 {
            let array = record["native3dBounds"]["bounds"]
                .as_array()
                .filter(|v| v.len() == 6)
                .ok_or_else(|| Error("Invalid retained model bounds".into()))?;
            let mut v = [0.; 6];
            for (i, value) in array.iter().enumerate() {
                v[i] = value
                    .as_f64()
                    .ok_or_else(|| Error("Invalid retained model bounds".into()))?;
            }
            let bounds = Bounds {
                min: [v[0], v[1], v[2]],
                max: [v[3], v[4], v[5]],
            };
            bounds.validate()?;
            return Ok(bounds);
        }
        Ok(Bounds {
            min: [-1.; 3],
            max: [1.; 3],
        })
    }
}
#[derive(Default)]
pub struct LoadedAssets {
    models: HashMap<String, (Value, std::sync::Arc<crate::import::Model>)>,
}
impl LoadedAssets {
    pub fn insert(&mut self, id: String, record: Value, model: crate::import::Model) {
        self.models.insert(id, (record, std::sync::Arc::new(model)));
    }
    pub fn insert_shared(
        &mut self,
        id: String,
        record: Value,
        model: std::sync::Arc<crate::import::Model>,
    ) {
        self.models.insert(id, (record, model));
    }
    fn loaded_model(&self, id: &str, record: &Value) -> Result<&crate::import::Model> {
        let (loaded, model) = self
            .models
            .get(id)
            .ok_or_else(|| Error(format!("3D model geometry is not loaded: {id}")))?;
        if loaded != record {
            return fail("3D model source changed; reload geometry");
        }
        Ok(model)
    }
    pub fn remove(&mut self, id: &str) {
        self.models.remove(id);
    }
}
impl AssetGeometry for LoadedAssets {
    fn bounds(&self, id: &str, record: &Value, time: f64) -> Result<Bounds> {
        let (loaded, model) = self
            .models
            .get(id)
            .ok_or_else(|| Error(format!("3D model geometry is not loaded: {id}")))?;
        if loaded != record {
            return fail("3D model source changed; reload geometry");
        }
        model_bounds(model, time)
    }
    fn source_fit(&self, id: &str, record: &Value) -> Result<gltf::Fit> {
        let (loaded, model) = self
            .models
            .get(id)
            .ok_or_else(|| Error(format!("3D model geometry is not loaded: {id}")))?;
        if loaded != record {
            return fail("3D model source changed; reload geometry");
        }
        source_fit(model)
    }
    fn pick_model_part(
        &self,
        id: &str,
        record: &Value,
        world: math::Mat4,
        time: f64,
        parts: &Value,
        origin: Vec3,
        direction: Vec3,
    ) -> Result<Option<PartPlacement>> {
        let model = self.loaded_model(id, record)?;
        let crate::import::Model::Gltf(model) = model else {
            return Ok(None);
        };
        let picker = gltf::pick::Picker::new(std::sync::Arc::new((**model).clone()));
        Ok(pick_model_part(
            &picker, world, time, parts, origin, direction,
        ))
    }
    fn model_part(
        &self,
        id: &str,
        record: &Value,
        world: math::Mat4,
        time: f64,
        parts: &Value,
        key: &str,
    ) -> Result<Option<PartPlacement>> {
        let crate::import::Model::Gltf(model) = self.loaded_model(id, record)? else {
            return Ok(None);
        };
        Ok(model_part(model, world, time, parts, key))
    }
    fn part_at(
        &self,
        id: &str,
        record: &Value,
        world: [f64; 16],
        time: f64,
        origin: Vec3,
        direction: Vec3,
    ) -> Result<Option<PartHit>> {
        let (loaded, model) = self
            .models
            .get(id)
            .ok_or_else(|| Error(format!("3D model geometry is not loaded: {id}")))?;
        if loaded != record {
            return fail("3D model source changed; reload geometry");
        }
        let crate::import::Model::Gltf(m) = &**model else {
            return Ok(None);
        };
        let picker = gltf::pick::Picker::new(std::sync::Arc::new((**m).clone()));
        part_hit(&picker, world, time, origin, direction)
    }
}
/// The first part a world ray meets on a posed, placed glTF model.
pub fn part_hit(
    picker: &gltf::pick::Picker,
    world: [f64; 16],
    time: f64,
    origin: Vec3,
    direction: Vec3,
) -> Result<Option<PartHit>> {
    if !time.is_finite()
        || !world
            .iter()
            .chain(&origin)
            .chain(&direction)
            .all(|v| v.is_finite())
        || length(direction) < 1e-12
    {
        return fail("Invalid part pick");
    }
    let posed = picker.pose(&world, time);
    let ray = gltf::pick::Ray {
        origin,
        direction: normalize(direction),
    };
    let model = &picker.model;
    let Some(hit) = picker.pick(
        &posed,
        &ray,
        0.,
        f64::INFINITY,
        &|_| gltf::pick::Side::Double,
        &|_| false,
    ) else {
        return Ok(None);
    };
    let gltf::ObjectKind::Drawable(d) = &model.objects[hit.object].kind else {
        return Ok(None);
    };
    Ok(model
        .material_sources
        .get(d.material)
        .copied()
        .flatten()
        .map(|slot| PartHit {
            slot,
            name: model
                .materials
                .get(d.material)
                .map(|m| m.name.trim().to_owned())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| format!("Material {}", slot + 1)),
            point: hit.point,
            distance: hit.distance,
        }))
}
/// Snapshot-approved originals, resolved by the same media host as exports.
/// Decode with Studio's shared Resource loader, independent of editor views.
/// Retained boxes cannot stand in for the current pose or missing originals.
pub struct FileAssets {
    paths: HashMap<String, std::path::PathBuf>,
    loaded: std::sync::Mutex<HashMap<String, (Value, std::sync::Arc<crate::render::Resource>)>>,
}
impl FileAssets {
    pub fn new(paths: HashMap<String, std::path::PathBuf>) -> Self {
        Self {
            paths,
            loaded: Default::default(),
        }
    }
    fn resource(
        &self,
        id: &str,
        record: &Value,
    ) -> Result<std::sync::Arc<crate::render::Resource>> {
        let mut loaded = self
            .loaded
            .lock()
            .map_err(|_| Error("Model geometry cache poisoned".into()))?;
        if let Some((saved, resource)) = loaded.get(id) {
            if saved != record {
                return fail("3D model source changed; reload geometry");
            }
            return Ok(resource.clone());
        }
        let path = self
            .paths
            .get(id)
            .ok_or_else(|| Error(format!("3D model original is offline: {id}")))?;
        use std::io::Read;
        let mut bytes = vec![];
        std::fs::File::open(path)
            .map_err(|e| Error(format!("3D model resource: {e}")))?
            .take(crate::import::MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| Error(e.to_string()))?;
        // Owned media caches need not have an extension. As in the renderer,
        // saved format/name selects only the decoder, never the byte source.
        let name = record["format"]
            .as_str()
            .map(|format| format!("resource.{format}"))
            .or_else(|| record["name"].as_str().map(str::to_owned))
            .or_else(|| path.to_str().map(str::to_owned))
            .ok_or_else(|| Error("Invalid model resource path".into()))?;
        let resource = crate::render::Resource::load(&name, &bytes)?;
        loaded.insert(id.into(), (record.clone(), resource.clone()));
        Ok(resource)
    }
}
impl AssetGeometry for FileAssets {
    fn bounds(&self, id: &str, record: &Value, time: f64) -> Result<Bounds> {
        model_bounds(&self.resource(id, record)?.model, time)
    }
    fn source_fit(&self, id: &str, record: &Value) -> Result<gltf::Fit> {
        source_fit(&self.resource(id, record)?.model)
    }
    fn pick_model_part(
        &self,
        id: &str,
        record: &Value,
        world: math::Mat4,
        time: f64,
        parts: &Value,
        origin: Vec3,
        direction: Vec3,
    ) -> Result<Option<PartPlacement>> {
        Ok(self
            .resource(id, record)?
            .picker()
            .and_then(|picker| pick_model_part(&picker, world, time, parts, origin, direction)))
    }
    fn model_part(
        &self,
        id: &str,
        record: &Value,
        world: math::Mat4,
        time: f64,
        parts: &Value,
        key: &str,
    ) -> Result<Option<PartPlacement>> {
        let resource = self.resource(id, record)?;
        let crate::import::Model::Gltf(model) = &resource.model else {
            return Ok(None);
        };
        Ok(model_part(model, world, time, parts, key))
    }
    fn part_at(
        &self,
        id: &str,
        record: &Value,
        world: [f64; 16],
        time: f64,
        origin: Vec3,
        direction: Vec3,
    ) -> Result<Option<PartHit>> {
        match self.resource(id, record)?.picker() {
            Some(picker) => part_hit(&picker, world, time, origin, direction),
            None => Ok(None),
        }
    }
}
fn placed_parts(
    model: &gltf::Model,
    layer: &math::Mat4,
    time: f64,
    parts: &pose::PartTransforms,
) -> (pose::Pose, Vec<math::Mat4>, Vec<bool>) {
    let mut sample = pose::Pose::at(model, time);
    let visible = pose::apply_parts(model, &mut sample, parts);
    let (parent, placed) = pose::placed(model, sample.clone(), layer);
    (sample, pose::world(model, &placed, Some(&parent)), visible)
}

/// Bake one overridden sample when a skeleton needs the new bone and bind
/// matrices. Geometry and texture buffers remain shared with the source.
fn baked_model(model: &gltf::Model, sample: &pose::Pose) -> gltf::Model {
    let mut model = model.clone();
    model.clips.clear();
    for (i, object) in model.objects.iter_mut().enumerate() {
        object.position = sample.position[i];
        object.quaternion = sample.quaternion[i];
        object.scale = sample.scale[i];
        if let gltf::ObjectKind::Drawable(d) = &mut object.kind {
            d.morph_influences.clone_from(&sample.influences[i]);
        }
    }
    model
}

fn model_part(
    model: &gltf::Model,
    layer: math::Mat4,
    time: f64,
    parts: &Value,
    key: &str,
) -> Option<PartPlacement> {
    let index = pose::part_index(model, key)?;
    let part = part_info(model, index)?;
    let transforms = pose::part_transforms(parts);
    let (sample, worlds, _) = placed_parts(model, &layer, time, &transforms);
    let mut stack = vec![index];
    let mut bounds: Option<Bounds> = None;
    while let Some(node) = stack.pop() {
        stack.extend(model.objects[node].children.iter().copied());
        let gltf::ObjectKind::Drawable(d) = &model.objects[node].kind else { continue };
        let Some((min, max)) = pose::local_box(model, node, d, &sample, &worlds) else { continue };
        let Ok(next) = transform(Bounds { min, max }, worlds[node]) else { continue };
        bounds = Some(bounds.map_or(next, |prior| Bounds {
            min: std::array::from_fn(|i| prior.min[i].min(next.min[i])),
            max: std::array::from_fn(|i| prior.max[i].max(next.max[i])),
        }));
    }
    Some(PartPlacement {
        transform: transforms.get(key).copied().unwrap_or_default(),
        part,
        pivot: math::apply(&worlds[index], [0.; 3]),
        bounds,
        layer_world: layer,
    })
}

fn pick_model_part(
    picker: &gltf::pick::Picker,
    layer: math::Mat4,
    time: f64,
    parts: &Value,
    origin: Vec3,
    direction: Vec3,
) -> Option<PartPlacement> {
    let transforms = pose::part_transforms(parts);
    let model = &picker.model;
    let (sample, worlds, visible) = placed_parts(model, &layer, time, &transforms);
    let baked = (!transforms.is_empty() && !model.skins.is_empty())
        .then(|| gltf::pick::Picker::new(std::sync::Arc::new(baked_model(model, &sample))));
    let picker = baked.as_ref().unwrap_or(picker);
    let mut posed = picker.pose(&layer, if baked.is_some() { 0. } else { time });
    posed.world = worlds;
    let hit = picker.pick(
        &posed,
        &gltf::pick::Ray {
            origin,
            direction: normalize(direction),
        },
        0.,
        f64::INFINITY,
        &|_| gltf::pick::Side::Double,
        &|index| !visible[index],
    )?;
    let part = part_info(model, hit.object)?;
    Some(PartPlacement {
        transform: transforms.get(&part.key).copied().unwrap_or_default(),
        part,
        pivot: math::apply(&posed.world[hit.object], [0.; 3]),
        bounds: None,
        layer_world: layer,
    })
}

impl crate::render::Resource {
    /// The native Studio surfaces at one time, with evaluated layer overrides.
    pub fn parts_with_overrides(
        &self,
        time: f64,
        parts: &Value,
    ) -> Result<Vec<crate::render::Part>> {
        let transforms = pose::part_transforms(parts);
        let crate::import::Model::Gltf(model) = &self.model else {
            return self.parts(time);
        };
        if transforms.is_empty() || !time.is_finite() {
            return self.parts(time);
        }
        let (sample, worlds, visible) = placed_parts(model, &math::IDENTITY, time, &transforms);
        let surfaces = if model.skins.is_empty() {
            self.parts(time)?
        } else {
            let baked = crate::import::Model::Gltf(Box::new(baked_model(model, &sample)));
            Self::new(baked).parts(0.)?
        };
        let mut placements = vec![];
        for i in model.traversal() {
            let gltf::ObjectKind::Drawable(d) = &model.objects[i].kind else {
                continue;
            };
            if let Some(instances) = &d.instances {
                placements.extend(instances.matrices.iter().map(|matrix| {
                    (
                        visible[i],
                        math::multiply(&worlds[i], &matrix.map(f64::from)),
                    )
                }));
            } else {
                placements.push((visible[i], worlds[i]));
            }
        }
        if surfaces.len() != placements.len() {
            return fail("Model surface placement changed");
        }
        Ok(surfaces
            .into_iter()
            .zip(placements)
            .filter_map(|(mut part, (visible, world))| {
                part.world = world;
                visible.then_some(part)
            })
            .collect())
    }
}

fn source_fit(model: &crate::import::Model) -> Result<gltf::Fit> {
    let crate::import::Model::Gltf(model) = model else {
        return fail("Real-world import scale requires glTF/GLB geometry in metres");
    };
    let fit = model.fit;
    if !fit.radius.is_finite() || fit.radius <= 0. || !fit.center.iter().all(|v| v.is_finite()) {
        return fail("Invalid original model scale");
    }
    Ok(fit)
}
fn model_bounds(model: &crate::import::Model, time: f64) -> Result<Bounds> {
    if !time.is_finite() {
        return fail("Invalid model sample time");
    }
    let b = match model {
        crate::import::Model::Gltf(m) => {
            let pose = gltf::pose::Pose::at(m, time);
            let (min, max) = gltf::pose::scene_box(m, &pose)
                .ok_or_else(|| Error("Model has no geometry".into()))?;
            Bounds {
                min: std::array::from_fn(|i| (min[i] - m.fit.center[i]) / m.fit.radius),
                max: std::array::from_fn(|i| (max[i] - m.fit.center[i]) / m.fit.radius),
            }
        }
        crate::import::Model::Obj(m) => points(
            m.positions
                .chunks_exact(3)
                .map(|p| [p[0] as f64, p[1] as f64, p[2] as f64]),
        )?,
        crate::import::Model::Splats(d) => {
            let (center, r) = d.normalization()?;
            let core = d.dense_core()?;
            Bounds {
                min: [
                    (core.min[0] - center[0]) / r,
                    -(core.max[1] - center[1]) / r,
                    -(core.max[2] - center[2]) / r,
                ],
                max: [
                    (core.max[0] - center[0]) / r,
                    -(core.min[1] - center[1]) / r,
                    -(core.min[2] - center[2]) / r,
                ],
            }
        }
    };
    b.validate()?;
    Ok(b)
}
fn points(points: impl Iterator<Item = Vec3>) -> Result<Bounds> {
    let mut b = Bounds {
        min: [f64::INFINITY; 3],
        max: [f64::NEG_INFINITY; 3],
    };
    for p in points {
        for (i, v) in p.into_iter().enumerate() {
            b.min[i] = b.min[i].min(v);
            b.max[i] = b.max[i].max(v);
        }
    }
    b.validate()?;
    Ok(b)
}
pub fn local_bounds(
    source: &Value,
    assets: &dyn AssetGeometry,
    records: &Value,
    time: f64,
) -> Result<Bounds> {
    schema::source(source)?;
    if let Some(id) = source["assetId"].as_str() {
        return assets.bounds(id, &records[id], time);
    }
    let n = |k: &str| source["parameters"][k].as_f64();
    if source["primitive"] == "roundedBox" {
        let size = [
            n("width").unwrap_or(1.),
            n("height").unwrap_or(1.),
            n("depth").unwrap_or(1.),
        ];
        return Ok(Bounds {
            min: size.map(|x| -x / 2.),
            max: size.map(|x| x / 2.),
        });
    }
    let source = if let Some(name) = source["primitive"].as_str() {
        Source::primitive(
            name,
            &Parameters {
                width: n("width"),
                height: n("height"),
                depth: n("depth"),
                radius: n("radius"),
                radius_top: n("radiusTop"),
                radius_bottom: n("radiusBottom"),
                tube: n("tube"),
                segments: n("segments"),
                detail: n("detail"),
            },
        )
        .ok_or_else(|| Error("Unknown primitive".into()))?
    } else if source.get("mesh").is_some() {
        let m = &source["mesh"];
        let floats = |k: &str| {
            m[k].as_array()
                .map(|a| a.iter().map(|x| x.as_f64().unwrap()).collect())
        };
        Source::Mesh(MeshSource {
            positions: floats("positions").unwrap(),
            indices: m["indices"]
                .as_array()
                .map(|a| a.iter().map(|x| x.as_f64().unwrap() as u32).collect()),
            normals: floats("normals"),
            uvs: floats("uvs"),
            groups: vec![],
        })
    } else {
        let key = if source.get("lathe").is_some() {
            "lathe"
        } else {
            "extrude"
        };
        let points = source[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| [p[0].as_f64().unwrap(), p[1].as_f64().unwrap()])
            .collect();
        if key == "lathe" {
            Source::Lathe {
                points,
                segments: source["segments"].as_f64().unwrap_or(48.),
            }
        } else {
            Source::Extrude {
                points,
                depth: source["depth"].as_f64().unwrap(),
                bevel: source["bevel"].as_f64().unwrap_or(0.),
            }
        }
    };
    let g = source.geometry();
    points(
        g.positions
            .chunks_exact(3)
            .map(|p| [p[0] as f64, p[1] as f64, p[2] as f64]),
    )
}
pub fn transform(b: Bounds, m: [f64; 16]) -> Result<Bounds> {
    b.validate()?;
    if !m.iter().all(|v| v.is_finite()) {
        return fail("Invalid world transform");
    }
    points((0..8).map(|n| {
        math::apply(
            &m,
            std::array::from_fn(|i| {
                if n & (1 << i) == 0 {
                    b.min[i]
                } else {
                    b.max[i]
                }
            }),
        )
    }))
}
pub fn inverse(m: [f64; 16]) -> Result<[f64; 16]> {
    if !m.iter().all(|v| v.is_finite()) || math::determinant(&m).abs() < 1e-15 {
        return fail("Parent transform is singular");
    }
    Ok(math::invert(&m))
}
pub fn parent_delta(comp: &Value, l: &Value, delta: Vec3) -> Result<Vec3> {
    if l["parent"].is_null() && l["group"].is_null() {
        return Ok(delta);
    }
    let id = l["id"]
        .as_str()
        .ok_or_else(|| Error("Missing layer id".into()))?;
    let m = matrix(&comp["_scene3dParentInverse"][id])?;
    let origin = math::apply(&m, [0.; 3]);
    Ok(sub(math::apply(&m, delta), origin))
}
pub fn matrix(v: &Value) -> Result<[f64; 16]> {
    let a = v
        .as_array()
        .filter(|a| a.len() == 16)
        .ok_or_else(|| Error("Evaluated parent transform unavailable".into()))?;
    let mut m = [0.; 16];
    for (i, v) in a.iter().enumerate() {
        m[i] = v
            .as_f64()
            .filter(|v| v.is_finite())
            .ok_or_else(|| Error("Invalid parent matrix".into()))?;
    }
    Ok(m)
}
pub fn needs_geometry(args: &Value) -> bool {
    matches!(
        args["operation"].as_str(),
        Some("explode" | "drop_to_floor" | "place_on" | "auto_studio")
    )
}
pub fn descendants(l: &Value, group: &str, layers: &[Value]) -> bool {
    let mut current = l;
    let mut seen = std::collections::HashSet::new();
    while let Some(id) = current["group"].as_str() {
        if id == group {
            return true;
        }
        if !seen.insert(id) {
            return false;
        }
        let Some(next) = layers.iter().find(|l| l["id"] == id) else {
            return false;
        };
        current = next;
    }
    false
}
pub fn required_ids(comp: &Value, args: &Value, selection: &[String]) -> Result<Vec<String>> {
    let ls = comp["layers"]
        .as_array()
        .ok_or_else(|| Error("No composition layers".into()))?;
    let target = args["target"]
        .as_str()
        .or(args["id"].as_str())
        .or_else(|| selection.first().map(String::as_str));
    let resolve = |name: &str| -> Result<String> {
        let mut found = ls.iter().filter(|l| l["id"] == name);
        if let Some(l) = found.next() {
            return Ok(l["id"].as_str().unwrap().into());
        }
        let matches = ls.iter().filter(|l| l["name"] == name).collect::<Vec<_>>();
        if matches.len() != 1 {
            return fail("Choose an unambiguous layer ID");
        }
        Ok(matches[0]["id"].as_str().unwrap().into())
    };
    match args["operation"].as_str() {
        Some("auto_studio") if args["target"].is_string() || args["id"].is_string() => {
            let id = resolve(args["target"].as_str().or(args["id"].as_str()).unwrap())?;
            if !ls
                .iter()
                .any(|l| l["id"] == id && schema::role(l) == Some(schema::Role::Object))
            {
                return fail("Choose a model for Studio setup");
            }
            Ok(vec![id])
        }
        Some("auto_studio") => Ok(ls
            .iter()
            .filter(|l| schema::role(l) == Some(schema::Role::Object))
            .filter_map(|l| l["id"].as_str().map(str::to_owned))
            .collect()),
        Some("explode") => {
            let group = resolve(target.ok_or_else(|| Error("Choose a model group".into()))?)?;
            Ok(ls
                .iter()
                .filter(|l| {
                    schema::role(l) == Some(schema::Role::Object) && descendants(l, &group, ls)
                })
                .filter_map(|l| l["id"].as_str().map(str::to_owned))
                .collect())
        }
        Some("place_on") => Ok(vec![
            resolve(target.ok_or_else(|| Error("Choose a model".into()))?)?,
            resolve(
                args["on_target"]
                    .as_str()
                    .ok_or_else(|| Error("Choose a model to place it on".into()))?,
            )?,
        ]),
        Some("drop_to_floor") => Ok(vec![resolve(
            target.ok_or_else(|| Error("Choose a model".into()))?,
        )?]),
        _ => Ok(vec![]),
    }
}
pub fn bounds_json(b: Bounds) -> Value {
    json!([b.min[0], b.min[1], b.min[2], b.max[0], b.max[1], b.max[2]])
}
#[cfg(test)]
mod part_tests {
    use super::*;
    /// Two quads side by side facing +z, with their own materials: a body on
    /// the left (material 0) and a lens on the right (material 1).
    fn two_parts() -> Vec<u8> {
        crate::import::quads_glb(&[
            (
                "Body",
                [
                    [-2., -1., 0.],
                    [-0.5, -1., 0.],
                    [-0.5, 1., 0.],
                    [-2., 1., 0.],
                ],
            ),
            (
                "Lens",
                [[0.5, -1., 0.], [2., -1., 0.], [2., 1., 0.], [0.5, 1., 0.]],
            ),
        ])
    }
    fn assets() -> (LoadedAssets, Value, gltf::Fit) {
        let resource = crate::render::Resource::load("Camera.glb", &two_parts()).unwrap();
        let crate::import::Model::Gltf(m) = &resource.model else {
            panic!("glTF")
        };
        let fit = m.fit;
        let record = json!({"kind":"model","name":"Camera.glb"});
        let mut assets = LoadedAssets::default();
        assets.insert(
            "a".into(),
            record.clone(),
            crate::import::Model::Gltf(m.clone()),
        );
        (assets, record, fit)
    }
    #[test]
    fn a_ray_names_the_material_slot_of_the_part_it_meets_first() {
        let (assets, record, fit) = assets();
        // The renderer's placement: the fit's centre and radius normalize it.
        let x = |source: f64| (source - fit.center[0]) / fit.radius;
        let world = math::IDENTITY;
        let pick = |at: f64| {
            assets
                .part_at("a", &record, world, 0., [x(at), 0., 5.], [0., 0., -1.])
                .unwrap()
        };
        let body = pick(-1.).expect("hits the body");
        assert_eq!((body.slot, body.name.as_str()), (0, "Body"));
        assert!((body.distance - 5.).abs() < 1e-6, "{body:?}");
        assert_eq!(pick(1.2).expect("hits the lens").slot, 1);
        // The gap between the parts, and beyond them, is no part.
        assert_eq!(pick(0.), None);
        assert_eq!(pick(5.), None);
        // Seen from behind (double-sided) it is still the lens.
        let back = assets
            .part_at("a", &record, world, 0., [x(1.2), 0., -5.], [0., 0., 1.])
            .unwrap();
        assert_eq!(back.map(|h| h.slot), Some(1));
    }
    #[test]
    fn saved_node_offsets_and_visibility_match_native_surfaces_and_picking() {
        let resource = crate::render::Resource::load("Camera.glb", &two_parts()).unwrap();
        let crate::import::Model::Gltf(model) = &resource.model else {
            panic!("glTF")
        };
        let baseline = resource.parts(0.).unwrap();
        let picker = resource.picker().unwrap();
        let x = |source: f64| (source - model.fit.center[0]) / model.fit.radius;
        let pick = |at, parts: &Value| {
            pick_model_part(
                &picker,
                math::IDENTITY,
                0.,
                parts,
                [at, 0., 5.],
                [0., 0., -1.],
            )
        };
        let body = pick(x(-1.), &Value::Null).unwrap();
        let lens = pick(x(1.), &Value::Null).unwrap();
        assert_ne!(body.part.key, lens.part.key);
        let edits = json!({
            (&body.part.key): {"name":"Body","p":{"x":{"v":-3.},"rz":{"v":0.}}},
            (&lens.part.key): {"name":"Lens","p":{"visible":false}}
        });
        let surfaces = resource.parts_with_overrides(0., &edits).unwrap();
        assert_eq!(surfaces.len(), 1);
        assert_eq!(surfaces[0].source_material, Some(0));
        assert!((surfaces[0].world[12] - baseline[0].world[12] + 3.).abs() < 1e-9);
        assert!(pick(x(-1.), &edits).is_none());
        assert!(pick(x(1.), &edits).is_none());
        assert_eq!(pick(x(-1.) - 3., &edits).unwrap().part.key, body.part.key);
        assert!(
            resource
                .parts_with_overrides(0., &json!({"0":{"p":{"visible":false}}}))
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn the_layer_matrix_places_the_model_it_picks() {
        let (assets, record, fit) = assets();
        let x = |source: f64| (source - fit.center[0]) / fit.radius;
        // Moved 10 units along x: the old place is empty, the new one hits.
        let mut world = math::IDENTITY;
        world[12] = 10.;
        let at = |dx: f64| {
            assets
                .part_at(
                    "a",
                    &record,
                    world,
                    0.,
                    [x(1.2) + dx, 0., 5.],
                    [0., 0., -1.],
                )
                .unwrap()
                .map(|h| h.slot)
        };
        assert_eq!(at(0.), None);
        assert_eq!(at(10.), Some(1));
        // A changed source asks for a reload rather than answering stale.
        assert!(
            assets
                .part_at(
                    "a",
                    &json!({"kind":"model"}),
                    world,
                    0.,
                    [0.; 3],
                    [0., 0., -1.]
                )
                .is_err()
        );
    }

    #[test]
    fn clean_part_names_strips_exporter_prefix_and_disambiguates_duplicates() {
        let mk = |key: &str, raw: &str| ModelPart {
            key: key.into(),
            name: raw.into(),
            raw_name: raw.into(),
            parent: None,
            depth: 0,
            material: None,
            material_name: None,
            swatch: None,
        };
        let mut parts = vec![
            mk("0", "flow100_obj_JoyConL_00__Mt_JoyconL_AL80"),
            mk("1", "flow100_obj_JoyConL_00__Mt_JoyconL_BLU"),
            mk("2", "flow100_obj_JoyConL_00__Mt_JoyconL_GLSMi"),
            mk("3", "flow100_obj_JoyConL_00__Mt_JoyconL_GLSMi"),
            mk("4", "flow100_obj_JoyConL_00__Mt_JoyconL_GLSMi"),
            mk("5", "flow100_obj_JoyConL_00__lamp_mt"),
        ];
        clean_part_names(&mut parts);
        let names: Vec<&str> = parts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["AL80", "BLU", "GLSMi #1", "GLSMi #2", "GLSMi #3", "lamp_mt"]
        );
        // Original raw glTF names stay intact for search aliases & tooltips.
        assert_eq!(parts[2].raw_name, "flow100_obj_JoyConL_00__Mt_JoyconL_GLSMi");
    }
}
