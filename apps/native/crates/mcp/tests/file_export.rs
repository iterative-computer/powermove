use pm_mcp::Automation;
use pm_model::json::{Value, parse};
use std::path::PathBuf;

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(pm_kernel::new_id("mcp-export-test"));
        std::fs::create_dir_all(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn utf16_ingress_and_named_saves_preserve_the_model_and_original_copy() {
    let scratch = Scratch::new();
    let source = scratch.0.join("working.pmv");
    let original = scratch.0.join("original.pmv");
    let bytes =
        include_bytes!("../../../../../engine/crates/pm-model/tests/fixtures/containers/pmv4.pmv");
    std::fs::write(&source, bytes).unwrap();
    std::fs::write(&original, bytes).unwrap();
    let exports = scratch.0.join("exports");
    let mut host = Automation::new(scratch.0.join("profile"), exports.clone()).unwrap();
    let opened = host.open_file("utf16", source.clone()).unwrap();
    let id = opened["projectId"].as_str().unwrap();
    assert!(
        pm_model::json::to_canonical_string(&host.read_document_model(id).unwrap())
            .contains("\\ud800")
    );
    let revision = host.revision(id).unwrap();
    host.apply_commands(
        id,
        vec![
            pm_kernel::CompCommand::Set {
                patch: Box::new(pm_kernel::commands::comps::CompPatch {
                    name: Some("Synthetic edited UTF-16".into()),
                    ..Default::default()
                }),
            }
            .into(),
        ],
        "Synthetic edit",
        pm_kernel::Origin::Mcp,
        revision,
        1,
    )
    .unwrap();
    let before = host.read_document_model(id).unwrap();
    assert!(pm_model::json::to_canonical_string(&before).contains("\\ud800"));
    let revision = host.revision(id).unwrap();
    let path = host.store(id).unwrap().path().unwrap().to_owned();
    assert!(host.store(id).unwrap().is_dirty());
    for _ in 0..2 {
        let saved = host.save_project(id).unwrap();
        let destination = PathBuf::from(saved["path"].as_str().unwrap());
        assert_eq!(destination, source);
        let reopened =
            pm_kernel::project_io::open_project(&destination, Default::default()).unwrap();
        let reopened = pm_kernel::DocumentStore::open(reopened);
        let mut actual =
            pm_model::canonical::realize_project(reopened.snapshot().document.to_value().unwrap());
        actual
            .as_object_mut()
            .unwrap()
            .insert("revision", Value::Number(revision as f64));
        assert_eq!(actual, before);
        assert_eq!(host.store(id).unwrap().path(), Some(path.as_path()));
        assert!(!host.store(id).unwrap().is_dirty());
    }
    assert!(!exports.exists());
    assert_eq!(host.revision(id).unwrap(), revision);
    assert_eq!(std::fs::read(original).unwrap(), bytes);
}

#[test]
fn unrelated_authored_utf16_does_not_prevent_ingress() {
    let scratch = Scratch::new();
    let path = scratch.0.join("utf16.json");
    let source = r#"{"id":"synthetic","name":"Synthetic","w":32,"h":18,"fps":30,"dur":2,"layers":[],"comps":{},"opaque":{"\ud800":"\udfff"}}"#;
    std::fs::write(&path, source).unwrap();
    let mut host = Automation::new(scratch.0.join("profile"), scratch.0.join("exports")).unwrap();
    let result = host.open_file("synthetic", path).unwrap();
    assert_eq!(result["name"], "Synthetic");
    let actual = host
        .read_document_model(result["projectId"].as_str().unwrap())
        .unwrap();
    assert_eq!(actual.get("opaque"), parse(source).unwrap().get("opaque"));
}

#[test]
fn named_save_keeps_offline_source_metadata_external() {
    let scratch = Scratch::new();
    let path = scratch.0.join("working.json");
    let bytes = include_bytes!("../../kernel/src/project_io/fixtures/main-document.json");
    std::fs::write(&path, bytes).unwrap();
    let exports = scratch.0.join("exports");
    let mut host = Automation::new(scratch.0.join("profile"), exports.clone()).unwrap();
    let opened = host.open_file("offline", path.clone()).unwrap();
    let id = opened["projectId"].as_str().unwrap();
    let before = host.read_document_model(id).unwrap();
    let owned = host.store(id).unwrap().media().clone();
    // The reader materializes the history-reachable record. The orphan and
    // unknown records stay intact in nativeLegacyMediaArchive, checked below.
    assert_eq!(owned.len(), 1);
    assert!(owned.get("undo-only").is_some());
    assert!(owned.get("linked").is_none());
    let saved = host.save_project(id).unwrap();
    assert_eq!(saved["path"], serde_json::json!(path));
    let reopened = pm_kernel::project_io::open_project(&path, Default::default()).unwrap();
    assert_eq!(reopened.file.media.len(), owned.len());
    assert!(reopened.file.media.get("linked").is_none());
    for (i, entry) in owned.iter().enumerate() {
        let saved = reopened.file.media.get(entry.id.as_str().unwrap()).unwrap();
        assert_eq!(saved.media_type, entry.media_type);
        assert_eq!(saved.revision, entry.revision);
        let a = scratch.0.join(format!("before-{i}"));
        let b = scratch.0.join(format!("after-{i}"));
        entry.copy_to(&a).unwrap();
        saved.copy_to(&b).unwrap();
        assert_eq!(std::fs::read(a).unwrap(), std::fs::read(b).unwrap());
    }
    let container = pm_model::container::read_container(
        &std::fs::File::open(&path).unwrap(),
        pm_model::container::ReadOptions { verify_media: true },
    )
    .unwrap();
    let fixture = parse(std::str::from_utf8(bytes).unwrap()).unwrap();
    assert_eq!(
        container.document.get("nativeLegacyMediaArchive"),
        fixture.get("media")
    );
    let store = pm_kernel::DocumentStore::open(reopened);
    let actual =
        pm_model::canonical::realize_project(store.snapshot().document.to_value().unwrap());
    assert_eq!(actual.get("assets"), before.get("assets"));
    assert!(!exports.exists());
}
