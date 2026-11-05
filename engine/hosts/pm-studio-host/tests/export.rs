use std::process::Command;
#[test]
fn cli_requires_explicit_scene_destination_and_frame() {
    let out = Command::new(env!("CARGO_BIN_EXE_pm-studio-host"))
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("usage:"));
}
#[test]
fn headless_final_exports_each_id_without_overwriting_outputs() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let out = root.join("final");
    let render = || {
        Command::new(env!("CARGO_BIN_EXE_pm-studio-host"))
            .arg("hero")
            .arg(&out)
            .args(["32", "24", "11"])
            .output()
            .unwrap()
    };
    let result = render();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let bytes = std::fs::read(out.join("manifest.json")).unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(manifest["samples"], 8);
    assert_eq!(manifest["salt"], 11);
    let layers = manifest["layers"].as_array().unwrap();
    assert!(layers.len() > 2);
    let mut covered = 0;
    for layer in layers {
        let file = std::fs::File::open(out.join(layer["file"].as_str().unwrap())).unwrap();
        let mut reader = png::Decoder::new(std::io::BufReader::new(file))
            .read_info()
            .unwrap();
        let mut image = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut image).unwrap();
        assert_eq!((info.width, info.height), (32, 24));
        covered += image[..info.buffer_size()]
            .chunks_exact(4)
            .filter(|p| p[3] > 0)
            .count();
        assert!(layer["id"].as_str().unwrap().starts_with("object-"));
    }
    assert!(covered > 20, "real product pixels must be exported");
    assert!(
        !render().status.success(),
        "existing output must be preserved"
    );
    assert_eq!(bytes, std::fs::read(out.join("manifest.json")).unwrap());
}
