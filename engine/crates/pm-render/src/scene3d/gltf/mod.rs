//! Imported glTF and GLB models in the 3D scene pass (plan section 8
//! "Materials" and "Animation", section 13, docs/engine/gltf.md).
//!
//! A model object is today's runtime clone of the importer's root
//! (`cloneSkinned(asset.object3d)` under the layer's world matrix, the
//! fit's scale, and the scene moved by −center), posed by the mixer rule
//! at the layer's local time, with `SceneRuntime.sync`'s material rule:
//! every mesh (not points or lines) takes the object's material unless it
//! keeps its source materials; a source `MeshStandardMaterial` or
//! `MeshPhysicalMaterial` is reused and keeps everything `sync` does not
//! set (its extensions, vertex colors, flat shading, alpha test, bump
//! map), an unlit one is replaced by a new `MeshStandardMaterial`; slots
//! make the material an array, which draws nothing on a mesh without
//! groups. Embedded punctual lights light every depth set and count toward
//! neither the 16-light nor the 4-shadow limit.

pub mod asset;
pub(crate) mod gpu;
pub mod material;

use std::sync::Arc;

use pm_geom3d::gltf::{self as g, Mode, ObjectKind, pose};

use super::model::{
    Geometry3d, GeometryPayload, Light3d, LightKind, Material3d, MaterialKind, Materials, Mesh3d,
    Texture3d,
};
use crate::three::materials::Side;
use crate::three::math::{self, Mat4};
use crate::three::program::Parameters;

pub use asset::{DecodedImage, GltfAsset};

/// How a model asset's file resolves: loaded, or missing (offline,
/// unreadable, or a file the reader refuses). The reader accepts exactly
/// the files today's importer loads (`gltf-acceptance.test.ts`), so a
/// refused file is one whose asset has no scene today either: the
/// missing-model box.
pub fn lookup(path: &std::path::Path) -> super::model::ModelLookup {
    use super::model::ModelLookup;
    if std::fs::metadata(path).is_err() {
        asset::forget(path);
        return ModelLookup::Missing;
    }
    match asset::load(path) {
        Ok(a) => ModelLookup::Gltf(a),
        Err(_) => ModelLookup::Missing,
    }
}

/// A glTF texture a material samples: the decoded image, its sampler,
/// color space, UV channel, and `texture.matrix`.
#[derive(Debug, Clone, PartialEq)]
pub struct GltfMap {
    pub image: Arc<DecodedImage>,
    pub sampler: g::Sampler,
    pub srgb: bool,
    pub channel: u32,
    pub transform: [f64; 9],
}

/// A map slot's texture: the object's own map (a scene texture asset,
/// `new THREE.Texture(bitmap)` on UV channel 0) or one from the file.
#[derive(Debug, Clone, PartialEq)]
pub enum MapSource {
    Scene(Texture3d),
    Gltf(GltfMap),
}

impl MapSource {
    pub fn channel(&self) -> u32 {
        match self {
            MapSource::Scene(_) => 0,
            MapSource::Gltf(m) => m.channel,
        }
    }

    pub fn transform(&self) -> [f64; 9] {
        match self {
            MapSource::Scene(t) => t.transform,
            MapSource::Gltf(m) => m.transform,
        }
    }
}

/// What a glTF mesh's material holds beyond [`Material3d`]'s shared
/// fields (which the editor looks edit): the three.js material state
/// (`source`: class, physical parameters, vertex colors, flat shading,
/// alpha test, scales) and the file's maps by uniform name.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialExt {
    pub source: g::Material,
    pub maps: Vec<(&'static str, GltfMap)>,
    /// `material.id` order (painter sort of opaque items).
    pub order: u64,
}

impl MaterialExt {
    pub fn map(&self, name: &str) -> Option<&GltfMap> {
        self.maps.iter().find(|(n, _)| *n == name).map(|(_, m)| m)
    }
}

/// `geometry.morphAttributes` as `WebGLMorphtargets` packs them: a
/// `DataArrayTexture` of `width × height` texels per target (RGBA f32).
#[derive(PartialEq)]
pub struct MorphTexture {
    pub key: u64,
    pub width: u32,
    pub height: u32,
    pub layers: u32,
    pub data: Arc<Vec<f32>>,
}

/// Attributes of a glTF geometry beyond position, normal, and `uv`
/// (named by the geometry's key, which the debug form and content keys
/// carry instead of the data).
#[derive(PartialEq)]
pub struct VertexExtra {
    pub tangent: Option<Arc<Vec<f32>>>,
    /// `color` and its item size (3 or 4).
    pub color: Option<(Arc<Vec<f32>>, usize)>,
    pub uv1: Option<Arc<Vec<f32>>>,
    pub uv2: Option<Arc<Vec<f32>>>,
    pub uv3: Option<Arc<Vec<f32>>>,
    pub skin_index: Option<Arc<Vec<f32>>>,
    pub skin_weight: Option<Arc<Vec<f32>>>,
    pub mode: Mode,
    pub has_normal: bool,
    pub has_uv: bool,
    /// Source counts before CPU-safe padding; checked against the final
    /// material after late scene-map failures have been resolved.
    pub short_attributes: Vec<(&'static str, usize)>,
    pub fetched_vertices: usize,
    /// Morph targets: position, normal, color present; count; texture.
    pub morph: Option<(bool, bool, bool, u32, Arc<MorphTexture>)>,
}

/// Skinning state of a `SkinnedMesh` draw.
#[derive(Clone, PartialEq)]
pub struct Skinning {
    pub bind_matrix: Mat4,
    pub bind_matrix_inverse: Mat4,
    /// `boneTexture`: side and RGBA f32 texels.
    pub size: u32,
    pub bones: Arc<Vec<f32>>,
    pub key: u64,
}

/// What a glTF drawable adds to [`Mesh3d`].
#[derive(Clone, PartialEq)]
pub struct MeshExt {
    pub mode: Mode,
    pub skinning: Option<Skinning>,
    pub instances: Option<(u64, Arc<g::Instances>)>,
    pub influences: Vec<f64>,
    /// Object-level bounding sphere (instanced meshes); `None` for skinned
    /// meshes (never culled: three.js keeps the first pose's sphere).
    pub sphere: Option<([f64; 3], f64)>,
    pub skinned: bool,
    /// `object.id` order.
    pub id: u64,
}

// Debug forms name large data by its content key (content keys hash the
// debug form of a scene; `Geometry3d` does the same).
impl std::fmt::Debug for MorphTexture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MorphTexture")
            .field("key", &format_args!("{:016x}", self.key))
            .field("size", &(self.width, self.height, self.layers))
            .finish()
    }
}

impl std::fmt::Debug for VertexExtra {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VertexExtra")
            .field("mode", &self.mode)
            .field("tangent", &self.tangent.is_some())
            .field("color", &self.color.as_ref().map(|c| c.1))
            .field(
                "uv",
                &(
                    self.has_uv,
                    self.uv1.is_some(),
                    self.uv2.is_some(),
                    self.uv3.is_some(),
                ),
            )
            .field("normal", &self.has_normal)
            .field("short_attributes", &self.short_attributes)
            .field("fetched_vertices", &self.fetched_vertices)
            .field("skin", &self.skin_index.is_some())
            .field("morph", &self.morph)
            .finish()
    }
}

impl std::fmt::Debug for Skinning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Skinning")
            .field("bind_matrix", &self.bind_matrix)
            .field("bind_matrix_inverse", &self.bind_matrix_inverse)
            .field("bones", &format_args!("{:016x} ({}²)", self.key, self.size))
            .finish()
    }
}

impl std::fmt::Debug for MeshExt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MeshExt")
            .field("mode", &self.mode)
            .field("skinning", &self.skinning)
            .field(
                "instances",
                &self.instances.as_ref().map(|(k, i)| {
                    format!(
                        "{k:016x} ×{} color {}",
                        i.matrices.len(),
                        i.colors.is_some()
                    )
                }),
            )
            .field("influences", &self.influences)
            .field("sphere", &self.sphere)
            .field("skinned", &self.skinned)
            .field("id", &self.id)
            .finish()
    }
}

fn key(parts: &[&[u8]]) -> u64 {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for p in parts {
        h.update((p.len() as u64).to_le_bytes());
        h.update(p);
    }
    u64::from_le_bytes(h.finalize()[..8].try_into().unwrap())
}

fn f32_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// WebGL fetches an active vertex attribute only for indices in the draw.
/// A short attribute's unused tail does not invalidate an indexed primitive.
fn fetched_vertex_count(geometry: &g::Geometry) -> usize {
    geometry.index.as_ref().map_or_else(
        || geometry.vertex_count(),
        |indices| {
            indices
                .iter()
                .copied()
                .max()
                .map_or(0, |i| (i as usize).saturating_add(1))
        },
    )
}

/// `WebGLMorphtargets`' texture (max texture size 16,384, as ANGLE on
/// Metal reports).
fn morph_texture(
    asset_key: u64,
    index: usize,
    geometry: &g::Geometry,
) -> Option<Arc<MorphTexture>> {
    let m = &geometry.morph;
    let count = m.count();
    if count == 0 {
        return None;
    }
    let stride = m.stride() as usize;
    let vertices = geometry.vertex_count();
    let mut width = vertices * stride;
    let mut height = 1usize;
    const MAX: usize = 16384;
    if width > MAX {
        height = width.div_ceil(MAX);
        width = MAX;
    }
    let layer = width * height * 4;
    let mut data = vec![0f32; layer * count];
    for i in 0..count {
        let offset = layer * i;
        let primary = if m.position.is_some() {
            0
        } else if m.normal.is_some() {
            1
        } else {
            2
        };
        let source_count = m.source_counts[primary].get(i).copied().unwrap_or(vertices);
        // WebGLMorphtargets iterates the primary target's count. The
        // Float32Array starts at zero, so a short primary target leaves
        // the remaining texels zero. A secondary target shorter than the
        // primary yields undefined Vector4 components, stored as NaN.
        for j in 0..source_count.min(vertices) {
            let s = j * stride;
            if let Some(p) = m.position.as_ref().and_then(|p| p.get(i)) {
                let at = offset + s * 4;
                let short = j >= m.source_counts[0].get(i).copied().unwrap_or(vertices);
                data[at] = if short { f32::NAN } else { p.data[j * 3] };
                data[at + 1] = if short { f32::NAN } else { p.data[j * 3 + 1] };
                data[at + 2] = if short { f32::NAN } else { p.data[j * 3 + 2] };
                data[at + 3] = 0.0;
            }
            if let Some(n) = m.normal.as_ref().and_then(|n| n.get(i)) {
                let at = offset + s * 4 + 4;
                let short = j >= m.source_counts[1].get(i).copied().unwrap_or(vertices);
                data[at] = if short { f32::NAN } else { n.data[j * 3] };
                data[at + 1] = if short { f32::NAN } else { n.data[j * 3 + 1] };
                data[at + 2] = if short { f32::NAN } else { n.data[j * 3 + 2] };
                data[at + 3] = 0.0;
            }
            if let Some(c) = m.color.as_ref().and_then(|c| c.get(i)) {
                let at = offset + s * 4 + 8;
                let size = c.size;
                let short = j >= m.source_counts[2].get(i).copied().unwrap_or(vertices);
                data[at] = if short { f32::NAN } else { c.data[j * size] };
                data[at + 1] = if short {
                    f32::NAN
                } else {
                    c.data[j * size + 1]
                };
                data[at + 2] = if short {
                    f32::NAN
                } else {
                    c.data[j * size + 2]
                };
                data[at + 3] = if size == 4 {
                    if short {
                        f32::NAN
                    } else {
                        c.data[j * size + 3]
                    }
                } else {
                    1.0
                };
            }
        }
    }
    Some(Arc::new(MorphTexture {
        key: key(&[
            b"gltf morph 1",
            &asset_key.to_le_bytes(),
            &(index as u64).to_le_bytes(),
        ]),
        width: width as u32,
        height: height as u32,
        layers: count as u32,
        data: Arc::new(data),
    }))
}

/// The renderer's geometry for glTF geometry `index` of `asset`.
pub fn geometry(asset: &GltfAsset, index: usize) -> Geometry3d {
    let mut cache = asset.geometry_cache.lock().unwrap();
    if let Some(found) = cache.get(&index) {
        return found.clone();
    }
    let gm = &asset.model.geometries[index];
    let n = gm.vertex_count();
    let generated_indices = match (&gm.index, gm.mode) {
        // `LineLoop` draws as segments including the closing one.
        (ix, Mode::LineLoop) => {
            let len = ix.as_ref().map_or(n, |ix| ix.len());
            let at = |k: usize| ix.as_ref().map_or(k as u32, |ix| ix[k]);
            let mut out = Vec::with_capacity(len * 2);
            for k in 0..len {
                out.push(at(k));
                out.push(at((k + 1) % len));
            }
            Some(Arc::new(out))
        }
        (Some(_), _) => None,
        (None, _) => Some(Arc::new((0..n as u32).collect())),
    };
    let (center, radius) = gm.bounding_sphere();
    let morph = morph_texture(asset.key, index, gm).map(|t| {
        (
            gm.morph.position.is_some(),
            gm.morph.normal.is_some(),
            gm.morph.color.is_some(),
            gm.morph.count() as u32,
            t,
        )
    });
    let extra = VertexExtra {
        tangent: gm.tangent.as_ref().map(|a| a.data.clone()),
        color: gm.color.as_ref().map(|a| (a.data.clone(), a.size)),
        uv1: gm.uv[1].as_ref().map(|a| a.data.clone()),
        uv2: gm.uv[2].as_ref().map(|a| a.data.clone()),
        uv3: gm.uv[3].as_ref().map(|a| a.data.clone()),
        skin_index: gm.skin_index.as_ref().map(|a| a.data.clone()),
        skin_weight: gm.skin_weight.as_ref().map(|a| a.data.clone()),
        mode: gm.mode,
        has_normal: gm.normal.is_some(),
        has_uv: gm.uv[0].is_some(),
        short_attributes: gm.short_attributes.clone(),
        fetched_vertices: fetched_vertex_count(gm),
        morph,
    };
    let g3 = Geometry3d {
        key: key(&[
            b"gltf geometry 1",
            &asset.key.to_le_bytes(),
            &(index as u64).to_le_bytes(),
        ]),
        geometry: GeometryPayload::Gltf {
            source: Arc::new(gm.clone()),
            generated_indices,
        },
        sphere: (center, radius),
        extra: Some(Arc::new(extra)),
    };
    cache.insert(index, g3.clone());
    g3
}

/// How an object's meshes take materials (`sync`).
pub enum Override<'a> {
    /// `useSourceMaterials` without slots: the file's materials.
    Source,
    /// The object's material (built as `sync` builds it).
    Single(&'a Material3d),
    /// Slots: an array of materials (a glTF mesh has no groups, so it draws
    /// nothing).
    Slots(&'a [Material3d]),
}

/// The file's maps of a material that loaded, by uniform name.
fn gltf_maps(asset: &GltfAsset, m: &g::Material) -> Vec<(&'static str, GltfMap)> {
    let mut out = Vec::new();
    for (name, r) in m.maps() {
        let Some(t) = asset.model.textures.get(r.texture) else {
            continue;
        };
        let Some(Some(image)) = asset.images.get(t.image) else {
            continue;
        };
        let srgb = matches!(
            name,
            "map" | "emissiveMap" | "sheenColorMap" | "specularColorMap"
        );
        out.push((
            name,
            GltfMap {
                image: image.clone(),
                sampler: t.sampler,
                srgb,
                channel: r.channel,
                transform: r.transform,
            },
        ));
    }
    out
}

/// The six `sync` sets on a reused material.
const SYNC_SLOTS: [&str; 6] = [
    "map",
    "normalMap",
    "roughnessMap",
    "metalnessMap",
    "emissiveMap",
    "aoMap",
];

/// A loaded material as three.js holds it (texture maps that did not
/// load dropped; the program depends on which loaded).
pub(crate) fn source_material(asset: &GltfAsset, m: &g::Material, order: u64) -> Material3d {
    let maps = gltf_maps(asset, m);
    let mut src = m.clone();
    // A map whose texture did not load is `null`.
    let loads = |name: &str| maps.iter().any(|(n, _)| *n == name);
    let clear = |slot: &mut Option<g::MapRef>, name: &str| {
        if !loads(name) {
            *slot = None;
        }
    };
    clear(&mut src.map, "map");
    clear(&mut src.bump_map, "bumpMap");
    clear(&mut src.normal_map, "normalMap");
    clear(&mut src.emissive_map, "emissiveMap");
    clear(&mut src.ao_map, "aoMap");
    clear(&mut src.metalness_map, "metalnessMap");
    clear(&mut src.roughness_map, "roughnessMap");
    clear(&mut src.sheen_color_map, "sheenColorMap");
    clear(&mut src.sheen_roughness_map, "sheenRoughnessMap");
    clear(&mut src.clearcoat_map, "clearcoatMap");
    clear(&mut src.clearcoat_roughness_map, "clearcoatRoughnessMap");
    clear(&mut src.clearcoat_normal_map, "clearcoatNormalMap");
    clear(&mut src.iridescence_map, "iridescenceMap");
    clear(
        &mut src.iridescence_thickness_map,
        "iridescenceThicknessMap",
    );
    clear(&mut src.transmission_map, "transmissionMap");
    clear(&mut src.thickness_map, "thicknessMap");
    clear(&mut src.anisotropy_map, "anisotropyMap");
    clear(&mut src.specular_color_map, "specularColorMap");
    clear(&mut src.specular_intensity_map, "specularIntensityMap");
    let kind = match m.kind {
        g::MaterialType::Standard => MaterialKind::Standard,
        g::MaterialType::Physical => MaterialKind::Physical,
        g::MaterialType::Basic => MaterialKind::Basic,
        g::MaterialType::Points => MaterialKind::Points,
        g::MaterialType::Line => MaterialKind::Line,
    };
    Material3d {
        kind,
        name: m.name.clone(),
        color: m.color,
        roughness: m.roughness,
        metalness: m.metalness,
        emissive: m.emissive,
        emissive_intensity: m.emissive_intensity,
        opacity: m.opacity,
        transparent: m.transparent,
        depth_write: m.depth_write,
        side: if m.double_sided {
            Side::Double
        } else {
            Side::Front
        },
        wireframe: false,
        maps: Default::default(),
        ext: Some(Arc::new(MaterialExt {
            source: src,
            maps,
            order,
        })),
    }
}

/// `sync`'s material for a mesh whose loaded material is `loaded`: the
/// object's `definition` applied to the reused material (standard and
/// physical) or to a new `MeshStandardMaterial` (unlit).
fn synced(loaded: &Material3d, definition: &Material3d, order: u64) -> Material3d {
    let ext = loaded.ext.as_ref().expect("glTF material");
    let (mut source, mut maps, order) = match loaded.kind {
        MaterialKind::Standard | MaterialKind::Physical => {
            (ext.source.clone(), ext.maps.clone(), ext.order)
        }
        _ => (g::Material::standard(), Vec::new(), order),
    };
    maps.retain(|(n, _)| !SYNC_SLOTS.contains(n));
    source.map = None;
    source.normal_map = None;
    source.roughness_map = None;
    source.metalness_map = None;
    source.emissive_map = None;
    source.ao_map = None;
    let kind = match loaded.kind {
        MaterialKind::Physical => MaterialKind::Physical,
        _ => MaterialKind::Standard,
    };
    Material3d {
        kind,
        ext: Some(Arc::new(MaterialExt {
            source,
            maps,
            order,
        })),
        ..definition.clone()
    }
}

/// An instanced mesh's bounding sphere (`InstancedMesh.computeBoundingSphere`).
fn instances_sphere(geometry: &g::Geometry, inst: &g::Instances) -> Option<([f64; 3], f64)> {
    let (gc, gr) = geometry.bounding_sphere();
    let mut center = [0.0; 3];
    let mut radius = -1.0f64;
    let expand = |center: &mut [f64; 3], radius: &mut f64, p: [f64; 3]| {
        if *radius < 0.0 {
            *center = p;
            *radius = 0.0;
            return;
        }
        let v = [p[0] - center[0], p[1] - center[1], p[2] - center[2]];
        let l2 = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
        if l2 > *radius * *radius {
            let l = l2.sqrt();
            let delta = (l - *radius) * 0.5;
            for k in 0..3 {
                center[k] += v[k] * (delta / l);
            }
            *radius += delta;
        }
    };
    for m in &inst.matrices {
        let m: Mat4 = std::array::from_fn(|i| f64::from(m[i]));
        let c = g::math::apply(&m, gc);
        let r = gr * g::math::max_scale_on_axis(&m);
        if r < 0.0 {
            continue;
        }
        if radius < 0.0 {
            center = c;
            radius = r;
            continue;
        }
        if c == center {
            radius = radius.max(r);
            continue;
        }
        let d = [c[0] - center[0], c[1] - center[1], c[2] - center[2]];
        let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        let s = if l == 0.0 { 0.0 } else { r / l };
        let d = [d[0] * s, d[1] * s, d[2] * s];
        expand(
            &mut center,
            &mut radius,
            [c[0] + d[0], c[1] + d[1], c[2] + d[2]],
        );
        expand(
            &mut center,
            &mut radius,
            [c[0] - d[0], c[1] - d[1], c[2] - d[2]],
        );
    }
    (radius >= 0.0).then_some((center, radius))
}

/// A model object placed and posed: its drawables and embedded lights.
pub struct Instance {
    pub meshes: Vec<Mesh3d>,
    pub lights: Vec<Light3d>,
}

/// Places `asset` under the layer's `world` matrix at mixer time `time`
/// with the object's material rule. `object` orders material and object
/// ids across objects.
#[allow(clippy::too_many_arguments)]
pub fn instantiate(
    asset: &GltfAsset,
    id: &str,
    object: usize,
    world: &Mat4,
    time: f64,
    visible: bool,
    materials: Override<'_>,
    cast_shadow: bool,
    receive_shadow: bool,
) -> Instance {
    let model = &asset.model;
    let (parent, pose) = pose::placed(model, pose::Pose::at(model, time), world);
    let worlds = pose::world(model, &pose, Some(&parent));
    let mut meshes = Vec::new();
    let mut lights = Vec::new();
    let base = (object as u64) << 32;
    for (n, i) in model.traversal().into_iter().enumerate() {
        let o = &model.objects[i];
        match &o.kind {
            ObjectKind::Drawable(d) => {
                let gm = &model.geometries[d.geometry];
                // A bad index refuses every WebGL draw. Short attributes
                // depend on the final material and are checked below.
                if gm.undrawable.is_some() {
                    continue;
                }
                let geometry = geometry(asset, d.geometry);
                let order = base | n as u64;
                let loaded = source_material(asset, &model.materials[d.material], order);
                let is_mesh = gm.mode == Mode::Triangles;
                let materials = if !is_mesh {
                    Materials::Single(Box::new(loaded))
                } else {
                    match materials {
                        Override::Source => Materials::Single(Box::new(loaded)),
                        Override::Single(def) => {
                            Materials::Single(Box::new(synced(&loaded, def, order | 1 << 31)))
                        }
                        Override::Slots(defs) => Materials::Slots(
                            defs.iter()
                                .enumerate()
                                .map(|(k, def)| {
                                    if k == 0 {
                                        synced(&loaded, def, order | 1 << 31)
                                    } else {
                                        synced(
                                            &Material3d {
                                                kind: MaterialKind::Basic,
                                                ..loaded.clone()
                                            },
                                            def,
                                            order | 1 << 31 | (k as u64) << 20,
                                        )
                                    }
                                })
                                .collect(),
                        ),
                    }
                };
                let skinning = d.skin.map(|s| {
                    let bones = pose::bone_matrices(model, s, &worlds);
                    let (size, data) = pose::bone_texture(&bones);
                    let k = key(&[b"gltf bones 1", &f32_bytes(&data)]);
                    Skinning {
                        bind_matrix: math::IDENTITY,
                        bind_matrix_inverse: math::invert(&worlds[i]),
                        size,
                        bones: Arc::new(data),
                        key: k,
                    }
                });
                let instances = d.instances.as_ref().map(|inst| {
                    let bytes: Vec<u8> = inst
                        .matrices
                        .iter()
                        .flat_map(|m| m.iter().flat_map(|v| v.to_le_bytes()))
                        .collect();
                    (key(&[b"gltf instances 1", &bytes]), inst.clone())
                });
                let sphere = if d.skinned {
                    None
                } else if let Some(inst) = &d.instances {
                    instances_sphere(gm, inst)
                } else {
                    Some(geometry.sphere)
                };
                let mesh = Mesh3d {
                    geometry,
                    world: worlds[i],
                    materials,
                    cast_shadow: is_mesh && cast_shadow,
                    receive_shadow: is_mesh && receive_shadow,
                    gltf: Some(Arc::new(MeshExt {
                        mode: gm.mode,
                        skinning,
                        instances,
                        influences: pose.influences[i].clone(),
                        sphere,
                        skinned: d.skinned,
                        id: base | n as u64,
                    })),
                };
                meshes.push(mesh);
            }
            ObjectKind::Light(l) => {
                let light = &model.lights[*l];
                let w = worlds[i];
                // The runtime clone's `target` is a detached copy keeping the
                // loader's world matrix (`Model::light_targets`).
                let target = model.light_targets.get(i).copied().flatten().unwrap_or([
                    w[12],
                    w[13],
                    w[14] - 1.0,
                ]);
                lights.push(Light3d {
                    id: format!("{id}#light{}", lights.len()),
                    kind: match light.kind {
                        g::LightKind::Directional => LightKind::Directional,
                        g::LightKind::Point => LightKind::Point,
                        g::LightKind::Spot => LightKind::Spot,
                    },
                    world: w,
                    target,
                    color: light.color,
                    intensity: light.intensity,
                    distance: light.distance,
                    decay: 2.0,
                    angle: light.angle,
                    penumbra: light.penumbra,
                    width: 0.0,
                    height: 0.0,
                    visible,
                    cast_shadow: false,
                });
            }
            _ => {}
        }
    }
    Instance { meshes, lights }
}

/// Apply WebGL's active short-attribute rule to the resolved draw scene.
/// The model and plan retain this mesh for queries, snapshots, and a map
/// whose image fails only after planning. GPU preparation calls this after
/// removing any late-failed scene maps from the final materials.
pub(crate) fn short_attribute_blocks_draw(mesh: &Mesh3d, params: &Parameters) -> bool {
    let Some(extra) = mesh.geometry.extra.as_deref() else {
        return false;
    };
    extra.short_attributes.iter().any(|(name, count)| {
        *count < extra.fetched_vertices && material::consumes_short_attribute(params, name)
    })
}

#[cfg(test)]
mod short_morph_tests {
    use super::*;
    use pm_geom3d::model_import::glb::GlbWriter;
    use serde_json::{Map, json};

    fn attr(values: &[f32]) -> g::Attribute {
        g::Attribute {
            data: Arc::new(values.to_vec()),
            size: 3,
        }
    }

    fn geometry(normal_count: usize) -> g::Geometry {
        g::Geometry {
            position: attr(&[0., 0., 0., 1., 0., 0., 0., 1., 0.]),
            normal: None,
            tangent: None,
            uv: [None, None, None, None],
            color: None,
            skin_index: None,
            skin_weight: None,
            skin_weight_bits: None,
            index: None,
            morph: g::Morph {
                position: Some(vec![attr(&[1., 2., 3., 4., 5., 6.])]),
                normal: Some(vec![attr(&[7., 8., 9., 10., 11., 12.])]),
                color: None,
                source_counts: [vec![2], vec![normal_count], vec![]],
            },
            bounding_box: None,
            mode: Mode::Triangles,
            undrawable: None,
            short_attributes: vec![],
        }
    }

    #[test]
    fn short_primary_morph_leaves_tail_texel_zero() {
        let packed = morph_texture(1, 0, &geometry(2)).unwrap();
        assert_eq!(&packed.data[0..4], &[1., 2., 3., 0.]);
        assert_eq!(&packed.data[4..8], &[7., 8., 9., 0.]);
        assert_eq!(&packed.data[16..24], &[0.; 8]);
    }

    #[test]
    fn shorter_secondary_morph_stores_nan_inside_primary_loop() {
        let packed = morph_texture(1, 0, &geometry(1)).unwrap();
        assert_eq!(&packed.data[8..12], &[4., 5., 6., 0.]);
        assert!(packed.data[12..15].iter().all(|v| v.is_nan()));
        assert_eq!(packed.data[15], 0.);
    }

    #[test]
    fn indexed_draw_does_not_fetch_unused_short_attribute_tail() {
        let mut geom = geometry(2);
        geom.index = Some(Arc::new(vec![0, 1, 2]));
        assert_eq!(fetched_vertex_count(&geom), 3);
        geom.index = Some(Arc::new(vec![0, 1, 1]));
        assert_eq!(fetched_vertex_count(&geom), 2);
        geom.index = None;
        assert_eq!(fetched_vertex_count(&geom), 3);
    }

    #[test]
    fn generated_indices_live_with_asset_across_draw_rebuilds() {
        let mut writer = GlbWriter::default();
        let positions = writer.floats(&[[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]], true);
        let mut document = Map::new();
        document.insert("asset".into(), json!({"version":"2.0"}));
        document.insert(
            "meshes".into(),
            json!([{"primitives":[{"attributes":{"POSITION":positions}}]}]),
        );
        document.insert("nodes".into(), json!([{"mesh":0}]));
        document.insert("scenes".into(), json!([{"nodes":[0]}]));
        let asset = asset::from_bytes(&writer.finish(document)).unwrap();

        let first = super::geometry(&asset, 0);
        let GeometryPayload::Gltf {
            generated_indices: Some(indices),
            ..
        } = &first.geometry
        else {
            panic!("unindexed glTF requires generated draw indices")
        };
        let index_handle = Arc::downgrade(indices);
        let extra_handle = Arc::downgrade(first.extra.as_ref().unwrap());
        drop(first);

        let second = super::geometry(&asset, 0);
        let GeometryPayload::Gltf {
            generated_indices: Some(indices),
            ..
        } = &second.geometry
        else {
            panic!("unindexed glTF requires generated draw indices")
        };
        assert!(Arc::ptr_eq(indices, &index_handle.upgrade().unwrap()));
        assert!(Arc::ptr_eq(
            second.extra.as_ref().unwrap(),
            &extra_handle.upgrade().unwrap()
        ));
        drop(second);
        drop(asset);
        assert!(index_handle.upgrade().is_none());
        assert!(extra_handle.upgrade().is_none());
    }
}
