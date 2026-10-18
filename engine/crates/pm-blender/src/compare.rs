//! Section 25's "Blender snapshots" parity class: for every 3D corpus frame
//! (preview and final, every shutter sample, nested compositions) the
//! engine's snapshot equals today's `renderSnapshot` output — geometry
//! within 1e-6; materials, maps, blocks, background list, camera, and lights
//! identical; texture bytes equal or listed in appendix A.
//!
//! Geometry is every number under a mesh's `positions`, `normals`, `uvs`,
//! and `matrix`. Everything else compares exactly, numbers by value (the
//! corpus writes −0 as 0), arrays element by element in order, and objects
//! by their key sets (key order carries no meaning in JSON).

use pm_model::Value;

/// The geometry bound (section 25).
pub const GEOMETRY_TOLERANCE: f64 = 1e-6;

/// One difference between today's snapshot and the engine's.
#[derive(Debug, Clone, PartialEq)]
pub struct Difference {
    /// JSON path, `snapshot.objects[2].meshes[0].positions[17]`.
    pub path: String,
    pub today: String,
    pub engine: String,
}

/// The comparison of one snapshot pair.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Comparison {
    pub differences: Vec<Difference>,
    /// Geometry numbers compared, and the largest absolute difference.
    pub geometry_values: usize,
    pub geometry_max: f64,
    /// Exact numbers compared.
    pub exact_values: usize,
}

impl Comparison {
    pub fn equal(&self) -> bool {
        self.differences.is_empty()
    }
}

fn brief(v: Option<&Value>) -> String {
    match v {
        None => "(absent)".into(),
        Some(v) => {
            let s = pm_model::json::to_canonical_string(v);
            if s.len() > 160 {
                format!("{}…", &s[..s.floor_char_boundary(160)])
            } else {
                s
            }
        }
    }
}

/// Compares `engine` with `today` (two `snapshot` objects).
pub fn compare(today: &Value, engine: &Value) -> Comparison {
    let mut c = Comparison::default();
    walk(&mut c, "snapshot", today, engine, false);
    c
}

fn is_geometry_key(key: &str) -> bool {
    matches!(key, "positions" | "normals" | "uvs" | "matrix")
}

fn walk(c: &mut Comparison, path: &str, a: &Value, b: &Value, geometry: bool) {
    let differ = |c: &mut Comparison| {
        // Long lists report once, not per element.
        if c.differences.len() < 200 {
            c.differences.push(Difference {
                path: path.to_owned(),
                today: brief(Some(a)),
                engine: brief(Some(b)),
            });
        }
    };
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            if geometry {
                c.geometry_values += 1;
                let d = (x - y).abs();
                if d.is_nan() || d > GEOMETRY_TOLERANCE {
                    differ(c);
                } else {
                    c.geometry_max = c.geometry_max.max(d);
                }
            } else {
                c.exact_values += 1;
                if x != y && !(x.is_nan() && y.is_nan()) {
                    differ(c);
                }
            }
        }
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                c.differences.push(Difference {
                    path: format!("{path}.length"),
                    today: x.len().to_string(),
                    engine: y.len().to_string(),
                });
                return;
            }
            for (i, (p, q)) in x.iter().zip(y).enumerate() {
                walk(c, &format!("{path}[{i}]"), p, q, geometry);
            }
        }
        (Value::Object(x), Value::Object(y)) => {
            for (k, v) in x.iter() {
                let key = k.to_string_lossy();
                let sub = format!("{path}.{key}");
                match y.get_key(k) {
                    Some(w) => walk(c, &sub, v, w, geometry || is_geometry_key(&key)),
                    None => c.differences.push(Difference {
                        path: sub,
                        today: brief(Some(v)),
                        engine: brief(None),
                    }),
                }
            }
            for (k, w) in y.iter() {
                if x.get_key(k).is_none() {
                    c.differences.push(Difference {
                        path: format!("{path}.{}", k.to_string_lossy()),
                        today: brief(None),
                        engine: brief(Some(w)),
                    });
                }
            }
        }
        _ => {
            if a != b {
                differ(c);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Value {
        pm_model::json::parse(s).unwrap()
    }

    #[test]
    fn geometry_has_a_bound_and_the_rest_is_exact() {
        let today = v(
            r#"{"objects":[{"meshes":[{"positions":[0,1,2],"matrix":[1,0],"material":{"p":{"roughness":{"v":0.4}}}}]}],"camera":{"fov":45}}"#,
        );
        let near = v(
            r#"{"objects":[{"meshes":[{"positions":[0,1.0000005,2],"matrix":[1,0.0000001],"material":{"p":{"roughness":{"v":0.4}}}}]}],"camera":{"fov":45}}"#,
        );
        let c = compare(&today, &near);
        assert!(c.equal(), "{:?}", c.differences);
        assert_eq!(c.geometry_values, 5);
        assert!(c.geometry_max > 4e-7);
        let far = v(
            r#"{"objects":[{"meshes":[{"positions":[0,1.00001,2],"matrix":[1,0],"material":{"p":{"roughness":{"v":0.4000000001}}}}]}],"camera":{"fov":45}}"#,
        );
        let c = compare(&today, &far);
        let paths: Vec<&str> = c.differences.iter().map(|d| d.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "snapshot.objects[0].meshes[0].material.p.roughness.v",
                "snapshot.objects[0].meshes[0].positions[1]"
            ]
        );
    }

    #[test]
    fn missing_keys_and_lengths_differ() {
        let c = compare(&v(r#"{"a":[1,2],"b":null}"#), &v(r#"{"a":[1],"c":1}"#));
        let paths: Vec<&str> = c.differences.iter().map(|d| d.path.as_str()).collect();
        assert_eq!(paths, ["snapshot.a.length", "snapshot.b", "snapshot.c"]);
        assert!(compare(&v(r#"{"x":-0}"#), &v(r#"{"x":0}"#)).equal());
    }
}
