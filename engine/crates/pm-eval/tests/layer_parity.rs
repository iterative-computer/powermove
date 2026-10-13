//! Replays `tests/vectors/layer-vectors.json.gz`, written by
//! `tests/vectors/generate-layer-vectors.ts` from the real TypeScript layer
//! queries under bun (JavaScriptCore): 2D and 3D matrices, parenting and
//! group spaces, perspective, homographies, plane hits, depth ordering, pose
//! decomposition, aims, cycles, mattes, size anchors, source times, group
//! spans, transform roots, text animator state and posed glyphs, and the
//! text-animation helpers.
//!
//! Numbers compare within plan section 25's evaluation tolerance
//! (`1e-9 + 1e-9·|b|`; transcendental functions are `libm` here and the
//! platform's in JavaScriptCore); the test also reports how many are bit
//! identical. `PM_LAYER_VECTORS=<file>` replays another set (for example a
//! `PM_LAYER_SCALE=10` run).

use std::collections::BTreeMap;
use std::io::Read;

use pm_eval::anchor::{BoundsValues, font_anchor_offset};
use pm_eval::evaluator::Content;
use pm_eval::retime::Retiming;
use pm_eval::text_anim::{self, Shape, UnitKind};
use pm_eval::{Comp, Evaluator, JsValue, Scope};
use pm_model::json::{Value, parse};
use pm_model::{Document, JsString, Node};
use pm_text::compat::GlyphLayout;

/// A decoded JavaScript value.
#[derive(Clone, Debug, PartialEq)]
enum V {
    U,
    Z,
    N(f64),
    S(String),
    B(bool),
    A(Vec<V>),
    O(BTreeMap<String, V>),
    Throw(String),
}

fn hexf(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}

fn get<'v>(v: &'v Value, k: &str) -> Option<&'v Value> {
    match v {
        Value::Object(m) => m.get_key(&JsString::from(k)),
        _ => None,
    }
}

fn dec(v: &Value) -> V {
    let Value::Object(m) = v else { panic!("encoded value") };
    let (k, x) = m.iter().next().unwrap();
    match k.as_str().unwrap() {
        "u" => V::U,
        "z" => V::Z,
        "n" => V::N(hexf(x)),
        "s" => V::S(x.as_js_string().unwrap().to_string_lossy()),
        "b" => V::B(matches!(x, Value::Bool(true))),
        "a" => V::A(x.as_array().unwrap().iter().map(dec).collect()),
        "o" => {
            let Value::Object(o) = x else { unreachable!() };
            V::O(o.iter().map(|(k, v)| (k.to_string_lossy(), dec(v))).collect())
        }
        "throw" => V::Throw(x.as_js_string().unwrap().to_string_lossy()),
        other => panic!("unknown encoding {other}"),
    }
}

/// A decoded value as a JSON value (for building inputs).
fn to_value(v: &V) -> Value {
    match v {
        V::U | V::Z | V::Throw(_) => Value::Null,
        V::N(n) => Value::Number(*n),
        V::S(s) => Value::String(JsString::from(s.as_str())),
        V::B(b) => Value::Bool(*b),
        V::A(a) => Value::Array(a.iter().map(to_value).collect()),
        V::O(o) => Value::Object(o.iter().map(|(k, v)| (JsString::from(k.as_str()), to_value(v))).collect()),
    }
}

fn to_js(v: &V) -> JsValue {
    match v {
        V::U | V::Throw(_) => JsValue::Undefined,
        V::Z => JsValue::Null,
        V::N(n) => JsValue::Number(*n),
        V::S(s) => JsValue::from(s.as_str()),
        V::B(b) => JsValue::Bool(*b),
        other => JsValue::from(to_value(other)),
    }
}

fn from_js(v: &JsValue) -> V {
    match v {
        JsValue::Undefined => V::U,
        JsValue::Null => V::Z,
        JsValue::Bool(b) => V::B(*b),
        JsValue::Number(n) => V::N(*n),
        JsValue::String(s) => V::S(s.to_string_lossy()),
        JsValue::Object(v) => from_json(v),
    }
}

fn from_json(v: &Value) -> V {
    match v {
        Value::Null => V::Z,
        Value::Bool(b) => V::B(*b),
        Value::Number(n) => V::N(*n),
        Value::String(s) => V::S(s.to_string_lossy()),
        Value::Array(a) => V::A(a.iter().map(from_json).collect()),
        Value::Object(o) => V::O(o.iter().map(|(k, v)| (k.to_string_lossy(), from_json(v))).collect()),
    }
}

fn content(c: &Content) -> V {
    V::O(c.iter().map(|(k, v)| (k.to_string_lossy(), from_js(v))).collect())
}

fn nums(xs: &[f64]) -> V {
    V::A(xs.iter().map(|x| V::N(*x)).collect())
}

#[derive(Default)]
struct Stats {
    numbers: usize,
    exact: usize,
    values: usize,
    failures: Vec<String>,
    max_error: f64,
    /// Angles that matched only modulo 360 (see `same`).
    wrapped: usize,
    ops: BTreeMap<String, usize>,
}

impl Stats {
    fn same(&mut self, ours: &V, want: &V) -> bool {
        match (ours, want) {
            (V::N(a), V::N(b)) => {
                self.numbers += 1;
                if a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan()) {
                    self.exact += 1;
                    return true;
                }
                if !a.is_finite() || !b.is_finite() {
                    return false;
                }
                let e = (a - b).abs();
                let ok = e <= 1e-9 + 1e-9 * b.abs();
                if ok {
                    self.max_error = self.max_error.max(e / (1.0 + b.abs()));
                }
                ok
            }
            (V::A(a), V::A(b)) => a.len() == b.len() && a.iter().zip(b).all(|(x, y)| self.same(x, y)),
            (V::O(a), V::O(b)) => {
                a.len() == b.len()
                    && a.iter().all(|(k, x)| {
                        b.get(k).is_some_and(|y| {
                            // Angles decomposed by atan2 at its branch cut: the
                            // platform's and libm's sin and cos can leave the
                            // input's sign on either side of zero, giving 180
                            // and -180 for the same pose.
                            if k.starts_with("rotation")
                                && let (V::N(p), V::N(q)) = (x, y)
                                && ((p - q).abs() - 360.0).abs() <= 1e-9 + 1e-9 * q.abs()
                            {
                                self.numbers += 1;
                                self.wrapped += 1;
                                return true;
                            }
                            self.same(x, y)
                        })
                    })
            }
            (V::Throw(_), V::Throw(_)) => {
                self.values += 1;
                true
            }
            (a, b) => {
                self.values += 1;
                a == b
            }
        }
    }

    fn check(&mut self, what: impl FnOnce() -> String, ours: V, want: &V) {
        if !self.same(&ours, want) && self.failures.len() < 40 {
            self.failures.push(format!("{}: ours {ours:?}, want {want:?}", what()));
        }
    }
}

fn result<T>(r: Result<T, pm_eval::JsThrow>, f: impl FnOnce(T) -> V) -> V {
    match r {
        Ok(v) => f(v),
        Err(e) => V::Throw(e.message),
    }
}

fn load() -> Value {
    let path = std::env::var("PM_LAYER_VECTORS").unwrap_or_else(|_| {
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/vectors/layer-vectors.json.gz").to_owned()
    });
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let mut text = String::new();
    flate2::read::GzDecoder::new(&bytes[..]).read_to_string(&mut text).unwrap();
    parse(&text).unwrap()
}

fn hexes(case: &Value, key: &str) -> Vec<f64> {
    get(case, key).unwrap().as_array().unwrap().iter().map(hexf).collect()
}

fn rect_obj(r: &pm_eval::space3d::Rect) -> V {
    V::O(
        [("x0", r.x0), ("y0", r.y0), ("x1", r.x1), ("y1", r.y1)]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), V::N(v)))
            .collect(),
    )
}

fn pairs(list: &[(&str, f64)]) -> V {
    V::O(list.iter().map(|(k, v)| ((*k).to_owned(), V::N(*v))).collect())
}

#[test]
fn layer_queries_match_typescript() {
    let vectors = load();
    let mut stats = Stats::default();
    for entry in get(&vectors, "docs").unwrap().as_array().unwrap() {
        let root = parse(get(entry, "doc").unwrap().as_str().unwrap()).unwrap();
        let comp_id = get(&root, "compId").unwrap().as_str().unwrap().to_owned();
        let doc = Document::new(Some(root), 1);
        let comp = Comp::resolve(&doc, &comp_id).unwrap();
        let layers = comp.layers();
        let mut ev = Evaluator::new();
        for case in get(entry, "cases").unwrap().as_array().unwrap() {
            let op = get(case, "op").unwrap().as_str().unwrap();
            *stats.ops.entry(op.to_owned()).or_default() += 1;
            let t = hexf(get(case, "t").unwrap());
            let want = dec(get(case, "out").unwrap());
            let mut scope = Scope::open(&mut ev, &comp);
            let index = get(case, "layer").map(|i| match i {
                Value::Number(n) => *n as usize,
                _ => unreachable!(),
            });
            let layer = index.map(|i| &layers[i]);
            let target_node = get(case, "target").map(|v| Node::from_value(to_value(&dec(v)), 1));
            let ours = match op {
                "depth" => {
                    let all: Vec<&Node> = layers.iter().collect();
                    result(scope.depth_ordered_layers(&all, t), |o| {
                        V::A(o.iter().map(|l| V::N(layers.iter().position(|x| std::ptr::eq(x, *l)).unwrap() as f64)).collect())
                    })
                }
                "roots" => {
                    let ids: Vec<JsValue> = get(case, "ids").unwrap().as_array().unwrap().iter().map(|v| to_js(&dec(v))).collect();
                    result(scope.transform_roots(&ids), |o| {
                        V::A(o.iter().map(|l| V::N(layers.iter().position(|x| std::ptr::eq(x, *l)).unwrap() as f64)).collect())
                    })
                }
                _ => {
                    let l = layer.unwrap();
                    match op {
                        "active" => result(scope.active(l, t), V::B),
                        "local" => result(scope.local_matrix(l, t), |m| nums(&m)),
                        "world" => result(scope.world_matrix(l, t), |m| nums(&m)),
                        "parent2d" => result(scope.transform_parent_matrix(l, t, None), |m| nums(&m)),
                        "opacity" => result(scope.world_opacity(l, t), V::N),
                        "local3d" => result(scope.local3d(l, t), |m| nums(&m)),
                        "parent3d" => result(scope.parent3d(l, t, None), |m| nums(&m)),
                        "world3d" => result(scope.world3d(l, t, false), |m| nums(&m)),
                        "world3dpos" => result(scope.world3d(l, t, true), |m| nums(&m)),
                        "is3d" => result(scope.is_3d_layer(l), V::B),
                        "persp" => result(scope.perspective_amount(l, t), V::N),
                        "plane" => result(scope.plane_matrix(l, t, false), |m| nums(&m)),
                        "planepos" => result(scope.plane_matrix(l, t, true), |m| nums(&m)),
                        "affine" => result(scope.plane_matrix(l, t, false), |m| {
                            pm_eval::space3d::affine_plane(&m).map_or(V::Z, |a| nums(&a))
                        }),
                        "inverse3d" => {
                            let world: [f64; 16] = hexes(case, "world").try_into().unwrap();
                            pm_eval::space3d::inverse3d(&world).map_or(V::Z, |a| nums(&a))
                        }
                        "pose3d" => {
                            let world: [f64; 16] = hexes(case, "world").try_into().unwrap();
                            let parent: [f64; 16] = hexes(case, "parent").try_into().unwrap();
                            result(scope.pose_values3d(l, &world, t, Some(&parent)), |p| pairs(&p.channels()))
                        }
                        "pose2d" | "pose2dzero" => {
                            let world: [f64; 6] = hexes(case, "world").try_into().unwrap();
                            let parent: [f64; 6] = hexes(case, "parent").try_into().unwrap();
                            result(scope.world_pose_values(l, &world, t, Some(&parent), op == "pose2dzero"), |p| pairs(&p.channels()))
                        }
                        "aim" => result(scope.world_aim3d(l, t), |a| a.map_or(V::Z, |a| nums(&a))),
                        "aimvalues" => {
                            let aim: [f64; 3] = hexes(case, "aim").try_into().unwrap();
                            let parent: [f64; 16] = hexes(case, "parent").try_into().unwrap();
                            result(scope.aim_values3d(l, &aim, t, Some(&parent)), |v| {
                                pairs(&[("targetX", v[0]), ("targetY", v[1]), ("targetZ", v[2])])
                            })
                        }
                        "contains" => {
                            let b: Vec<f64> = get(case, "b").unwrap().as_array().unwrap().iter().map(hexf).collect();
                            let rect = pm_eval::space3d::Rect { x0: b[0], y0: b[1], x1: b[2], y1: b[3] };
                            let (x, y) = (hexf(get(case, "x").unwrap()), hexf(get(case, "y").unwrap()));
                            result(scope.plane_contains(l, t, x, y, &rect), V::B)
                        }
                        "cycle" => {
                            let target = target_node.as_ref().unwrap();
                            result(scope.would_cycle(l, &pm_eval::node::Got::Node(target)), V::B)
                        }
                        "matte" => {
                            let target = pm_eval::node::js_value(target_node.as_ref().unwrap());
                            V::B(pm_eval::matte::valid_matte_source(layers, l, &target))
                        }
                        "sizeBounds" => result(scope.size_bounds(l, t), |b| b.map_or(V::Z, |b| rect_obj(&b))),
                        "sizeAnchor" => result(scope.size_anchor_offset(l, t), |o| pairs(&[("x", o.x), ("y", o.y)])),
                        "sourceTime" => result(scope.source_time_with(l, t, Retiming::Legacy, None), V::N),
                        "span" => result(scope.group_span(l), |s| pairs(&[("from", s.from), ("dur", s.dur)])),
                        "expand" => {
                            let id = pm_eval::node::get(l, b"id").to_js();
                            result(scope.expand_groups(&[id]), |o| {
                                V::A(o.iter().map(|x| from_js(&pm_eval::node::get(x, b"id").to_js())).collect())
                            })
                        }
                        other => panic!("unknown op {other}"),
                    }
                }
            };
            stats.check(|| format!("{comp_id} {op} layer {index:?} t={t}"), ours, &want);
        }
    }

    // Text animators and posed glyphs.
    for entry in get(&vectors, "texts").unwrap().as_array().unwrap() {
        let root = parse(get(entry, "doc").unwrap().as_str().unwrap()).unwrap();
        let comp_id = get(&root, "compId").unwrap().as_str().unwrap().to_owned();
        let doc = Document::new(Some(root), 1);
        let comp = Comp::resolve(&doc, &comp_id).unwrap();
        let layer = &comp.layers()[0];
        let layout: GlyphLayout = serde_json::from_str(get(entry, "layout").unwrap().as_str().unwrap()).unwrap();
        let mut ev = Evaluator::new();
        for case in get(entry, "cases").unwrap().as_array().unwrap() {
            *stats.ops.entry("text".into()).or_default() += 1;
            let t = hexf(get(case, "t").unwrap());
            let want = dec(get(case, "out").unwrap());
            let mut scope = Scope::open(&mut ev, &comp);
            let ours = (|| -> Result<V, pm_eval::JsThrow> {
                let state = scope.text_animation_state(layer, t, &layout)?;
                let d = scope.ev.resolve_content(&comp, layer, t)?;
                let glyphs = scope.animated_glyphs(&d, &layout, &state);
                let animators = V::A(
                    state
                        .animators
                        .iter()
                        .map(|a| {
                            V::O(BTreeMap::from([
                                ("id".to_owned(), from_js(&a.id)),
                                ("mode".to_owned(), V::S(a.mode.name().into())),
                                ("unit".to_owned(), V::S(a.unit.name().into())),
                                ("values".to_owned(), content(&a.values)),
                                ("weights".to_owned(), nums(&a.weights)),
                            ]))
                        })
                        .collect(),
                );
                let styles = V::A(state.styles.iter().map(|s| content(&s.item)).collect());
                let glyph_list = V::A(
                    glyphs
                        .iter()
                        .map(|g| {
                            let style = |k: &str| g.style.get(&JsString::from(k)).map_or(V::U, from_js);
                            V::O(BTreeMap::from([
                                ("index".to_owned(), V::N(g.unit.index as f64)),
                                ("x".to_owned(), V::N(g.unit.x)),
                                ("matrix".to_owned(), nums(&g.matrix)),
                                ("opacity".to_owned(), V::N(g.opacity)),
                                ("blur".to_owned(), V::N(g.blur)),
                                ("color".to_owned(), g.color.as_ref().map_or(V::Z, |c| V::S(c.to_string_lossy()))),
                                ("tracking".to_owned(), V::N(g.tracking)),
                                ("trackingShift".to_owned(), V::N(g.tracking_shift)),
                                (
                                    "style".to_owned(),
                                    V::O(
                                        ["color", "font", "size"]
                                            .iter()
                                            .map(|k| ((*k).to_owned(), style(k)))
                                            .filter(|(_, v)| *v != V::U)
                                            .collect(),
                                    ),
                                ),
                            ]))
                        })
                        .collect(),
                );
                let identity = V::A(glyphs.iter().map(|g| V::B(g.is_identity())).collect());
                Ok(V::O(BTreeMap::from([
                    ("animators".to_owned(), animators),
                    ("styles".to_owned(), styles),
                    ("glyphs".to_owned(), glyph_list),
                    ("identity".to_owned(), identity),
                ])))
            })()
            .unwrap_or_else(|e| V::Throw(e.message));
            stats.check(|| format!("{comp_id} text t={t}"), ours, &want);
        }
    }

    // Helpers.
    let mut springs = pm_eval::easing::SpringTables::new();
    for case in get(&vectors, "helpers").unwrap().as_array().unwrap() {
        let op = get(case, "op").unwrap().as_str().unwrap();
        *stats.ops.entry(op.to_owned()).or_default() += 1;
        let want = dec(get(case, "out").unwrap());
        let num = |k: &str| match get(case, k) {
            Some(Value::Number(n)) => *n,
            _ => unreachable!(),
        };
        let ours = match op {
            "countUnits" => {
                let lines: Vec<JsString> = get(case, "lines").unwrap().as_array().unwrap().iter().map(|l| l.as_js_string().unwrap().clone()).collect();
                let unit = match get(case, "unit").unwrap().as_str().unwrap() {
                    "words" => UnitKind::Words,
                    "lines" => UnitKind::Lines,
                    _ => UnitKind::Characters,
                };
                V::N(text_anim::count_text_units(&lines, unit) as f64)
            }
            "rank" => {
                let order = match dec(get(case, "order").unwrap()) {
                    V::S(s) => match s.as_str() {
                        "reverse" => text_anim::Order::Reverse,
                        "center" => text_anim::Order::Center,
                        "edges" => text_anim::Order::Edges,
                        "random" => text_anim::Order::Random,
                        _ => text_anim::Order::Forward,
                    },
                    _ => text_anim::Order::Forward,
                };
                V::N(text_anim::unit_rank(order, num("index") as usize, num("count") as usize, hexf(get(case, "seed").unwrap())))
            }
            "weight" => {
                let V::O(p) = dec(get(case, "p").unwrap()) else { unreachable!() };
                let p: Content = p.iter().map(|(k, v)| (JsString::from(k.as_str()), to_js(v))).collect();
                let shape = match dec(get(case, "shape").unwrap()) {
                    V::S(s) => match s.as_str() {
                        "rampUp" => Shape::RampUp,
                        "rampDown" => Shape::RampDown,
                        "triangle" => Shape::Triangle,
                        "round" => Shape::Round,
                        _ => Shape::Square,
                    },
                    _ => Shape::Square,
                };
                V::N(text_anim::animator_weight(&p, hexf(get(case, "rank").unwrap()), num("count") as usize, shape))
            }
            "ease" => {
                let easing = match dec(get(case, "easing").unwrap()) {
                    V::U => None,
                    v => Some(Node::from_value(to_value(&v), 1)),
                };
                // `Ease.spring` reads `tbl[NaN]` (undefined) for a NaN time; every
                // caller turns it into NaN arithmetic, which the port returns.
                let v = text_anim::ease_progress(&mut springs, easing.as_ref(), hexf(get(case, "progress").unwrap()));
                if v.is_nan() && want == V::U { V::U } else { V::N(v) }
            }
            "fontAnchor" => {
                let node = |k: &str| Node::from_value(to_value(&dec(get(case, k).unwrap())), 1);
                let (r, b) = (node("ref"), node("bounds"));
                let o = font_anchor_offset(
                    BoundsValues::of(Some(&r)).as_ref(),
                    BoundsValues::of(Some(&b)).as_ref(),
                    &to_js(&dec(get(case, "ax").unwrap())),
                    &to_js(&dec(get(case, "ay").unwrap())),
                );
                pairs(&[("x", o.x), ("y", o.y)])
            }
            other => panic!("unknown helper {other}"),
        };
        stats.check(|| format!("helper {op} {}", pm_model::json::to_canonical_string(case)), ours, &want);
    }

    eprintln!(
        "layer vectors: {} numbers ({} bit-identical, {} angles equal modulo 360, max relative error {:.3e}), {} other values",
        stats.numbers, stats.exact, stats.wrapped, stats.max_error, stats.values
    );
    eprintln!("ops: {:?}", stats.ops);
    for f in &stats.failures {
        eprintln!("  {f}");
    }
    assert!(stats.failures.is_empty(), "{} mismatches", stats.failures.len());
}
