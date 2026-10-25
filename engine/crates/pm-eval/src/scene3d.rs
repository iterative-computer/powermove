//! The evaluated 3D scene of a composition (`core/scene3d/layers.ts`
//! `compositionScene`, and the camera and light placement of
//! `core/scene3d/service.ts` `compositionRuntime`; plan sections 5 rule 10,
//! 7 "3D content", and 8).
//!
//! 3D object, light, and camera layers are ordinary layers whose content
//! (`d.data`) carries channels in the namespaces `m.`, `shader.`,
//! `slots.<id>.` (and `slots.<id>.shader.`), `light.`, `camera.`, and
//! `environment.`; [`crate::address`] finds them. This module evaluates them
//! into the scene description the renderer and Blender jobs consume:
//!
//! - every active object layer with its transform (world units, scale in
//!   percent / 100, orientation added to rotation), material, shader, and
//!   slot values; its `parent` only when the parent is an active object;
//! - every active light layer with its values and transform position;
//! - the topmost active camera layer's camera and environment, or the
//!   studio defaults (a camera at (3, 2, 6), field of view 45°);
//! - with no light layer, the studio sun (`default_sun`, at (−3, 5, 4),
//!   intensity 3, casting shadows);
//! - the camera's far plane at least 0.01 beyond its near plane.
//!
//! An evaluated channel replaces the stored value only when it has the
//! stored value's JavaScript type, is finite, and passes the channel's
//! schema (ranges and color syntax, `validateSceneChannel`); otherwise the
//! stored `v` is used, as today. Shared material channels (`shader.`,
//! `slots.<id>.shader.`) keep composition timing: their keys may lie before
//! the layer start (down to −10⁸ s local), and evaluation is ordinary
//! layer-local evaluation of those keys.
//!
//! Scene placement ([`Scope::scene_camera`], [`Scope::scene_light_aim`])
//! follows `compositionRuntime`: positions are world positions
//! (`world3D`), and aims are stored in the layer's parent space and mapped
//! through `parent3D`.
//!
//! Recipe parts (objects a model group generated) whose group's recipe is
//! animated take the recipe's geometry and base offset ([`crate::recipe`],
//! `animatedPart`); they are listed in [`EvaluatedScene::recipe_parts`].
//! Not here: `parseScene`'s geometry budget checks.

use pm_model::json::{Map, Value};
use pm_model::{JsString, Node};

use crate::js_sem::{JsResult, JsThrow, JsValue};
use crate::node;
use crate::parenting::Role3d;
use crate::scope::{Scope, js_max};

/// The object transform channels and the layer channels they read
/// (`TRANSFORM_PATHS`).
pub const TRANSFORM_PATHS: [(&str, &str); 9] = [
    ("x", "position.x"),
    ("y", "position.y"),
    ("z", "position.z"),
    ("rx", "rotation.x"),
    ("ry", "rotation.y"),
    ("rz", "rotation"),
    ("sx", "scale.x"),
    ("sy", "scale.y"),
    ("sz", "scale.z"),
];

/// A channel's schema (`scene3d/schema.ts`).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Field {
    Num(f64, f64),
    Color,
    Bool,
}

const NUM: Field = Field::Num(-100_000.0, 100_000.0);

fn material_field(key: &str) -> Option<Field> {
    Some(match key {
        "color" | "emissive" => Field::Color,
        "roughness" | "metalness" | "opacity" => Field::Num(0.0, 1.0),
        "emissiveIntensity" => Field::Num(0.0, 100.0),
        _ => return None,
    })
}

fn light_field(key: &str) -> Option<Field> {
    Some(match key {
        "x" | "y" | "z" | "targetX" | "targetY" | "targetZ" => NUM,
        "color" => Field::Color,
        "intensity" => Field::Num(0.0, 10_000.0),
        "distance" => Field::Num(0.0, 100_000.0),
        "decay" => Field::Num(0.0, 4.0),
        "angle" => Field::Num(1.0, 89.0),
        "penumbra" => Field::Num(0.0, 1.0),
        "width" | "height" => Field::Num(0.001, 10_000.0),
        "visible" => Field::Bool,
        _ => return None,
    })
}

fn camera_field(key: &str) -> Option<Field> {
    Some(match key {
        "x" | "y" | "z" | "targetX" | "targetY" | "targetZ" => NUM,
        "fov" => Field::Num(1.0, 150.0),
        "zoom" => Field::Num(0.001, 1000.0),
        "near" => Field::Num(0.0001, 10_000.0),
        "far" => Field::Num(0.01, 100_000.0),
        _ => return None,
    })
}

fn environment_field(key: &str) -> Option<Field> {
    Some(match key {
        "ambient" => Field::Num(0.0, 100.0),
        "ambientColor" => Field::Color,
        "exposure" => Field::Num(0.01, 20.0),
        _ => return None,
    })
}

/// `color`: `/^#[0-9a-f]{6}$/i`.
fn is_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

impl Field {
    fn accepts(self, v: &JsValue) -> bool {
        match (self, v) {
            (Field::Num(lo, hi), JsValue::Number(n)) => n.is_finite() && *n >= lo && *n <= hi,
            (Field::Color, JsValue::String(s)) => s.as_str().is_some_and(is_color),
            (Field::Bool, JsValue::Bool(_)) => true,
            _ => false,
        }
    }
}

/// `validateSceneChannel(path, value)` as `compositionScene` uses it:
/// `Some(field)` to check, `None` for paths it does not validate (shader,
/// slot, model, and transform channels), `Err` for unknown keys.
fn validation(path: &str) -> Result<Option<Field>, ()> {
    let (prefix, key) = match path.split_once('.') {
        Some(x) => x,
        None => return Ok(None),
    };
    if key.contains('.') {
        // `o.<id>.…`, `light.<id>.…`: not paths compositionScene evaluates.
        return Ok(None);
    }
    let field = match prefix {
        "m" => material_field(key),
        "light" => light_field(key),
        "camera" => camera_field(key),
        "environment" => environment_field(key),
        _ => return Ok(None),
    };
    field.map(Some).ok_or(())
}

/// The JavaScript `typeof` of a stored value.
fn type_of(v: &JsValue) -> &'static str {
    v.type_of()
}

/// A channel object `{ v, kf: [], expr: null }`.
fn channel(v: Value) -> Value {
    Value::Object(
        [
            (JsString::from("expr"), Value::Null),
            (JsString::from("kf"), Value::Array(Vec::new())),
            (JsString::from("v"), v),
        ]
        .into_iter()
        .collect(),
    )
}

/// The evaluated scene (`compositionScene`'s answer).
#[derive(Clone, Debug, PartialEq)]
pub struct EvaluatedScene {
    /// The scene description: `{ version, objects, lights, camera, environment }`.
    pub scene: Value,
    /// Object layers generated by a model group recipe (evaluated with
    /// `animatedPart`; a renderer places them with
    /// [`Scope::recipe_world_offset`]).
    pub recipe_parts: Vec<JsString>,
}

/// `p` with every schema channel `parseScene` defaults filled in.
fn with_defaults(p: Value, defaults: &Value) -> Value {
    let (Value::Object(mut p), Value::Object(d)) = (p, defaults) else {
        unreachable!("channel maps are objects")
    };
    for (k, v) in d.iter() {
        if p.get_key(k).is_none() {
            p.insert(k.clone(), v.clone());
        }
    }
    Value::Object(p)
}

/// `materialSchema.shape.p` defaults.
fn material_defaults() -> Value {
    let mut p = Map::new();
    for (k, v) in [
        ("color", Value::String(JsString::from("#C7C4FF"))),
        ("roughness", Value::Number(0.4)),
        ("metalness", Value::Number(0.1)),
        ("emissive", Value::String(JsString::from("#000000"))),
        ("emissiveIntensity", Value::Number(1.0)),
        ("opacity", Value::Number(1.0)),
    ] {
        p.insert(k, channel(v));
    }
    Value::Object(p)
}

/// The studio camera (`parseScene({}).camera`).
fn default_camera() -> Value {
    let mut p = Map::new();
    for (k, v) in [
        ("x", 3.0),
        ("y", 2.0),
        ("z", 6.0),
        ("targetX", 0.0),
        ("targetY", 0.0),
        ("targetZ", 0.0),
        ("fov", 45.0),
        ("zoom", 1.0),
        ("near", 0.01),
        ("far", 1000.0),
    ] {
        p.insert(k, channel(Value::Number(v)));
    }
    let mut m = Map::new();
    m.insert("projection", Value::String(JsString::from("perspective")));
    m.insert("p", Value::Object(p));
    Value::Object(m)
}

/// The studio environment (`parseScene({}).environment`).
fn default_environment() -> Value {
    let mut p = Map::new();
    p.insert("ambient", channel(Value::Number(0.5)));
    p.insert(
        "ambientColor",
        channel(Value::String(JsString::from("#FFFFFF"))),
    );
    p.insert("exposure", channel(Value::Number(1.0)));
    let mut m = Map::new();
    m.insert("p", Value::Object(p));
    m.insert("background", Value::Null);
    m.insert("shadows", Value::Bool(true));
    Value::Object(m)
}

/// The studio sun (`parseScene({lights: [{id, type: 'sun'}]}).lights[0]`).
fn default_sun(id: &str) -> Value {
    let mut p = Map::new();
    for (k, v) in [
        ("x", Value::Number(-3.0)),
        ("y", Value::Number(5.0)),
        ("z", Value::Number(4.0)),
        ("targetX", Value::Number(0.0)),
        ("targetY", Value::Number(0.0)),
        ("targetZ", Value::Number(0.0)),
        ("color", Value::String(JsString::from("#FFFFFF"))),
        ("intensity", Value::Number(3.0)),
        ("distance", Value::Number(0.0)),
        ("decay", Value::Number(2.0)),
        ("angle", Value::Number(45.0)),
        ("penumbra", Value::Number(0.4)),
        ("width", Value::Number(1.0)),
        ("height", Value::Number(1.0)),
        ("visible", Value::Bool(true)),
    ] {
        p.insert(k, channel(v));
    }
    let mut m = Map::new();
    m.insert("id", Value::String(JsString::from(id)));
    m.insert("name", Value::String(JsString::from("Light")));
    m.insert("type", Value::String(JsString::from("sun")));
    m.insert("p", Value::Object(p));
    m.insert("castShadow", Value::Bool(true));
    Value::Object(m)
}

fn object_of(v: Option<&Node>) -> Map {
    match v.map(Node::to_value) {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    }
}

/// A camera's placement as `compositionRuntime` sets it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraPose {
    /// World position.
    pub position: [f64; 3],
    /// World aim point.
    pub target: [f64; 3],
    /// World up direction (normalized).
    pub up: [f64; 3],
}

impl<'d> Scope<'_, 'd> {
    /// The `ev` of `compositionScene`: the evaluated channel when it keeps
    /// the stored type, is finite, and validates; otherwise the stored `v`.
    fn scene_value(&mut self, layer: &'d Node, prop: &'d Node, path: &str, time: f64) -> Value {
        let stored = node::js_value_opt(node::member(Some(prop), "v"));
        let fallback = stored.to_json().unwrap_or(Value::Null);
        let Ok(value) = self.channel_p(layer, Some(prop), time, path) else {
            return fallback;
        };
        if type_of(&value) != type_of(&stored)
            || matches!(value, JsValue::Number(n) if !n.is_finite())
        {
            return fallback;
        }
        match validation(path) {
            Ok(None) => value.to_json().unwrap_or(fallback),
            Ok(Some(field)) if field.accepts(&value) => value.to_json().unwrap_or(fallback),
            _ => fallback,
        }
    }

    /// `values(layer, p, prefix)`: every channel of `p` evaluated.
    fn scene_values(
        &mut self,
        layer: &'d Node,
        p: Option<&'d Node>,
        prefix: &str,
        time: f64,
    ) -> Value {
        let mut out = Map::new();
        if let Some(p) = p.filter(|p| matches!(p.kind(), pm_model::NodeKind::Object(_))) {
            for (key, prop) in node::entries(p) {
                let path = format!("{prefix}.{}", key.to_string_lossy());
                let v = match prop.node() {
                    Some(n) => self.scene_value(layer, n, &path, time),
                    None => Value::Null,
                };
                out.insert(key, channel(v));
            }
        }
        Value::Object(out)
    }

    /// `transform(layer)`: object transform channels in scene units.
    fn scene_transform(&mut self, layer: &'d Node, time: f64) -> Map {
        let p = node::member(Some(layer), "p");
        let mut out = Map::new();
        for (key, path) in TRANSFORM_PATHS {
            let scale = key.starts_with('s');
            let raw = match node::member(p, path) {
                Some(prop) if node::truthy(Some(prop)) => {
                    match self.scene_value(layer, prop, path, time) {
                        Value::Number(n) => n,
                        other => JsValue::from(other).to_number(),
                    }
                }
                _ => {
                    if scale {
                        100.0
                    } else {
                        0.0
                    }
                }
            };
            out.insert(
                key,
                channel(Value::Number(raw / if scale { 100.0 } else { 1.0 })),
            );
        }
        out
    }

    /// The composition's 3D layers in stack order.
    fn scene_layers(&self) -> Vec<(&'d Node, Role3d)> {
        self.layers()
            .iter()
            .filter_map(|l| Role3d::of(l).map(|r| (l, r)))
            .collect()
    }

    /// `compositionScene(PM, time, comp)`.
    pub fn composition_scene(&mut self, time: f64) -> JsResult<EvaluatedScene> {
        let layers = self.scene_layers();
        let mut active = Vec::new();
        for (l, r) in &layers {
            if self.active(l, time)? {
                active.push((*l, *r));
            }
        }
        let camera_layer = active
            .iter()
            .find(|(_, r)| *r == Role3d::Camera)
            .map(|(l, _)| *l);
        let object_layers: Vec<&'d Node> = active
            .iter()
            .filter(|(_, r)| *r == Role3d::Object)
            .map(|(l, _)| *l)
            .collect();
        let light_layers: Vec<&'d Node> = active
            .iter()
            .filter(|(_, r)| *r == Role3d::Light)
            .map(|(l, _)| *l)
            .collect();
        let object_ids: Vec<JsValue> = object_layers
            .iter()
            .map(|l| node::get(l, b"id").to_js())
            .collect();
        let mut recipe_parts = Vec::new();

        let mut objects = Vec::new();
        for layer in &object_layers {
            let data = node::member(node::member(Some(layer), "d"), "data");
            let object = node::member(data, "object");
            let mut out = object_of(object);
            if node::truthy(node::member(object, "generation")) {
                recipe_parts.push(node::to_js_string(node::member(Some(layer), "id")));
            }
            let mut p = self.scene_transform(layer, time);
            // An animated recipe moves its parts by their new base
            // (`animatedPart`), before the orientation.
            let part = self.animated_part(layer, time)?;
            if let Some(part) = &part {
                let delta = part.delta();
                for (k, key) in crate::recipe::BASE_KEYS.iter().enumerate() {
                    if let Some(Value::Object(ch)) = p.get_key_mut(&JsString::from(*key))
                        && let Some(Value::Number(v)) = ch.get_key_mut(&JsString::from("v"))
                    {
                        *v += delta[k];
                    }
                }
            }
            // Native orientation is applied before the separately animated rotation.
            for (key, axis) in [("rx", "x"), ("ry", "y"), ("rz", "z")] {
                let path = format!("orientation.{axis}");
                if let Some(prop) = node::member(node::member(Some(layer), "p"), &path)
                    .filter(|n| node::truthy(Some(n)))
                {
                    let add = match self.scene_value(layer, prop, &path, time) {
                        Value::Number(n) => n,
                        other => JsValue::from(other).to_number(),
                    };
                    if let Some(Value::Object(ch)) = p.get_key_mut(&JsString::from(key))
                        && let Some(Value::Number(v)) = ch.get_key_mut(&JsString::from("v"))
                    {
                        *v += add;
                    }
                }
            }
            if let Some(part) = part {
                out.insert("source", part.source);
            }
            out.insert(
                "id",
                node::js_value_opt(node::member(Some(layer), "id"))
                    .to_json()
                    .unwrap_or(Value::Null),
            );
            // An absent JS `layer.name` is `undefined`: `parseScene` applies
            // its default. Null is an explicit invalid name and stays null.
            out.remove("name");
            out.insert(
                "name",
                node::js_value_opt(node::member(Some(layer), "name"))
                    .to_json()
                    .unwrap_or_else(|| Value::String("Object".into())),
            );
            let parent = node::get(layer, b"parent").to_js();
            let parent_is_object = object_ids
                .iter()
                .any(|id| crate::js_sem::strict_eq(id, &parent));
            out.insert(
                "parent",
                if parent_is_object {
                    parent.to_json().unwrap_or(Value::Null)
                } else {
                    Value::Null
                },
            );
            // `parseScene` fills the transform schema's `visible` default.
            p.insert("visible", channel(Value::Bool(true)));
            out.insert("p", Value::Object(p));
            let material = node::member(object, "material");
            let mut m = object_of(material);
            // `materialSchema`'s `doubleSided` default (`parseScene` fills it).
            if m.get_key(&JsString::from("doubleSided")).is_none() {
                m.insert("doubleSided", Value::Bool(true));
            }
            m.insert(
                "p",
                with_defaults(
                    self.scene_values(layer, node::member(material, "p"), "m", time),
                    &material_defaults(),
                ),
            );
            if let Some(shader) = node::member(material, "shader").filter(|s| node::truthy(Some(s)))
            {
                let mut s = object_of(Some(shader));
                s.insert(
                    "p",
                    self.scene_values(layer, node::member(Some(shader), "p"), "shader", time),
                );
                m.insert("shader", Value::Object(s));
            }
            out.insert("material", Value::Object(m));
            let mut slots = Vec::new();
            for slot in node::array(node::member(object, "slots")).unwrap_or(&[]) {
                let id = node::to_js_string(node::member(Some(slot), "id")).to_string_lossy();
                let material = node::member(Some(slot), "material");
                let mut sm = object_of(material);
                if sm.get_key(&JsString::from("doubleSided")).is_none() {
                    sm.insert("doubleSided", Value::Bool(true));
                }
                sm.insert(
                    "p",
                    with_defaults(
                        self.scene_values(
                            layer,
                            node::member(material, "p"),
                            &format!("slots.{id}"),
                            time,
                        ),
                        &material_defaults(),
                    ),
                );
                if let Some(shader) =
                    node::member(material, "shader").filter(|s| node::truthy(Some(s)))
                {
                    let mut s = object_of(Some(shader));
                    s.insert(
                        "p",
                        self.scene_values(
                            layer,
                            node::member(Some(shader), "p"),
                            &format!("slots.{id}.shader"),
                            time,
                        ),
                    );
                    sm.insert("shader", Value::Object(s));
                }
                let mut so = object_of(Some(slot));
                so.insert("material", Value::Object(sm));
                slots.push(Value::Object(so));
            }
            out.insert("slots", Value::Array(slots));
            validate_transform(&out)?;
            objects.push(Value::Object(out));
        }

        let mut lights = Vec::new();
        for layer in &light_layers {
            let light = node::member(
                node::member(node::member(Some(layer), "d"), "data"),
                "light",
            );
            let mut out = object_of(light);
            let Value::Object(mut p) =
                self.scene_values(layer, node::member(light, "p"), "light", time)
            else {
                unreachable!()
            };
            let t = self.scene_transform(layer, time);
            for k in ["x", "y", "z"] {
                p.insert(
                    k,
                    t.get_key(&JsString::from(k))
                        .cloned()
                        .unwrap_or(Value::Null),
                );
            }
            out.insert(
                "id",
                node::js_value_opt(node::member(Some(layer), "id"))
                    .to_json()
                    .unwrap_or(Value::Null),
            );
            out.remove("name");
            out.insert(
                "name",
                node::js_value_opt(node::member(Some(layer), "name"))
                    .to_json()
                    .unwrap_or_else(|| Value::String("Light".into())),
            );
            validate_position(&p, "light")?;
            let Value::Object(sun) = default_sun("") else {
                unreachable!()
            };
            let p = with_defaults(Value::Object(p), sun.get_key(&JsString::from("p")).unwrap());
            out.insert("p", p);
            lights.push(Value::Object(out));
        }

        let mut camera = default_camera();
        let mut environment = default_environment();
        if let Some(layer) = camera_layer {
            let data = node::member(node::member(Some(layer), "d"), "data");
            let cam = node::member(data, "camera");
            let mut c = object_of(cam);
            let Value::Object(mut p) =
                self.scene_values(layer, node::member(cam, "p"), "camera", time)
            else {
                unreachable!()
            };
            let t = self.scene_transform(layer, time);
            for k in ["x", "y", "z"] {
                p.insert(
                    k,
                    t.get_key(&JsString::from(k))
                        .cloned()
                        .unwrap_or(Value::Null),
                );
            }
            validate_position(&p, "camera")?;
            let Value::Object(base) = default_camera() else {
                unreachable!()
            };
            c.insert(
                "p",
                with_defaults(
                    Value::Object(p),
                    base.get_key(&JsString::from("p")).unwrap(),
                ),
            );
            camera = Value::Object(c);
            let env = node::member(data, "environment");
            let mut e = object_of(env);
            let Value::Object(base) = default_environment() else {
                unreachable!()
            };
            e.insert(
                "p",
                with_defaults(
                    self.scene_values(layer, node::member(env, "p"), "environment", time),
                    base.get_key(&JsString::from("p")).unwrap(),
                ),
            );
            environment = Value::Object(e);
        }
        if light_layers.is_empty() {
            let mut id = String::from("default_sun");
            while object_ids
                .iter()
                .any(|o| matches!(o, JsValue::String(s) if s.as_str() == Some(id.as_str())))
            {
                id.push('x');
            }
            lights.push(default_sun(&id));
        }
        // `camera.p.far.v = Math.max(far, near + .01)`.
        if let Value::Object(c) = &mut camera
            && let Some(Value::Object(p)) = c.get_key_mut(&JsString::from("p"))
        {
            let read =
                |p: &Map, k: &str| match p.get_key(&JsString::from(k)).and_then(|c| match c {
                    Value::Object(m) => m.get_key(&JsString::from("v")).cloned(),
                    _ => None,
                }) {
                    Some(v) => JsValue::from(v).to_number(),
                    None => f64::NAN,
                };
            let far = js_max(read(p, "far"), read(p, "near") + 0.01);
            if let Some(Value::Object(f)) = p.get_key_mut(&JsString::from("far")) {
                f.insert("v", Value::Number(far));
            }
        }
        let mut scene = Map::new();
        scene.insert("version", Value::Number(1.0));
        scene.insert("objects", Value::Array(objects));
        scene.insert("lights", Value::Array(lights));
        scene.insert("camera", camera);
        scene.insert("environment", environment);
        pm_geom3d::scene_validation::validate_scene(&scene).map_err(|message| JsThrow {
            kind: "ZodError",
            message: format!("Invalid 3D scene: {message}"),
        })?;
        Ok(EvaluatedScene {
            scene: Value::Object(scene),
            recipe_parts,
        })
    }

    /// The camera `compositionRuntime` renders with: the topmost active
    /// camera layer at its world position, aimed at its stored target mapped
    /// through its parent space, with the layer's world Y as up; or the
    /// studio camera.
    pub fn scene_camera(&mut self, time: f64) -> JsResult<CameraPose> {
        for (layer, role) in self.scene_layers() {
            if role != Role3d::Camera || !self.active(layer, time)? {
                continue;
            }
            let world = self.world3d(layer, time, false)?;
            let target = self.world_aim3d(layer, time)?.unwrap_or([0.0; 3]);
            // `up.set(0, 1, 0).transformDirection(world)`.
            let up = [world[4], world[5], world[6]];
            let len = (up[0] * up[0] + up[1] * up[1] + up[2] * up[2]).sqrt();
            let up = if len > 0.0 {
                [up[0] / len, up[1] / len, up[2] / len]
            } else {
                up
            };
            return Ok(CameraPose {
                position: [world[12], world[13], world[14]],
                target,
                up,
            });
        }
        Ok(CameraPose {
            position: [3.0, 2.0, 6.0],
            target: [0.0; 3],
            up: [0.0, 1.0, 0.0],
        })
    }

    /// A light layer's world position and world aim (`compositionRuntime`:
    /// the target stored in the parent space, mapped through `parent3D`).
    pub fn scene_light_aim(
        &mut self,
        layer: &'d Node,
        time: f64,
    ) -> JsResult<Option<([f64; 3], [f64; 3])>> {
        if Role3d::of(layer) != Some(Role3d::Light) {
            return Ok(None);
        }
        let world = self.world3d(layer, time, false)?;
        let Some(target) = self.world_aim3d(layer, time)? else {
            return Ok(None);
        };
        Ok(Some(([world[12], world[13], world[14]], target)))
    }

    /// `layer.from`-relative time of a glTF clip: `max(0, t − layer.from)`
    /// (section 5 rule 10: speed, trim, and time remap do not apply).
    pub fn clip_time(&self, layer: &Node, time: f64) -> f64 {
        js_max(
            0.0,
            time - node::to_number(node::member(Some(layer), "from")),
        )
    }
}

/// `parseScene`'s position range for a light or camera (`num()`).
fn validate_position(p: &Map, what: &str) -> JsResult<()> {
    for key in ["x", "y", "z"] {
        let v = match p.get_key(&JsString::from(key)) {
            Some(Value::Object(c)) => c.get_key(&JsString::from("v")).cloned(),
            _ => None,
        };
        if !matches!(v, Some(Value::Number(n)) if n.is_finite() && (-100_000.0..=100_000.0).contains(&n))
        {
            return Err(JsThrow {
                kind: "ZodError",
                message: format!("Invalid 3D {what} position: {key}"),
            });
        }
    }
    Ok(())
}

/// `parseScene`'s transform ranges for an object (`p(transform)`): the
/// legacy scene throws a validation error for values outside them.
fn validate_transform(object: &Map) -> JsResult<()> {
    let Some(Value::Object(p)) = object.get_key(&JsString::from("p")) else {
        return Ok(());
    };
    for (key, (lo, hi)) in [
        ("x", (-100_000.0, 100_000.0)),
        ("y", (-100_000.0, 100_000.0)),
        ("z", (-100_000.0, 100_000.0)),
        ("rx", (-360_000.0, 360_000.0)),
        ("ry", (-360_000.0, 360_000.0)),
        ("rz", (-360_000.0, 360_000.0)),
        ("sx", (-10_000.0, 10_000.0)),
        ("sy", (-10_000.0, 10_000.0)),
        ("sz", (-10_000.0, 10_000.0)),
    ] {
        let v = match p.get_key(&JsString::from(key)) {
            Some(Value::Object(c)) => c.get_key(&JsString::from("v")).cloned(),
            _ => None,
        };
        let ok = matches!(v, Some(Value::Number(n)) if n.is_finite() && n >= lo && n <= hi);
        if !ok {
            return Err(JsThrow {
                kind: "ZodError",
                message: format!("Invalid 3D object transform: {key}"),
            });
        }
    }
    Ok(())
}

/// A camera's world matrix from its pose (three.js `lookAt` for cameras):
/// columns x, y, z, position.
pub fn camera_matrix(pose: &CameraPose) -> [f64; 16] {
    let sub = |a: [f64; 3], b: [f64; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let norm = |v: [f64; 3]| {
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        if l == 0.0 {
            v
        } else {
            [v[0] / l, v[1] / l, v[2] / l]
        }
    };
    let cross = |a: [f64; 3], b: [f64; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let mut z = sub(pose.position, pose.target);
    if z == [0.0; 3] {
        z[2] = 1.0;
    }
    z = norm(z);
    let mut x = cross(pose.up, z);
    if x == [0.0; 3] {
        if pose.up[2].abs() == 1.0 {
            z[0] += 0.0001;
        } else {
            z[2] += 0.0001;
        }
        z = norm(z);
        x = cross(pose.up, z);
    }
    x = norm(x);
    let y = cross(z, x);
    [
        x[0],
        x[1],
        x[2],
        0.0,
        y[0],
        y[1],
        y[2],
        0.0,
        z[0],
        z[1],
        z[2],
        0.0,
        pose.position[0],
        pose.position[1],
        pose.position[2],
        1.0,
    ]
}
