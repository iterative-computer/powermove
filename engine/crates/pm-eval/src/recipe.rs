//! Generated model groups whose recipe is animated (`core/scene3d/
//! recipe-preview.ts` `recipeParts` and `animatedPart`, `modeling.ts`
//! `evaluatedRecipe`).
//!
//! A part layer (`d.data.object.generation`) of a model group whose
//! `d.modeling.p` channels evaluate to parameters other than the stored
//! recipe's draws the recipe's cheap parametric geometry for the evaluated
//! parameters: `compositionScene` replaces the part's source and moves its
//! transform channels by the part's new base minus the stored one, and
//! `compositionRuntime` moves the placed object by the same base offset
//! through its parent space ([`Scope::recipe_world_offset`]).

use std::collections::HashMap;

use pm_model::json::{Map, Value};
use pm_model::{JsString, Node, NodeKind};

use crate::js_sem::{JsResult, JsThrow, JsValue};
use crate::node;
use crate::scope::Scope;

/// The base keys of a part, in `recipeParts`' order.
pub const BASE_KEYS: [&str; 9] = ["x", "y", "z", "rx", "ry", "rz", "sx", "sy", "sz"];

/// A part whose recipe is animated.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimatedPart {
    /// The part's base for the evaluated recipe ([`BASE_KEYS`]).
    pub base: [f64; 9],
    /// The stored base (`d.modeling.parts[part].base`; NaN for a missing key).
    pub prior: [f64; 9],
    /// The parametric source for the evaluated recipe.
    pub source: Value,
}

impl AnimatedPart {
    /// `base[key] − prior[key]` for each base key.
    pub fn delta(&self) -> [f64; 9] {
        std::array::from_fn(|k| self.base[k] - self.prior[k])
    }
}

/// `RECIPE_PARAMETERS` defaults and ranges (`min`, `max`, `integer`).
fn field(kind: &str, key: &str) -> Option<(f64, f64, f64, bool)> {
    Some(match (kind, key) {
        ("rocket", "length") => (2.8, 0.1, 50.0, false),
        ("rocket", "radius") => (0.6, 0.05, 10.0, false),
        ("rocket", "nose") => (1.0, 0.05, 20.0, false),
        ("rocket", "fins") => (4.0, 0.0, 16.0, true),
        ("rocket", "finSize") => (0.8, 0.05, 10.0, false),
        ("staircase", "steps") => (12.0, 1.0, 128.0, true),
        ("staircase", "radius") => (1.5, 0.1, 30.0, false),
        ("staircase", "rise") => (0.22, 0.01, 5.0, false),
        ("staircase", "turn") => (30.0, -180.0, 180.0, false),
        ("staircase", "width") => (1.2, 0.05, 10.0, false),
        ("staircase", "depth") => (0.5, 0.05, 10.0, false),
        ("text", "size") => (1.0, 0.01, 50.0, false),
        ("text", "depth") => (0.12, 0.0, 5.0, false),
        ("text", "bevel") => (0.02, 0.0, 1.0, false),
        ("lathe", "segments") => (48.0, 3.0, 128.0, true),
        ("extrude", "depth") => (0.2, 0.001, 50.0, false),
        _ => return None,
    })
}

/// `RECIPE_PARAMETERS[kind]`'s default values.
fn defaults(kind: &str) -> Vec<(&'static str, Value)> {
    let n = |k: &'static str, v: f64| (k, Value::Number(v));
    match kind {
        "rocket" => vec![
            n("length", 2.8),
            n("radius", 0.6),
            n("nose", 1.0),
            n("fins", 4.0),
            n("finSize", 0.8),
        ],
        "staircase" => vec![
            n("steps", 12.0),
            n("radius", 1.5),
            n("rise", 0.22),
            n("turn", 30.0),
            n("width", 1.2),
            n("depth", 0.5),
        ],
        "text" => vec![
            ("text", Value::String(JsString::from("Powermove"))),
            n("size", 1.0),
            n("depth", 0.12),
            n("bevel", 0.02),
        ],
        "lathe" => vec![n("segments", 48.0)],
        "extrude" => vec![n("depth", 0.2)],
        _ => Vec::new(),
    }
}

fn num(v: f64) -> Value {
    Value::Number(v)
}

fn nums(v: &[f64]) -> Value {
    Value::Array(v.iter().map(|x| Value::Number(*x)).collect())
}

fn object(entries: Vec<(&str, Value)>) -> Value {
    let mut m = Map::new();
    for (k, v) in entries {
        m.insert(k, v);
    }
    Value::Object(m)
}

fn primitive(name: &str, params: Vec<(&str, f64)>) -> Value {
    object(vec![
        ("primitive", Value::String(JsString::from(name))),
        (
            "parameters",
            object(params.into_iter().map(|(k, v)| (k, num(v))).collect()),
        ),
    ])
}

fn f32s(v: &[f32]) -> Vec<f64> {
    v.iter().map(|x| f64::from(*x)).collect()
}

/// `Vector3.normalize()` of a stored normal, stored back as `f32`.
fn normalized(v: [f64; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let l = if l == 0.0 || l.is_nan() { 1.0 } else { l };
    let s = 1.0 / l;
    [(v[0] * s) as f32, (v[1] * s) as f32, (v[2] * s) as f32]
}

/// `geometry.rotateY(theta)` (`applyMatrix4(makeRotationY(theta))`):
/// positions through the matrix, normals through its normal matrix and
/// normalized, both stored as `Float32Array` values.
fn rotate_y(g: &mut pm_geom3d::Geometry, theta: f64) {
    let (c, s) = (libm::cos(theta), libm::sin(theta));
    // Column-major `makeRotationY`.
    let m = [
        c, 0.0, -s, 0.0, 0.0, 1.0, 0.0, 0.0, s, 0.0, c, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    for p in g.positions.chunks_exact_mut(3) {
        let (x, y, z) = (f64::from(p[0]), f64::from(p[1]), f64::from(p[2]));
        let w = 1.0 / (m[3] * x + m[7] * y + m[11] * z + m[15]);
        p[0] = ((m[0] * x + m[4] * y + m[8] * z + m[12]) * w) as f32;
        p[1] = ((m[1] * x + m[5] * y + m[9] * z + m[13]) * w) as f32;
        p[2] = ((m[2] * x + m[6] * y + m[10] * z + m[14]) * w) as f32;
    }
    // `Matrix3.getNormalMatrix`: setFromMatrix4, invert, transpose.
    let (n11, n21, n31) = (m[0], m[1], m[2]);
    let (n12, n22, n32) = (m[4], m[5], m[6]);
    let (n13, n23, n33) = (m[8], m[9], m[10]);
    let t11 = n33 * n22 - n32 * n23;
    let t12 = n32 * n13 - n33 * n12;
    let t13 = n23 * n12 - n22 * n13;
    let det = n11 * t11 + n21 * t12 + n31 * t13;
    let mut e = [0.0; 9];
    if det != 0.0 {
        let d = 1.0 / det;
        e = [
            t11 * d,
            (n31 * n23 - n33 * n21) * d,
            (n32 * n21 - n31 * n22) * d,
            t12 * d,
            (n33 * n11 - n31 * n13) * d,
            (n31 * n12 - n32 * n11) * d,
            t13 * d,
            (n21 * n13 - n23 * n11) * d,
            (n22 * n11 - n21 * n12) * d,
        ];
    }
    e.swap(1, 3);
    e.swap(2, 6);
    e.swap(5, 7);
    for n in g.normals.chunks_exact_mut(3) {
        let (x, y, z) = (f64::from(n[0]), f64::from(n[1]), f64::from(n[2]));
        let v = normalized([
            e[0] * x + e[3] * y + e[6] * z,
            e[1] * x + e[4] * y + e[7] * z,
            e[2] * x + e[5] * y + e[8] * z,
        ]);
        n.copy_from_slice(&v);
    }
}

/// `recipeParts(recipe).get(part)`: the part's source and base, or `None`
/// when the recipe makes no such part. `params` are the recipe's
/// parameters (`n(key, default)` reads a number or the default).
pub fn recipe_part(
    kind: &str,
    params: &HashMap<String, JsValue>,
    profile: Option<&[[f64; 2]]>,
    part: &str,
) -> Option<(Value, [f64; 9])> {
    let n = |k: &str, d: f64| match params.get(k) {
        Some(JsValue::Number(v)) => *v,
        _ => d,
    };
    let base = |x: f64, y: f64, z: f64| [x, y, z, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
    match kind {
        "rocket" => {
            let radius = n("radius", 0.6);
            let length = n("length", 2.8);
            let nose = n("nose", 1.0);
            let fins = n("fins", 4.0).floor();
            let fin = n("finSize", 0.8);
            match part {
                "body" => Some((
                    primitive("cylinder", vec![("radius", radius), ("height", length)]),
                    base(0.0, 0.0, 0.0),
                )),
                "nose" => Some((
                    primitive("cone", vec![("radius", radius), ("height", nose)]),
                    base(0.0, length / 2.0 + nose / 2.0, 0.0),
                )),
                "engine" => Some((
                    primitive(
                        "cylinder",
                        vec![
                            ("radiusTop", radius * 0.75),
                            ("radiusBottom", radius * 0.55),
                            ("height", radius * 0.7),
                        ],
                    ),
                    base(0.0, -length / 2.0 - radius * 0.35, 0.0),
                )),
                p => {
                    let i: u32 = p.strip_prefix("fin_")?.parse().ok()?;
                    if f64::from(i) >= fins || format!("fin_{i:02}") != p {
                        return None;
                    }
                    let vertices = [
                        radius * 0.8,
                        -length * 0.15,
                        -0.06,
                        radius + fin,
                        -length * 0.55,
                        -0.06,
                        radius * 0.8,
                        -length * 0.5,
                        -0.06,
                        radius * 0.8,
                        -length * 0.15,
                        0.06,
                        radius + fin,
                        -length * 0.55,
                        0.06,
                        radius * 0.8,
                        -length * 0.5,
                        0.06,
                    ];
                    let indices = [
                        0.0, 2.0, 1.0, 3.0, 4.0, 5.0, 0.0, 1.0, 4.0, 0.0, 4.0, 3.0, 1.0, 2.0, 5.0,
                        1.0, 5.0, 4.0, 2.0, 0.0, 3.0, 2.0, 3.0, 5.0,
                    ];
                    let angle = (f64::from(i) * std::f64::consts::PI * 2.0) / fins.max(1.0);
                    let (c, s) = (libm::cos(angle), libm::sin(angle));
                    let positions: Vec<f64> = vertices
                        .iter()
                        .enumerate()
                        .map(|(k, &v)| match k % 3 {
                            1 => v,
                            0 => v * c + vertices[k + 2] * s,
                            _ => -vertices[k - 2] * s + v * c,
                        })
                        .collect();
                    Some((
                        object(vec![(
                            "mesh",
                            object(vec![
                                ("positions", nums(&positions)),
                                ("indices", nums(&indices)),
                            ]),
                        )]),
                        base(0.0, 0.0, 0.0),
                    ))
                }
            }
        }
        "staircase" => {
            let count = n("steps", 12.0).floor();
            let radius = n("radius", 1.5);
            let rise = n("rise", 0.22);
            let turn = n("turn", 30.0);
            let width = n("width", 1.2);
            let depth = n("depth", 0.5);
            let i: u32 = part.strip_prefix("step_")?.parse().ok()?;
            if f64::from(i) >= count || format!("step_{i:03}") != part {
                return None;
            }
            let angle = (f64::from(i) * turn * std::f64::consts::PI) / 180.0;
            let mut g = pm_geom3d::Primitive::Box {
                width,
                height: rise * 0.6,
                depth,
            }
            .geometry();
            rotate_y(&mut g, -angle);
            let indices: Vec<f64> = g.indices.iter().map(|i| f64::from(*i)).collect();
            Some((
                object(vec![(
                    "mesh",
                    object(vec![
                        ("positions", nums(&f32s(&g.positions))),
                        ("normals", nums(&f32s(&g.normals))),
                        ("uvs", nums(&f32s(&g.uvs))),
                        ("indices", nums(&indices)),
                    ]),
                )]),
                base(
                    radius * libm::cos(angle),
                    f64::from(i) * rise,
                    radius * libm::sin(angle),
                ),
            ))
        }
        "lathe" => {
            let profile = profile?;
            (part == "model").then(|| {
                let points = Value::Array(profile.iter().map(|p| nums(p)).collect());
                (
                    object(vec![
                        ("lathe", points),
                        ("segments", num(n("segments", 48.0).floor())),
                    ]),
                    base(0.0, 0.0, 0.0),
                )
            })
        }
        "extrude" => {
            let profile = profile?;
            if part != "model" {
                return None;
            }
            let depth = n("depth", 0.2);
            let mut g = pm_geom3d::shapes::extrude_geometry(profile, depth, 0.0);
            // `.translate(0, 0, -depth / 2)`: z through the matrix, normals
            // renormalized by `applyNormalMatrix`.
            let dz = -depth / 2.0;
            for p in g.positions.chunks_exact_mut(3) {
                p[2] = (f64::from(p[2]) + dz) as f32;
            }
            for c in g.normals.chunks_exact_mut(3) {
                let v = normalized([f64::from(c[0]), f64::from(c[1]), f64::from(c[2])]);
                c.copy_from_slice(&v);
            }
            Some((
                object(vec![(
                    "mesh",
                    object(vec![
                        ("positions", nums(&f32s(&g.positions))),
                        ("normals", nums(&f32s(&g.normals))),
                        ("uvs", nums(&f32s(&g.uvs))),
                    ]),
                )]),
                base(0.0, 0.0, 0.0),
            ))
        }
        _ => None,
    }
}

impl<'d> Scope<'_, 'd> {
    /// `evaluatedRecipe(PM, group, time)`: the group's stored recipe with
    /// its parameters (`RECIPE_PARAMETERS`' defaults, then the stored ones,
    /// then the evaluated `d.modeling.p` channels), or `None` for a group
    /// without `d.modeling`. Invalid parameters throw a `ZodError`, as
    /// `recipeSchema.parse` does.
    pub fn evaluated_recipe(&mut self, group: &'d Node, time: f64) -> JsResult<Option<Value>> {
        let modeling = node::member(node::member(Some(group), "d"), "modeling");
        if !node::truthy(modeling) {
            return Ok(None);
        }
        let Some(recipe) = node::member(modeling, "recipe") else {
            return Ok(None);
        };
        let Value::Object(mut out) = recipe.to_value() else {
            return Ok(None);
        };
        let kind = node::string(node::member(Some(recipe), "kind"))
            .map(|s| s.to_string_lossy())
            .unwrap_or_default();
        let mut parameters = Map::new();
        for (k, v) in defaults(&kind) {
            parameters.insert(k, v);
        }
        if let Some(Value::Object(stored)) = out.get("parameters") {
            for (k, v) in stored.iter() {
                parameters.insert(k.clone(), v.clone());
            }
        }
        if let Some(p) = node::member(modeling, "p")
            && let NodeKind::Object(map) = p.kind()
        {
            for (k, prop) in map.iter() {
                let key = k.to_string_lossy();
                let v = self.channel_p(group, Some(prop), time, &format!("model.{key}"))?;
                let valid = match (field(&kind, &key), &v) {
                    (Some((_, lo, hi, integer)), JsValue::Number(n)) => {
                        n.is_finite() && *n >= lo && *n <= hi && (!integer || n.fract() == 0.0)
                    }
                    _ => false,
                };
                if !valid {
                    return Err(JsThrow {
                        kind: "ZodError",
                        message: format!("Invalid model parameter: {key}"),
                    });
                }
                parameters.insert(k.clone(), v.to_json().unwrap_or(Value::Null));
            }
        }
        out.insert("parameters", Value::Object(parameters));
        if out.get("modifiers").is_none() {
            out.insert("modifiers", Value::Array(Vec::new()));
        }
        Ok(Some(Value::Object(out)))
    }

    /// `animatedPart(PM, layer, time)`: the part's evaluated recipe geometry
    /// and base when its group's recipe is animated, else `None`.
    pub fn animated_part(&mut self, layer: &'d Node, time: f64) -> JsResult<Option<AnimatedPart>> {
        let object = node::member(
            node::member(node::member(Some(layer), "d"), "data"),
            "object",
        );
        let Some(generation) = node::member(object, "generation").filter(|g| node::truthy(Some(g)))
        else {
            return Ok(None);
        };
        let group_id = node::get(generation, b"groupId");
        let Some(group) = self.layer_by_id(&group_id) else {
            return Ok(None);
        };
        let modeling = node::member(node::member(Some(group), "d"), "modeling");
        if !node::truthy(modeling) {
            return Ok(None);
        }
        let recipe = node::member(modeling, "recipe");
        let kind = node::string(node::member(recipe, "kind"))
            .map(|s| s.to_string_lossy())
            .unwrap_or_default();
        // The stored parameters, then the evaluated model channels
        // (`evaluatedRecipe`).
        let mut params: HashMap<String, JsValue> = HashMap::new();
        if let Some(NodeKind::Object(map)) = node::member(recipe, "parameters").map(Node::kind) {
            for (k, v) in map.iter() {
                params.insert(k.to_string_lossy(), node::js_value_opt(Some(v)));
            }
        }
        let mut changed = false;
        if let Some(p) = node::member(modeling, "p")
            && let NodeKind::Object(map) = p.kind()
        {
            for (k, prop) in map.iter() {
                let key = k.to_string_lossy();
                let v = self.channel_p(group, Some(prop), time, &format!("model.{key}"))?;
                // `recipeSchema`'s parameter checks (a ZodError today).
                let valid = match (field(&kind, &key), &v) {
                    (Some((_, lo, hi, integer)), JsValue::Number(n)) => {
                        n.is_finite() && *n >= lo && *n <= hi && (!integer || n.fract() == 0.0)
                    }
                    _ => false,
                };
                if !valid {
                    return Err(JsThrow {
                        kind: "ZodError",
                        message: format!("Invalid model parameter: {key}"),
                    });
                }
                if params.get(&key) != Some(&v) {
                    changed = true;
                }
                params.insert(key, v);
            }
        }
        // The profile changes only with the stored recipe.
        if !changed {
            return Ok(None);
        }
        let profile: Option<Vec<[f64; 2]>> =
            node::array(node::member(recipe, "profile")).map(|a| {
                a.iter()
                    .map(|p| {
                        let xy = node::array(Some(p)).unwrap_or(&[]);
                        [node::to_number(xy.first()), node::to_number(xy.get(1))]
                    })
                    .collect()
            });
        let part_id =
            node::to_js_string(node::member(Some(generation), "partId")).to_string_lossy();
        let Some(prior) = node::member(
            node::member(node::member(modeling, "parts"), &part_id),
            "base",
        ) else {
            return Ok(None);
        };
        let Some((source, base)) = recipe_part(&kind, &params, profile.as_deref(), &part_id) else {
            return Ok(None);
        };
        let prior =
            BASE_KEYS.map(|k| node::number(node::member(Some(prior), k)).unwrap_or(f64::NAN));
        Ok(Some(AnimatedPart {
            base,
            prior,
            source,
        }))
    }

    /// `compositionRuntime`'s offset of a part's placed object: the base
    /// offset (x, y, z) mapped through `parent3D` (`delta.applyMatrix4(parent)
    /// .sub(origin)`).
    pub fn recipe_world_offset(
        &mut self,
        layer: &'d Node,
        time: f64,
        part: &AnimatedPart,
    ) -> JsResult<[f64; 3]> {
        let d = part.delta();
        let parent = self.parent3d(layer, time, None)?;
        let apply = |v: [f64; 3]| {
            let (x, y, z) = (v[0], v[1], v[2]);
            let w = 1.0 / (parent[3] * x + parent[7] * y + parent[11] * z + parent[15]);
            [
                (parent[0] * x + parent[4] * y + parent[8] * z + parent[12]) * w,
                (parent[1] * x + parent[5] * y + parent[9] * z + parent[13]) * w,
                (parent[2] * x + parent[6] * y + parent[10] * z + parent[14]) * w,
            ]
        };
        let a = apply([d[0], d[1], d[2]]);
        let o = apply([0.0; 3]);
        Ok([a[0] - o[0], a[1] - o[1], a[2] - o[2]])
    }
}
