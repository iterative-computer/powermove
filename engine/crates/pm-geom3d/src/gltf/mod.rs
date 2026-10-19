//! glTF 2.0 models as three.js r180's `GLTFLoader` builds them and
//! today's importer (`core/scene3d/import-model.ts`, `loadGltf`) prepares
//! them (plan sections 8 and 13; docs/engine/gltf.md).
//!
//! [`load`] parses a GLB or a self-contained glTF (the `gltf` crate reads
//! the container and document; extensions it does not model are read from
//! the raw JSON) into a [`Model`]: the three.js object tree the loader
//! creates (groups, bones, meshes, skinned and instanced meshes, points,
//! lines, punctual lights with their targets), its geometries (shared per
//! `createPrimitiveKey`, strips and fans converted, quantized and sparse
//! accessors decoded, skin weights normalized), its materials with every
//! extension r180 honors and `assignFinalMaterial`'s variants, textures
//! with their samplers and transforms, skins, and animation clips with
//! their tracks bound by name exactly as `PropertyBinding` binds them on
//! the runtime's clone (tracks of unnamed nodes do not bind there).
//!
//! [`anim`] is the `AnimationMixer` rule (every clip at weight 1, looping,
//! blended equally), [`pose`] evaluates the tree at a time (world
//! matrices, bone matrices, morph influences, lights, bounds), and
//! [`pick`] raycasts posed models with three.js's semantics.

pub mod anim;
mod load;
pub(crate) mod promise_order;
pub mod math;
pub mod pick;
pub mod pose;

pub use load::{Limits, load, load_with};
pub(crate) use load::{container, js_number};

use math::{Mat4, Quat, Vec3};

/// Why a model does not load (`loadGltf` throws; the asset stays without a
/// scene, which renders as the missing-model box).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn err<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error(msg.into()))
}

/// How a primitive draws (`Mesh`, `Points`, `LineSegments`, `Line`,
/// `LineLoop`; strips and fans are converted to triangles).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    Triangles,
    Points,
    LineSegments,
    LineStrip,
    LineLoop,
}

/// A vertex attribute as WebGL reads it: f32 components (normalized
/// integers already mapped to floats) and the item size.
#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    pub data: Vec<f32>,
    pub size: usize,
}

impl Attribute {
    pub fn count(&self) -> usize {
        self.data.len().checked_div(self.size).unwrap_or(0)
    }
}

/// `geometry.morphAttributes` (relative targets, `morphTargetsRelative`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Morph {
    pub position: Option<Vec<Attribute>>,
    pub normal: Option<Vec<Attribute>>,
    pub color: Option<Vec<Attribute>>,
    /// Original accessor counts before CPU-safe padding. Three's JS texture
    /// packer iterates the position target's count and leaves later texels
    /// zero; a shorter normal/color target inside that loop writes NaN.
    pub source_counts: [Vec<usize>; 3],
}

impl Morph {
    /// `morphAttributes.position || normal || color`.length.
    pub fn count(&self) -> usize {
        self.position
            .as_ref()
            .or(self.normal.as_ref())
            .or(self.color.as_ref())
            .map_or(0, Vec::len)
    }

    /// `morphTextureStride`: the last of position (1), normal (2), color (3)
    /// present.
    pub fn stride(&self) -> u32 {
        if self.color.is_some() {
            3
        } else if self.normal.is_some() {
            2
        } else if self.position.is_some() {
            1
        } else {
            0
        }
    }
}

/// A `BufferGeometry` the loader built for one primitive.
#[derive(Debug, Clone, PartialEq)]
pub struct Geometry {
    pub position: Attribute,
    pub normal: Option<Attribute>,
    pub tangent: Option<Attribute>,
    /// `uv`, `uv1`, `uv2`, `uv3` (`TEXCOORD_0` to `TEXCOORD_3`).
    pub uv: [Option<Attribute>; 4],
    /// `COLOR_0` (3 or 4 components).
    pub color: Option<Attribute>,
    /// `JOINTS_0` as floats (`vertexAttribPointer`, not normalized).
    pub skin_index: Option<Attribute>,
    /// `WEIGHTS_0` (normalized by `normalizeSkinWeights` for skinned meshes).
    pub skin_weight: Option<Attribute>,
    /// Whether `skin_weight` is a normalized 8 or 16-bit accessor (its
    /// normalized weights are re-quantized, as `setXYZW` stores them).
    pub skin_weight_bits: Option<u32>,
    pub index: Option<Vec<u32>>,
    pub morph: Morph,
    /// `computeBounds`: the `POSITION` accessor's min and max grown by the
    /// largest morph displacement (`None` without min and max).
    pub bounding_box: Option<(Vec3, Vec3)>,
    pub mode: Mode,
    /// A bad index refuses a WebGL draw regardless of the material.
    pub undrawable: Option<String>,
    /// Attributes whose source accessor is shorter than POSITION. The CPU
    /// arrays are padded for safe poses; only a final shader that consumes
    /// one of these attributes makes WebGL refuse the draw.
    pub short_attributes: Vec<(&'static str, usize)>,
}

impl Geometry {
    pub fn vertex_count(&self) -> usize {
        self.position.count()
    }

    /// The bounding box three.js uses: [`Geometry::bounding_box`], or
    /// `computeBoundingBox` over positions and relative morph targets.
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        if let Some(b) = self.bounding_box {
            return Some(b);
        }
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        let mut grow = |p: &[f32]| {
            for k in 0..3 {
                let v = f64::from(p[k]);
                lo[k] = lo[k].min(v);
                hi[k] = hi[k].max(v);
            }
        };
        let positions: Vec<&[f32]> = self.position.data.chunks_exact(3).collect();
        if positions.is_empty() {
            return None;
        }
        for p in &positions {
            grow(p);
        }
        // `computeBoundingBox` with relative morph attributes: each
        // target's box added to the running box's corners.
        for (i, target) in self.morph.position.iter().flatten().enumerate() {
            let mut tlo = [f64::INFINITY; 3];
            let mut thi = [f64::NEG_INFINITY; 3];
            let source_len = self.morph.source_counts[0].get(i).copied().unwrap_or(target.count());
            for p in target.data[..source_len * 3].chunks_exact(3) {
                for k in 0..3 {
                    tlo[k] = tlo[k].min(f64::from(p[k]));
                    thi[k] = thi[k].max(f64::from(p[k]));
                }
            }
            let a = [lo[0] + tlo[0], lo[1] + tlo[1], lo[2] + tlo[2]];
            for k in 0..3 {
                lo[k] = lo[k].min(a[k]);
                hi[k] = hi[k].max(a[k]);
            }
            let b = [hi[0] + thi[0], hi[1] + thi[1], hi[2] + thi[2]];
            for k in 0..3 {
                lo[k] = lo[k].min(b[k]);
                hi[k] = hi[k].max(b[k]);
            }
        }
        Some((lo, hi))
    }

    /// `geometry.boundingSphere` as three.js has it: from
    /// [`Geometry::bounding_box`] (`computeBounds`: the box center and half
    /// its diagonal), else `computeBoundingSphere` over positions.
    pub fn bounding_sphere(&self) -> (Vec3, f64) {
        if let Some((lo, hi)) = self.bounding_box {
            let c = [
                (lo[0] + hi[0]) * 0.5,
                (lo[1] + hi[1]) * 0.5,
                (lo[2] + hi[2]) * 0.5,
            ];
            let d = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
            return (c, (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() / 2.0);
        }
        let Some((lo, hi)) = self.bounds() else {
            return ([0.0; 3], 0.0);
        };
        let c = [
            (lo[0] + hi[0]) * 0.5,
            (lo[1] + hi[1]) * 0.5,
            (lo[2] + hi[2]) * 0.5,
        ];
        let mut max = 0f64;
        let mut visit = |p: [f64; 3]| {
            let d = [c[0] - p[0], c[1] - p[1], c[2] - p[2]];
            max = max.max(d[0] * d[0] + d[1] * d[1] + d[2] * d[2]);
        };
        for p in self.position.data.chunks_exact(3) {
            visit([f64::from(p[0]), f64::from(p[1]), f64::from(p[2])]);
        }
        for target in self.morph.position.iter().flatten() {
            for (p, t) in self
                .position
                .data
                .chunks_exact(3)
                .zip(target.data.chunks_exact(3))
            {
                visit([
                    f64::from(p[0]) + f64::from(t[0]),
                    f64::from(p[1]) + f64::from(t[1]),
                    f64::from(p[2]) + f64::from(t[2]),
                ]);
            }
        }
        (c, max.sqrt())
    }
}

/// `texture.magFilter` and `minFilter` (WebGL constants).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Filter {
    Nearest,
    Linear,
    NearestMipmapNearest,
    LinearMipmapNearest,
    NearestMipmapLinear,
    LinearMipmapLinear,
}

impl Filter {
    fn from_gl(v: Option<u64>, default: Filter) -> Filter {
        match v {
            Some(9728) => Filter::Nearest,
            Some(9729) => Filter::Linear,
            Some(9984) => Filter::NearestMipmapNearest,
            Some(9985) => Filter::LinearMipmapNearest,
            Some(9986) => Filter::NearestMipmapLinear,
            Some(9987) => Filter::LinearMipmapLinear,
            _ => default,
        }
    }

    /// Whether sampling reads mip levels (`generateMipmaps` in the loader).
    pub fn mipmapped(self) -> bool {
        !matches!(self, Filter::Nearest | Filter::Linear)
    }
}

/// `texture.wrapS` and `wrapT`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Wrap {
    Repeat,
    Clamp,
    Mirror,
}

impl Wrap {
    fn from_gl(v: Option<u64>) -> Wrap {
        match v {
            Some(33071) => Wrap::Clamp,
            Some(33648) => Wrap::Mirror,
            _ => Wrap::Repeat,
        }
    }
}

/// A texture's sampler (`loadTextureImage`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Sampler {
    pub mag: Filter,
    pub min: Filter,
    pub wrap_s: Wrap,
    pub wrap_t: Wrap,
}

/// An image as stored in the file (decoded by the renderer; geometry
/// queries never need texels).
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub bytes: std::sync::Arc<Vec<u8>>,
    pub mime: String,
}

/// A glTF texture: its image (after `EXT_texture_webp` and
/// `EXT_texture_avif`) and sampler.
#[derive(Debug, Clone, PartialEq)]
pub struct Texture {
    pub image: usize,
    pub sampler: Sampler,
}

/// A map a material samples (`assignTexture`): texture, UV channel, and
/// `KHR_texture_transform` (`texture.matrix`, column-major 3 × 3).
#[derive(Debug, Clone, PartialEq)]
pub struct MapRef {
    pub texture: usize,
    pub channel: u32,
    pub transform: [f64; 9],
}

/// The three.js material class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MaterialType {
    Standard,
    Physical,
    /// `KHR_materials_unlit`.
    Basic,
    /// `assignFinalMaterial` for points.
    Points,
    /// `assignFinalMaterial` for lines.
    Line,
}

/// A material as three.js holds it after `GLTFLoader` (every parameter
/// three.js's program and uniforms read; linear colors).
#[derive(Debug, Clone, PartialEq)]
pub struct Material {
    pub kind: MaterialType,
    pub name: String,
    pub color: Vec3,
    pub opacity: f64,
    pub transparent: bool,
    pub depth_write: bool,
    pub double_sided: bool,
    pub alpha_test: f64,
    pub vertex_colors: bool,
    pub flat_shading: bool,
    pub map: Option<MapRef>,
    // MeshStandardMaterial
    pub roughness: f64,
    pub metalness: f64,
    pub emissive: Vec3,
    pub emissive_intensity: f64,
    pub roughness_map: Option<MapRef>,
    pub metalness_map: Option<MapRef>,
    pub normal_map: Option<MapRef>,
    pub normal_scale: [f64; 2],
    pub ao_map: Option<MapRef>,
    pub ao_map_intensity: f64,
    pub emissive_map: Option<MapRef>,
    pub bump_map: Option<MapRef>,
    pub bump_scale: f64,
    // MeshPhysicalMaterial
    pub ior: f64,
    pub clearcoat: f64,
    pub clearcoat_roughness: f64,
    pub clearcoat_map: Option<MapRef>,
    pub clearcoat_roughness_map: Option<MapRef>,
    pub clearcoat_normal_map: Option<MapRef>,
    pub clearcoat_normal_scale: [f64; 2],
    pub dispersion: f64,
    pub iridescence: f64,
    pub iridescence_ior: f64,
    pub iridescence_thickness_range: [f64; 2],
    pub iridescence_map: Option<MapRef>,
    pub iridescence_thickness_map: Option<MapRef>,
    pub sheen: f64,
    pub sheen_color: Vec3,
    pub sheen_roughness: f64,
    pub sheen_color_map: Option<MapRef>,
    pub sheen_roughness_map: Option<MapRef>,
    pub transmission: f64,
    pub transmission_map: Option<MapRef>,
    pub thickness: f64,
    pub thickness_map: Option<MapRef>,
    pub attenuation_distance: f64,
    pub attenuation_color: Vec3,
    pub specular_intensity: f64,
    pub specular_color: Vec3,
    pub specular_intensity_map: Option<MapRef>,
    pub specular_color_map: Option<MapRef>,
    pub anisotropy: f64,
    pub anisotropy_rotation: f64,
    pub anisotropy_map: Option<MapRef>,
    /// `PointsMaterial.sizeAttenuation` (false for glTF points).
    pub size_attenuation: bool,
}

impl Material {
    /// `new MeshStandardMaterial()` (three.js defaults).
    pub fn standard() -> Material {
        Material {
            kind: MaterialType::Standard,
            name: String::new(),
            color: [1.0; 3],
            opacity: 1.0,
            transparent: false,
            depth_write: true,
            double_sided: false,
            alpha_test: 0.0,
            vertex_colors: false,
            flat_shading: false,
            map: None,
            roughness: 1.0,
            metalness: 0.0,
            emissive: [0.0; 3],
            emissive_intensity: 1.0,
            roughness_map: None,
            metalness_map: None,
            normal_map: None,
            normal_scale: [1.0, 1.0],
            ao_map: None,
            ao_map_intensity: 1.0,
            emissive_map: None,
            bump_map: None,
            bump_scale: 1.0,
            ior: 1.5,
            clearcoat: 0.0,
            clearcoat_roughness: 0.0,
            clearcoat_map: None,
            clearcoat_roughness_map: None,
            clearcoat_normal_map: None,
            clearcoat_normal_scale: [1.0, 1.0],
            dispersion: 0.0,
            iridescence: 0.0,
            iridescence_ior: 1.3,
            iridescence_thickness_range: [100.0, 400.0],
            iridescence_map: None,
            iridescence_thickness_map: None,
            sheen: 0.0,
            sheen_color: [0.0; 3],
            sheen_roughness: 1.0,
            sheen_color_map: None,
            sheen_roughness_map: None,
            transmission: 0.0,
            transmission_map: None,
            thickness: 0.0,
            thickness_map: None,
            attenuation_distance: f64::INFINITY,
            attenuation_color: [1.0; 3],
            specular_intensity: 1.0,
            specular_color: [1.0; 3],
            specular_intensity_map: None,
            specular_color_map: None,
            anisotropy: 0.0,
            anisotropy_rotation: 0.0,
            anisotropy_map: None,
            size_attenuation: true,
        }
    }

    /// `createDefaultMaterial`: white, metalness 1, roughness 1.
    pub fn gltf_default() -> Material {
        Material {
            metalness: 1.0,
            ..Material::standard()
        }
    }

    /// Every map with its three.js uniform name, in `WebGLMaterials`'
    /// refresh order.
    pub fn maps(&self) -> Vec<(&'static str, &MapRef)> {
        let all: [(&'static str, &Option<MapRef>); 19] = [
            ("map", &self.map),
            ("bumpMap", &self.bump_map),
            ("normalMap", &self.normal_map),
            ("emissiveMap", &self.emissive_map),
            ("aoMap", &self.ao_map),
            ("metalnessMap", &self.metalness_map),
            ("roughnessMap", &self.roughness_map),
            ("sheenColorMap", &self.sheen_color_map),
            ("sheenRoughnessMap", &self.sheen_roughness_map),
            ("clearcoatMap", &self.clearcoat_map),
            ("clearcoatRoughnessMap", &self.clearcoat_roughness_map),
            ("clearcoatNormalMap", &self.clearcoat_normal_map),
            ("iridescenceMap", &self.iridescence_map),
            ("iridescenceThicknessMap", &self.iridescence_thickness_map),
            ("transmissionMap", &self.transmission_map),
            ("thicknessMap", &self.thickness_map),
            ("anisotropyMap", &self.anisotropy_map),
            ("specularColorMap", &self.specular_color_map),
            ("specularIntensityMap", &self.specular_intensity_map),
        ];
        all.into_iter()
            .filter_map(|(n, m)| m.as_ref().map(|m| (n, m)))
            .collect()
    }
}

/// A `KHR_lights_punctual` light (`_loadLight`).
#[derive(Debug, Clone, PartialEq)]
pub struct Light {
    pub kind: LightKind,
    /// Linear.
    pub color: Vec3,
    pub intensity: f64,
    /// `distance` (`range`, 0 when absent).
    pub distance: f64,
    /// Spot: `angle` (outer cone) and `penumbra` (1 − inner / outer).
    pub angle: f64,
    pub penumbra: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LightKind {
    Directional,
    Point,
    Spot,
}

/// `EXT_mesh_gpu_instancing`: `instanceMatrix` (f32, as `setMatrixAt`
/// stores it) and `instanceColor` (`_COLOR_0`).
#[derive(Debug, Clone, PartialEq)]
pub struct Instances {
    pub matrices: Vec<[f32; 16]>,
    pub colors: Option<Attribute>,
}

/// A drawable object (`Mesh`, `SkinnedMesh`, `InstancedMesh`, `Points`,
/// or a line).
#[derive(Debug, Clone, PartialEq)]
pub struct Drawable {
    /// `model.geometries` index.
    pub geometry: usize,
    /// `model.materials` index (the final material).
    pub material: usize,
    /// A `SkinnedMesh` bound to `model.skins[skin]` (`None`: not skinned,
    /// or a skinned mesh without a skeleton).
    pub skin: Option<usize>,
    /// Whether three.js made a `SkinnedMesh` (the mesh is skinned somewhere).
    pub skinned: bool,
    pub instances: Option<std::sync::Arc<Instances>>,
    /// `morphTargetInfluences` as loaded (mesh then node weights).
    pub morph_influences: Vec<f64>,
}

/// What an object of the tree is.
#[derive(Debug, Clone, PartialEq)]
pub enum ObjectKind {
    /// `Group` (the scene, a node with several attachments, a mesh with
    /// several primitives).
    Group,
    /// A plain node (`Object3D`).
    Node,
    Bone,
    Camera,
    Drawable(Drawable),
    /// `model.lights` index.
    Light(usize),
    /// A directional or spot light's `target` (at (0, 0, −1)).
    LightTarget,
}

/// An object of the three.js tree the loader builds.
#[derive(Debug, Clone, PartialEq)]
pub struct Object {
    pub name: String,
    pub kind: ObjectKind,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub position: Vec3,
    pub quaternion: Quat,
    pub scale: Vec3,
}

impl Object {
    pub fn local(&self) -> Mat4 {
        math::compose(self.position, self.quaternion, self.scale)
    }
}

/// `new Skeleton(bones, boneInverses)`.
#[derive(Debug, Clone, PartialEq)]
pub struct Skin {
    /// Bone objects.
    pub bones: Vec<usize>,
    /// f32 values (`Matrix4.fromArray` of a `Float32Array`).
    pub inverses: Vec<Mat4>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Interpolation {
    Linear,
    Step,
    CubicSpline,
}

/// The property a track animates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Property {
    Position,
    Quaternion,
    Scale,
    MorphTargetInfluences,
}

/// A `KeyframeTrack` and the object `PropertyBinding` binds it to.
#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    /// `"<name>.<property>"`.
    pub name: String,
    /// The bound object (`None`: no object of that name on the runtime's
    /// clone, so the track does nothing).
    pub target: Option<usize>,
    pub property: Property,
    pub interpolation: Interpolation,
    /// f32 values.
    pub times: Vec<f32>,
    pub values: Vec<f32>,
    /// `getValueSize()` (for cubic splines a third of it is the value).
    pub value_size: usize,
}

/// An `AnimationClip`.
#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    pub name: String,
    pub tracks: Vec<Track>,
    /// `resetDuration()`: the largest last key time.
    pub duration: f64,
}

/// `loadGltf`'s fit: the scene is moved by −`center` and scaled by
/// 1 / `radius` under the asset root.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    pub center: Vec3,
    pub radius: f64,
}

/// A loaded model.
#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    /// The tree; `objects[0]` is the glTF scene's `Group`.
    pub objects: Vec<Object>,
    pub geometries: Vec<Geometry>,
    pub materials: Vec<Material>,
    pub textures: Vec<Texture>,
    pub images: Vec<Image>,
    pub lights: Vec<Light>,
    pub skins: Vec<Skin>,
    pub clips: Vec<Clip>,
    pub extensions_used: Vec<String>,
    pub fit: Fit,
    /// `loadGltf`'s counts over meshes (points and lines excluded).
    pub vertices: usize,
    pub triangles: usize,
    /// Per object: the world position a light's `target` keeps on the
    /// runtime's clone (directional and spot lights).
    pub light_targets: Vec<Option<Vec3>>,
}

impl Model {
    /// Objects in `traverse` order (depth first, parents first).
    pub fn traversal(&self) -> Vec<usize> {
        let mut out = Vec::with_capacity(self.objects.len());
        let mut stack = vec![0usize];
        while let Some(i) = stack.pop() {
            out.push(i);
            for &c in self.objects[i].children.iter().rev() {
                stack.push(c);
            }
        }
        out
    }
}
