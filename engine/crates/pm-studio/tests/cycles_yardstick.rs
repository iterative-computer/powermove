//! Optional cheap external-reference regression. Never used by app/build/package.
//! Blender absence skips before optional packages/builds. The `yardstick`
//! feature lets Cargo provide the exporter directly; otherwise the runner
//! builds that test-only feature after detecting Blender.
use std::{
    path::{Path, PathBuf},
    process::Command,
};

#[test]
fn cycles_corpus_stays_within_scene_tolerances_when_blender_is_present() {
    let blender = std::env::var_os("BLENDER").map(PathBuf::from).or_else(|| {
        let default = PathBuf::from("/Applications/Blender.app/Contents/MacOS/Blender");
        if default.is_file() {
            Some(default)
        } else {
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                .map(|p| p.join("blender"))
                .find(|p| p.is_file())
        }
    });
    let Some(blender) = blender.filter(|p| p.is_file()) else {
        eprintln!("SKIP: Blender absent; optional test-only Cycles yardstick");
        return;
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let out = std::env::var_os("PM_STUDIO_YARDSTICK_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!(
                "pm-shots/studio3d/cycles-regression-{}",
                std::process::id()
            ))
        });
    let mut command = Command::new("python3");
    command
        .arg(root.join("engine/tools/cycles-yardstick/run.py"))
        .args(["--regression", "--out"])
        .arg(out)
        .arg("--blender")
        .arg(blender);
    #[cfg(feature = "yardstick")]
    command
        .arg("--studio")
        .arg(env!("CARGO_BIN_EXE_studio-yardstick"));
    let status = command
        .status()
        .expect("Python 3 with NumPy/Pillow required when Blender is installed");
    assert!(
        status.success(),
        "optional Cycles regression failed; inspect the report above"
    );
}
