//! PM-409: v4 panels and the toolbar share the kit's AppKit segmented
//! control. AppKit draws the capsule at each context's control size.
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
    let kit_path = root().join("crates/shell/src/native_control.rs");
    for file in &sources {
        let text = std::fs::read_to_string(file).unwrap();
        for banned in [
            "pub fn segmented(",
            "setSelectedSegmentBezelColor",
            "segment_face(",
        ] {
            assert!(
                !text.contains(banned),
                "{}: custom segmented rendering ({banned})",
                file.display()
            );
        }
        // Only the kit can build a system control. Other callers may use
        // NSSegmentedControl as the action sender's type.
        assert!(
            !text.contains("NSSegmentedControl::"),
            "{}: create the control through SegmentedControl::new",
            file.display()
        );
        if *file != kit_path {
            assert!(
                !text.contains("super(NSSegmentedControl)"),
                "{}: a second segmented control subclass",
                file.display()
            );
        }
    }
    let kit = std::fs::read_to_string(kit_path).unwrap();
    assert_eq!(kit.matches("super(NSSegmentedControl)").count(), 1);
    assert!(kit.contains("control.setBorderShape(NSControlBorderShape::Capsule)"));
    assert!(kit.contains("SegmentedControl::new(mtm, size, options.len())"));
    assert!(kit.contains("setLabel_forSegment"));
    let toolbar = std::fs::read_to_string(root().join("crates/shell/src/native.rs")).unwrap();
    assert!(toolbar.contains("SegmentedControl::new(mtm, NSControlSize::Regular, TOOLS.len())"));
    // A row of options drawn as radio buttons in Rust (a two-word toggle as
    // segments) is a segmented control of its own. The kit's lone radio
    // button and the first-launch choice cards are other controls.
    for file in &sources {
        let name = file.file_name().unwrap().to_string_lossy().to_string();
        if matches!(name.as_str(), "kit.rs" | "welcome_view.rs") || name.contains("test") {
            continue;
        }
        let text = std::fs::read_to_string(file).unwrap();
        assert!(
            !text.contains("Role::RadioButton"),
            "{}: options drawn as radio buttons; use the kit's segmented control",
            file.display()
        );
    }
    let view = std::fs::read_to_string(root().join("crates/app/src/v4_view.rs")).unwrap();
    assert!(view.contains("Some(Component::SegmentedControl) => Some(Kind::Segmented)"));
}
