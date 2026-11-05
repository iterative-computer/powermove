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
fn utf16_ingress_and_portable_exports_preserve_the_model_and_working_file() {
    let scratch = Scratch::new();
    let source = scratch.0.join("working.pmv");
    let bytes =
        include_bytes!("../../../../../engine/crates/pm-model/tests/fixtures/containers/pmv4.pmv");
    std::fs::write(&source, bytes).unwrap();
    let exports = scratch.0.join("exports");
    let mut host = Automation::new(scratch.0.join("profile"), exports.clone()).unwrap();
    let opened = host.open_file("utf16", source.clone()).unwrap();
    let id = opened["projectId"].as_str().unwrap();
    let before = host.read_document_model(id).unwrap();
    assert!(pm_model::json::to_canonical_string(&before).contains("\\ud800"));
    let revision = host.revision(id).unwrap();
    let path = host.store(id).unwrap().path().unwrap().to_owned();
    let dirty = host.store(id).unwrap().is_dirty();
    for _ in 0..2 {
        let saved = host.save_project(id).unwrap();
        let destination = PathBuf::from(saved["path"].as_str().unwrap());
        assert_eq!(destination.parent(), Some(exports.as_path()));
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
        assert_eq!(host.store(id).unwrap().is_dirty(), dirty);
    }
    assert_eq!(std::fs::read_dir(exports).unwrap().count(), 2);
    assert_eq!(host.revision(id).unwrap(), revision);
    assert_eq!(std::fs::read(source).unwrap(), bytes);
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
