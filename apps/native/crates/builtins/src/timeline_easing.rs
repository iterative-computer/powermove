//! Worker adapter for the existing Easing library service and kernel settings.
//! Archive and publication share one CAS transaction. No localStorage fallback,
//! invented preset defaults, synchronous actor wait or second store is used.
use crate::data::{model, wire};
use pm_kernel::{
    KernelHandle,
    state::{StateOp, StateTransaction},
};
use pm_services::surfaces::easing::{Library, STORAGE_KEY};
use serde_json::{Value, json};

pub(crate) const STORAGE: &str = "ext.builtin-easing";
pub(crate) const ARCHIVES: &str = "archives";
fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str, String> {
    v[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("Missing easing {key}"))
}
fn read(source: Value) -> Result<Library, String> {
    let source = if let Some(json) = source.as_str() {
        serde_json::from_str(json).map_err(|e| format!("Easing library JSON: {e}"))?
    } else {
        source
    };
    Library::read(source).map_err(|e| format!("Easing library: {:?}", e.problem))
}
pub(crate) fn inspect(kernel: &KernelHandle) -> Value {
    let state = kernel.state();
    let stored = state.settings.get(STORAGE).map(wire).unwrap_or(Value::Null);
    json!({"stateRevision":state.revision.to_string(),"source":stored[STORAGE_KEY],"archives":stored[ARCHIVES],"storage":stored})
}
pub(crate) async fn dispatch(kernel: &KernelHandle, input: &Value) -> Result<(), String> {
    let state = kernel.state();
    let expected = text(input, "stateRevision")?
        .parse::<u64>()
        .map_err(|_| "Invalid easing state revision")?;
    if state.revision != expected {
        return Err("Stale easing library".into());
    }
    let mut stored = match state.settings.get(STORAGE).map(wire) {
        None => serde_json::Map::new(),
        Some(Value::Object(stored)) => stored,
        Some(_) => return Err("Unrecognized easing storage; retained unchanged".into()),
    };
    let previous = stored.get(STORAGE_KEY).cloned();
    let mut archives = match stored.get(ARCHIVES).cloned() {
        None => vec![],
        Some(Value::Array(archives)) => archives,
        Some(_) => return Err("Unrecognized easing archive; retained unchanged".into()),
    };
    let action = text(input, "action")?;
    let next = match &previous {
        None if action == "import" => read(input["source"].clone())?.source().clone(),
        None => {
            return Err("Easing library not initialized; import the source library first".into());
        }
        Some(source) => {
            let library = read(source.clone())?;
            let change = match action {
                "addGroup" => library.add_group(text(input, "id")?, text(input, "name")?),
                "save" => {
                    let group = text(input, "group")?;
                    let id = text(input, "id")?;
                    let name = text(input, "name")?;
                    let curve = input["curve"].clone();
                    // Imported libraries can exist without the panel's built-in group.
                    // Create that group and save the curve in the same settings update.
                    let groups = library.source()["groups"].as_array().unwrap();
                    if group == "default" && !groups.iter().any(|g| g["id"] == group) {
                        library.add_group(group, "1").and_then(|added| {
                            Library::read(added.preview().clone())
                                .map_err(|e| e.problem)?
                                .save_preset(group, id, name, curve)
                        })
                    } else {
                        library.save_preset(group, id, name, curve)
                    }
                }
                "duplicate" => library.duplicate_preset(
                    text(input, "id")?,
                    text(input, "newId")?,
                    text(input, "name")?,
                ),
                "color" => library.set_color(
                    text(input, "id")?,
                    input["color"].as_str().ok_or("Missing easing color")?,
                ),
                "rename" => library.rename(text(input, "id")?, text(input, "name")?),
                "move" => library.move_preset(
                    text(input, "id")?,
                    text(input, "group")?,
                    input["before"].as_str(),
                ),
                "archive" => library.remove(text(input, "id")?),
                "import" => {
                    let incoming = read(input["source"].clone())?;
                    let fresh = input["ids"]
                        .as_array()
                        .ok_or("Missing easing import ids")?
                        .iter()
                        .map(|v| {
                            v.as_str()
                                .map(str::to_owned)
                                .ok_or("Invalid easing import id")
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    library.merge(&incoming, &fresh)
                }
                _ => return Err("Unknown easing library action".into()),
            }
            .map_err(|e| format!("Easing library: {e:?}"))?;
            // Change::preview is already validated by the service. This host
            // supplies its required atomic archive-and-publish implementation.
            change.preview().clone()
        }
    };
    if let Some(previous) = previous {
        archives.push(previous);
    }
    stored.insert(STORAGE_KEY.into(), next);
    stored.insert(ARCHIVES.into(), Value::Array(archives));
    kernel
        .state_transaction(StateTransaction {
            expected_revision: Some(expected),
            ops: vec![StateOp::Setting {
                key: STORAGE.into(),
                value: Some(model(&Value::Object(stored))?),
            }],
        })
        .await
        .ok_or("Kernel closed")?
        .map_err(|e| format!("{e:?}"))?;

    Ok(())
}

pub(crate) fn preset_name(id: &str) -> String {
    let mut name = String::new();
    for (i, c) in id.chars().enumerate() {
        if i > 0 && c.is_uppercase() {
            name.push(' ');
        }
        if i == 0 {
            name.extend(c.to_uppercase());
        } else {
            name.push(c);
        }
    }
    name
}

#[derive(Default)]
pub(crate) struct SelectedEase<'a> {
    pub curve: Option<[f64; 4]>,
    pub mixed: bool,
    pub interpolation: Option<&'a str>,
    pub mixed_interpolation: bool,
    pub segments: usize,
}

fn string(node: Option<&pm_model::Node>) -> Option<&str> {
    match node?.kind() {
        pm_model::NodeKind::String(value) => value.as_str(),
        _ => None,
    }
}

fn interpolation(key: &pm_model::Node) -> &str {
    let flag = |name| {
        matches!(
            key.get(name).map(pm_model::Node::kind),
            Some(pm_model::NodeKind::Bool(true))
        )
    };
    if flag("autoBezier") {
        "auto"
    } else if flag("hold") {
        "hold"
    } else if flag("continuous") {
        "continuous"
    } else {
        string(key.get("outInterp")).unwrap_or("linear")
    }
}

#[derive(Default)]
struct LayerLookup {
    layers: Option<pm_model::Node>,
    indices: std::collections::HashMap<pm_model::JsString, usize>,
}

impl LayerLookup {
    fn positions(
        &mut self,
        stack: &pm_model::Node,
        ids: &std::collections::HashSet<&str>,
    ) -> Vec<usize> {
        use pm_model::NodeKind;
        let NodeKind::Array(layers) = stack.kind() else {
            return Vec::new();
        };
        let current = self.layers.as_ref().is_some_and(|previous| {
            matches!(previous.kind(), NodeKind::Array(previous) if std::sync::Arc::ptr_eq(previous, layers))
        });
        if !current {
            self.indices.clear();
            self.indices.reserve(layers.len());
            for (i, layer) in layers.iter().enumerate() {
                if let Some(NodeKind::String(id)) = layer.get("id").map(pm_model::Node::kind) {
                    self.indices.entry(id.clone()).or_insert(i);
                }
            }
            self.layers = Some(stack.clone());
        }
        let mut positions: Vec<_> = ids
            .iter()
            .filter_map(|id| self.indices.get(id.as_bytes()).copied())
            .collect();
        positions.sort_unstable();
        positions
    }
}

thread_local! {
    // The easing realm keeps one immutable stack and reuses its index across
    // selection and transport publications. A changed stack replaces it.
    static LAYER_LOOKUP: std::cell::RefCell<LayerLookup> = Default::default();
}

/// Read the selected keys directly from layers in the open composition.
pub(crate) fn selected_ease<'a>(
    root: &'a pm_model::Node,
    selection: &pm_kernel::selection::Selection,
) -> SelectedEase<'a> {
    use pm_kernel::model::{comp_node, open_comp_id};
    use pm_model::{Node, NodeKind};
    use std::collections::{HashMap, HashSet};

    let mut selected = SelectedEase::default();
    if selection.keys().is_empty() {
        return selected;
    }
    let Some(comp) = comp_node(root, open_comp_id(root).as_deref()) else {
        return selected;
    };
    let Some(stack) = comp.get("layers") else {
        return selected;
    };
    let NodeKind::Array(layers) = stack.kind() else {
        return selected;
    };
    let ranks: HashMap<_, _> = selection
        .keys()
        .iter()
        .map(String::as_str)
        .enumerate()
        .map(|(rank, id)| (id, rank))
        .collect();
    let mut owners: HashMap<&str, HashSet<&str>> = HashMap::new();
    for (layer, key) in selection.key_layers() {
        owners.entry(key).or_default().insert(layer);
    }
    let has_unqualified_keys = owners.len() < ranks.len();
    let mut layer_ids: HashSet<_> = selection
        .key_layers()
        .iter()
        .map(|(layer, _)| layer.as_str())
        .collect();
    if has_unqualified_keys {
        layer_ids.extend(selection.layers().iter().map(String::as_str));
    }
    let all_layers = has_unqualified_keys && selection.layers().is_empty();

    fn visit_tracks<'a>(node: &'a Node, read: &mut impl FnMut(&'a [Node])) {
        match node.kind() {
            NodeKind::Object(members) => {
                for (name, child) in members.iter() {
                    if name.as_str() == Some("kf") {
                        if let NodeKind::Array(keys) = child.kind() {
                            if !keys.is_empty() {
                                read(keys);
                            }
                        }
                    } else {
                        visit_tracks(child, read);
                    }
                }
            }
            NodeKind::Array(items) => {
                for child in items.iter() {
                    visit_tracks(child, read);
                }
            }
            _ => {}
        }
    }

    let mut curve_rank = usize::MAX;
    let mut first_curve = None;
    let mut interpolation_rank = usize::MAX;
    let mut first_interp: Option<&'a str> = None;
    // A key selected by ID alone and without a layer can be on any layer.
    let positions = if all_layers {
        Vec::new()
    } else {
        LAYER_LOOKUP.with_borrow_mut(|index| index.positions(stack, &layer_ids))
    };
    let unknown = (0..layers.len()).take(if all_layers { layers.len() } else { 0 });
    for i in unknown.chain(positions) {
        let layer = &layers[i];
        let layer_id = string(layer.get("id"));
        visit_tracks(layer, &mut |keys| {
            let mut sel: Vec<(usize, usize)> = Vec::new();
            for (idx, key) in keys.iter().enumerate() {
                let Some(id) = string(key.get("i")) else {
                    continue;
                };
                let Some(&rank) = ranks.get(id) else {
                    continue;
                };
                if owners
                    .get(id)
                    .is_some_and(|allowed| !layer_id.is_some_and(|id| allowed.contains(id)))
                {
                    continue;
                }
                let interp = interpolation(key);
                let first = *first_interp.get_or_insert(interp);
                if interp != first {
                    selected.mixed_interpolation = true;
                }
                if rank < interpolation_rank {
                    selected.interpolation = Some(interp);
                    interpolation_rank = rank;
                }
                sel.push((idx, rank));
            }
            if sel.is_empty() {
                return;
            }
            let mut record_curve = |curve: [f64; 4], rank: usize| {
                let first = first_curve.get_or_insert(curve);
                selected.mixed |= curve
                    .iter()
                    .zip(first.iter())
                    .any(|(a, b)| (a - b).abs() > 1e-5);
                if rank < curve_rank {
                    selected.curve = Some(curve);
                    curve_rank = rank;
                }
            };
            // Consecutive selected keyframes on a native temporal track define
            // transition segments `(k[i].outEase, k[i+1].inEase)`. Reading the
            // actual segment avoids mixing in untouched outer handles!
            let mut covered = vec![false; sel.len()];
            for pos in 0..sel.len().saturating_sub(1) {
                let (a, rank_a) = sel[pos];
                let (b, rank_b) = sel[pos + 1];
                if b == a + 1 {
                    selected.segments += 1;
                    let native_pair =
                        keys[a].get("outEase").is_some() || keys[b].get("inEase").is_some();
                    if native_pair {
                        covered[pos] = true;
                        covered[pos + 1] = true;
                        if let Some(curve) = segment_curve(keys, a, b) {
                            record_curve(curve, rank_a.min(rank_b));
                        }
                    }
                }
            }
            for (pos, &(idx, rank)) in sel.iter().enumerate() {
                if covered[pos] {
                    continue;
                }
                if let Some(curve) = key_curve(keys, idx) {
                    record_curve(curve, rank);
                }
            }
        });
    }
    selected
}

/// Read the cubic easing curve of a single consecutive segment `(from -> to)`.
fn segment_curve(keys: &[pm_model::Node], from: usize, to: usize) -> Option<[f64; 4]> {
    use pm_eval::temporal::{Side, make_ease, unit_handle_from_ease};
    let a = wire(&keys.get(from)?.to_value());
    let b = wire(&keys.get(to)?.to_value());
    let dt = b["t"].as_f64()? - a["t"].as_f64()?;
    let dv = b["v"].as_f64()? - a["v"].as_f64()?;
    let span = (dt.abs(), dv * dt.signum());
    let handle = |key: &Value, field: &str, interp: &str, legacy: &str, side| {
        let linear = if legacy == "eo" { [0., 0.] } else { [1., 1.] };
        if key[field].is_object() {
            if key[interp] != "bezier" {
                return Some(linear);
            }
            unit_handle_from_ease(
                make_ease(
                    key[field]["speed"].as_f64()?,
                    key[field]["influence"].as_f64()?,
                ),
                span.0,
                span.1,
                side,
            )
        } else if let Some(h) = key[legacy].as_array() {
            Some([h.first()?.as_f64()?, h.get(1)?.as_f64()?])
        } else {
            Some(linear)
        }
    };
    let out = handle(&a, "outEase", "outInterp", "eo", Side::Out)?;
    let incoming = handle(&b, "inEase", "inInterp", "ei", Side::In)?;
    Some([out[0], out[1], incoming[0], incoming[1]])
}

#[cfg(test)]
fn selected_curve(root: &pm_model::Node, ids: &[String]) -> (Option<[f64; 4]>, bool) {
    let selection = pm_kernel::selection::Selection::from_parts(vec![], ids.to_vec(), None);
    let selected = selected_ease(root, &selection);
    (selected.curve, selected.mixed)
}
// A key owns both handles, measured against its previous/next spans, as
// SetEasing writes them. Reading the next key's incoming handle would lose
// the selected preset when just one key is selected.
fn key_curve(keys: &[pm_model::Node], index: usize) -> Option<[f64; 4]> {
    use pm_eval::temporal::{Side, make_ease, unit_handle_from_ease};
    let key = wire(&keys.get(index)?.to_value());
    let next = keys.get(index + 1).map(|n| wire(&n.to_value()));
    let previous = index
        .checked_sub(1)
        .and_then(|i| keys.get(i))
        .map(|n| wire(&n.to_value()));
    let span = |other: Option<&Value>| -> Option<(f64, f64)> {
        let Some(other) = other else {
            return Some((1., 1.));
        };
        let dt = other["t"].as_f64()? - key["t"].as_f64()?;
        Some((
            dt.abs(),
            (other["v"].as_f64()? - key["v"].as_f64()?) * dt.signum(),
        ))
    };
    let handle = |field: &str, interp: &str, legacy: &str, side, span: Option<(f64, f64)>| {
        let linear = if legacy == "eo" { [0., 0.] } else { [1., 1.] };
        if key[field].is_object() {
            if key[interp] != "bezier" {
                return Some(linear);
            }
            let (dt, dv) = span?;
            unit_handle_from_ease(
                make_ease(
                    key[field]["speed"].as_f64()?,
                    key[field]["influence"].as_f64()?,
                ),
                dt,
                dv,
                side,
            )
        } else if let Some(h) = key[legacy].as_array() {
            Some([h.first()?.as_f64()?, h.get(1)?.as_f64()?])
        } else {
            Some(linear)
        }
    };
    let out = handle(
        "outEase",
        "outInterp",
        "eo",
        Side::Out,
        span(next.as_ref().or(previous.as_ref())),
    )?;
    let incoming = handle(
        "inEase",
        "inInterp",
        "ei",
        Side::In,
        span(previous.as_ref().or(next.as_ref())),
    )?;
    Some([out[0], out[1], incoming[0], incoming[1]])
}

#[cfg(test)]
mod panel_tests {
    use super::*;
    #[test]
    fn easing_short_circuits_and_indexes_key_owners_on_639_layers() {
        let stack: Vec<_> = (0..639)
            .map(|i| {
                json!({
                    "id": format!("l{i}"),
                    "p": {"x":{"kf":[{"i":"selected","t":0,"v":0,"hold":i != 638,
                        "eo":[0.25,0],"ei":[0.75,1]}]}}
                })
            })
            .collect();
        let root = pm_model::Node::from_value(model(&json!({"layers":stack})).unwrap(), 1);
        let empty = pm_kernel::selection::Selection::default();
        let owner = pm_kernel::selection::Selection::from_parts(
            vec!["l638".into()],
            vec!["selected".into()],
            None,
        );
        let time = |selection: &pm_kernel::selection::Selection, expected: Option<&str>, cold: bool| {
            let mut samples = Vec::new();
            for _ in 0..21 {
                if cold {
                    LAYER_LOOKUP.with_borrow_mut(|index| *index = LayerLookup::default());
                }
                let start = std::time::Instant::now();
                let state = selected_ease(&root, selection);
                samples.push(start.elapsed());
                assert_eq!(state.interpolation, expected);
                assert_eq!(state.curve, expected.map(|_| [0.25, 0., 0.75, 1.]));
            }
            samples.sort_unstable();
            samples[samples.len() / 2]
        };
        let empty = time(&empty, None, false);
        let cold = time(&owner, Some("linear"), true);
        let cached = time(&owner, Some("linear"), false);
        eprintln!(
            "639-layer Easing median: empty {:.6} ms, indexed {:.6} ms; cold {:.6} ms",
            empty.as_secs_f64() * 1e3,
            cached.as_secs_f64() * 1e3,
            cold.as_secs_f64() * 1e3
        );
        assert!(empty < std::time::Duration::from_micros(500), "{empty:?}");
        assert!(cached < std::time::Duration::from_micros(500), "{cached:?}");
        assert!(cold < std::time::Duration::from_micros(500), "{cold:?}");
    }

    #[test]
    fn selected_curve_reads_stored_temporal_handles_and_mixed_tracks() {
        let project = crate::data::model(&json!({"layers":[{"p":{"x":{"kf":[
            {"i":"a","t":0,"v":0,"outInterp":"bezier","outEase":{"speed":0,"influence":42},"inInterp":"bezier","inEase":{"speed":0,"influence":42}},
            {"i":"b","t":1,"v":100,"inInterp":"bezier","inEase":{"speed":0,"influence":42}}
        ]},"y":{"kf":[{"i":"c","t":0,"v":0,"eo":[0,0]},{"i":"d","t":1,"v":1,"ei":[1,1]}]}}}]}));
        let document = pm_model::Document::new(Some(project.unwrap()), 1);
        let root = document.lookup(&[]).unwrap();
        assert_eq!(selected_curve(root, &[]), (None, false));
        let (curve, mixed) = selected_curve(root, &["a".into()]);
        assert!(!mixed);
        for (a, b) in curve.unwrap().iter().zip([0.42, 0., 0.58, 1.]) {
            assert!((a - b).abs() < 1e-12);
        }
        assert!(selected_curve(root, &["a".into(), "c".into()]).1);
        assert_eq!(preset_name("expoOut"), "Expo Out");
    }
    #[test]
    fn selected_ease_reads_curve_and_interpolation_only_from_selected_layers() {
        let value = crate::data::model(&json!({
            "compId": "current",
            "comps": {
                "current": {"layers": [
                    {"id":"unselected","p":{"x":{"kf":[{"i":"a","t":0,"v":0,"hold":true}]}}},
                    {"id":"selected","p":{"x":{"kf":[
                        {"i":"a","t":0,"v":0,"continuous":true,"outInterp":"bezier","eo":[0.4,0],"ei":[0.6,1]},
                        {"i":"b","t":1,"v":1,"autoBezier":true}
                    ]}}}
                ]},
                "other": {"layers":[{"id":"selected","p":{"x":{"kf":[{"i":"c","t":0,"v":0,"hold":true}]}}}]}
            }
        })).unwrap();
        let document = pm_model::Document::new(Some(value), 1);
        let root = document.root().unwrap();
        let selection = |layers: &[&str], keys: &[&str]| {
            pm_kernel::selection::Selection::from_parts(
                layers.iter().map(|s| (*s).into()).collect(),
                keys.iter().map(|s| (*s).into()).collect(),
                None,
            )
        };

        let selected = selected_ease(root, &selection(&["selected"], &["a", "b", "c"]));
        assert_eq!(selected.interpolation, Some("continuous"));
        assert_eq!(selected.curve, Some([0.4, 0., 0.6, 1.]));
        assert!(selected.mixed);
        let fallback = selected_ease(root, &selection(&[], &["b", "c"]));
        assert_eq!(fallback.interpolation, Some("auto"));
        assert!(!fallback.mixed);
        let other_comp = selected_ease(root, &selection(&[], &["c"]));
        assert_eq!(other_comp.interpolation, None);
        assert_eq!(other_comp.curve, None);
    }

    #[test]
    fn selected_ease_reads_segment_curve_for_consecutive_native_keyframes() {
        // 3 native keyframes: k0 -> k1 has [0.7, 0, 1, 1], k1 -> k2 has [0.2, 0, 0.4, 1].
        let value = crate::data::model(&json!({
            "layers": [{
                "id": "L1",
                "p": {
                    "x": {
                        "kf": [
                            {"i": "k0", "t": 0, "v": 0, "inInterp": "linear", "outInterp": "bezier",
                             "inEase": {"speed": 0, "influence": 33.33}, "outEase": {"speed": 0, "influence": 70}},
                            {"i": "k1", "t": 1, "v": 100, "inInterp": "linear", "outInterp": "bezier",
                             "inEase": {"speed": 0, "influence": 33.33}, "outEase": {"speed": 0, "influence": 20}},
                            {"i": "k2", "t": 2, "v": 200, "inInterp": "bezier", "outInterp": "linear",
                             "inEase": {"speed": 0, "influence": 60}, "outEase": {"speed": 0, "influence": 33.33}}
                        ]
                    }
                }
            }]
        }))
        .unwrap();
        let document = pm_model::Document::new(Some(value), 1);
        let root = document.root().unwrap();
        let sel = |keys: &[&str]| {
            pm_kernel::selection::Selection::from_parts(
                vec!["L1".into()],
                keys.iter().map(|s| (*s).into()).collect(),
                None,
            )
        };
        // Selecting [k1, k2] reads segment k1 -> k2 ([0.2, 0, 0.4, 1]), NOT mixed!
        let pair = selected_ease(root, &sel(&["k1", "k2"]));
        assert_eq!(pair.segments, 1);
        assert!(!pair.mixed);
        let curve = pair.curve.expect("segment curve");
        for (a, b) in curve.iter().zip([0.2, 0.0, 0.4, 1.0]) {
            assert!((a - b).abs() < 1e-6, "got {curve:?}");
        }
        // Selecting all 3 keys [k0, k1, k2] spans 2 distinct segments -> mixed!
        let all = selected_ease(root, &sel(&["k0", "k1", "k2"]));
        assert_eq!(all.segments, 2);
        assert!(all.mixed);
    }
}
