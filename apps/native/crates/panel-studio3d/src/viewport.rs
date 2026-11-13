//! The viewer's 3D model for pointer work (3d-studio.md §4–6): the camera the
//! viewer looks through, each pickable layer's world box and pivot, and the
//! projection between world space and composition pixels. Built on a worker
//! from one immutable publication; the host draws handles from it at once.
//!
//! Picking here is a box test, an interim until the engine's surface query
//! reaches the viewer. Imported models use their retained import bounds before
//! geometry loads, with a unit envelope only for older unmeasured imports.
use crate::{Result, invalid};
use pm_kernel::Published;
use pm_scene3d::{
    from_model,
    geometry::{AssetGeometry, NoAssets, RetainedAssets},
    navigation::{Bounds, Lens, Pose, Vec3, add, cross, dot, length, mul, normalize, sub},
    schema::{self, Role},
};

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: String,
    pub role: Role,
    pub name: String,
    pub locked: bool,
    /// World-space box (axis aligned).
    pub bounds: Bounds,
    /// Where handles sit: the layer's world origin.
    pub pivot: Vec3,
    /// The light's aim point, for lights (world space).
    pub target: Option<Vec3>,
    /// The light's target channels (`light.targetX/Y/Z`), which an aim drag
    /// edits by the same world delta it moves the point.
    pub aim: Option<Vec3>,
    pub light: Option<String>,
    /// A light's emitter: softbox width and height, and a spotlight's spread
    /// (degrees). Zeros for everything else.
    pub shape: Vec3,
    /// Evaluated channels at the time: position (scene units), rotation
    /// x/y/z (degrees) and scale x/y/z (percent).
    pub position: Vec3,
    pub rotation: Vec3,
    pub scale: Vec3,
    /// Extruded text and shapes keep composition-pixel channels (y down, z
    /// into the screen): scene units per pixel, `None` for Studio layers.
    pub pixels: Option<f64>,
}
#[derive(Clone, Debug)]
pub struct Scene {
    /// The composition's camera pose (the shot).
    pub shot: Pose,
    pub lens: Lens,
    pub width: f64,
    pub height: f64,
    pub items: Vec<Item>,
    pub revision: u64,
    pub time: f64,
}
/// One camera, as the viewer shows it: the shot, or a Free View pose.
#[derive(Clone, Copy, Debug)]
pub struct Eye {
    pub pose: Pose,
    pub lens: Lens,
    pub width: f64,
    pub height: f64,
}
pub struct Ray {
    pub origin: Vec3,
    pub dir: Vec3,
}
pub struct Hit {
    pub id: String,
    pub point: Vec3,
    pub normal: Vec3,
    pub distance: f64,
}
fn basis(pose: Pose) -> (Vec3, Vec3, Vec3) {
    let f = normalize(sub(pose.target, pose.position));
    let mut r = cross(f, [0., 1., 0.]);
    if length(r) < 1e-6 {
        r = [1., 0., 0.];
    }
    let r = normalize(r);
    (f, r, cross(r, f))
}
impl Eye {
    /// Half the view's vertical extent at unit distance (perspective), or in
    /// scene units (orthographic, as the renderer's 3-unit span / zoom).
    fn half(&self) -> f64 {
        match self.lens {
            Lens::Perspective { fov, .. } => (fov.to_radians() / 2.).tan() / self.pose.zoom,
            Lens::Orthographic { .. } => 3. / self.pose.zoom,
        }
    }
    fn ortho(&self) -> bool {
        matches!(self.lens, Lens::Orthographic { .. })
    }
    /// Composition pixels and depth, or `None` behind the camera.
    pub fn project(&self, p: Vec3) -> Option<([f64; 2], f64)> {
        let (f, r, u) = basis(self.pose);
        let d = sub(p, self.pose.position);
        let z = dot(d, f);
        let aspect = self.width / self.height;
        let h = self.half();
        let (x, y) = if self.ortho() {
            (dot(d, r) / (h * aspect), dot(d, u) / h)
        } else {
            if z <= 1e-4 {
                return None;
            }
            (dot(d, r) / (z * h * aspect), dot(d, u) / (z * h))
        };
        Some((
            [(x * 0.5 + 0.5) * self.width, (0.5 - y * 0.5) * self.height],
            z,
        ))
    }
    /// The ray through a composition pixel.
    pub fn ray(&self, px: [f64; 2]) -> Ray {
        let (f, r, u) = basis(self.pose);
        let aspect = self.width / self.height;
        let h = self.half();
        let x = (px[0] / self.width - 0.5) * 2.;
        let y = (0.5 - px[1] / self.height) * 2.;
        if self.ortho() {
            Ray {
                origin: add(
                    self.pose.position,
                    add(mul(r, x * h * aspect), mul(u, y * h)),
                ),
                dir: f,
            }
        } else {
            Ray {
                origin: self.pose.position,
                dir: normalize(add(f, add(mul(r, x * h * aspect), mul(u, y * h)))),
            }
        }
    }
    /// World units per composition pixel at a point (for handle sizes).
    pub fn units_per_pixel(&self, p: Vec3) -> f64 {
        let (f, _, _) = basis(self.pose);
        let z = if self.ortho() {
            1.
        } else {
            dot(sub(p, self.pose.position), f).max(1e-3)
        };
        2. * self.half() * z / self.height
    }
    /// Screen right and up, for pan.
    pub fn right_up(&self) -> (Vec3, Vec3) {
        let (_, r, u) = basis(self.pose);
        (r, u)
    }
}
impl Scene {
    pub fn eye(&self, free: Option<Pose>) -> Eye {
        Eye {
            pose: free.unwrap_or(self.shot),
            lens: self.lens,
            width: self.width,
            height: self.height,
        }
    }
    /// Everything pickable, or only the selection, as one box.
    pub fn bounds(&self, only: Option<&[String]>) -> Option<Bounds> {
        let mut out: Option<Bounds> = None;
        for item in self
            .items
            .iter()
            .filter(|i| i.role == Role::Object && only.is_none_or(|o| o.contains(&i.id)))
        {
            let b = item.bounds;
            out = Some(match out {
                None => b,
                Some(o) => Bounds {
                    min: std::array::from_fn(|i| o.min[i].min(b.min[i])),
                    max: std::array::from_fn(|i| o.max[i].max(b.max[i])),
                },
            });
        }
        out
    }
    /// The nearest layer the ray meets, with the face it meets.
    pub fn pick(&self, ray: &Ray) -> Option<Hit> {
        self.items
            .iter()
            .filter_map(|item| {
                let (t, normal) = slab(ray, item.bounds)?;
                Some(Hit {
                    id: item.id.clone(),
                    point: add(ray.origin, mul(ray.dir, t)),
                    normal,
                    distance: t,
                })
            })
            .min_by(|a, b| a.distance.total_cmp(&b.distance))
    }
}
/// Ray–box entry distance and the entered face's normal.
fn slab(ray: &Ray, b: Bounds) -> Option<(f64, Vec3)> {
    let (mut near, mut far) = (f64::NEG_INFINITY, f64::INFINITY);
    let mut axis = 0;
    for i in 0..3 {
        let d = ray.dir[i];
        if d.abs() < 1e-12 {
            if ray.origin[i] < b.min[i] || ray.origin[i] > b.max[i] {
                return None;
            }
            continue;
        }
        let (mut t0, mut t1) = (
            (b.min[i] - ray.origin[i]) / d,
            (b.max[i] - ray.origin[i]) / d,
        );
        if t0 > t1 {
            std::mem::swap(&mut t0, &mut t1);
        }
        if t0 > near {
            near = t0;
            axis = i;
        }
        far = far.min(t1);
        if near > far {
            return None;
        }
    }
    if far < 0. {
        return None;
    }
    let t = near.max(0.);
    let mut normal = [0.; 3];
    normal[axis] = -ray.dir[axis].signum();
    Some((t, normal))
}
/// Where the matrix puts the local origin (convention-free).
fn translation(m: &[f64; 16]) -> Vec3 {
    pm_scene3d::geometry::transform(
        Bounds {
            min: [0.; 3],
            max: [0.; 3],
        },
        *m,
    )
    .map_or([0.; 3], |b| b.min)
}
/// Whether a composition node has a 3D object, light or camera layer
/// (`schema::role`, read from the nodes).
fn has_roles(comp: &pm_model::Node) -> bool {
    let Some(pm_model::NodeKind::Array(layers)) = comp.get("layers").map(pm_model::Node::kind)
    else {
        return false;
    };
    layers.iter().any(|layer| {
        layer
            .get("d")
            .and_then(|d| d.get("definition"))
            .is_some_and(|definition| match definition.kind() {
                pm_model::NodeKind::String(s) => {
                    let s = s.to_string_lossy();
                    [schema::OBJECT, schema::LIGHT, schema::CAMERA].contains(&s.as_str())
                }
                _ => false,
            })
    })
}
/// Read on a worker from one publication at `time`.
/// The part of the imported model on layer `layer` the eye ray meets first
/// (3d-studio.md: a double-click picks a model's part). `Ok(None)`: the ray
/// misses the model, the layer isn't an imported model, or its format has no
/// material slots.
pub fn pick_part(
    snapshot: &pm_kernel::Snapshot,
    time: f64,
    layer: &str,
    ray: &Ray,
    assets: &dyn AssetGeometry,
) -> Result<Option<pm_scene3d::geometry::PartHit>> {
    let comp_id = snapshot.open_comp();
    let root = snapshot.root().ok_or_else(|| invalid("No project"))?;
    let comp = pm_kernel::model::comp_node(root, comp_id.as_deref())
        .ok_or_else(|| invalid("No composition"))?;
    let comp = from_model(&comp.to_value())?;
    let Some(l) = comp["layers"]
        .as_array()
        .and_then(|layers| layers.iter().find(|l| l["id"] == layer))
    else {
        return Ok(None);
    };
    let Some(asset) = l["d"]["data"]["object"]["source"]["assetId"].as_str() else {
        return Ok(None);
    };
    let r = pm_kernel::commands::scene3d::resolve_scene(snapshot, comp_id.as_deref(), time)
        .map_err(|e| invalid(e.to_string()))?;
    let Some(world) = r.model_worlds.get(layer) else {
        return Ok(None);
    };
    let records = from_model(
        &root
            .get("assets")
            .map(pm_model::Node::to_value)
            .unwrap_or(pm_model::Value::Null),
    )?;
    assets
        .part_at(
            asset,
            &records[asset],
            *world,
            time - l["from"].as_f64().unwrap_or(0.),
            ray.origin,
            ray.dir,
        )
        .map_err(|e| invalid(e.to_string()))
}
/// The composition camera's eye (the shot), as `scene` sees it.
fn eye_of(r: &pm_scene3d::service::ResolvedScene, comp: &serde_json::Value) -> Eye {
    let cam = &r.scene["camera"];
    let n = |k: &str, d: f64| cam["p"][k]["v"].as_f64().unwrap_or(d);
    let pose = Pose {
        position: r
            .camera_position
            .unwrap_or([n("x", 3.), n("y", 2.), n("z", 6.)]),
        target: r
            .camera_id
            .as_ref()
            .and_then(|id| r.targets.get(id))
            .copied()
            .unwrap_or([n("targetX", 0.), n("targetY", 0.), n("targetZ", 0.)]),
        zoom: n("zoom", 1.),
    };
    let width = comp["w"].as_f64().unwrap_or(1920.);
    let height = comp["h"].as_f64().unwrap_or(1080.);
    let lens = if cam["projection"] == "orthographic" {
        Lens::Orthographic { width, height }
    } else {
        Lens::Perspective {
            fov: n("fov", 45.),
            aspect: width / height,
            near: n("near", 0.01).max(1e-4),
            far: n("far", 1000.).max(1.),
        }
    };
    Eye {
        pose,
        lens,
        width,
        height,
    }
}
/// The imported-model part under composition pixel `at` through the shot
/// camera, nearest first across every model layer: `(layer, hit)`.
pub fn pick_part_at(
    snapshot: &pm_kernel::Snapshot,
    time: f64,
    at: [f64; 2],
    assets: &dyn AssetGeometry,
) -> Result<Option<(String, pm_scene3d::geometry::PartHit)>> {
    let comp_id = snapshot.open_comp();
    let root = snapshot.root().ok_or_else(|| invalid("No project"))?;
    let comp = pm_kernel::model::comp_node(root, comp_id.as_deref())
        .ok_or_else(|| invalid("No composition"))?;
    let comp = from_model(&comp.to_value())?;
    let r = pm_kernel::commands::scene3d::resolve_scene(snapshot, comp_id.as_deref(), time)
        .map_err(|e| invalid(e.to_string()))?;
    let ray = eye_of(&r, &comp).ray(at);
    let mut best: Option<(String, pm_scene3d::geometry::PartHit)> = None;
    for l in comp["layers"].as_array().into_iter().flatten() {
        let id = l["id"].as_str().unwrap_or_default();
        if l["on"] == false || !r.model_worlds.contains_key(id) {
            continue;
        }
        if let Some(hit) = pick_part(snapshot, time, id, &ray, assets)?
            && best.as_ref().is_none_or(|(_, b)| hit.distance < b.distance)
        {
            best = Some((id.to_owned(), hit));
        }
    }
    Ok(best)
}
pub fn scene(p: &Published, time: f64) -> Result<Scene> {
    let comp_id = p.snapshot.open_comp();
    let root = p.snapshot.root().ok_or_else(|| invalid("No project"))?;
    let comp = pm_kernel::model::comp_node(root, comp_id.as_deref())
        .ok_or_else(|| invalid("No composition"))?;
    // Asked for every revision of every project: a composition with no 3D
    // layer answers from its nodes, before anything is converted.
    if !has_roles(comp) {
        return Err(invalid("No 3D layers"));
    }
    let comp = from_model(&comp.to_value())?;
    let layers = comp["layers"].as_array().cloned().unwrap_or_default();
    let r = pm_kernel::commands::scene3d::resolve_scene(&p.snapshot, comp_id.as_deref(), time)
        .map_err(|e| invalid(e.to_string()))?;
    let records = from_model(
        &root
            .get("assets")
            .map(pm_model::Node::to_value)
            .unwrap_or(pm_model::Value::Null),
    )?;
    let Eye {
        pose: shot,
        lens,
        width,
        height,
    } = eye_of(&r, &comp);
    let mut mirror = pm_kernel::QueryMirror::new(p);
    let mut channels = |id: &str, keys: [&str; 3], d: f64| -> Vec3 {
        std::array::from_fn(|i| {
            mirror
                .queries()
                .evaluate(None, id, keys[i], time)
                .ok()
                .and_then(|v| v.as_f64())
                .unwrap_or(d)
        })
    };
    let mut items = vec![];
    for l in &layers {
        let Some(role) = schema::role(l) else {
            continue;
        };
        let id = l["id"].as_str().unwrap_or_default().to_string();
        let data = &l["d"]["data"];
        let name = l["name"].as_str().unwrap_or("").to_string();
        let locked = l["lock"] == true;
        match role {
            Role::Object => {
                // Environments (sky, fog, terrain, scatter) aren't picked in
                // the viewer: clicking the ground shouldn't select the world.
                if crate::fields::environment(data).is_some() || l["on"] == false {
                    continue;
                }
                let Some(world) = r.model_worlds.get(&id) else {
                    continue;
                };
                let source = &data["object"]["source"];
                let local = pm_scene3d::geometry::local_bounds(
                    source,
                    if source.get("assetId").is_some() {
                        &RetainedAssets as &dyn AssetGeometry
                    } else {
                        &NoAssets
                    },
                    &records,
                    time - l["from"].as_f64().unwrap_or(0.),
                )
                .unwrap_or(Bounds {
                    min: [-0.5; 3],
                    max: [0.5; 3],
                });
                let Ok(bounds) = pm_scene3d::geometry::transform(local, *world) else {
                    continue;
                };
                items.push(Item {
                    position: channels(&id, ["position.x", "position.y", "position.z"], 0.),
                    rotation: channels(&id, ["rotation.x", "rotation.y", "rotation"], 0.),
                    scale: channels(&id, ["scale.x", "scale.y", "scale.z"], 100.),
                    pixels: None,
                    id,
                    role,
                    name,
                    locked,
                    bounds,
                    pivot: translation(world),
                    target: None,
                    aim: None,
                    light: None,
                    shape: [0.; 3],
                });
            }
            Role::Light => {
                let Some(world) = r.light_worlds.get(&id) else {
                    continue;
                };
                let pivot = translation(world);
                let s = 0.18;
                items.push(Item {
                    position: channels(&id, ["position.x", "position.y", "position.z"], 0.),
                    rotation: channels(&id, ["rotation.x", "rotation.y", "rotation"], 0.),
                    scale: [100.; 3],
                    pixels: None,
                    id: id.clone(),
                    role,
                    name,
                    locked,
                    bounds: Bounds {
                        min: [pivot[0] - s, pivot[1] - s, pivot[2] - s],
                        max: [pivot[0] + s, pivot[1] + s, pivot[2] + s],
                    },
                    pivot,
                    target: r.targets.get(&id).copied(),
                    aim: Some(channels(
                        &id,
                        ["light.targetX", "light.targetY", "light.targetZ"],
                        0.,
                    )),
                    light: data["light"]["type"].as_str().map(str::to_owned),
                    shape: {
                        let w = channels(&id, ["light.width", "light.height", "light.angle"], 0.);
                        [w[0], w[1], w[2]]
                    },
                });
            }
            Role::Camera => {}
        }
    }
    Ok(Scene {
        shot,
        lens,
        width,
        height,
        items,
        revision: p.revision(),
        time,
    })
}
/// The pose Frame moves Free View to: the selection, or everything.
pub fn framed(scene: &Scene, from: Pose, selection: &[String]) -> Option<Pose> {
    let only = scene
        .items
        .iter()
        .any(|i| selection.contains(&i.id))
        .then_some(selection);
    let b = scene.bounds(only).or_else(|| scene.bounds(None))?;
    Some(pm_scene3d::navigation::frame(from, b, scene.lens))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_composition_without_3d_layers_answers_from_its_nodes() {
        let has = |layers: serde_json::Value| {
            let parsed =
                pm_model::json::parse(&serde_json::json!({"id":"c","layers":layers}).to_string())
                    .unwrap();
            has_roles(pm_model::Document::new(Some(parsed), 1).root().unwrap())
        };
        assert!(!has(serde_json::json!([])));
        assert!(!has(serde_json::json!([
            {"id":"a","type":"solid","d":{"w":10,"h":10}},
            {"id":"b","type":"extension","d":{"definition":"some.other.layer"}}
        ])));
        for definition in [schema::OBJECT, schema::LIGHT, schema::CAMERA] {
            let layers = serde_json::json!([
                {"id":"a","type":"solid","d":{}},
                {"id":"b","type":"extension","d":{"definition":definition}}
            ]);
            assert!(has(layers.clone()), "{definition}");
            // The same answer `schema::role` gives on the converted value.
            assert!(layers.as_array().unwrap().iter().any(|l| schema::role(l).is_some()));
        }
    }
    #[test]
    fn imported_outline_uses_retained_geometry_before_loading_the_mesh() {
        let record = serde_json::json!({"kind":"model", "native3dBounds":{
            "version":1, "bounds":[-0.2,-1.0,-0.2,0.2,1.0,0.2]
        }});
        let b = RetainedAssets.bounds("bottle", &record, 0.).unwrap();
        assert_eq!(b.min, [-0.2, -1., -0.2]);
        assert_eq!(b.max, [0.2, 1., 0.2]);
    }
    fn eye() -> Eye {
        Eye {
            pose: Pose {
                position: [0., 0., 10.],
                target: [0., 0., 0.],
                zoom: 1.,
            },
            lens: Lens::Perspective {
                fov: 45.,
                aspect: 16. / 9.,
                near: 0.01,
                far: 1000.,
            },
            width: 1920.,
            height: 1080.,
        }
    }
    #[test]
    fn projecting_and_casting_round_trip() {
        let e = eye();
        let (c, z) = e.project([0., 0., 0.]).unwrap();
        assert!((c[0] - 960.).abs() < 1e-6 && (c[1] - 540.).abs() < 1e-6 && (z - 10.).abs() < 1e-9);
        // Up in the world is up on screen; right is right.
        let (up, _) = e.project([0., 1., 0.]).unwrap();
        let (right, _) = e.project([1., 0., 0.]).unwrap();
        assert!(up[1] < 540. && right[0] > 960.);
        let ray = e.ray(up);
        let t = (0. - ray.origin[2]) / ray.dir[2];
        let back = add(ray.origin, mul(ray.dir, t));
        assert!((back[1] - 1.).abs() < 1e-9 && back[0].abs() < 1e-9);
    }
    #[test]
    fn picking_takes_the_nearest_box_and_its_face() {
        let e = eye();
        let b = |z: f64| Bounds {
            min: [-0.5, -0.5, z - 0.5],
            max: [0.5, 0.5, z + 0.5],
        };
        let item = |id: &str, z| Item {
            id: id.into(),
            role: Role::Object,
            name: id.into(),
            locked: false,
            bounds: b(z),
            pivot: [0., 0., z],
            target: None,
            aim: None,
            light: None,
            shape: [0.; 3],
            position: [0., 0., z],
            rotation: [0.; 3],
            scale: [100.; 3],
            pixels: None,
        };
        let scene = Scene {
            shot: e.pose,
            lens: e.lens,
            width: 1920.,
            height: 1080.,
            items: vec![item("back", -3.), item("front", 0.)],
            revision: 1,
            time: 0.,
        };
        let hit = scene.pick(&e.ray([960., 540.])).unwrap();
        assert_eq!(hit.id, "front");
        assert_eq!(hit.normal, [0., 0., 1.]);
        assert!((hit.point[2] - 0.5).abs() < 1e-9);
        assert!(scene.pick(&e.ray([10., 10.])).is_none());
    }
}
