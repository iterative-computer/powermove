//! The object graph three.js r180's GLTFLoader builds from a [`Model`], and
//! today's import (`loadGltf` in `core/scene3d/import-model.ts`): its checks,
//! the vertex and triangle counts it reports, and the normalization that
//! centers the model on its bounds and scales it to unit radius.

use super::math::{self, Box3, Mat4};
use super::{Error, Model, Result, Topology, err};
use serde_json::Value;
use std::collections::HashMap;

/// What three.js makes of a glTF object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectKind {
    Group,
    Object3D,
    Bone,
    Mesh,
    SkinnedMesh,
    InstancedMesh,
    Points,
    LineSegments,
    LineLoop,
    Line,
    DirectionalLight,
    PointLight,
    SpotLight,
    PerspectiveCamera,
    OrthographicCamera,
}

impl ObjectKind {
    /// three.js's `type` string (`InstancedMesh` keeps `Mesh`).
    pub fn type_name(self) -> &'static str {
        match self {
            ObjectKind::Group => "Group",
            ObjectKind::Object3D => "Object3D",
            ObjectKind::Bone => "Bone",
            ObjectKind::Mesh | ObjectKind::InstancedMesh => "Mesh",
            ObjectKind::SkinnedMesh => "SkinnedMesh",
            ObjectKind::Points => "Points",
            ObjectKind::LineSegments => "LineSegments",
            ObjectKind::LineLoop => "LineLoop",
            ObjectKind::Line => "Line",
            ObjectKind::DirectionalLight => "DirectionalLight",
            ObjectKind::PointLight => "PointLight",
            ObjectKind::SpotLight => "SpotLight",
            ObjectKind::PerspectiveCamera => "PerspectiveCamera",
            ObjectKind::OrthographicCamera => "OrthographicCamera",
        }
    }

    /// three.js's `isMesh` (what `loadGltf` counts).
    pub fn is_mesh(self) -> bool {
        matches!(
            self,
            ObjectKind::Mesh | ObjectKind::SkinnedMesh | ObjectKind::InstancedMesh
        )
    }
}

/// One object of the graph.
#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    pub kind: ObjectKind,
    pub name: String,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub matrix: Mat4,
    pub world: Mat4,
    /// The glTF node this object stands for, when it is one.
    pub node: Option<usize>,
    /// `(mesh, primitive)` for geometry-bearing objects.
    pub primitive: Option<(usize, usize)>,
    /// Instancing: the node's instance matrices.
    pub instances: Option<usize>,
    /// The glTF light or camera.
    pub light: Option<usize>,
    pub camera: Option<usize>,
    /// Skinned meshes: the skin.
    pub skin: Option<usize>,
}

/// The graph, in three.js traversal order (depth first, pre-order).
#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub objects: Vec<Object>,
    /// The object of each glTF node.
    pub node_objects: Vec<Option<usize>>,
}

/// What today's importer reports and applies.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportFacts {
    /// Vertices and triangles of every mesh object (`isMesh`): positions,
    /// and index (or position) counts over 3, rounded up at the end.
    pub vertices: usize,
    pub triangles: usize,
    /// The world box of the scene (`Box3.setFromObject`).
    pub bounds: Box3,
    /// The normalization: the scene moves by −center, then scales by 1 / radius.
    pub center: [f64; 3],
    pub radius: f64,
}

/// `PropertyBinding.sanitizeNodeName`: whitespace becomes `_`; `[ ] . : /`
/// are removed.
pub fn sanitize(name: &str) -> String {
    name.chars()
        .filter_map(|c| match c {
            c if c.is_whitespace() => Some('_'),
            '[' | ']' | '.' | ':' | '/' => None,
            c => Some(c),
        })
        .collect()
}

struct Names(HashMap<String, usize>);

impl Names {
    /// `createUniqueName`.
    fn unique(&mut self, name: &str) -> String {
        let s = sanitize(name);
        match self.0.get_mut(&s) {
            Some(n) => {
                *n += 1;
                format!("{s}_{n}")
            }
            None => {
                self.0.insert(s.clone(), 0);
                s
            }
        }
    }
}

struct Builder<'m> {
    model: &'m Model,
    objects: Vec<Object>,
    node_objects: Vec<Option<usize>>,
    skinned_meshes: Vec<bool>,
    names: ResolvedNames,
    /// `_getNodeRef`'s per-index use counters (meshes, cameras, lights).
    uses: [Vec<usize>; 3],
}

/// The names `createUniqueName` hands out, resolved in three.js's call
/// order: the scene, then node, camera, and light names synchronously in
/// traversal order (`_loadNodeShallow`), then mesh primitive names as the
/// meshes finish loading (`loadMesh` resolves after its primitives, in
/// request order).
struct ResolvedNames {
    scene: String,
    nodes: Vec<Option<String>>,
    cameras: Vec<String>,
    lights: Vec<String>,
    /// Per mesh, per primitive.
    meshes: Vec<Vec<String>>,
    /// `_markDefs` reference counts over every node of the file.
    refs: [Vec<usize>; 3],
}

impl ResolvedNames {
    fn resolve(model: &Model) -> ResolvedNames {
        let mut names = Names(HashMap::new());
        let mut refs = [
            vec![0; model.meshes.len()],
            vec![0; model.cameras.len()],
            vec![0; model.lights.len()],
        ];
        for n in &model.nodes {
            for (cache, index) in [n.mesh, n.camera, n.light].into_iter().enumerate() {
                if let Some(i) = index
                    && let Some(r) = refs[cache].get_mut(i)
                {
                    *r += 1;
                }
            }
        }
        let scene = if model.scene_name.is_empty() {
            String::new()
        } else {
            names.unique(&model.scene_name)
        };
        let mut out = ResolvedNames {
            scene,
            nodes: vec![None; model.nodes.len()],
            cameras: vec![String::new(); model.cameras.len()],
            lights: vec![String::new(); model.lights.len()],
            meshes: vec![vec![]; model.meshes.len()],
            refs,
        };
        let mut seen = vec![false; model.nodes.len()];
        let mut cameras = vec![false; model.cameras.len()];
        let mut lights = vec![false; model.lights.len()];
        let mut mesh_order = vec![];
        let mut stack: Vec<usize> = model.roots.iter().rev().copied().collect();
        while let Some(n) = stack.pop() {
            if n >= model.nodes.len() || std::mem::replace(&mut seen[n], true) {
                continue;
            }
            let def = &model.nodes[n];
            if !def.name.is_empty() {
                out.nodes[n] = Some(names.unique(&def.name));
            }
            if let Some(m) = def.mesh
                && !mesh_order.contains(&m)
            {
                mesh_order.push(m);
            }
            if let Some(c) = def.camera
                && c < cameras.len()
                && !std::mem::replace(&mut cameras[c], true)
                && !model.camera_names[c].is_empty()
            {
                out.cameras[c] = names.unique(&model.camera_names[c]);
            }
            if let Some(l) = def.light
                && l < lights.len()
                && !std::mem::replace(&mut lights[l], true)
            {
                let name = &model.lights[l].name;
                out.lights[l] = if name.is_empty() {
                    names.unique(&format!("light_{l}"))
                } else {
                    names.unique(name)
                };
            }
            stack.extend(def.children.iter().rev().copied());
        }
        for m in mesh_order {
            let def = &model.meshes[m];
            let base = if def.name.is_empty() {
                format!("mesh_{m}")
            } else {
                def.name.clone()
            };
            out.meshes[m] = def.primitives.iter().map(|_| names.unique(&base)).collect();
        }
        out
    }
}

impl Builder<'_> {
    fn add(&mut self, kind: ObjectKind, name: String, matrix: Mat4) -> usize {
        self.objects.push(Object {
            kind,
            name,
            parent: None,
            children: vec![],
            matrix,
            world: math::IDENTITY,
            node: None,
            primitive: None,
            instances: None,
            light: None,
            camera: None,
            skin: None,
        });
        self.objects.len() - 1
    }

    fn attach(&mut self, parent: usize, child: usize) {
        if let Some(old) = self.objects[child].parent {
            self.objects[old].children.retain(|&c| c != child);
        }
        self.objects[child].parent = Some(parent);
        self.objects[parent].children.push(child);
    }

    /// `loadMesh` (and EXT_mesh_gpu_instancing's `createNodeMesh`).
    fn mesh_object(&mut self, node: usize, mesh: usize) -> usize {
        let def = &self.model.meshes[mesh];
        let skinned = self.skinned_meshes[mesh];
        let instanced = self.model.nodes[node].instances.is_some();
        let mut prims = vec![];
        for (pi, p) in def.primitives.iter().enumerate() {
            let kind = match p.topology {
                Topology::Triangles if instanced => ObjectKind::InstancedMesh,
                Topology::Triangles if skinned => ObjectKind::SkinnedMesh,
                Topology::Triangles => ObjectKind::Mesh,
                Topology::Points => ObjectKind::Points,
                Topology::LineSegments => ObjectKind::LineSegments,
                Topology::LineLoop => ObjectKind::LineLoop,
                Topology::LineStrip => ObjectKind::Line,
            };
            let name = self.names.meshes[mesh][pi].clone();
            let o = self.add(kind, name, math::IDENTITY);
            self.objects[o].primitive = Some((mesh, pi));
            if kind == ObjectKind::InstancedMesh {
                self.objects[o].instances = Some(node);
            }
            prims.push(o);
        }
        let object = if prims.len() == 1 {
            prims[0]
        } else {
            let group = self.add(ObjectKind::Group, String::new(), math::IDENTITY);
            for p in prims {
                self.attach(group, p);
            }
            group
        };
        self.node_ref(0, mesh, object);
        object
    }

    /// `_getNodeRef`: an object used by more than one node is cloned per
    /// use and named `<name>_instance_<use>`.
    fn node_ref(&mut self, cache: usize, index: usize, object: usize) {
        if self.names.refs[cache][index] > 1 {
            let k = self.uses[cache][index];
            self.uses[cache][index] += 1;
            self.objects[object]
                .name
                .push_str(&format!("_instance_{k}"));
        }
    }

    /// `loadNode` (`_loadNodeShallow` plus children).
    fn node(&mut self, n: usize) -> usize {
        if let Some(node) = self.node_objects[n] { return node; }
        let def = &self.model.nodes[n];
        let node_name = self.names.nodes[n].clone();
        let mut objects = vec![];
        if let Some(m) = def.mesh {
            objects.push(self.mesh_object(n, m));
        }
        if let Some(c) = def.camera {
            let kind = match self.model.cameras.get(c) {
                Some(super::Camera::Orthographic { .. }) => ObjectKind::OrthographicCamera,
                _ => ObjectKind::PerspectiveCamera,
            };
            let name = self.names.cameras.get(c).cloned().unwrap_or_default();
            let o = self.add(kind, name, math::IDENTITY);
            self.objects[o].camera = Some(c);
            if c < self.names.refs[1].len() {
                self.node_ref(1, c, o);
            }
            objects.push(o);
        }
        if let Some(l) = def.light
            && let Some(light) = self.model.lights.get(l)
        {
            let kind = match light.kind {
                super::LightKind::Directional => ObjectKind::DirectionalLight,
                super::LightKind::Point => ObjectKind::PointLight,
                super::LightKind::Spot { .. } => ObjectKind::SpotLight,
            };
            let name = self.names.lights[l].clone();
            let o = self.add(kind, name, math::IDENTITY);
            self.objects[o].light = Some(l);
            self.node_ref(2, l, o);
            if kind != ObjectKind::PointLight {
                // The light's target sits at (0, 0, −1) under it.
                let mut m = math::IDENTITY;
                m[14] = -1.0;
                let t = self.add(ObjectKind::Object3D, String::new(), m);
                self.attach(o, t);
            }
            objects.push(o);
        }
        let node = if def.is_bone {
            self.add(ObjectKind::Bone, String::new(), math::IDENTITY)
        } else if objects.len() > 1 {
            self.add(ObjectKind::Group, String::new(), math::IDENTITY)
        } else if objects.len() == 1 {
            objects[0]
        } else {
            self.add(ObjectKind::Object3D, String::new(), math::IDENTITY)
        };
        if objects.first() != Some(&node) {
            for o in objects {
                self.attach(node, o);
            }
        }
        if let Some(name) = node_name {
            self.objects[node].name = name;
        }
        self.objects[node].matrix = def.matrix();
        self.objects[node].node = Some(n);
        if let Some(s) = def.skin {
            let mut stack = vec![node];
            while let Some(o) = stack.pop() {
                if self.objects[o].kind == ObjectKind::SkinnedMesh {
                    self.objects[o].skin = Some(s);
                }
                stack.extend(self.objects[o].children.iter().copied());
            }
        }
        self.node_objects[n] = Some(node);
        for c in def.children.clone() {
            let child = self.node(c);
            self.attach(node, child);
        }
        node
    }
}

/// The object graph of the default scene, ordered as three.js traverses it,
/// with world matrices (`scene.updateMatrixWorld()`).
pub fn build(model: &Model) -> Scene {
    let mut skinned = vec![false; model.meshes.len()];
    for n in &model.nodes {
        if let (Some(m), Some(_)) = (n.mesh, n.skin) {
            skinned[m] = true;
        }
    }
    let names = ResolvedNames::resolve(model);
    let uses = [
        vec![0; model.meshes.len()],
        vec![0; model.cameras.len()],
        vec![0; model.lights.len()],
    ];
    let scene_name = names.scene.clone();
    let mut b = Builder {
        model,
        objects: vec![],
        node_objects: vec![None; model.nodes.len()],
        skinned_meshes: skinned,
        names,
        uses,
    };
    let root = b.add(ObjectKind::Group, scene_name, math::IDENTITY);
    for &r in &model.roots {
        let o = b.node(r);
        b.attach(root, o);
    }
    // Reorder into traversal order and compute world matrices.
    let mut order = vec![];
    let mut stack = vec![root];
    while let Some(o) = stack.pop() {
        order.push(o);
        stack.extend(b.objects[o].children.iter().rev().copied());
    }
    let mut remap = vec![usize::MAX; b.objects.len()];
    for (new, &old) in order.iter().enumerate() {
        remap[old] = new;
    }
    let mut objects: Vec<Object> = order
        .iter()
        .map(|&old| {
            let mut o = b.objects[old].clone();
            o.parent = o.parent.map(|p| remap[p]);
            o.children = o.children.iter().map(|c| remap[*c]).collect();
            o
        })
        .collect();
    for i in 0..objects.len() {
        objects[i].world = match objects[i].parent {
            Some(p) => math::multiply(&objects[p].world, &objects[i].matrix),
            None => objects[i].matrix,
        };
    }
    Scene {
        node_objects: b.node_objects.iter().map(|o| o.map(|o| remap[o])).collect(),
        objects,
    }
}

/// `BufferGeometry.computeBoundingBox()`: positions, grown by relative
/// morph targets as three.js grows it.
pub fn geometry_box(p: &super::Primitive) -> Box3 {
    let mut b = Box3::default();
    for v in &p.positions {
        b.expand_by_point(v.map(f64::from));
    }
    for t in &p.targets {
        let Some(pos) = &t.position else { continue };
        let mut tb = Box3::default();
        for v in pos {
            tb.expand_by_point(v.map(f64::from));
        }
        let lo: [f64; 3] = std::array::from_fn(|c| b.min[c] + tb.min[c]);
        b.expand_by_point(lo);
        let hi: [f64; 3] = std::array::from_fn(|c| b.max[c] + tb.max[c]);
        b.expand_by_point(hi);
    }
    b
}

/// `SkinnedMesh.computeBoundingBox()` at rest: each vertex (with the mesh's
/// morph influences) through `applyBoneTransform`. GLTFLoader binds with
/// the mesh's world matrix before any world update (the identity), and
/// `scene.updateMatrixWorld()` then sets `bindMatrixInverse` to the mesh's
/// inverse world matrix (attached bind mode), so the box is in mesh space
/// and the mesh's own placement cancels.
fn skinned_box(model: &Model, scene: &Scene, o: &Object) -> Box3 {
    let (mi, pi) = o.primitive.expect("geometry");
    let p = &model.meshes[mi].primitives[pi];
    let mut b = Box3::default();
    let Some(skin) = o.skin.and_then(|s| model.skins.get(s)) else {
        return geometry_box(p);
    };
    let influences: Vec<f32> = o
        .node
        .or_else(|| o.parent.and_then(|q| scene.objects[q].node))
        .and_then(|n| model.nodes[n].weights.clone())
        .unwrap_or_else(|| model.meshes[mi].weights.clone());
    let bones: Vec<Mat4> = skin
        .joints
        .iter()
        .zip(&skin.inverse_bind)
        .map(|(&j, ibm)| {
            let w = scene.node_objects[j]
                .map(|o| scene.objects[o].world)
                .unwrap_or(math::IDENTITY);
            math::multiply(&w, ibm)
        })
        .collect();
    let (Some(joints), Some(weights)) = (&p.joints, &p.weights) else {
        return geometry_box(p);
    };
    let bind_inverse = math::invert(&o.world);
    for (i, v) in p.positions.iter().enumerate() {
        // getVertexPosition: morphs (relative), then the bone transform.
        let mut base = v.map(f64::from);
        let mut morph = [0.0f64; 3];
        for (t, inf) in p.targets.iter().zip(&influences) {
            if *inf == 0.0 {
                continue;
            }
            if let Some(d) = &t.position {
                for c in 0..3 {
                    // Past a short target three.js reads NaN.
                    morph[c] += d.get(i).map_or(f64::NAN, |d| f64::from(d[c])) * f64::from(*inf);
                }
            }
        }
        for c in 0..3 {
            base[c] += morph[c];
        }
        // normalizeSkinWeights already ran (stored back in the attribute's type).
        let w = normalized(weights[i], p.weights_type);
        let mut out = [0.0f64; 3];
        for k in 0..4 {
            if w[k] == 0.0 {
                continue;
            }
            let Some(m) = bones.get(joints[i][k] as usize) else {
                continue;
            };
            let t = math::apply(m, base);
            for c in 0..3 {
                out[c] += t[c] * w[k];
            }
        }
        b.expand_by_point(math::apply(&bind_inverse, out));
    }
    b
}

/// `SkinnedMesh.normalizeSkinWeights()`: each weight vector scaled by
/// 1 / its Manhattan length, zero vectors set to (1, 0, 0, 0), stored back
/// in the attribute's own type (f32, or normalized u8 or u16, which
/// `setXYZW` rounds) and read back as three.js reads it.
fn normalized(w: [f32; 4], component_type: u32) -> [f64; 4] {
    let v = w.map(f64::from);
    let scale = 1.0 / (v[0].abs() + v[1].abs() + v[2].abs() + v[3].abs());
    let n = if scale.is_finite() {
        v.map(|x| x * scale)
    } else {
        [1.0, 0.0, 0.0, 0.0]
    };
    match component_type {
        5121 => n.map(|x| (x * 255.0).round() / 255.0),
        5123 => n.map(|x| (x * 65535.0).round() / 65535.0),
        _ => n.map(|x| x as f32 as f64),
    }
}

/// `Box3.setFromObject(scene)` with precise = false.
pub fn bounds(model: &Model, scene: &Scene) -> Box3 {
    let mut out = Box3::default();
    for o in &scene.objects {
        let Some((mi, pi)) = o.primitive else {
            continue;
        };
        let p = &model.meshes[mi].primitives[pi];
        let local = match o.kind {
            ObjectKind::SkinnedMesh => skinned_box(model, scene, o),
            ObjectKind::InstancedMesh => {
                let g = geometry_box(p);
                let mut b = Box3::default();
                for m in model.nodes[o.instances.expect("instanced")]
                    .instances
                    .iter()
                    .flatten()
                {
                    let m: Mat4 = std::array::from_fn(|i| f64::from(m[i]));
                    b.union(&g.apply_matrix(&m));
                }
                b
            }
            _ => geometry_box(p),
        };
        out.union(&local.apply_matrix(&o.world));
    }
    out
}

/// The facts and normalization of a scene, or `loadGltf`'s error.
pub fn facts(model: &Model, scene: &Scene) -> Result<ImportFacts> {
    let (mut vertices, mut triangles) = (0usize, 0f64);
    for o in &scene.objects {
        if !o.kind.is_mesh() {
            continue;
        }
        let (mi, pi) = o.primitive.expect("meshes have geometry");
        let p = &model.meshes[mi].primitives[pi];
        vertices += p.positions.len();
        triangles += p.indices.as_ref().map_or(p.positions.len(), Vec::len) as f64 / 3.0;
    }
    let triangles = triangles.ceil() as usize;
    if vertices == 0 || triangles > 500_000 {
        return err("GLB has no supported geometry or exceeds 500,000 triangles");
    }
    let b = bounds(model, scene);
    let radius = b.diagonal() / 2.0;
    if !radius.is_finite() || radius < 1e-10 {
        return err("GLB model has no measurable size");
    }
    Ok(ImportFacts {
        vertices,
        triangles,
        center: b.center(),
        bounds: b,
        radius,
    })
}

/// The three.js material an object gets (GLTFLoader's `loadMaterial` and
/// `assignFinalMaterial`): its class and the parameters the renderer reads.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialFacts {
    /// `MeshStandardMaterial`, `MeshPhysicalMaterial`, `MeshBasicMaterial`,
    /// `PointsMaterial`, or `LineBasicMaterial`.
    pub class: &'static str,
    pub name: String,
    /// `FrontSide` (0) or `DoubleSide` (2).
    pub side: u8,
    pub transparent: bool,
    pub opacity: f32,
    pub alpha_test: f32,
    pub vertex_colors: bool,
    pub flat_shading: bool,
    pub color: [f32; 3],
    /// Mesh materials other than basic.
    pub emissive: Option<[f32; 3]>,
    pub emissive_intensity: Option<f32>,
    pub roughness: Option<f32>,
    pub metalness: Option<f32>,
    /// `normalScale` after GLTFLoader's tangent-space flip (y negated
    /// without a TANGENT attribute).
    pub normal_scale: Option<[f32; 2]>,
    /// Maps by three.js property name with their UV channel.
    pub maps: Vec<(&'static str, u32)>,
}

const PHYSICAL: [&str; 10] = [
    "KHR_materials_clearcoat",
    "KHR_materials_sheen",
    "KHR_materials_transmission",
    "KHR_materials_ior",
    "KHR_materials_specular",
    "KHR_materials_iridescence",
    "KHR_materials_anisotropy",
    "KHR_materials_dispersion",
    "KHR_materials_volume",
    "EXT_materials_bump",
];

/// Texture slots of the material extensions (`extendMaterialParams`'s
/// `assignTexture` calls): extension, textureInfo key, three.js property.
const EXTENSION_MAPS: [(&str, &str, &str); 13] = [
    (
        "KHR_materials_clearcoat",
        "clearcoatTexture",
        "clearcoatMap",
    ),
    (
        "KHR_materials_clearcoat",
        "clearcoatRoughnessTexture",
        "clearcoatRoughnessMap",
    ),
    (
        "KHR_materials_clearcoat",
        "clearcoatNormalTexture",
        "clearcoatNormalMap",
    ),
    ("KHR_materials_sheen", "sheenColorTexture", "sheenColorMap"),
    (
        "KHR_materials_sheen",
        "sheenRoughnessTexture",
        "sheenRoughnessMap",
    ),
    (
        "KHR_materials_transmission",
        "transmissionTexture",
        "transmissionMap",
    ),
    ("KHR_materials_volume", "thicknessTexture", "thicknessMap"),
    (
        "KHR_materials_iridescence",
        "iridescenceTexture",
        "iridescenceMap",
    ),
    (
        "KHR_materials_iridescence",
        "iridescenceThicknessTexture",
        "iridescenceThicknessMap",
    ),
    (
        "KHR_materials_specular",
        "specularTexture",
        "specularIntensityMap",
    ),
    (
        "KHR_materials_specular",
        "specularColorTexture",
        "specularColorMap",
    ),
    (
        "KHR_materials_anisotropy",
        "anisotropyTexture",
        "anisotropyMap",
    ),
    ("EXT_materials_bump", "bumpTexture", "bumpMap"),
];

/// The material facts of a geometry-bearing object.
pub fn material_facts(model: &Model, object: &Object) -> Option<MaterialFacts> {
    let (mi, pi) = object.primitive?;
    let p = &model.meshes[mi].primitives[pi];
    let def = p.material.and_then(|m| model.materials.get(m));
    let unlit = def.is_some_and(|m| m.unlit);
    let physical = def.is_some_and(|m| PHYSICAL.iter().any(|e| m.extensions.get(*e).is_some()));
    let mesh_class = if unlit {
        "MeshBasicMaterial"
    } else if physical {
        "MeshPhysicalMaterial"
    } else {
        "MeshStandardMaterial"
    };
    let class = match object.kind {
        ObjectKind::Points => "PointsMaterial",
        ObjectKind::Line | ObjectKind::LineLoop | ObjectKind::LineSegments => "LineBasicMaterial",
        _ => mesh_class,
    };
    let is_mesh = object.kind.is_mesh();
    let mut maps = vec![];
    let mut add = |name: &'static str, r: &Option<super::TextureRef>| {
        if let Some(r) = r {
            maps.push((
                name,
                r.transform.and_then(|t| t.tex_coord).unwrap_or(r.tex_coord),
            ));
        }
    };
    let (color, opacity, side, transparent, alpha_test, name);
    match def {
        Some(m) => {
            color = [m.base_color[0], m.base_color[1], m.base_color[2]];
            opacity = m.base_color[3];
            side = if m.double_sided { 2 } else { 0 };
            transparent = m.alpha_mode == super::AlphaMode::Blend;
            alpha_test = if m.alpha_mode == super::AlphaMode::Mask {
                m.alpha_cutoff
            } else {
                0.0
            };
            name = m.name.clone();
            add("map", &m.base_color_texture);
            if is_mesh && !unlit {
                add("metalnessMap", &m.metallic_roughness_texture);
                add("roughnessMap", &m.metallic_roughness_texture);
                add("normalMap", &m.normal_texture);
                add("aoMap", &m.occlusion_texture);
                add("emissiveMap", &m.emissive_texture);
                for (ext, info, name) in EXTENSION_MAPS {
                    let info = &m.extensions[ext][info];
                    if info["index"].as_u64().is_some() {
                        let channel = info["extensions"]["KHR_texture_transform"]["texCoord"]
                            .as_u64()
                            .or_else(|| info["texCoord"].as_u64())
                            .unwrap_or(0);
                        maps.push((name, channel as u32));
                    }
                }
            }
        }
        None => {
            // createDefaultMaterial.
            color = [1.0; 3];
            opacity = 1.0;
            side = 0;
            transparent = false;
            alpha_test = 0.0;
            name = String::new();
        }
    }
    let standard = is_mesh && !unlit;
    let normal_scale = (standard && def.is_some_and(|m| m.normal_texture.is_some())).then(|| {
        let s = def.map_or(1.0, |m| m.normal_scale);
        [s, if p.tangents.is_none() { -s } else { s }]
    });
    maps.sort();
    Some(MaterialFacts {
        class,
        name,
        side,
        transparent,
        opacity,
        alpha_test,
        vertex_colors: p.colors.is_some(),
        flat_shading: p.normals.is_none(),
        color,
        emissive: standard.then(|| def.map_or([0.0; 3], |m| m.emissive)),
        emissive_intensity: standard.then(|| def.map_or(1.0, |m| m.emissive_strength)),
        roughness: standard.then(|| def.map_or(1.0, |m| m.roughness)),
        metalness: standard.then(|| def.map_or(1.0, |m| m.metallic)),
        normal_scale,
        maps,
    })
}

const MAX_BYTES: usize = 128 * 1024 * 1024;

fn data_uri_allowed(uri: &str) -> bool {
    let Some(rest) = uri.strip_prefix("data:") else {
        return false;
    };
    let Some((mime, b64)) = rest.split_once(";base64,") else {
        return false;
    };
    let mime = mime.to_ascii_lowercase();
    matches!(
        mime.as_str(),
        "application/octet-stream"
            | "application/gltf-buffer"
            | "image/png"
            | "image/jpeg"
            | "image/webp"
    ) && b64.bytes().all(|c| {
        c.is_ascii_alphanumeric() || matches!(c, b'+' | b'/' | b'=') || c.is_ascii_whitespace()
    })
}

/// `loadGltf`'s checks before decoding: size, container, self-contained
/// resources, and the complexity budget.
pub fn check(bytes: &[u8]) -> Result<Value> {
    if bytes.len() > MAX_BYTES {
        return err("GLB models must be smaller than 128 MB");
    }
    let json = if bytes.len() >= 20 && bytes.starts_with(b"glTF") {
        let u = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap()) as usize;
        if u(4) != 2 || u(8) != bytes.len() {
            return err("Invalid GLB container");
        }
        let length = u(12);
        if u(16) != 0x4e4f_534a || 20 + length > bytes.len() {
            return err("Invalid GLB scene chunk");
        }
        serde_json::from_slice::<Value>(&bytes[20..20 + length])
            .map_err(|e| Error(format!("GLB JSON: {e}")))?
    } else {
        serde_json::from_slice::<Value>(bytes).map_err(|e| Error(format!("glTF JSON: {e}")))?
    };
    for dep in json["buffers"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(json["images"].as_array().into_iter().flatten())
    {
        if let Some(uri) = dep["uri"].as_str()
            && !data_uri_allowed(uri)
        {
            return err(
                "Use a self-contained GLB with embedded textures and geometry. External glTF files are not loaded.",
            );
        }
    }
    for a in json["accessors"].as_array().into_iter().flatten() {
        let ok = a["count"].as_u64().is_some_and(|c| c <= 1_500_000);
        if !ok {
            return err("Invalid or oversized glTF geometry");
        }
    }
    let mut predicted = 0u64;
    for m in json["meshes"].as_array().into_iter().flatten() {
        for p in m["primitives"].as_array().into_iter().flatten() {
            if let Some(i) = p["attributes"]["POSITION"].as_u64() {
                predicted += json["accessors"][i as usize]["count"].as_u64().unwrap_or(0);
            }
        }
    }
    if predicted > 1_000_000 || json["nodes"].as_array().map_or(0, Vec::len) > 4096 {
        return err("GLB exceeds the scene complexity budget");
    }
    Ok(json)
}

/// Today's import of a GLB or self-contained glTF: the checks, the decode,
/// the object graph, and the facts.
pub fn import(bytes: &[u8]) -> Result<(Model, Scene, ImportFacts)> {
    check(bytes)?;
    // Which files load is decided once, by the reader the renderer and the
    // picks use (`crate::gltf::load`: GLTFLoader's acceptance and
    // `loadGltf`'s checks), so `assets.get().object3d` exists exactly when
    // the engine draws the model.
    crate::gltf::load(bytes).map_err(|e| super::Error(e.0))?;
    let model = super::load(bytes)?;
    let scene = build(&model);
    let facts = facts(&model, &scene)?;
    Ok((model, scene, facts))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_sanitize_and_stay_unique() {
        assert_eq!(
            sanitize("Transmission, volume, IOR"),
            "Transmission,_volume,_IOR"
        );
        assert_eq!(sanitize("a.b[c]:d/e"), "abcde");
        let mut n = Names(HashMap::new());
        assert_eq!(n.unique("Points and lines"), "Points_and_lines");
        assert_eq!(n.unique("Points and lines"), "Points_and_lines_1");
        assert_eq!(n.unique("Points and lines"), "Points_and_lines_2");
    }

    #[test]
    fn checks_match_load_gltf() {
        assert_eq!(
            check(b"glTF\x01\0\0\0\x14\0\0\0\0\0\0\0JSON")
                .unwrap_err()
                .0,
            "Invalid GLB container"
        );
        let external =
            br#"{"asset":{"version":"2.0"},"buffers":[{"uri":"mesh.bin","byteLength":4}]}"#;
        assert!(
            check(external)
                .unwrap_err()
                .0
                .starts_with("Use a self-contained GLB")
        );
        let remote =
            br#"{"asset":{"version":"2.0"},"images":[{"uri":"data:image/gif;base64,AAAA"}]}"#;
        assert!(check(remote).is_err());
        let huge = br#"{"asset":{"version":"2.0"},"accessors":[{"count":1500001,"componentType":5126,"type":"VEC3"}]}"#;
        assert_eq!(
            check(huge).unwrap_err().0,
            "Invalid or oversized glTF geometry"
        );
        assert!(check(br#"{"asset":{"version":"2.0"}}"#).is_ok());
    }
}
