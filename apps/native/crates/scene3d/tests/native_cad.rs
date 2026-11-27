#![cfg(feature = "native-cad-tests")]
use pm_scene3d::import::{
    self,
    cad::{Format, OpenCascade, Settings, Tessellator},
};
use std::path::PathBuf;
struct Temp(PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn fixture() -> (Temp, OpenCascade) {
    let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
        "pm-cad-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let helper = std::env::var_os("PM_SCENE3D_CAD_HELPER")
        .expect("Set the absolute built native CAD helper path")
        .into();
    let generator = std::env::var_os("PM_SCENE3D_CAD_FIXTURES")
        .expect("Set the absolute built CAD fixture generator path");
    assert!(
        std::process::Command::new(generator)
            .arg(&root)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    );
    let converter = OpenCascade {
        helper,
        scratch_root: root.clone(),
    };
    (Temp(root), converter)
}
#[test]
fn step_keeps_parts_names_colors_and_y_up() {
    let (root, cad) = fixture();
    let input = std::fs::read(root.0.join("product.step")).unwrap();
    let bytes = cad
        .convert(Format::Step, &input, Settings::default())
        .unwrap();
    let import::Model::Gltf(model) = import::load("product.glb", &bytes).unwrap() else {
        panic!()
    };
    assert!(model.triangles >= 24);
    assert!(model.objects.iter().any(|o| o.name == "Body"));
    assert!(model.objects.iter().any(|o| o.name == "Cap"));
    assert!(
        model.materials.len() >= 3,
        "part and face colors remain distinct"
    );
    assert!(
        (model.fit.radius - 0.019).abs() < 1e-8,
        "millimetres converted to metres: {:?}",
        model.fit
    );
    assert!(
        (model.fit.center[1] - 0.006).abs() < 1e-6,
        "Z-up to Y-up: {:?}",
        model.fit
    );
}
#[test]
fn iges_reads_real_geometry() {
    let (root, cad) = fixture();
    let input = std::fs::read(root.0.join("product.iges")).unwrap();
    let bytes = cad
        .convert(Format::Iges, &input, Settings::default())
        .unwrap();
    let import::Model::Gltf(model) = import::load("product.glb", &bytes).unwrap() else {
        panic!()
    };
    assert!(model.triangles >= 24);
}
#[test]
fn brep_reads_real_geometry() {
    let (root, cad) = fixture();
    let input = std::fs::read(root.0.join("product.brep")).unwrap();
    let bytes = cad
        .convert(Format::Brep, &input, Settings::default())
        .unwrap();
    let import::Model::Gltf(model) = import::load("product.glb", &bytes).unwrap() else {
        panic!()
    };
    assert!(model.triangles >= 24);
}
#[test]
fn malformed_native_cad_fails_and_cleans_only_owned_scratch() {
    let (root, cad) = fixture();
    let before = std::fs::read_dir(&root.0).unwrap().count();
    assert!(
        cad.convert(Format::Step, b"invalid CAD", Settings::default())
            .is_err()
    );
    assert_eq!(std::fs::read_dir(&root.0).unwrap().count(), before);
    assert!(root.0.join("product.step").exists());
}
