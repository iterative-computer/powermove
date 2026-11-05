//! Portable tool acceptance through the production MCP session and kernel.
//! No EngineHook, SurfaceHook, fake capability or replacement service is used.
use crate::fixture::{Fixture, tool_error, without_revision};
use pm_mcp::transport::ToolConnection;
use serde_json::{Value, json};
type Check = Result<(), String>;
pub const TOOLS: &[(&str, &[&str])] = &[
    ("list_projects", &["A495"]),
    ("create_project", &["A496", "A498"]),
    ("open_project", &["A497", "A498"]),
    ("manage_project", &["A505"]),
    ("start_session", &["A498", "W030.interleaved", "W030.lock"]),
    ("finish_session", &["A501", "W033.disconnect"]),
    ("rollback_changes", &["A531", "W030.interleaved"]),
    ("get_timeline", &["A503"]),
    ("select_layers", &["A517", "W046.selection"]),
    (
        "apply_commands",
        &["W030.atomic", "W030.interleaved", "A529", "W030.lock"],
    ),
    ("edit_layer_strips", &["A504"]),
    ("edit_video", &["A530", "W057.mcp-refusals"]),
    ("export_captions", &["A550"]),
    ("manage_media", &["A516"]),
    ("get_panel_layout", &["A520"]),
    ("get_3d_scene", &["A508"]),
    ("validate_effect", &["A532"]),
    ("save_project", &["A506"]),
];
fn scenario(id: &str) -> Check {
    let case = crate::scenarios::all()
        .into_iter()
        .find(|s| s.id == id)
        .ok_or_else(|| format!("missing real tool assertion {id}"))?;
    (case.run)()
}
pub fn check(name: &str) -> Check {
    let (_, ids) = TOOLS.iter().find(|t| t.0 == name).ok_or("unprobed tool")?;
    // These are the actual public name/description/input schemas, including the
    // two explicitly authorized Studio overlays; source snapshots stay frozen.
    let expected = crate::main_reference::expected_catalog();
    assert_eq!(
        expected["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == name),
        pm_mcp::catalog::spec(name)
    );
    lifecycle(name)?;
    if ["get_timeline", "get_3d_scene"].contains(&name) {
        refreshed_baseline(name)?;
    }
    for id in *ids {
        scenario(id)?;
    }
    Ok(())
}
fn exact_error(f: &mut Fixture, name: &str, args: Value, text: &str) -> Check {
    let before = f.document()?;
    let revision = f.revision();
    let response = f.session.call(name, args);
    assert_eq!(
        response,
        json!({"content":[{"type":"text","text":text}],"isError":true})
    );
    assert_eq!(f.document()?, before);
    assert_eq!(f.revision(), revision);
    Ok(())
}
fn lifecycle(name: &str) -> Check {
    let mut f = Fixture::new()?;
    f.call("finish_session", json!({"commit":true}))?;
    if ![
        "list_projects",
        "create_project",
        "open_project",
        "start_session",
        "finish_session",
    ]
    .contains(&name)
    {
        // The session gate precedes even malformed tool arguments.
        exact_error(
            &mut f,
            name,
            json!(null),
            "Call start_session before using Powermove tools.",
        )?;
        f.call(
            "start_session",
            json!({"prompt":"Synthetic planning","access":"editor"}),
        )?;
        let expected = crate::main_reference::expected_catalog();
        if !expected["inspection"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n == name)
        {
            exact_error(
                &mut f,
                name,
                json!(null),
                "This planning run can only inspect the project.",
            )?;
        }
        f.call("finish_session", json!({"commit":true}))?;
        f.call(
            "start_session",
            json!({"prompt":"Synthetic app","context":"app"}),
        )?;
        if !expected["app"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n == name)
        {
            exact_error(
                &mut f,
                name,
                json!(null),
                "This agent has no project attached. Open a project to use composition tools.",
            )?;
        }
    }
    f.session.disconnect().map_err(|e| e.to_string())?;
    exact_error(&mut f, name, json!({}), "This MCP connection is closed.")
}
/// Source fixture is a vendored public synthetic PMV4, never a user's project.
/// Exercise file ingress -> edit -> MCP save -> reader, including opaque media.
pub fn packed_save() -> Check {
    let mut f = Fixture::new()?;
    f.call("finish_session", json!({"commit":true}))?;
    let source = f.scratch.0.join("source.pmv");
    let working = f.scratch.0.join("working.pmv");
    let bytes = include_bytes!("../data/fixtures/portable-media.pmv");
    std::fs::write(&source, bytes).map_err(|e| e.to_string())?;
    std::fs::write(&working, bytes).map_err(|e| e.to_string())?;
    let original = pm_kernel::project_io::open_project(&source, Default::default())
        .map_err(|e| e.to_string())?;
    let original = pm_kernel::DocumentStore::open(original);
    assert!(
        !original.media().is_empty(),
        "fixture must contain real embedded bytes"
    );
    let opened = f
        .host
        .lock()
        .unwrap()
        .open_file("packed-input", working.clone())
        .map_err(|e| e.to_string())?;
    f.id = opened["projectId"]
        .as_str()
        .ok_or("missing opened identity")?
        .into();
    f.call("start_session", json!({"prompt":"Synthetic packed save"}))?;
    f.add("packed-save-edit")?;
    f.call("finish_session", json!({"commit":true}))?;
    f.call(
        "start_session",
        json!({"prompt":"Synthetic packed save readback"}),
    )?;
    let before = without_revision(f.document()?);
    let saved = f.call("save_project", json!({}))?;
    assert_eq!(saved["projectId"], f.id);
    let saved_path = std::path::Path::new(saved["path"].as_str().ok_or("saved path missing")?);
    let reopened = pm_kernel::project_io::open_project(saved_path, Default::default())
        .map_err(|e| e.to_string())?;
    let mut reopened = pm_kernel::DocumentStore::open(reopened);
    let doc =
        pm_model::canonical::realize_project(reopened.snapshot().document.to_value().unwrap());
    let doc: Value = serde_json::from_str(&pm_model::json::to_canonical_string(&doc))
        .map_err(|e| e.to_string())?;
    assert_eq!(without_revision(doc), before);
    assert_eq!(reopened.media().len(), original.media().len());
    for (i, entry) in original.media().iter().enumerate() {
        let target = reopened
            .media()
            .get(&entry.id.to_string_lossy())
            .ok_or("saved media missing")?;
        assert_eq!(target.revision, entry.revision);
        assert_eq!(target.media_type, entry.media_type);
        let a = f.scratch.0.join(format!("source-bytes-{i}"));
        let b = f.scratch.0.join(format!("saved-bytes-{i}"));
        entry.copy_to(&a).map_err(|e| e.to_string())?;
        target.copy_to(&b).map_err(|e| e.to_string())?;
        assert_eq!(
            std::fs::read(a).map_err(|e| e.to_string())?,
            std::fs::read(b).map_err(|e| e.to_string())?
        );
    }
    reopened.undo().map_err(|e| e.to_string())?;
    assert!(!reopened.layers().iter().any(|l| l.id == "packed-save-edit"));
    reopened.redo().map_err(|e| e.to_string())?;
    assert!(reopened.layers().iter().any(|l| l.id == "packed-save-edit"));
    assert_eq!(std::fs::read(source).map_err(|e| e.to_string())?, bytes);
    let revision = f.revision();
    // Saving repeatedly is a real write, with no authored edit/history change.
    let repeated = f.call("save_project", json!({}))?;
    assert_eq!(f.revision(), revision);
    assert_eq!(without_revision(f.document()?), before);
    // c7f46a056 legacy/assistant/mcp-project-tools.ts checks filePath first:
    // named files use PM.saveProject; only library projects call PM.Export.run.
    assert_eq!(saved_path, working);
    assert_eq!(repeated["path"], saved["path"]);
    assert_ne!(
        std::fs::read(&working).map_err(|e| e.to_string())?,
        bytes,
        "the named working file must receive the committed edit"
    );
    assert!(!f.scratch.0.join("exports").exists());
    // A library project instead exports independent, uniquely named files.
    let mut untitled = Fixture::new()?;
    untitled.add("untitled-save")?;
    let revision = untitled.revision();
    let dirty = untitled
        .host
        .lock()
        .unwrap()
        .store(&untitled.id)
        .unwrap()
        .is_dirty();
    let first = untitled.call("save_project", json!({}))?;
    let first_path = std::path::Path::new(first["path"].as_str().ok_or("export path missing")?);
    let first_bytes = std::fs::read(first_path).map_err(|e| e.to_string())?;
    let second = untitled.call("save_project", json!({}))?;
    assert_ne!(first["path"], second["path"]);
    for exported in [&first, &second] {
        let path = std::path::Path::new(exported["path"].as_str().ok_or("export path missing")?);
        assert_eq!(
            path.parent(),
            Some(untitled.scratch.0.join("exports").as_path())
        );
        let opened = pm_kernel::project_io::open_project(path, Default::default())
            .map_err(|e| e.to_string())?;
        let store = pm_kernel::DocumentStore::open(opened);
        assert!(
            store
                .layers()
                .iter()
                .any(|layer| layer.id == "untitled-save")
        );
    }
    assert_eq!(
        std::fs::read(first_path).map_err(|e| e.to_string())?,
        first_bytes
    );
    untitled.verify_saved()?;
    assert_eq!(untitled.revision(), revision);
    let host = untitled.host.lock().unwrap();
    let store = host.store(&untitled.id).map_err(|e| e.to_string())?;
    assert!(store.path().is_none());
    assert_eq!(store.is_dirty(), dirty);
    drop(host);
    tool_error(
        &mut untitled.session,
        "open_project",
        json!({"projectId":"missing"}),
    )?;
    Ok(())
}

/// A model-valid lone UTF-16 code unit is authored data, not invalid JSON in
/// Powermove's JavaScript/PMV model. Keep the current MCP ingress panic visible.
pub fn utf16_ingress() -> Check {
    let mut f = Fixture::new()?;
    f.call("finish_session", json!({"commit":true}))?;
    let path = f.scratch.0.join("utf16-input.pmv");
    let bytes = include_bytes!("../data/fixtures/utf16-media.pmv");
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    // The kernel reader independently admits this unchanged public fixture.
    pm_kernel::project_io::open_project(&path, Default::default()).map_err(|e| e.to_string())?;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        f.host
            .lock()
            .unwrap()
            .open_file("utf16-input", path.clone())
    }));
    assert_eq!(std::fs::read(&path).map_err(|e| e.to_string())?, bytes);
    match result {
        Ok(result) => result.map(|_| ()).map_err(|e| e.to_string()),
        Err(_) => Err(
            "MCP ingress rejects kernel-valid authored lone UTF-16; serde_json conversion panics"
                .into(),
        ),
    }
}

fn refreshed_baseline(name: &str) -> Check {
    let mut f = Fixture::new()?;
    let revision = f.revision();
    let mut document = f.document()?;
    document["name"] = json!("Independent writer");
    f.host
        .lock()
        .unwrap()
        .restore(&f.id, document, revision, 911)
        .map_err(|e| e.to_string())?;
    let before = f.document()?;
    let error = tool_error(
        &mut f.session,
        "apply_commands",
        json!({"commands":[{"type":"add_layer","id":"stale","layerType":"solid"}]}),
    )?;
    assert_eq!(error["code"], "revision_conflict");
    assert_eq!(f.document()?, before);
    f.call(name, json!({}))?;
    f.add("fresh-after-read")?;
    assert!(
        f.document()?["layers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["id"] == "fresh-after-read")
    );
    Ok(())
}
