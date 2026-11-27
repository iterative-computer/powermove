#![cfg(feature = "native-scene3d-cad-tests")]
mod common;
use pm_kernel::{DocumentStore, Origin, Scene3dCommand};
use pm_scene3d::import::{self, ImportFile, archive::Archive, cad::OpenCascade};
use serde_json::json;
fn roundtrip(extension: &str) {
    let mut dir = common::TempDir::new("native-cad-import");
    dir.0 = std::fs::canonicalize(&dir.0).unwrap();
    let generator = std::env::var_os("PM_SCENE3D_CAD_FIXTURES")
        .expect("Set absolute native CAD fixture generator path");
    assert!(
        std::process::Command::new(generator)
            .arg(&dir.0)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    );
    let helper = std::env::var_os("PM_SCENE3D_CAD_HELPER")
        .expect("Set absolute native CAD helper path")
        .into();
    let cad = OpenCascade {
        helper,
        scratch_root: dir.0.clone(),
    };
    let original = std::fs::read(dir.path(&format!("product.{extension}"))).unwrap();
    let prepared = import::prepare(
        vec![ImportFile {
            name: format!("product.{extension}"),
            mime: "application/octet-stream".into(),
            bytes: original.clone(),
        }],
        Some(&cad),
    )
    .unwrap();
    let converted = prepared.bytes.clone();
    let archive = Archive::open(&dir.path("archive")).unwrap();
    let receipt = std::sync::Arc::new(archive.admit(prepared).unwrap());
    let mut s = DocumentStore::new_untitled(pm_scene3d::to_model(
        &json!({"id":"P","name":"CAD","compId":"C","w":640,"h":360,"fps":30,"dur":10,"layers":[],"assets":{}}),
    ));
    s.execute(
        Scene3dCommand::Import {
            comp: None,
            asset_id: "cad".into(),
            layer_id: "product".into(),
            receipt,
            time: 0.,
            setup_studio: true,
        }
        .into(),
        Origin::Interface,
    )
    .unwrap();
    let path = dir.path("project.pmv");
    common::save_as(&mut s, &path).unwrap();
    let mut s = DocumentStore::open(
        pm_kernel::project_io::open_project(&path, Default::default()).unwrap(),
    );
    assert_eq!(s.media().len(), 2);
    for (id, bytes) in [("cad", converted), ("cad-source-0", original)] {
        let output = dir.path(&format!("{id}.bin"));
        s.media().get(id).unwrap().copy_to(&output).unwrap();
        assert_eq!(std::fs::read(output).unwrap(), bytes);
    }
    s.undo().unwrap();
    assert!(s.snapshot().layers(None).is_empty());
    s.redo().unwrap();
    assert_eq!(s.snapshot().layers(None).len(), 5);
}
#[test]
fn step_import_undo_save_reopen_keeps_original_and_converted_bytes() {
    roundtrip("step");
}
#[test]
fn iges_import_undo_save_reopen_keeps_original_and_converted_bytes() {
    roundtrip("iges");
}
#[test]
fn brep_import_undo_save_reopen_keeps_original_and_converted_bytes() {
    roundtrip("brep");
}
