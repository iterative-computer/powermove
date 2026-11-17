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
                "save" => library.save_preset(
                    text(input, "group")?,
                    text(input, "id")?,
                    text(input, "name")?,
                    input["curve"].clone(),
                ),
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

/// Read only the selected properties, using the same temporal math as playback.
pub(crate) fn selected_curve(root: &pm_model::Node, ids: &[String]) -> (Option<[f64; 4]>, bool) {
    use pm_model::NodeKind;
    fn visit(node: &pm_model::Node, ids: &[String], curves: &mut Vec<[f64; 4]>) {
        match node.kind() {
            NodeKind::Object(members) => {
                for (name, child) in members.iter() {
                    if name.as_str() == Some("kf") {
                        if let NodeKind::Array(keys) = child.kind() {
                            for (i, key) in keys.iter().enumerate() {
                                let selected = key
                                    .get("i")
                                    .and_then(|n| match n.kind() {
                                        NodeKind::String(s) => s.as_str(),
                                        _ => None,
                                    })
                                    .is_some_and(|id| ids.iter().any(|s| s == id));
                                if !selected {
                                    continue;
                                }
                                if let Some(c) = key_curve(keys, i) {
                                    curves.push(c);
                                }
                            }
                        }
                    } else {
                        visit(child, ids, curves);
                    }
                }
            }
            NodeKind::Array(items) => {
                for child in items.iter() {
                    visit(child, ids, curves);
                }
            }
            _ => {}
        }
    }
    let mut curves = vec![];
    if !ids.is_empty() {
        visit(root, ids, &mut curves);
    }
    let first = curves.first().copied();
    let mixed = first.is_some_and(|a| {
        curves
            .iter()
            .any(|b| a.iter().zip(b).any(|(x, y)| (x - y).abs() > 1e-5))
    });
    (first, mixed)
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
}
