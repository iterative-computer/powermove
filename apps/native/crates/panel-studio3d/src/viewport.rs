//! The viewer's 3D model for pointer work (3d-studio.md §4–6): the camera the
//! viewer looks through, each pickable layer's world box and pivot, and the
//! projection between world space and composition pixels. Built on a worker
//! from one immutable publication; the host draws handles from it at once.
//!
//! Picking here is a box test, an interim until the engine's surface query
//! reaches the viewer. Imported models use their retained import bounds before
//! geometry loads, with a unit envelope only for older unmeasured imports.
use crate::{Result, invalid};
use pm_eval::{
    parenting::transform_aim,
    space3d::{
        Camera3d, default_camera_25d, multiply, pixel_to_scene, scene_units_per_pixel,
        uses_default_25d_camera,
    },
};
use pm_kernel::Published;
use pm_scene3d::{
    from_model,
    geometry::{AssetGeometry, NoAssets, RetainedAssets},
    navigation::{Bounds, Lens, Pose, Vec3, add, cross, dot, length, mul, normalize, sub},
    schema::{self, Role},
    to_model,
};

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: String,
    pub role: Role,
    /// True for 3D Null layers (empty scene-unit transform locators).
    pub is_null: bool,
    /// A flat 3D layer uses the ordinary selection tools through the shot.
    pub is_plane: bool,
    pub name: String,
    pub locked: bool,
    /// Evaluated world transform matrix (column-major 4×4).
    pub world: [f64; 16],
    /// Evaluated parent world transform matrix (column-major 4×4; identity when unparented).
    pub parent_world: [f64; 16],
    /// World-space box (axis aligned).
    pub bounds: Bounds,
    /// Oriented 8 world-space corners of the object's local bounding box
    /// (bit 0 = +X, bit 1 = +Y, bit 2 = +Z), so selection wireframes match
    /// rotated 3D objects tightly rather than expanding to a world AABB.
    pub box_corners: Option<[Vec3; 8]>,
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
    /// Flat 3D layers and extrusions keep composition-pixel channels (y down, z
    /// into the screen): scene units per pixel, `None` for Studio layers.
    pub pixels: Option<f64>,
}
#[derive(Clone, Debug)]
pub struct Scene {
    /// The composition's camera pose (the shot).
    pub shot: Pose,
    /// An active authored camera can store camera navigation.
    pub has_camera: bool,
    pub lens: Lens,
    /// Perspective lens retained separately from an orthographic shot camera.
    pub editor_lens: Lens,
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
    {
        let u = cross(r, f);
        let (s, c) = pose.roll.to_radians().sin_cos();
        (f, add(mul(r, c), mul(u, s)), add(mul(u, c), mul(r, -s)))
    }
}
impl Eye {
    /// Half the view's vertical extent at unit distance (perspective), or in
    /// scene units (orthographic, as the renderer's 3-unit span / zoom).
    fn half(&self) -> f64 {
        match self.lens {
            Lens::Perspective { fov, .. } => (fov.to_radians() / 2.).tan() / self.pose.zoom,
            Lens::Orthographic { height, .. } => {
                if self.pose.ortho {
                    height * 0.5 / self.pose.zoom
                } else {
                    3. / self.pose.zoom
                }
            }
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
            lens: free.map_or(self.lens, |p| {
                pm_scene3d::navigation::view_lens(p, self.editor_lens)
            }),
            width: self.width,
            height: self.height,
        }
    }
    /// Everything pickable, or only the selection, as one box.
    pub fn bounds(&self, only: Option<&[String]>) -> Option<Bounds> {
        let union = |include_nulls: bool| -> Option<Bounds> {
            let mut out: Option<Bounds> = None;
            for item in self.items.iter().filter(|i| {
                i.role == Role::Object
                    && (include_nulls || !i.is_null)
                    && only.is_none_or(|o| o.contains(&i.id))
            }) {
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
        };
        if only.is_some() {
            union(true)
        } else {
            union(false).or_else(|| union(true))
        }
    }
    /// The nearest layer the ray meets, with the face it meets.
    pub fn pick(&self, ray: &Ray) -> Option<Hit> {
        self.pick_where(ray, |_| true)
    }

    pub fn pick_where(&self, ray: &Ray, include: impl Fn(&Item) -> bool) -> Option<Hit> {
        self.items
            .iter()
            .filter(|item| include(item))
            .filter_map(|item| {
                let (t, normal) = if item.is_plane {
                    let c = item.box_corners?;
                    let (x, y) = (sub(c[1], c[0]), sub(c[2], c[0]));
                    let normal = normalize(cross(x, y));
                    let facing = dot(ray.dir, normal);
                    if facing.abs() < 1e-12 {
                        return None;
                    }
                    let t = dot(sub(c[0], ray.origin), normal) / facing;
                    let p = sub(add(ray.origin, mul(ray.dir, t)), c[0]);
                    let xx = dot(x, x);
                    let yy = dot(y, y);
                    let xy = dot(x, y);
                    let area = xx * yy - xy * xy;
                    let u = (dot(p, x) * yy - dot(p, y) * xy) / area;
                    let v = (dot(p, y) * xx - dot(p, x) * xy) / area;
                    if t < 0. || !(0. ..=1.).contains(&u) || !(0. ..=1.).contains(&v) {
                        return None;
                    }
                    (t, normal)
                } else {
                    slab(ray, item.bounds)?
                };
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
/// Whether the open composition supports 3D navigation at any playhead time.
pub fn supports_navigation(p: &Published) -> bool {
    p.snapshot.root().is_some_and(|root| {
        pm_kernel::model::comp_node(root, p.snapshot.open_comp().as_deref())
            .is_some_and(has_roles)
    })
}

/// Whether the composition supports 3D navigation, read from its layer nodes.
fn has_roles(comp: &pm_model::Node) -> bool {
    let Some(pm_model::NodeKind::Array(layers)) = comp.get("layers").map(pm_model::Node::kind)
    else {
        return false;
    };
    layers.iter().any(|layer| {
        let is_role = layer
            .get("d")
            .and_then(|d| d.get("definition"))
            .is_some_and(|definition| match definition.kind() {
                pm_model::NodeKind::String(s) => {
                    let s = s.to_string_lossy();
                    [schema::OBJECT, schema::LIGHT, schema::CAMERA].contains(&s.as_str())
                }
                _ => false,
            });
        is_role
            || matches!(
                layer.get("threeD").map(pm_model::Node::kind),
                Some(pm_model::NodeKind::Bool(true))
            )
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
/// Evaluate the same model sample for Properties selection and viewport picking.
pub fn model_part(
    snapshot: &pm_kernel::Snapshot,
    time: f64,
    layer: &str,
    key: Option<&str>,
    ray: Option<&Ray>,
    assets: &dyn AssetGeometry,
) -> Result<Option<pm_scene3d::geometry::PartPlacement>> {
    let comp = snapshot.open_comp();
    let root = snapshot.root().ok_or_else(|| invalid("No project"))?;
    let r = pm_kernel::commands::scene3d::resolve_scene(snapshot, comp.as_deref(), time)
        .map_err(|e| invalid(e.to_string()))?;
    let Some(world) = r.model_worlds.get(layer) else {
        return Ok(None);
    };
    let Some(object) = r.scene["objects"]
        .as_array()
        .and_then(|objects| objects.iter().find(|object| object["id"] == layer))
    else {
        return Ok(None);
    };
    let Some(asset) = object["source"]["assetId"].as_str() else {
        return Ok(None);
    };
    let Some(record) = root.get("assets").and_then(|assets| assets.get(asset)) else {
        return Ok(None);
    };
    let record = from_model(&record.to_value())?;
    // Native Studio samples the import mixer at composition time.
    let parts = &object["parts"];
    let result = if let Some(ray) = ray {
        assets.pick_model_part(asset, &record, *world, time, parts, ray.origin, ray.dir)
    } else if let Some(key) = key {
        assets.model_part(asset, &record, *world, time, parts, key)
    } else {
        Ok(None)
    };
    result.map_err(Into::into)
}

fn eye_of(r: &pm_scene3d::service::ResolvedScene, comp: &serde_json::Value) -> Eye {
    let cam = &r.scene["camera"];
    let n = |k: &str, d: f64| cam["p"][k]["v"].as_f64().unwrap_or(d);
    let mut pose = Pose {
        ortho: false,
        roll: 0.,
        position: r
            .camera_position
            .unwrap_or([n("x", 0.), n("y", 0.), n("z", 6.)]),
        target: r
            .camera_id
            .as_ref()
            .and_then(|id| r.targets.get(id))
            .copied()
            .unwrap_or([n("targetX", 0.), n("targetY", 0.), n("targetZ", 0.)]),
        zoom: n("zoom", 1.),
    };
    if let Some(up) = r.camera_up {
        pose.roll = Camera3d {
            position: pose.position,
            target: pose.target,
            roll: 0.,
            fov: n("fov", 45.),
            zoom: pose.zoom,
            ortho: false,
            ortho_height: 6.,
        }
        .roll_from_up(up);
    }
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
    let mut mirror = pm_kernel::QueryMirror::new(p);
    scene_with_bounds(p, time, |id| {
        mirror.queries().bounds(None, id, time).ok().flatten()
    })
}

/// Build the viewport with the host's typography-aware bounds query.
pub fn scene_with_bounds(
    p: &Published,
    time: f64,
    mut bounds_of: impl FnMut(&str) -> Option<pm_kernel::Bounds>,
) -> Result<Scene> {
    let comp_id = p.snapshot.open_comp();
    let root = p.snapshot.root().ok_or_else(|| invalid("No project"))?;
    let comp = pm_kernel::model::comp_node(root, comp_id.as_deref())
        .ok_or_else(|| invalid("No composition"))?;
    // Asked for every revision of every project: a composition with no 3D
    // layer answers from its nodes, before anything is converted.
    if !has_roles(comp) {
        return Err(invalid("No 3D layers"));
    }
    let default_25d = uses_default_25d_camera(pm_kernel::model::layers_of(comp));
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
        pose: mut shot,
        mut lens,
        width,
        height,
    } = eye_of(&r, &comp);
    let n = |key: &str, default| r.scene["camera"]["p"][key]["v"].as_f64().unwrap_or(default);
    let default_camera = default_camera_25d(width, height);
    let locked = pm_kernel::model::list_layers(root, comp_id.as_deref())
        .into_iter()
        .filter(|layer| layer.effectively_locked)
        .map(|layer| layer.id)
        .collect::<std::collections::HashSet<_>>();
    let legacy_framing = default_25d && r.camera_id.is_none();
    let editor_lens = Lens::Perspective {
        fov: if legacy_framing {
            default_camera.fov
        } else {
            n("fov", 45.)
        },
        aspect: width / height,
        near: n("near", 0.01).max(1e-4),
        far: n("far", 1000.).max(1.),
    };
    if legacy_framing {
        shot.position = default_camera.position;
        shot.target = default_camera.target;
        lens = editor_lens;
    }
    let mut mirror = pm_kernel::QueryMirror::new(p);
    let channels = |q: &mut pm_kernel::QueryContext, id: &str, keys: [&str; 3], d: f64| -> Vec3 {
        std::array::from_fn(|i| {
            q.evaluate(None, id, keys[i], time)
                .ok()
                .and_then(|v| v.as_f64())
                .unwrap_or(d)
        })
    };
    const IDENTITY: [f64; 16] = [
        1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
    ];
    let mut items = vec![];
    for (index, l) in layers.iter().enumerate() {
        if l["type"] == "null" && l["threeD"] == true {
            if l["on"] == false {
                continue;
            }
            let id = l["id"].as_str().unwrap_or_default().to_string();
            let Some(world) = r.null_worlds.get(&id) else {
                continue;
            };
            let parent_world = r.parent_worlds.get(&id).copied().unwrap_or(IDENTITY);
            let name = l["name"].as_str().unwrap_or("").to_string();
            let locked = l["lock"] == true;
            let pivot = translation(world);
            let local = Bounds {
                min: [-0.25; 3],
                max: [0.25; 3],
            };
            let transformed = pm_scene3d::geometry::transform(local, *world).unwrap_or(Bounds {
                min: [pivot[0] - 0.25, pivot[1] - 0.25, pivot[2] - 0.25],
                max: [pivot[0] + 0.25, pivot[1] + 0.25, pivot[2] + 0.25],
            });
            // Keep a minimum picking envelope even when a null is scaled tiny.
            let pad = 0.12;
            let bounds = Bounds {
                min: std::array::from_fn(|i| transformed.min[i].min(pivot[i] - pad)),
                max: std::array::from_fn(|i| transformed.max[i].max(pivot[i] + pad)),
            };
            let box_corners = (0..8)
                .map(|n| {
                    let c: Vec3 = std::array::from_fn(|i| {
                        if n & (1 << i) == 0 {
                            local.min[i]
                        } else {
                            local.max[i]
                        }
                    });
                    pm_scene3d::geometry::transform(Bounds { min: c, max: c }, *world)
                        .ok()
                        .map(|b| b.min)
                })
                .collect::<Option<Vec<Vec3>>>()
                .and_then(|v| v.try_into().ok());
            items.push(Item {
                position: channels(
                    mirror.queries(),
                    &id,
                    ["position.x", "position.y", "position.z"],
                    0.,
                ),
                rotation: channels(
                    mirror.queries(),
                    &id,
                    ["rotation.x", "rotation.y", "rotation"],
                    0.,
                ),
                scale: channels(
                    mirror.queries(),
                    &id,
                    ["scale.x", "scale.y", "scale.z"],
                    100.,
                ),
                pixels: None,
                id,
                role: Role::Object,
                is_null: true,
                is_plane: false,
                name,
                locked,
                world: *world,
                parent_world,
                bounds,
                box_corners,
                pivot,
                target: None,
                aim: None,
                light: None,
                shape: [0.; 3],
            });
            continue;
        }
        let Some(role) = schema::role(l) else {
            let Some(id) = l["id"].as_str() else { continue };
            let q = mirror.queries();
            if matches!(l["type"].as_str(), Some("group" | "audio" | "adjustment"))
                || pm_scene3d::extrusion::enabled(l)
                || !q.is_3d_layer(None, id, time).unwrap_or(false)
                || !q.active(None, id, time).unwrap_or(false)
            {
                continue;
            }
            let (Some(local), Ok(pixel_world), Ok(position_world)) = (
                bounds_of(id),
                q.world_3d(None, id, time, false),
                q.world_3d(None, id, time, true),
            ) else {
                continue;
            };
            let position = channels(q, id, ["position.x", "position.y", "position.z"], 0.);
            let bridge = pixel_to_scene(width, height);
            let world = multiply(&bridge, &pixel_world);
            let parent = multiply(
                &position_world,
                &pm_eval::space3d::translation(0., 0., -position[2]),
            );
            let parent_world = multiply(&bridge, &parent);
            let local = Bounds {
                min: [local.x0, local.y0, 0.],
                max: [local.x1, local.y1, 0.],
            };
            let Ok(bounds) = pm_scene3d::geometry::transform(local, world) else {
                continue;
            };
            let corners = std::array::from_fn(|n| {
                let p = std::array::from_fn(|i| {
                    if n & (1 << i) == 0 {
                        local.min[i]
                    } else {
                        local.max[i]
                    }
                });
                transform_aim(&world, &p)
            });
            items.push(Item {
                id: id.into(),
                role: Role::Object,
                is_null: false,
                is_plane: true,
                name: l["name"].as_str().unwrap_or_default().into(),
                locked: locked.contains(id),
                world,
                parent_world,
                bounds,
                box_corners: Some(corners),
                pivot: transform_aim(&parent_world, &position),
                target: None,
                aim: None,
                light: None,
                shape: [0.; 3],
                position,
                rotation: channels(q, id, ["rotation.x", "rotation.y", "rotation"], 0.),
                scale: channels(q, id, ["scale.x", "scale.y", "scale.z"], 100.),
                pixels: Some(scene_units_per_pixel(height)),
            });
            continue;
        };
        let id = l["id"].as_str().unwrap_or_default().to_string();
        let data = &l["d"]["data"];
        let name = l["name"].as_str().unwrap_or("").to_string();
        let locked = l["lock"] == true;
        let parent_world = r.parent_worlds.get(&id).copied().unwrap_or(IDENTITY);
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
                let box_corners = (0..8)
                    .map(|n| {
                        let c: Vec3 = std::array::from_fn(|i| {
                            if n & (1 << i) == 0 {
                                local.min[i]
                            } else {
                                local.max[i]
                            }
                        });
                        pm_scene3d::geometry::transform(Bounds { min: c, max: c }, *world)
                            .ok()
                            .map(|b| b.min)
                    })
                    .collect::<Option<Vec<Vec3>>>()
                    .and_then(|v| v.try_into().ok());
                items.push(Item {
                    position: channels(
                        mirror.queries(),
                        &id,
                        ["position.x", "position.y", "position.z"],
                        0.,
                    ),
                    rotation: channels(
                        mirror.queries(),
                        &id,
                        ["rotation.x", "rotation.y", "rotation"],
                        0.,
                    ),
                    scale: channels(
                        mirror.queries(),
                        &id,
                        ["scale.x", "scale.y", "scale.z"],
                        100.,
                    ),
                    pixels: None,
                    id,
                    role,
                    is_null: false,
                    is_plane: false,
                    name,
                    locked,
                    world: *world,
                    parent_world,
                    bounds,
                    box_corners,
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
                    position: channels(
                        mirror.queries(),
                        &id,
                        ["position.x", "position.y", "position.z"],
                        0.,
                    ),
                    rotation: channels(
                        mirror.queries(),
                        &id,
                        ["rotation.x", "rotation.y", "rotation"],
                        0.,
                    ),
                    scale: [100.; 3],
                    pixels: None,
                    id: id.clone(),
                    role,
                    is_null: false,
                    is_plane: false,
                    name,
                    locked,
                    world: *world,
                    parent_world,
                    bounds: Bounds {
                        min: [pivot[0] - s, pivot[1] - s, pivot[2] - s],
                        max: [pivot[0] + s, pivot[1] + s, pivot[2] + s],
                    },
                    box_corners: None,
                    pivot,
                    target: r.targets.get(&id).copied(),
                    aim: Some(light_values(
                        p,
                        index,
                        &data["light"]["p"],
                        time,
                        ["targetX", "targetY", "targetZ"],
                    )),
                    light: data["light"]["type"].as_str().map(str::to_owned),
                    shape: light_values(
                        p,
                        index,
                        &data["light"]["p"],
                        time,
                        ["width", "height", "angle"],
                    ),
                });
            }
            Role::Camera => {}
        }
    }
    // Empty scenes still have an editor camera. Layer timing and visibility
    // only affect the items available for selection and framing.
    Ok(Scene {
        shot,
        has_camera: r.camera_id.is_some(),
        lens,
        editor_lens,
        width,
        height,
        items,
        revision: p.revision(),
        time,
    })
}
/// A light's own channels (`light.<key>`) at the time, through the kernel's
/// property evaluation as the light tool reads them; 0 where unset.
fn light_values(
    p: &Published,
    index: usize,
    props: &serde_json::Value,
    time: f64,
    keys: [&str; 3],
) -> Vec3 {
    let mut mirror = pm_kernel::QueryMirror::new(p);
    let open = p.snapshot.open_comp();
    std::array::from_fn(|i| {
        let q = serde_json::json!({"op":"evP","comp":open,"open":open,"layer":index,"t":time,
            "key":format!("light.{}", keys[i]),"prop":props[keys[i]]});
        mirror
            .queries()
            .query(&to_model(&q))
            .ok()
            .and_then(|v| from_model(&v).ok())
            .and_then(|v| v.as_f64())
            .unwrap_or(0.)
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
    fn grouped_25d_scene_keeps_its_editor_camera_outside_the_layers_span() {
        let p = serde_json::json!({
            "id": "p", "compId": "c", "w": 640, "h": 360, "fps": 30, "dur": 5,
            "layers": [
                {"id":"card","type":"group","threeD":true,"on":true,"from":0,"dur":2,"p":{}},
                {"id":"child","type":"shape","group":"card","on":true,"from":0,"dur":2,
                 "p":{},"d":{"w":120,"h":80}},
                {"id":"control","type":"null","threeD":true,"on":true,"from":0,"dur":2,"p":{}}
            ]
        });
        let actor = pm_kernel::KernelActor::spawn(pm_kernel::KernelConfig::default());
        let kernel = actor.handle();
        kernel
            .open("p", pm_kernel::OpenSource::Untitled(pm_scene3d::to_model(&p)))
            .wait()
            .unwrap()
            .unwrap();
        let published = kernel.published("p").unwrap();
        assert!(supports_navigation(&published));

        let active = scene(&published, 0.5).unwrap();
        assert!(!active.has_camera);
        assert!(active.items.iter().any(|item| item.id == "control"));
        let empty = scene(&published, 3.).unwrap();
        assert!(!empty.has_camera);
        assert!(empty.items.is_empty());
        assert_eq!(empty.shot, active.shot);
        assert_eq!(empty.shot.position, default_camera_25d(640., 360.).position);
    }

    #[test]
    fn flat_3d_layers_enable_navigation_with_the_legacy_eye() {
        let value = serde_json::json!({"w":800,"h":500,"layers":[{"id":"flat","type":"solid","threeD":true}]});
        let root =
            pm_model::Node::from_value(pm_model::json::parse(&value.to_string()).unwrap(), 1);
        assert!(has_roles(&root));
        assert!(uses_default_25d_camera(pm_kernel::model::layers_of(&root)));
        let camera = default_camera_25d(800., 500.);
        let eye = Eye {
            pose: Pose {
                position: camera.position,
                target: camera.target,
                roll: camera.roll,
                ortho: camera.ortho,
                zoom: camera.zoom,
            },
            lens: Lens::Perspective {
                fov: camera.fov,
                aspect: 800. / 500.,
                near: 0.01,
                far: 1000.,
            },
            width: 800.,
            height: 500.,
        };
        let world = transform_aim(&pixel_to_scene(800., 500.), &[120., 75., 0.]);
        let (at, _) = eye.project(world).unwrap();
        assert!((at[0] - 120.).abs() < 1e-9);
        assert!((at[1] - 75.).abs() < 1e-9);
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
                ortho: false,
                roll: 0.,
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
            is_null: false,
            is_plane: false,
            name: id.into(),
            locked: false,
            bounds: b(z),
            box_corners: None,
            pivot: [0., 0., z],
            target: None,
            aim: None,
            light: None,
            shape: [0.; 3],
            position: [0., 0., z],
            rotation: [0.; 3],
            scale: [100.; 3],
            world: [1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.],
            parent_world: [1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.],
            pixels: None,
        };
        let scene = Scene {
            shot: e.pose,
            has_camera: true,
            lens: e.lens,
            editor_lens: e.lens,
            width: 1920.,
            height: 1080.,
            items: vec![item("back", -3.), item("front", 0.)],
            revision: 1,
            time: 0.,
        };
        for pose in [
            e.pose,
            Pose {
                ortho: true,
                ..e.pose
            },
            Pose {
                ortho: true,
                roll: 15.,
                ..e.pose
            },
        ] {
            let e = scene.eye(Some(pose));
            let hit = scene.pick(&e.ray([960., 540.])).unwrap();
            assert_eq!(hit.id, "front");
            assert!(scene.pick(&e.ray([10., 10.])).is_none());
        }
        let hit = scene.pick(&e.ray([960., 540.])).unwrap();
        assert_eq!(hit.id, "front");
        assert_eq!(hit.normal, [0., 0., 1.]);
        assert!((hit.point[2] - 0.5).abs() < 1e-9);
        assert!(scene.pick(&e.ray([10., 10.])).is_none());
    }
    #[test]
    fn ortho_picking_projection_and_roll_share_the_same_basis() {
        let pose = Pose {
            position: [0., 0., 10.],
            target: [0.; 3],
            zoom: 1.,
            ortho: false,
            roll: 0.,
        };
        let scene = Scene {
            editor_lens: Lens::Perspective {
                fov: 45.,
                aspect: 1.,
                near: 0.01,
                far: 1000.,
            },
            shot: pose,
            has_camera: true,
            lens: Lens::Perspective {
                fov: 45.,
                aspect: 1.,
                near: 0.01,
                far: 1000.,
            },
            width: 1000.,
            height: 1000.,
            items: vec![],
            revision: 0,
            time: 0.,
        };
        let p = [1., 1., 0.];
        let a = scene.eye(Some(pose));
        let b = scene.eye(Some(Pose {
            ortho: true,
            ..pose
        }));
        let pa = a.project(p).unwrap().0;
        let pb = b.project(p).unwrap().0;
        assert!((pa[0] - pb[0]).abs() < 1e-9 && (pa[1] - pb[1]).abs() < 1e-9);
        for e in [
            a,
            b,
            scene.eye(Some(Pose {
                ortho: true,
                roll: 15.,
                ..pose
            })),
        ] {
            let px = e.project(p).unwrap().0;
            let ray = e.ray(px);
            let t = -ray.origin[2] / ray.dir[2];
            let hit = add(ray.origin, mul(ray.dir, t));
            assert!(length(sub(hit, p)) < 1e-9);
            assert!(e.units_per_pixel(p) > 0.);
        }
        let ortho_shot = Scene {
            lens: Lens::Orthographic {
                width: 1000.,
                height: 1000.,
            },
            ..scene.clone()
        };
        assert!(matches!(
            ortho_shot.eye(None).lens,
            Lens::Orthographic { .. }
        ));
        assert!(matches!(
            ortho_shot.eye(Some(pose)).lens,
            Lens::Perspective { .. }
        ));
        let projected = ortho_shot
            .eye(Some(Pose {
                ortho: true,
                ..pose
            }))
            .project(p)
            .unwrap()
            .0;
        assert!((projected[0] - pb[0]).abs() < 1e-9 && (projected[1] - pb[1]).abs() < 1e-9);
        let rolled = scene.eye(Some(Pose { roll: 90., ..pose }));
        assert!(length(sub(rolled.right_up().0, [0., 1., 0.])) < 1e-9);
    }
    #[test]
    fn bounds_prefers_objects_for_scene_framing_and_frames_selected_nulls() {
        let e = eye();
        let identity = [
            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ];
        let mk = |id: &str, is_null: bool, x: f64| Item {
            id: id.into(),
            role: Role::Object,
            is_null,
            is_plane: false,
            name: id.into(),
            locked: false,
            bounds: Bounds {
                min: [x - 0.5, -0.5, -0.5],
                max: [x + 0.5, 0.5, 0.5],
            },
            box_corners: None,
            pivot: [x, 0., 0.],
            target: None,
            aim: None,
            light: None,
            shape: [0.; 3],
            position: [x, 0., 0.],
            rotation: [0.; 3],
            scale: [100.; 3],
            world: identity,
            parent_world: identity,
            pixels: None,
        };
        let scene = Scene {
            shot: e.pose,
            has_camera: true,
            lens: e.lens,
            editor_lens: e.lens,
            width: 1920.,
            height: 1080.,
            items: vec![mk("cube", false, 0.), mk("rig", true, 20.)],
            revision: 1,
            time: 0.,
        };
        // Whole-scene framing focuses on objects, not distant rig nulls.
        let all = scene.bounds(None).unwrap();
        assert!((all.max[0] - 0.5).abs() < 1e-9, "{all:?}");
        // Explicitly framing the selected null frames its locator bounds.
        let sel = scene.bounds(Some(&["rig".to_string()])).unwrap();
        assert!((sel.min[0] - 19.5).abs() < 1e-9 && (sel.max[0] - 20.5).abs() < 1e-9, "{sel:?}");
        // When a scene has only 3D nulls, whole-scene framing falls back to them.
        let null_only = Scene {
            items: vec![mk("rig", true, 5.)],
            ..scene
        };
        let fallback = null_only.bounds(None).unwrap();
        assert!((fallback.min[0] - 4.5).abs() < 1e-9, "{fallback:?}");
    }
}
