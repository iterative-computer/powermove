use crate::fixture::{Fixture, Scratch, clip_project, layer, tool_error, without_revision};
use pm_kernel::{
    Call, Caller, Command, HistoryCall, KernelActor, KernelConfig, OpenSource, Origin, Reply,
    Request, TimingCommand, WriteBody,
};
use pm_mcp::{ActorAutomation, CaptureRequest, EngineHook, SurfaceHook};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

type Check = Result<(), String>;
pub struct Scenario {
    pub id: &'static str,
    pub workflow: &'static str,
    pub title: &'static str,
    pub run: fn() -> Check,
}
pub fn all() -> Vec<Scenario> {
    vec![
        Scenario {
            id: "W061.frozen-set-easing",
            workflow: "W061",
            title: "Frozen sequential set_property Glide retains both complete authored handles",
            run: crate::curve_oracle::set,
        },
        Scenario {
            id: "W061.frozen-replace-easing",
            workflow: "W061",
            title: "Frozen replace_keyframes Glide matches independent complete-curve oracle",
            run: crate::curve_oracle::replace,
        },
        Scenario {
            id: "A532",
            workflow: "W037",
            title: "MCP actual effect validator: frozen acceptance, exact diagnostic order and no project edits",
            run: crate::mcp_cases::validate_effect,
        },
        Scenario {
            id: "A539",
            workflow: "W031",
            title: "MCP evaluated structural lint, source bounds and measured audio peaks",
            run: crate::closure_mcp::lint,
        },
        Scenario {
            id: "A529",
            workflow: "W033",
            title: "MCP simplification scopes retain locked, expressed and nonredundant tracks",
            run: crate::closure_mcp::simplify,
        },
        Scenario {
            id: "B258",
            workflow: "W057",
            title: "MCP slip preserves strip and animation, enforces handles and persists Undo/Redo",
            run: crate::closure_mcp::slip,
        },
        Scenario {
            id: "A504",
            workflow: "W033",
            title: "All atomic strip operations, refusal and persisted history",
            run: crate::closure_mcp::strips,
        },
        Scenario {
            id: "A508",
            workflow: "W050",
            title: "3D scene digest against independent tagged fixtures",
            run: crate::closure_mcp::scene,
        },
        Scenario {
            id: "A513",
            workflow: "W031",
            title: "Media status, nested references, locks and paging",
            run: crate::closure_mcp::list_media,
        },
        Scenario {
            id: "A516",
            workflow: "W031",
            title: "Media folders, metadata, deletion refusal and persisted Undo/Redo",
            run: crate::closure_mcp::manage_media,
        },
        Scenario {
            id: "A530",
            workflow: "W033",
            title: "All nine MCP video edits, exact source timing and persisted Undo/Redo",
            run: crate::closure_mcp::video,
        },
        Scenario {
            id: "A550",
            workflow: "W067",
            title: "SRT/WebVTT composition-time clipping, normalization and escaping",
            run: crate::closure_mcp::captions,
        },
        Scenario {
            id: "W057.mcp-refusals",
            workflow: "W057",
            title: "MCP source handles and locked edits refuse atomically",
            run: crate::closure_mcp::video_refusals,
        },
        Scenario {
            id: "A494",
            workflow: "W033",
            title: "MCP union exposes frozen tools and implemented catalog members",
            run: crate::mcp_cases::union,
        },
        Scenario {
            id: "A503",
            workflow: "W033",
            title: "MCP timeline source ranges, stack order, flags, selection and paging",
            run: crate::mcp_cases::timeline,
        },
        Scenario {
            id: "A505",
            workflow: "W033",
            title: "MCP library list/create/open/rename/duplicate/save/trash/restore",
            run: crate::mcp_cases::manage,
        },
        Scenario {
            id: "A520",
            workflow: "W032",
            title: "MCP saved panel layout reads preserve project and app context",
            run: crate::mcp_cases::layout,
        },
        Scenario {
            id: "W079.shuttle",
            workflow: "W079",
            title: "State-dependent J/K/L and actor transport facts",
            run: crate::pro::shuttle,
        },
        Scenario {
            id: "W080.ripple-delete",
            workflow: "W080",
            title: "Ripple delete preserves untargeted tracks; locked refusal and persisted Undo",
            run: crate::pro::ripple_delete,
        },
        Scenario {
            id: "W080.ripple-trim",
            workflow: "W080",
            title: "Ripple trim exact deltas, locks and persisted Undo",
            run: crate::pro::ripple_trim,
        },
        Scenario {
            id: "W081.roll",
            workflow: "W081",
            title: "Roll source continuity, handles and persisted Undo",
            run: crate::pro::roll,
        },
        Scenario {
            id: "W081.slide",
            workflow: "W081",
            title: "Slide compensates neighbors without changing middle source",
            run: crate::pro::slide,
        },
        Scenario {
            id: "W082.source",
            workflow: "W082",
            title: "Three-point insert/overwrite splits preserve source, other tracks and persisted Undo",
            run: crate::pro::source,
        },
        Scenario {
            id: "W083.links",
            workflow: "W083",
            title: "Persistent A/V link, move, bypass, split and unlink",
            run: crate::pro::links,
        },
        Scenario {
            id: "W071.fractional-rate",
            workflow: "W071",
            title: "N3: an authored 29.97 composition rate must remain 29.97",
            run: fractional_rate,
        },
        Scenario {
            id: "W009.settings",
            workflow: "W009",
            title: "Composition size/rate/duration preserve exact settings through save",
            run: composition_settings,
        },
        Scenario {
            id: "W048.mask",
            workflow: "W048",
            title: "Typed mask edits retain identity, geometry, Undo and Redo",
            run: mask,
        },
        Scenario {
            id: "W049.text",
            workflow: "W049",
            title: "Authored Unicode and paragraph box preserve text size and scale",
            run: text,
        },
        Scenario {
            id: "W061.easing",
            workflow: "W061",
            title: "Authored Bézier handles persist without changing key values",
            run: easing,
        },
        Scenario {
            id: "A495",
            workflow: "W033",
            title: "MCP list_projects includes current and saved independent projects",
            run: list_projects,
        },
        Scenario {
            id: "A496",
            workflow: "W033",
            title: "MCP creates and opens an empty project without dialogs",
            run: create_project,
        },
        Scenario {
            id: "A497",
            workflow: "W033",
            title: "MCP reopens a saved project by ID",
            run: open_project,
        },
        Scenario {
            id: "A498",
            workflow: "W033",
            title: "MCP project and app sessions pin their context",
            run: start_session,
        },
        Scenario {
            id: "A501",
            workflow: "W033",
            title: "MCP finish commits to Undo and false restores the run baseline",
            run: finish_session,
        },
        Scenario {
            id: "A517",
            workflow: "W030",
            title: "MCP selects existing IDs, adds and clears selection",
            run: select_layers,
        },
        Scenario {
            id: "A531",
            workflow: "W033",
            title: "MCP rollback keeps earlier project work",
            run: rollback,
        },
        Scenario {
            id: "W006.library",
            workflow: "W006",
            title: "Rename, duplicate, trash and restore independent library projects",
            run: library,
        },
        Scenario {
            id: "W007.portable",
            workflow: "W007",
            title: "SaveAs then reopen PMV4 with persisted undo",
            run: portable,
        },
        Scenario {
            id: "W004.journal",
            workflow: "W004",
            title: "Acknowledged edits recover from the real kernel journal",
            run: journal,
        },
        Scenario {
            id: "W030.atomic",
            workflow: "W030",
            title: "Invalid late command leaves the whole edit batch unchanged",
            run: atomic,
        },
        Scenario {
            id: "W030.interleaved",
            workflow: "W030",
            title: "Stale revisions and interleaved rollback preserve foreign work",
            run: interleaved,
        },
        Scenario {
            id: "W030.lock",
            workflow: "W030",
            title: "Generated unlock-and-edit cannot defeat a layer lock",
            run: locked,
        },
        Scenario {
            id: "W033.disconnect",
            workflow: "W033",
            title: "Disconnect rolls back run edits and removes only its staging",
            run: disconnect,
        },
        Scenario {
            id: "W033.unavailable",
            workflow: "W033",
            title: "Pending tools return explicit errors with backing",
            run: unavailable,
        },
        Scenario {
            id: "W046.selection",
            workflow: "W046",
            title: "Selection is transient and preserves document revision/history",
            run: selection_transient,
        },
        Scenario {
            id: "W052.work",
            workflow: "W052",
            title: "Composition work bounds retain exact authored values",
            run: work,
        },
        Scenario {
            id: "W053.markers",
            workflow: "W053",
            title: "Marker commands persist authored ID/time and undo",
            run: markers,
        },
        Scenario {
            id: "W055.strips",
            workflow: "W055",
            title: "Explicit-target strip move is atomic and undoable",
            run: strips,
        },
        Scenario {
            id: "W056.split",
            workflow: "W056",
            title: "Typed split retains source continuity and local animation",
            run: split,
        },
        Scenario {
            id: "W057.slip",
            workflow: "W057",
            title: "Typed slip accepts 5+3*2 and rejects 8+3*2; undo/redo",
            run: slip,
        },
        Scenario {
            id: "W060.keys",
            workflow: "W060",
            title: "Keyframe IDs, authored times and values persist",
            run: keys,
        },
        Scenario {
            id: "W062.groups",
            workflow: "W062",
            title: "Group/ungroup maintains stable member identity",
            run: groups,
        },
        Scenario {
            id: "W063.properties",
            workflow: "W063",
            title: "Property batch retains f64 values across save/reopen",
            run: properties,
        },
        Scenario {
            id: "W075.history",
            workflow: "W075",
            title: "Actor Undo/Redo and source-edit cancellation use one writer",
            run: actor_history,
        },
        Scenario {
            id: "W076.engine-hook",
            workflow: "W076",
            title: "Engine double receives exact revision/time; stale request refused",
            run: engine_hook,
        },
        Scenario {
            id: "W032.surface-hook",
            workflow: "W032",
            title: "Surface double is inspected/driven by stable IDs",
            run: surface_hook,
        },
    ]
}
fn list_projects() -> Check {
    let mut f = Fixture::new()?;
    let id = f.id.clone();
    f.call("finish_session", json!({"commit":true}))?;
    let second = f.call("create_project", json!({"name":"Second synthetic"}))?;
    assert_ne!(second["projectId"], id);
    let listed = f.call("list_projects", json!({}))?;
    assert_eq!(listed["currentProjectId"], second["projectId"]);
    let rows = listed["projects"].as_array().unwrap();
    assert!(rows.iter().any(|p| p["id"] == id));
    assert!(rows.iter().any(|p| p["id"] == second["projectId"]));
    Ok(())
}
fn create_project() -> Check {
    let f = Fixture::new()?;
    let doc = f.document()?;
    assert_eq!(doc["w"], 320);
    assert_eq!(doc["h"], 180);
    assert_eq!(doc["fps"], 30);
    assert_eq!(doc["dur"], 12);
    assert_eq!(doc["layers"], json!([]));
    assert_eq!(
        f.host.lock().unwrap().active.as_deref(),
        Some(f.id.as_str())
    );
    Ok(())
}
fn open_project() -> Check {
    let mut f = Fixture::new()?;
    f.add("L-kept")?;
    f.call("finish_session", json!({"commit":true}))?;
    f.host
        .lock()
        .unwrap()
        .save_library(&f.id)
        .map_err(|e| e.to_string())?;
    f.call("create_project", json!({"name":"Second synthetic"}))?;
    let reopened = f.call("open_project", json!({"projectId":f.id}))?;
    assert_eq!(reopened["projectId"], f.id);
    assert_eq!(layer(&f.document()?, "L-kept")["id"], "L-kept");
    Ok(())
}
fn start_session() -> Check {
    let mut f = Fixture::new()?;
    assert_eq!(
        tool_error(
            &mut f.session,
            "create_project",
            json!({"name":"Forbidden while pinned"})
        )?["code"],
        "invalid_arguments"
    );
    f.call("finish_session", json!({"commit":true}))?;
    let app = f.call(
        "start_session",
        json!({"prompt":"Synthetic app","context":"app"}),
    )?;
    assert_eq!(app["projectId"], "powermove-global");
    assert!(tool_error(&mut f.session, "get_project_state", json!({}))?.is_object());
    f.call("finish_session", json!({"commit":true}))?;
    let project = f.call("start_session", json!({"prompt":"Synthetic project"}))?;
    assert_eq!(project["projectId"], f.id);
    Ok(())
}
fn finish_session() -> Check {
    let mut f = Fixture::new()?;
    f.add("L-one")?;
    f.add("L-two")?;
    let result = f.call("finish_session", json!({"commit":true}))?;
    assert_eq!(result["changed"], true);
    assert!(result["historyId"].is_string());
    assert_eq!(
        f.host.lock().unwrap().store(&f.id).unwrap().history().len(),
        1
    );
    f.host
        .lock()
        .unwrap()
        .undo(&f.id, Origin::Mcp, 41)
        .map_err(|e| e.to_string())?;
    assert_eq!(f.document()?["layers"], json!([]));
    f.call("start_session", json!({"prompt":"Synthetic cancel"}))?;
    f.add("L-cancel")?;
    f.call("finish_session", json!({"commit":false}))?;
    assert_eq!(f.document()?["layers"], json!([]));
    Ok(())
}
fn select_layers() -> Check {
    let mut f = Fixture::new()?;
    f.add("L-one")?;
    f.add("L-two")?;
    let selected = f.call("select_layers", json!({"layerIds":["L-one"]}))?;
    assert_eq!(selected["selection"]["layers"], json!(["L-one"]));
    let selected = f.call("select_layers", json!({"layerIds":["L-two"],"add":true}))?;
    assert_eq!(selected["selection"]["layers"], json!(["L-one", "L-two"]));
    let selected = f.call("select_layers", json!({"layerIds":[]}))?;
    assert_eq!(selected["selection"]["layers"], json!([]));
    Ok(())
}
fn rollback() -> Check {
    let mut f = Fixture::new()?;
    f.add("L-before")?;
    f.call("finish_session", json!({"commit":true}))?;
    let baseline = without_revision(f.document()?);
    f.call("start_session", json!({"prompt":"Synthetic rollback"}))?;
    f.add("L-run")?;
    f.call("rollback_changes", json!({}))?;
    assert_eq!(without_revision(f.document()?), baseline);
    Ok(())
}
fn library() -> Check {
    let mut f = Fixture::new()?;
    f.add("L-kept")?;
    f.call(
        "manage_project",
        json!({"operation":"rename","name":"Renamed synthetic"}),
    )?;
    assert_eq!(f.document()?["name"], "Renamed synthetic");
    f.call("finish_session", json!({"commit":true}))?;
    f.call(
        "start_session",
        json!({"prompt":"Synthetic library","context":"app"}),
    )?;
    let duplicate = f.call(
        "manage_project",
        json!({"operation":"duplicate","projectId":f.id,"name":"Synthetic copy"}),
    )?;
    let copy = duplicate["projectId"]
        .as_str()
        .ok_or_else(|| format!("missing duplicate id: {duplicate}"))?
        .to_owned();
    assert_ne!(copy, f.id);
    f.call(
        "manage_project",
        json!({"operation":"trash","projectId":copy}),
    )?;
    f.call(
        "manage_project",
        json!({"operation":"restore","projectId":copy}),
    )?;
    f.call("finish_session", json!({"commit":true}))?;
    f.call("open_project", json!({"projectId":copy}))?;
    assert_eq!(
        layer(
            &f.host
                .lock()
                .unwrap()
                .read_document(&copy)
                .map_err(|e| e.to_string())?,
            "L-kept"
        )["id"],
        "L-kept"
    );
    Ok(())
}
fn portable() -> Check {
    let mut f = Fixture::new()?;
    f.add("L-saved")?;
    f.call("finish_session", json!({"commit":true}))?;
    let saved = f
        .host
        .lock()
        .unwrap()
        .save_project(&f.id)
        .map_err(|e| e.to_string())?;
    let opened = pm_kernel::project_io::open_project(
        Path::new(saved["path"].as_str().unwrap()),
        Default::default(),
    )
    .map_err(|e| e.to_string())?;
    let mut store = pm_kernel::DocumentStore::open(opened);
    assert!(store.can_undo());
    store.undo().map_err(|e| e.to_string())?;
    assert!(store.layers().is_empty());
    Ok(())
}
fn journal() -> Check {
    let mut f = Fixture::new()?;
    f.add("L-journal")?;
    f.call("finish_session", json!({"commit":true}))?;
    let host = f.host.lock().unwrap();
    host.kernel().sync_all();
    match host.kernel().recover(&f.id, None, None) {
        pm_kernel::journal::Recovery::Recovered(recovered) => {
            assert_eq!(recovered.history.len(), 1);
            assert!(
                pm_model::json::to_canonical_string(&recovered.document.to_value().unwrap())
                    .contains("L-journal")
            );
        }
        other => return Err(format!("journal did not recover: {other:?}")),
    }
    Ok(())
}
fn atomic() -> Check {
    let mut f = Fixture::new()?;
    let before = f.document()?;
    let r = tool_error(
        &mut f.session,
        "apply_commands",
        json!({"commands":[{"type":"add_layer","id":"L-never","layerType":"solid"},{"type":"set_property","target":"missing","path":"opacity","value":42}]}),
    )?;
    assert_eq!(r["code"], "invalid_arguments");
    assert_eq!(f.document()?, before);
    Ok(())
}
fn interleaved() -> Check {
    let mut f = Fixture::new()?;
    f.add("L-run")?;
    {
        let mut host = f.host.lock().unwrap();
        let rev = host.revision(&f.id).unwrap();
        host.apply_commands(
            &f.id,
            vec![Command::SetLayerName {
                layer_id: "L-run".into(),
                name: "Foreign accepted edit".into(),
            }],
            "Foreign",
            Origin::Agent,
            rev,
            999,
        )
        .map_err(|e| e.to_string())?;
    }
    let before = f.document()?;
    assert_eq!(
        tool_error(
            &mut f.session,
            "apply_commands",
            json!({"commands":[{"type":"set_property","target":"L-run","path":"opacity","value":20}]})
        )?["code"],
        "revision_conflict"
    );
    assert_eq!(
        tool_error(&mut f.session, "rollback_changes", json!({}))?["code"],
        "revision_conflict"
    );
    assert_eq!(f.document()?, before);
    f.call("get_project_state", json!({}))?;
    assert_eq!(
        tool_error(&mut f.session, "rollback_changes", json!({}))?["code"],
        "revision_conflict"
    );
    Ok(())
}
fn locked() -> Check {
    let mut f = Fixture::new()?;
    f.edit(json!([{"type":"add_layer","id":"L-lock","layerType":"solid"},{"type":"set_layer","target":"L-lock","patch":{"locked":true}}]))?;
    let before = f.document()?;
    tool_error(
        &mut f.session,
        "apply_commands",
        json!({"commands":[{"type":"set_layer","target":"L-lock","patch":{"locked":false}},{"type":"set_property","target":"L-lock","path":"opacity","value":30}]}),
    )?;
    assert_eq!(f.document()?, before);
    Ok(())
}
fn disconnect() -> Check {
    let mut f = Fixture::new()?;
    f.add("L-run")?;
    let run_root = f.scratch.0.join("profile/mcp-runs");
    assert!(std::fs::read_dir(&run_root).unwrap().next().is_some());
    f.session.close().map_err(|e| e.to_string())?;
    assert_eq!(f.document()?["layers"], json!([]));
    assert!(std::fs::read_dir(run_root).unwrap().next().is_none());
    Ok(())
}
fn unavailable() -> Check {
    let mut f = Fixture::new()?;
    for spec in pm_mcp::catalog::tools() {
        let name = spec["name"].as_str().ok_or("catalog name missing")?;
        if pm_mcp::catalog::IMPLEMENTED.contains(&name) {
            continue;
        }
        let args = pending_arguments(&spec["inputSchema"])?;
        pm_mcp::catalog::validate(name, &args).map_err(|e| format!("{name}: {e}"))?;
        let error = tool_error(&mut f.session, name, args)?;
        assert_eq!(
            error["code"], "not_available_in_this_build",
            "{name}: {error}"
        );
        assert_eq!(error["backing"], pm_mcp::catalog::backing(name));
        assert!(!error["backing"].as_str().unwrap().is_empty());
    }
    Ok(())
}
/// Minimal synthetic schema inputs, validated by the production catalog before
/// dispatch. An unfamiliar schema fails this probe rather than reducing coverage.
pub fn pending_arguments(schema: &Value) -> Result<Value, String> {
    if let Some(values) = schema["enum"].as_array() {
        return values.first().cloned().ok_or("empty enum".into());
    }
    match schema["type"].as_str() {
        Some("object") => {
            let mut object = serde_json::Map::new();
            for key in schema["required"].as_array().into_iter().flatten() {
                let key = key.as_str().ok_or("invalid required key")?;
                object.insert(key.into(), pending_arguments(&schema["properties"][key])?);
            }
            Ok(Value::Object(object))
        }
        Some("array") => {
            let count = schema["minItems"].as_u64().unwrap_or(0);
            (0..count)
                .map(|_| pending_arguments(&schema["items"]))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array)
        }
        Some("string") => {
            let value = match schema["pattern"].as_str() {
                Some("^[A-Za-z0-9_-]+$") => "synthetic".into(),
                Some("^[a-z0-9][a-z0-9-]{1,63}$") => "synthetic-extension".into(),
                Some("^[a-f0-9]{32}$") => "0".repeat(32),
                Some(pattern) => {
                    return Err(format!("add a synthetic input for pattern {pattern}"));
                }
                None => "x".repeat(schema["minLength"].as_u64().unwrap_or(1) as usize),
            };
            Ok(Value::String(value))
        }
        Some("boolean") => Ok(json!(false)),
        Some("number" | "integer") => Ok(schema.get("minimum").cloned().unwrap_or(json!(0))),
        _ => Err(format!("add a synthetic input for schema {schema}")),
    }
}
fn selection_transient() -> Check {
    let mut f = Fixture::new()?;
    f.add("L-one")?;
    let before = f.document()?;
    let history = f.host.lock().unwrap().store(&f.id).unwrap().history().len();
    f.call("select_layers", json!({"layerIds":["L-one"]}))?;
    f.call("select_layers", json!({"layerIds":[]}))?;
    assert_eq!(f.document()?, before);
    assert_eq!(
        f.host.lock().unwrap().store(&f.id).unwrap().history().len(),
        history
    );
    Ok(())
}
fn work() -> Check {
    let mut f = Fixture::new()?;
    f.edit(json!([{"type":"set_composition","patch":{"workArea":[1.0,4.0]}}]))?;
    assert_eq!(f.document()?["work"], json!([1, 4]));
    f.verify_saved()?;
    Ok(())
}
fn markers() -> Check {
    let mut f = Fixture::new()?;
    f.edit(json!([{"type":"add_marker","time":1.0,"name":"Synthetic beat"}]))?;
    let marker = f.document()?["markers"][0].clone();
    assert_eq!(marker["t"], 1);
    assert!(marker["id"].is_string());
    f.verify_saved()?;
    f.edit(json!([{"type":"remove_marker","index":0}]))?;
    assert_eq!(f.document()?["markers"], json!([]));
    f.call("rollback_changes", json!({}))?;
    assert_eq!(f.document()?["markers"], json!([]));
    Ok(())
}
fn strips() -> Check {
    let mut f = Fixture::new()?;
    f.add("L-one")?;
    f.add("L-two")?;
    let a = layer(&f.document()?, "L-one")["from"].as_f64().unwrap();
    f.call(
        "edit_layer_strips",
        json!({"operation":"move","layerIds":["L-one","L-two"],"delta":1.5}),
    )?;
    assert_eq!(
        layer(&f.document()?, "L-one")["from"].as_f64(),
        Some(a + 1.5)
    );
    let before = f.document()?;
    tool_error(
        &mut f.session,
        "edit_layer_strips",
        json!({"operation":"move","layerIds":["L-one","missing"],"delta":2}),
    )?;
    assert_eq!(f.document()?, before);
    f.verify_saved()?;
    Ok(())
}
fn keys() -> Check {
    let mut f = Fixture::new()?;
    f.add("L-one")?;
    f.edit(json!([{"type":"replace_keyframes","target":"L-one","path":"opacity","keyframes":[{"id":"K-one","time":0.25,"value":25},{"id":"K-two","time":1.75,"value":75}]}]))?;
    let doc = f.document()?;
    let k = &layer(&doc, "L-one")["p"]["opacity"]["kf"];
    assert_eq!(k[0]["i"], "K-one");
    assert_eq!(k[0]["t"], 0.25);
    assert_eq!(k[1]["v"], 75);
    f.verify_saved()?;
    Ok(())
}
fn groups() -> Check {
    let mut f = Fixture::new()?;
    f.add("L-one")?;
    f.add("L-two")?;
    f.edit(json!([{"type":"group_layers","targets":["L-one","L-two"],"name":"Synthetic group"}]))?;
    let doc = f.document()?;
    let group = layer(&doc, "L-one")["group"].as_str().unwrap().to_owned();
    assert_eq!(layer(&doc, "L-two")["group"], group);
    f.edit(json!([{"type":"ungroup_layers","targets":[group]}]))?;
    let doc = f.document()?;
    assert!(layer(&doc, "L-one")["group"].is_null());
    assert!(layer(&doc, "L-two")["group"].is_null());
    f.verify_saved()?;
    Ok(())
}
fn properties() -> Check {
    let mut f = Fixture::new()?;
    f.add("L-one")?;
    let authored = 12.3456789012345;
    f.edit(json!([{"type":"set_property","target":"L-one","path":"position.x","value":authored,"mode":"static"}]))?;
    f.call("finish_session", json!({"commit":true}))?;
    let saved = f
        .host
        .lock()
        .unwrap()
        .save_project(&f.id)
        .map_err(|e| e.to_string())?;
    let opened = pm_kernel::project_io::open_project(
        Path::new(saved["path"].as_str().unwrap()),
        Default::default(),
    )
    .map_err(|e| e.to_string())?;
    let store = pm_kernel::DocumentStore::open(opened);
    let realized =
        pm_model::canonical::realize_project(store.snapshot().document.to_value().unwrap());
    let doc: Value = serde_json::from_str(&pm_model::json::to_canonical_string(&realized)).unwrap();
    assert_eq!(
        layer(&doc, "L-one")["p"]["position.x"]["v"].as_f64(),
        Some(authored)
    );
    Ok(())
}

fn actor_fixture() -> Result<(KernelActor, ActorAutomation), String> {
    let actor = KernelActor::spawn(KernelConfig::default());
    let kernel = actor.handle();
    kernel
        .open(
            "P-synthetic",
            OpenSource::Untitled(pm_model::json::parse(&clip_project().to_string()).unwrap()),
        )
        .wait()
        .ok_or("actor stopped")?
        .map_err(|e| format!("{e:?}"))?;
    Ok((actor, ActorAutomation::new(kernel)))
}
fn actor_call(host: &ActorAutomation, call: impl Into<Call>) -> Result<pm_kernel::Ack, String> {
    let ack = host
        .kernel
        .call(
            "P-synthetic",
            Request::new(Caller::realm(44, 1, Origin::Mcp), call),
        )
        .wait()
        .ok_or("actor stopped")?
        .map_err(|e| format!("{e:?}"))?;
    if matches!(ack.reply, Reply::Throw(_)) {
        return Err(format!("{:?}", ack.reply));
    }
    Ok(ack)
}
fn actor_edit(host: &ActorAutomation, command: Command) -> Check {
    actor_call(
        host,
        Call::Transact {
            label: Some("Synthetic acceptance".into()),
            body: WriteBody::Commands(vec![command]),
            expected_revision: Some(
                host.inspect_project("P-synthetic")
                    .map_err(|e| e.to_string())?
                    .revision(),
            ),
        },
    )?;
    Ok(())
}
fn slip() -> Check {
    let (_actor, host) = actor_fixture()?;
    let before = host
        .read_document("P-synthetic")
        .map_err(|e| e.to_string())?;
    actor_edit(
        &host,
        TimingCommand::Slip {
            layer_id: "L-clip".into(),
            source_in: 5.0,
        }
        .into(),
    )?;
    let slipped = host
        .read_document("P-synthetic")
        .map_err(|e| e.to_string())?;
    assert_eq!(layer(&slipped, "L-clip")["d"]["trim"]["v"], 5);
    assert_eq!(layer(&slipped, "L-clip")["from"], 2);
    assert_eq!(layer(&slipped, "L-clip")["dur"], 3);
    assert_eq!(
        layer(&slipped, "L-clip")["p"],
        layer(&before, "L-clip")["p"]
    );
    assert!(
        actor_edit(
            &host,
            TimingCommand::Slip {
                layer_id: "L-clip".into(),
                source_in: 8.0
            }
            .into()
        )
        .is_err()
    );
    assert_eq!(
        host.read_document("P-synthetic")
            .map_err(|e| e.to_string())?,
        slipped
    );
    actor_call(&host, HistoryCall::Undo)?;
    assert_eq!(
        without_revision(
            host.read_document("P-synthetic")
                .map_err(|e| e.to_string())?
        ),
        without_revision(before)
    );
    actor_call(&host, HistoryCall::Redo)?;
    assert_eq!(
        without_revision(
            host.read_document("P-synthetic")
                .map_err(|e| e.to_string())?
        ),
        without_revision(slipped)
    );
    Ok(())
}
fn split() -> Check {
    let (_actor, host) = actor_fixture()?;
    actor_edit(
        &host,
        TimingCommand::Split {
            layer_ids: vec!["L-clip".into()],
            time: 3.0,
            new_ids: vec!["L-tail".into()],
        }
        .into(),
    )?;
    let doc = host
        .read_document("P-synthetic")
        .map_err(|e| e.to_string())?;
    assert_eq!(layer(&doc, "L-clip")["dur"], 1);
    assert_eq!(layer(&doc, "L-tail")["from"], 3);
    assert_eq!(layer(&doc, "L-tail")["dur"], 2);
    let trim = &layer(&doc, "L-tail")["d"]["trim"];
    assert_eq!(trim.as_f64().or_else(|| trim["v"].as_f64()), Some(3.0));
    assert_eq!(layer(&doc, "L-tail")["p"]["opacity"]["kf"][1]["t"], 1);
    Ok(())
}
fn actor_history() -> Check {
    let (_actor, host) = actor_fixture()?;
    let before = host
        .read_document("P-synthetic")
        .map_err(|e| e.to_string())?;
    actor_edit(
        &host,
        Command::SetLayerName {
            layer_id: "L-clip".into(),
            name: "Edited".into(),
        },
    )?;
    actor_call(&host, HistoryCall::Undo)?;
    assert_eq!(
        without_revision(
            host.read_document("P-synthetic")
                .map_err(|e| e.to_string())?
        ),
        without_revision(before.clone())
    );
    actor_call(&host, HistoryCall::Redo)?;
    assert_eq!(
        layer(
            &host
                .read_document("P-synthetic")
                .map_err(|e| e.to_string())?,
            "L-clip"
        )["name"],
        "Edited"
    );
    actor_call(
        &host,
        pm_kernel::EditCall::Begin {
            label: "Cancelled source edit".into(),
        },
    )?;
    actor_edit(
        &host,
        Command::SetLayerName {
            layer_id: "L-clip".into(),
            name: "Cancelled".into(),
        },
    )?;
    actor_call(&host, pm_kernel::EditCall::Cancel)?;
    assert_eq!(
        layer(
            &host
                .read_document("P-synthetic")
                .map_err(|e| e.to_string())?,
            "L-clip"
        )["name"],
        "Edited"
    );
    Ok(())
}
struct RecordingEngine(Mutex<Vec<CaptureRequest>>);
impl EngineHook for RecordingEngine {
    fn capture(&self, request: &CaptureRequest) -> pm_mcp::Result<Value> {
        self.0.lock().unwrap().push(request.clone());
        Ok(json!({"testDouble":true,"revision":request.revision,"times":request.times}))
    }
}
fn engine_hook() -> Check {
    let mut f = Fixture::new()?;
    let engine = Arc::new(RecordingEngine(Mutex::new(vec![])));
    f.host.lock().unwrap().engine = Some(engine.clone());
    let rev = f.revision();
    let mut request = CaptureRequest {
        project_id: f.id.clone(),
        revision: rev,
        times: vec![0.0, 1.0 / 30.0],
        width: 320,
    };
    let before = f.document()?;
    let capture = f
        .host
        .lock()
        .unwrap()
        .capture_frames(&request)
        .map_err(|e| e.to_string())?;
    assert_eq!(capture["testDouble"], true);
    assert_eq!(engine.0.lock().unwrap()[0], request);
    assert_eq!(f.document()?, before);
    f.add("L-new")?;
    request.revision = rev;
    assert_eq!(
        f.host
            .lock()
            .unwrap()
            .capture_frames(&request)
            .unwrap_err()
            .code,
        "revision_conflict"
    );
    assert_eq!(engine.0.lock().unwrap().len(), 1);
    Ok(())
}
struct RecordingSurface(Mutex<Vec<Value>>);
impl SurfaceHook for RecordingSurface {
    fn list_panels(&self) -> pm_mcp::Result<Value> {
        Ok(json!([{"id":"synthetic.timeline"}]))
    }
    fn inspect(&self, panel: &str, element: Option<&str>) -> pm_mcp::Result<Value> {
        if panel != "synthetic.timeline" {
            return Err(pm_mcp::Error::invalid("unknown panel"));
        }
        Ok(json!({"panelId":panel,"elementId":element,"testDouble":true}))
    }
    fn drive(&self, panel: &str, element: &str, action: &Value) -> pm_mcp::Result<Value> {
        self.inspect(panel, Some(element))?;
        if element != "play" {
            return Err(pm_mcp::Error::invalid("unknown element"));
        }
        self.0.lock().unwrap().push(action.clone());
        Ok(json!({"testDouble":true}))
    }
    fn capture(&self, _: &str) -> pm_mcp::Result<Value> {
        Err(pm_mcp::Error::unavailable("needs macOS: real panel pixels"))
    }
}
fn surface_hook() -> Check {
    let f = Fixture::new()?;
    let surface = Arc::new(RecordingSurface(Mutex::new(vec![])));
    let mut host = f.host.lock().unwrap();
    host.surfaces = Some(surface.clone());
    assert_eq!(host.list_panels().unwrap()[0]["id"], "synthetic.timeline");
    assert_eq!(
        host.inspect_element("synthetic.timeline", Some("play"))
            .unwrap()["elementId"],
        "play"
    );
    host.drive_element("synthetic.timeline", "play", &json!({"type":"press"}))
        .map_err(|e| e.to_string())?;
    assert_eq!(surface.0.lock().unwrap().len(), 1);
    assert!(
        host.drive_element("synthetic.timeline", "missing", &json!({}))
            .is_err()
    );
    Ok(())
}

/// Public synthetic corpus copies; parsing and saving happen on private copies.
pub fn corpus_copy(source: &Path, ordinal: usize) -> Check {
    let scratch = Scratch::new().map_err(|_| "cannot create corpus scratch")?;
    let copy = scratch
        .copy(source, ordinal)
        .map_err(|_| "cannot copy corpus input")?;
    let opened = pm_kernel::project_io::open_project(&copy, Default::default())
        .map_err(|_| "kernel rejected corpus copy")?;
    let mut store = pm_kernel::DocumentStore::open(opened);
    let before = store
        .snapshot()
        .document
        .to_value()
        .ok_or("missing corpus document")?;
    let envelope = store.file().envelope.clone();
    let history = store.history().snapshot().to_saved();
    let mut media = vec![];
    for (i, entry) in store.media().iter().enumerate() {
        let bytes_path = scratch.0.join(format!("source-media-{i}"));
        entry
            .copy_to(&bytes_path)
            .map_err(|_| "cannot read source corpus media")?;
        media.push((
            entry.id.clone(),
            entry.revision.clone(),
            entry.media_type.clone(),
            bytes_path,
        ));
    }
    let target = scratch.0.join("roundtrip.pmv");
    store
        .prepare_save(
            pm_kernel::project_io::SaveTarget::SaveAs {
                path: target.clone(),
                replace: false,
            },
            Default::default(),
        )
        .map_err(|_| "cannot plan corpus save")?
        .run()
        .map_err(|_| "cannot save corpus copy")?;
    let reopened = pm_kernel::project_io::open_project(&target, Default::default())
        .map_err(|_| "cannot reopen saved corpus copy")?;
    let reopened = pm_kernel::DocumentStore::open(reopened);
    // Report faults without copying authored contents, paths or bytes to logs.
    let mut saved_extra = reopened.file().envelope.extra.clone();
    // The fileguard writer adds this known requirement when native-only state
    // (including archived history) needs it. It is not an unknown input field.
    // Preserve exact equality for every original field and any other addition.
    if !envelope.extra.contains_key("requiresEngine")
        && saved_extra
            .get("requiresEngine")
            .and_then(pm_model::Value::as_str)
            == Some("native")
    {
        saved_extra.remove("requiresEngine");
    }
    if saved_extra != envelope.extra {
        return Err("corpus unknown envelope fields changed".into());
    }
    if reopened
        .file()
        .envelope
        .workspace
        .as_ref()
        .unwrap_or(&pm_model::Value::Null)
        != envelope
            .workspace
            .as_ref()
            .unwrap_or(&pm_model::Value::Null)
    {
        return Err("corpus workspace changed".into());
    }
    if reopened.history().snapshot().to_saved() != history {
        return Err("corpus history/cursor changed".into());
    }
    if envelope.history.is_some()
        && store.history().is_empty()
        && reopened.file().envelope.history != envelope.history
    {
        return Err("corpus opaque history changed".into());
    }
    for (i, (id, revision, media_type, bytes_path)) in media.iter().enumerate() {
        // Even unreachable source records must not silently disappear from an
        // explicitly selected acceptance corpus; report a writer loss as fail.
        let entry = reopened
            .media()
            .get(&id.to_string_lossy())
            .ok_or("corpus media was omitted by writer")?;
        if &entry.revision != revision {
            return Err("corpus media revision changed".into());
        }
        if &entry.media_type != media_type {
            return Err("corpus media type changed".into());
        }
        let saved_path = scratch.0.join(format!("saved-media-{i}"));
        entry
            .copy_to(&saved_path)
            .map_err(|_| "cannot read saved corpus media")?;
        if !same_bytes(bytes_path, &saved_path).map_err(|_| "cannot compare saved corpus media")? {
            return Err("corpus media bytes changed".into());
        }
    }
    if reopened.snapshot().document.to_value() != Some(before) {
        return Err("corpus document changed on save/reopen".into());
    }
    Ok(())
}
fn same_bytes(left: &Path, right: &Path) -> std::io::Result<bool> {
    use std::io::Read;
    let mut a = std::fs::File::open(left)?;
    let mut b = std::fs::File::open(right)?;
    if a.metadata()?.len() != b.metadata()?.len() {
        return Ok(false);
    }
    let mut a_bytes = [0u8; 65536];
    let mut b_bytes = [0u8; 65536];
    loop {
        let n = a.read(&mut a_bytes)?;
        if n == 0 {
            return Ok(true);
        }
        b.read_exact(&mut b_bytes[..n])?;
        if a_bytes[..n] != b_bytes[..n] {
            return Ok(false);
        }
    }
}

fn composition_settings() -> Check {
    let mut f = Fixture::new()?;
    f.edit(json!([{"type":"set_composition","patch":{"width":2048,"height":858,"fps":24,"duration":24,"background":"#123456"}}]))?;
    let doc = f.document()?;
    assert_eq!(doc["w"], 2048);
    assert_eq!(doc["h"], 858);
    assert_eq!(doc["fps"].as_f64(), Some(24.0));
    assert_eq!(doc["dur"], 24);
    assert_eq!(doc["bg"], "#123456");
    f.verify_saved()?;
    Ok(())
}
fn mask() -> Check {
    use pm_kernel::{
        MaskCommand,
        commands::masks::{MaskPatch, MaskShape},
    };
    let (_actor, host) = actor_fixture()?;
    let before = host
        .read_document("P-synthetic")
        .map_err(|e| e.to_string())?;
    actor_edit(
        &host,
        MaskCommand::Add {
            layer_id: "L-clip".into(),
            mask_id: "M-mask".into(),
            shape: MaskShape::Ellipse,
        }
        .into(),
    )?;
    actor_edit(
        &host,
        MaskCommand::Set {
            layer_id: "L-clip".into(),
            mask_id: "M-mask".into(),
            patch: MaskPatch {
                mode: Some("subtract".into()),
                ..Default::default()
            },
        }
        .into(),
    )?;
    let doc = host
        .read_document("P-synthetic")
        .map_err(|e| e.to_string())?;
    let mask = &layer(&doc, "L-clip")["masks"][0];
    assert_eq!(mask["id"], "M-mask");
    assert_eq!(mask["shape"], "ellipse");
    assert_eq!(mask["mode"], "subtract");
    assert_eq!(mask["p"]["w"]["v"], 90);
    actor_call(&host, HistoryCall::Undo)?;
    actor_call(&host, HistoryCall::Undo)?;
    assert_eq!(
        without_revision(
            host.read_document("P-synthetic")
                .map_err(|e| e.to_string())?
        ),
        without_revision(before)
    );
    actor_call(&host, HistoryCall::Redo)?;
    actor_call(&host, HistoryCall::Redo)?;
    assert_eq!(
        without_revision(
            host.read_document("P-synthetic")
                .map_err(|e| e.to_string())?
        ),
        without_revision(doc)
    );
    Ok(())
}
fn text() -> Check {
    let mut f = Fixture::new()?;
    f.edit(json!([{"type":"add_layer","id":"L-text","layerType":"text","content":{"text":"Before","size":64}},{"type":"set_content","target":"L-text","patch":{"text":"Exact café e\u{301} 😀\n第二行"}}]))?;
    f.call("finish_session", json!({"commit":true}))?;
    f.call("start_session", json!({"prompt":"Synthetic text"}))?;
    let doc = f.document()?;
    let scale = layer(&doc, "L-text")["p"].clone();
    let rev = f.revision();
    f.host
        .lock()
        .unwrap()
        .apply_commands(
            &f.id,
            vec![
                pm_kernel::TextCommand::SetBox {
                    layer_id: "L-text".into(),
                    width: Some(300.5),
                    height: Some(100.25),
                    time: 0.0,
                }
                .into(),
            ],
            "Synthetic paragraph",
            Origin::Mcp,
            rev,
            808,
        )
        .map_err(|e| e.to_string())?;
    let doc = f.document()?;
    let layer = layer(&doc, "L-text");
    assert_eq!(layer["d"]["text"], "Exact café e\u{301} 😀\n第二行");
    assert_eq!(layer["d"]["size"], 64);
    assert_eq!(layer["d"]["boxWidth"]["v"], 300.5);
    assert_eq!(layer["p"], scale);
    f.verify_saved()?;
    Ok(())
}
fn easing() -> Check {
    let mut f = Fixture::new()?;
    f.add("L-one")?;
    f.edit(json!([{"type":"replace_keyframes","target":"L-one","path":"opacity","keyframes":[{"id":"K-one","time":0,"value":25},{"id":"K-two","time":2,"value":75}]},{"type":"set_easing","keyframes":["K-one"],"curve":[0.125,0.25,0.75,0.875]}]))?;
    let doc = f.document()?;
    let keys = &layer(&doc, "L-one")["p"]["opacity"]["kf"];
    assert_eq!(keys[0]["v"], 25);
    assert_eq!(keys[1]["v"], 75);
    assert_eq!(keys[0]["outInterp"], "bezier");
    assert_eq!(keys[0]["inInterp"], "bezier");
    assert_eq!(keys[0]["outEase"]["influence"], 12.5);
    assert_eq!(keys[0]["outEase"]["speed"], 50.0);
    assert_eq!(keys[0]["inEase"]["influence"], 25.0);
    assert_eq!(keys[0]["inEase"]["speed"], 12.5);
    f.verify_saved()?;
    Ok(())
}

fn fractional_rate() -> Check {
    let mut f = Fixture::new()?;
    f.edit(json!([{"type":"set_composition","patch":{"fps":29.97}}]))?;
    let actual = f.document()?["fps"]
        .as_f64()
        .ok_or("missing composition rate")?;
    if actual != 29.97 {
        return Err(format!(
            "authored fps=29.97 became {actual}; pm-kernel CompCommand::Set rounds fps (commands/comps.rs). N3 requires exact authored timing; fix belongs to the kernel lane"
        ));
    }
    f.verify_saved()?;
    Ok(())
}
