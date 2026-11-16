//! PM-409: the app has one segmented control, the UI kit's Liquid Glass one
//! (`SegmentedControl` in js/v4, the system's glass capsule natively). A
//! panel that draws its own, a second kit variant, or a native segmented
//! control that is not the glass one fails here. Settings' pages keep the
//! plain AppKit control (`v4_view`), and the toolbar's tool picker is the
//! toolbar's own system control.
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn files(dir: &Path, ext: &[&str], out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if path.is_dir() {
            if !matches!(name.as_str(), "node_modules" | "__snapshots__" | "dist") {
                files(&path, ext, out);
            }
        } else if ext.iter().any(|e| name.ends_with(e)) && !name.contains(".test.") {
            out.push(path);
        }
    }
}

#[test]
fn panels_use_the_kit_segmented_control_and_draw_none_of_their_own() {
    let mut panels = vec![];
    files(&root().join("js/v4/panels"), &[".ts", ".tsx"], &mut panels);
    assert!(panels.len() > 20, "found {} panel files", panels.len());
    for file in &panels {
        let text = std::fs::read_to_string(file).unwrap();
        for banned in [
            // A kit control the panel builds itself.
            "kit(\"segment",
            "\"segmented-control\"",
            "control: \"segmented\"",
            "control=\"segmented\"",
            // Two-option pills drawn as radio buttons in a row.
            "role=\"radio\"",
            "role: \"radio\"",
        ] {
            assert!(
                !text.contains(banned),
                "{} draws its own segmented control ({banned}): use SegmentedControl",
                file.display()
            );
        }
        for line in text.lines() {
            let line = line.trim_start();
            assert!(
                !(line.starts_with("function Segment") || line.starts_with("export function Segment")
                    || line.starts_with("const Segment")),
                "{}: a local segmented control ({line})",
                file.display()
            );
        }
    }
}

#[test]
fn the_kit_has_one_segmented_control() {
    let ui = std::fs::read_to_string(root().join("js/v4/src/ui.ts")).unwrap();
    let start = ui.find("export function Segmented(").expect("ui.Segmented");
    let body = &ui[start..ui[start..].find("\n}\n").map_or(ui.len(), |e| start + e)];
    assert!(
        body.contains("SegmentedControl("),
        "ui.Segmented must be the kit's SegmentedControl"
    );
    assert!(!ui.contains("control: \"segmented\""), "a second segmented control in ui.ts");
}

#[test]
fn every_native_segmented_control_is_the_glass_one() {
    let mut sources = vec![];
    files(&root().join("crates/app/src"), &[".rs"], &mut sources);
    files(&root().join("crates/shell/src"), &[".rs"], &mut sources);
    for file in &sources {
        let text = std::fs::read_to_string(file).unwrap();
        assert!(
            !text.contains("pub fn segmented("),
            "{}: a drawn segmented control in the kit",
            file.display()
        );
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if !line.contains("Kind::Segmented,") || !line.contains("kind:") {
                continue;
            }
            let spec = lines[i..(i + 20).min(lines.len())].join("\n");
            assert!(
                spec.contains("glass: true"),
                "{}:{}: a segmented control that is not the glass one",
                file.display(),
                i + 1
            );
        }
    }
    let view = std::fs::read_to_string(root().join("crates/app/src/v4_view.rs")).unwrap();
    assert!(
        view.contains("glass: matches!(kind, crate::v4_control::Kind::Segmented)"),
        "panels' segmented controls must be glass whatever their variant"
    );
}
