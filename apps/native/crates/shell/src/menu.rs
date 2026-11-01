//! One menu model for AppKit and table verification.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Equivalent {
    pub id: String,
    pub accelerator: String,
    pub command: String,
    pub label: String,
}
pub fn equivalents() -> Vec<Equivalent> {
    serde_json::from_str(include_str!("../data/menu-equivalents.json")).unwrap()
}
#[derive(Clone, Debug)]
pub struct Item {
    pub label: String,
    pub command: String,
    pub chord: String,
    pub role: Option<&'static str>,
}
impl Item {
    fn command(label: &str, command: &str, chord: &str) -> Self {
        Self {
            label: label.into(),
            command: command.into(),
            chord: chord.into(),
            role: None,
        }
    }
    fn role(label: &str, role: &'static str, chord: &str) -> Self {
        Self {
            role: Some(role),
            ..Self::command(label, role, chord)
        }
    }
}
pub fn menus() -> Vec<(&'static str, Vec<Item>)> {
    let eq = equivalents();
    let m = |command: &str| {
        let e = eq.iter().find(|e| e.command == command).unwrap();
        Item::command(&e.label, &e.command, &e.accelerator)
    };
    let sep = || Item::command("", "", "");
    vec![
        (
            "Powermove",
            vec![
                Item::role("About Powermove", "orderFrontStandardAboutPanel:", ""),
                sep(),
                m("settings"),
                sep(),
                Item::role("Services", "services", ""),
                sep(),
                Item::role("Hide Powermove", "hide:", "Command+H"),
                Item::role("Hide Others", "hideOtherApplications:", "Command+Alt+H"),
                sep(),
                Item::role("Quit Powermove", "terminate:", "Command+Q"),
            ],
        ),
        (
            "File",
            vec![
                m("newProject"),
                m("newWindow"),
                m("open"),
                Item::command("Open Recent", "openRecent", ""),
                sep(),
                m("closeTab"),
                sep(),
                m("save"),
                m("saveAs"),
                sep(),
                m("import"),
                Item::command("Import Image Sequence…", "importSequence", ""),
                Item::command("Import Folder…", "importFolder", ""),
                sep(),
                m("export"),
                sep(),
                m("closeWindow"),
            ],
        ),
        (
            "Edit",
            vec![
                m("contextUndo"),
                m("contextRedo"),
                sep(),
                m("contextCut"),
                m("contextCopy"),
                m("contextPaste"),
                m("contextSelectAll"),
                sep(),
                m("duplicate"),
                m("split"),
                sep(),
                m("bringForward"),
                m("sendBackward"),
                m("bringToFront"),
                m("sendToBack"),
                sep(),
                Item::role("Spelling and Grammar…", "showGuessPanel:", ""),
                Item::role(
                    "Check Spelling While Typing",
                    "toggleContinuousSpellChecking:",
                    "",
                ),
                Item::role("Writing Tools…", "showWritingTools:", ""),
                Item::role(
                    "Emoji & Symbols",
                    "orderFrontCharacterPalette:",
                    "Control+Command+Space",
                ),
            ],
        ),
        (
            "Composition",
            vec![
                m("newComposition"),
                m("compositionSettings"),
                sep(),
                m("precompose"),
            ],
        ),
        (
            "View",
            vec![
                m("zoomIn"),
                m("zoomOut"),
                m("actualSize"),
                m("fitComposition"),
                m("toggleLayerControls"),
                sep(),
                Item::role(
                    "Enter Full Screen",
                    "toggleFullScreen:",
                    "Control+Command+F",
                ),
                sep(),
                Item::command("Command Palette…", "palette", ""),
                Item::command("Field Menu…", "fieldMenu", ""),
                Item::command("Show/Hide Sidebar", "toggleSidebar", ""),
                Item::command("Show/Hide Inspector", "toggleInspector", ""),
                Item::command("Workspaces", "workspaces", ""),
            ],
        ),
        (
            "Window",
            vec![
                Item::role("Minimize", "performMiniaturize:", "Command+M"),
                Item::role("Zoom", "performZoom:", ""),
                sep(),
                m("nextTab"),
                m("previousTab"),
                Item::command("Move Tab to New Window", "moveTabToNewWindow", ""),
                sep(),
                Item::command("New Window", "newWindow", ""),
                sep(),
                Item::role("Bring All to Front", "arrangeInFront:", ""),
            ],
        ),
        ("Help", vec![Item::command("Powermove Help", "help", "")]),
    ]
}
/// AppKit's keyEquivalent uses the unmodified key and a separate modifier mask.
pub fn accelerator(s: &str) -> (String, u64) {
    let mut mods = 0;
    let mut key = String::new();
    for part in s.split('+') {
        match part {
            "CommandOrControl" | "Command" => mods |= 1 << 20,
            "Control" => mods |= 1 << 18,
            "Alt" => mods |= 1 << 19,
            "Shift" => mods |= 1 << 17,
            k => key = k.to_lowercase(),
        }
    }
    if key == "tab" {
        key = "\t".into();
    }
    (key, mods)
}
