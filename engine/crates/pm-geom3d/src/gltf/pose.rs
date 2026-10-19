//! A model's object tree at one time: local transforms after the mixer,
//! world matrices (`updateMatrixWorld`), bone matrices
//! (`Skeleton.update`), morph influences, posed vertices
//! (`getVertexPosition` with `applyBoneTransform`), and bounds
//! (`Box3.setFromObject`).

use super::math::{self, Mat4, Vec3};
use super::{Drawable, Model, ObjectKind, Property, anim};

/// Local transforms and morph influences of every object.
#[derive(Debug, Clone, PartialEq)]
pub struct Pose {
    pub position: Vec<Vec3>,
    pub quaternion: Vec<[f64; 4]>,
    pub scale: Vec<Vec3>,
    /// `morphTargetInfluences` per object (empty for objects without).
    pub influences: Vec<Vec<f64>>,
}

impl Pose {
    /// The tree as loaded.
    pub fn rest(model: &Model) -> Pose {
        Pose {
            position: model.objects.iter().map(|o| o.position).collect(),
            quaternion: model.objects.iter().map(|o| o.quaternion).collect(),
            scale: model.objects.iter().map(|o| o.scale).collect(),
            influences: model
                .objects
                .iter()
                .map(|o| match &o.kind {
                    ObjectKind::Drawable(d) => d.morph_influences.clone(),
                    _ => Vec::new(),
                })
                .collect(),
        }
    }

    /// The tree after `mixer.setTime(time)` with every clip playing.
    pub fn at(model: &Model, time: f64) -> Pose {
        let mut pose = Pose::rest(model);
        for a in anim::sample(model, time) {
            let o = a.object;
            if o >= pose.position.len() {
                continue;
            }
            match a.property {
                Property::Position if a.value.len() >= 3 => {
                    pose.position[o] = [a.value[0], a.value[1], a.value[2]];
                }
                Property::Quaternion if a.value.len() >= 4 => {
                    pose.quaternion[o] = [a.value[0], a.value[1], a.value[2], a.value[3]];
                }
                Property::Scale if a.value.len() >= 3 => {
                    pose.scale[o] = [a.value[0], a.value[1], a.value[2]];
                }
                Property::MorphTargetInfluences => {
                    // `setValue_array`: as many values as the array holds.
                    let dest = &mut pose.influences[o];
                    for (k, d) in dest.iter_mut().enumerate() {
                        if let Some(v) = a.value.get(k) {
                            *d = *v;
                        }
                    }
                }
                _ => {}
            }
        }
        pose
    }

    pub fn local(&self, o: usize) -> Mat4 {
        math::compose(self.position[o], self.quaternion[o], self.scale[o])
    }
}

/// World matrices of the objects reachable from the scene group, whose
/// parent's `matrixWorld` is `parent` (`None`: no parent, so the scene
/// group's world is its local matrix). Unreachable objects get identity.
pub fn world(model: &Model, pose: &Pose, parent: Option<&Mat4>) -> Vec<Mat4> {
    let mut out = vec![math::IDENTITY; model.objects.len()];
    let root_local = pose.local(0);
    out[0] = match parent {
        Some(p) => math::multiply(p, &root_local),
        None => root_local,
    };
    let mut stack: Vec<usize> = model.objects[0].children.iter().rev().copied().collect();
    while let Some(i) = stack.pop() {
        let p = model.objects[i].parent.unwrap_or(0);
        out[i] = math::multiply(&out[p], &pose.local(i));
        for &c in model.objects[i].children.iter().rev() {
            stack.push(c);
        }
    }
    out
}

/// Whether an object and all its ancestors are visible: glTF objects are
/// always visible, so only reachability counts.
pub fn reachable(model: &Model) -> Vec<bool> {
    let mut out = vec![false; model.objects.len()];
    for i in model.traversal() {
        out[i] = true;
    }
    out
}

/// `Skeleton.update()`: `bone.matrixWorld × boneInverse` per bone, f64
/// (the uploaded `boneMatrices` are these as f32).
pub fn bone_matrices(model: &Model, skin: usize, world: &[Mat4]) -> Vec<Mat4> {
    let s = &model.skins[skin];
    s.bones
        .iter()
        .zip(&s.inverses)
        .map(|(&b, inv)| math::multiply(&world[b], inv))
        .collect()
}

/// `boneTexture`: the `boneMatrices` `Float32Array` padded to a square
/// RGBA texture of side `max(4, ceil(sqrt(4 n) / 4) · 4)`.
pub fn bone_texture(bones: &[Mat4]) -> (u32, Vec<f32>) {
    let mut size = ((bones.len() * 4) as f64).sqrt();
    size = (size / 4.0).ceil() * 4.0;
    let size = size.max(4.0) as u32;
    let mut data = vec![0f32; (size * size * 4) as usize];
    for (k, m) in bones.iter().enumerate() {
        for (e, v) in m.iter().enumerate() {
            data[k * 16 + e] = *v as f32;
        }
    }
    (size, data)
}

/// `Mesh.getVertexPosition` (relative morph targets with `influences`)
/// followed, for a skinned drawable, by `applyBoneTransform` (bind matrix
/// identity, `bindMatrixInverse` the inverse of the mesh's world matrix).
pub struct Poser<'a> {
    pub model: &'a Model,
    pub drawable: &'a Drawable,
    pub influences: &'a [f64],
    /// `bone.matrixWorld × boneInverse` per bone (skinned drawables).
    pub bones: Option<Vec<Mat4>>,
    pub bind_inverse: Mat4,
}

impl<'a> Poser<'a> {
    pub fn new(
        model: &'a Model,
        object: usize,
        drawable: &'a Drawable,
        influences: &'a [f64],
        world: &[Mat4],
    ) -> Poser<'a> {
        let bones = drawable.skin.map(|s| bone_matrices(model, s, world));
        Poser {
            model,
            drawable,
            influences,
            bones,
            bind_inverse: math::invert(&world[object]),
        }
    }

    /// The vertex in the drawable's local space.
    pub fn vertex(&self, i: usize) -> Vec3 {
        let g = &self.model.geometries[self.drawable.geometry];
        let p = &g.position.data[i * 3..i * 3 + 3];
        let mut v = [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])];
        if let Some(targets) = &g.morph.position
            && !self.influences.is_empty()
        {
            let mut m = [0.0; 3];
            for (k, t) in targets.iter().enumerate() {
                let influence = self.influences.get(k).copied().unwrap_or(0.0);
                if influence == 0.0 {
                    continue;
                }
                let d = &t.data[i * 3..i * 3 + 3];
                for c in 0..3 {
                    m[c] += f64::from(d[c]) * influence;
                }
            }
            for c in 0..3 {
                v[c] += m[c];
            }
        }
        self.apply_bone(i, v)
    }

    /// `SkinnedMesh.applyBoneTransform`, also used on `normal + position`
    /// by today's Blender snapshot (normal morphs are deliberately not read).
    pub fn apply_bone(&self, i: usize, mut v: Vec3) -> Vec3 {
        let g = &self.model.geometries[self.drawable.geometry];
        if let (Some(bones), Some(si), Some(sw)) = (&self.bones, &g.skin_index, &g.skin_weight) {
            // `applyBoneTransform` with an identity bind matrix.
            let base = math::apply(&math::IDENTITY, v);
            let mut out = [0.0; 3];
            for k in 0..4 {
                let w = f64::from(sw.data[i * 4 + k]);
                if w != 0.0 {
                    let b = si.data[i * 4 + k] as usize;
                    let Some(m) = bones.get(b) else {
                        continue;
                    };
                    let q = math::apply(m, base);
                    for c in 0..3 {
                        out[c] += q[c] * w;
                    }
                }
            }
            v = math::apply(&self.bind_inverse, out);
        }
        v
    }
}

fn expand_box(b: &mut (Vec3, Vec3), lo: Vec3, hi: Vec3, m: &Mat4) {
    if lo[0] > hi[0] || lo[1] > hi[1] || lo[2] > hi[2] {
        return;
    }
    // `Box3.applyMatrix4`: the eight corners.
    for k in 0..8 {
        let p = [
            if k & 4 == 0 { lo[0] } else { hi[0] },
            if k & 2 == 0 { lo[1] } else { hi[1] },
            if k & 1 == 0 { lo[2] } else { hi[2] },
        ];
        grow(b, math::apply(m, p));
    }
}

/// `Box3.expandByPoint` (`Math.min` and `Math.max`: a NaN coordinate
/// makes the box NaN, which `loadGltf` refuses as a model without a
/// measurable size).
fn grow(b: &mut (Vec3, Vec3), p: Vec3) {
    let js = |a: f64, v: f64, f: fn(f64, f64) -> f64| {
        if a.is_nan() || v.is_nan() {
            f64::NAN
        } else {
            f(a, v)
        }
    };
    for ((lo, hi), v) in b.0.iter_mut().zip(b.1.iter_mut()).zip(p) {
        *lo = js(*lo, v, f64::min);
        *hi = js(*hi, v, f64::max);
    }
}

/// The object-level or geometry-level bounding box `expandByObject` uses
/// for a drawable, in its local space: a skinned mesh's posed vertices
/// (`SkinnedMesh.computeBoundingBox`), an instanced mesh's instances, or
/// the geometry's box.
pub fn local_box(
    model: &Model,
    object: usize,
    d: &Drawable,
    pose: &Pose,
    world: &[Mat4],
) -> Option<(Vec3, Vec3)> {
    let g = &model.geometries[d.geometry];
    if d.skinned {
        let mut b = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        let poser = Poser::new(model, object, d, &pose.influences[object], world);
        for i in 0..g.vertex_count() {
            grow(&mut b, poser.vertex(i));
        }
        return Some(b);
    }
    let (lo, hi) = g.bounds()?;
    if let Some(inst) = &d.instances {
        let mut b = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for m in &inst.matrices {
            let m: Mat4 = std::array::from_fn(|i| f64::from(m[i]));
            expand_box(&mut b, lo, hi, &m);
        }
        return Some(b);
    }
    Some((lo, hi))
}

/// `new Box3().setFromObject(scene)` over the tree with the scene group at
/// the root (no parent).
pub fn scene_box(model: &Model, pose: &Pose) -> Option<(Vec3, Vec3)> {
    let world = world(model, pose, None);
    let mut b = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for i in model.traversal() {
        if let ObjectKind::Drawable(d) = &model.objects[i].kind
            && let Some((lo, hi)) = local_box(model, i, d, pose, &world)
        {
            expand_box(&mut b, lo, hi, &world[i]);
        }
    }
    (b.0[0] <= b.1[0] && b.0[1] <= b.1[1] && b.0[2] <= b.1[2]).then_some(b)
}

/// The asset root's chain under a layer's world matrix: the matrix the
/// glTF scene group's parent has (`root` × the fit's `1 / radius` scale),
/// and the pose with the scene group moved by −`center`.
pub fn placed(model: &Model, mut pose: Pose, layer: &Mat4) -> (Mat4, Pose) {
    let fit = model.fit;
    let normalization = math::compose([0.0; 3], [0.0, 0.0, 0.0, 1.0], [1.0 / fit.radius; 3]);
    let parent = math::multiply(layer, &normalization);
    pose.position[0] = [
        0.0 - fit.center[0],
        0.0 - fit.center[1],
        0.0 - fit.center[2],
    ];
    (parent, pose)
}
