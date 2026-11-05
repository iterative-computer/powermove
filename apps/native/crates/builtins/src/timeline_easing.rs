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
