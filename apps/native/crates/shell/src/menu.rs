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
    /// Command arguments, as in the W1 registry row the chord comes from.
    pub args: Vec<serde_json::Value>,
}
impl Item {
    fn command(label: &str, command: &str, chord: &str) -> Self {
        Self {
            label: label.into(),
            command: command.into(),
            chord: chord.into(),
            role: None,
            args: vec![],
        }
    }
    fn with_args(mut self, args: serde_json::Value) -> Self {
        self.args = args.as_array().cloned().unwrap_or_default();
        self
    }
    fn role(label: &str, role: &'static str, chord: &str) -> Self {
        Self {
            role: Some(role),
            ..Self::command(label, role, chord)
        }
    }
}
/// Menus (and submenus, by their item command) whose key equivalents are shown
/// but never matched by AppKit. Their shortcuts come from the W1 registry and are
/// resolved only by the shell's ordered key resolver, so a declined key replays
/// to the focused control (typing `v` in a field) instead of firing a menu item.
pub const DISPLAY_ONLY: [&str; 3] = ["Layer", "Timeline", "tools"];
/// Commands whose items get a dynamic submenu (rebuilt when opened).
pub const SUBMENUS: [&str; 4] = ["openRecent", "workspaces", "panels", "tools"];
/// Items of the View ▸ Tools submenu, in toolbar order (registry chords).
pub fn tools() -> Vec<Item> {
    [
        ("Select", "toolSelect", "v"),
        ("Hand", "toolHand", "h"),
        ("Zoom", "toolZoom", "z"),
        ("Rotate", "toolRotate", "w"),
        ("Anchor Point", "toolAnchor", "y"),
        ("Shape", "toolShape", "q"),
        ("Pen", "toolPen", "g"),
        ("Text", "toolText", "cmd+t"),
    ]
    .iter()
    .map(|(l, c, k)| Item::command(l, c, k))
    .collect()
}
pub fn menus() -> Vec<(&'static str, Vec<Item>)> {
    let eq = equivalents();
    let m = |command: &str| {
        let e = eq.iter().find(|e| e.command == command).unwrap();
        Item::command(&e.label, &e.command, &e.accelerator)
    };
    let sep = || Item::command("", "", "");
    // Registry-only commands, labelled for the menu bar, with their W1 chords.
    let k = Item::command;
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
                Item::role("Show All", "unhideAllApplications:", ""),
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
                k("Projects…", "projects", "Command+P"),
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
                k("Delete", "delete", ""),
                m("contextSelectAll"),
                k("Deselect All", "deselect", "Command+Shift+A"),
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
            "Layer",
            vec![
                k("New Solid", "newSolid", "cmd+y"),
                k("New Shape", "newShape", "cmd+shift+y"),
                k("New Adjustment Layer", "newAdjustment", "cmd+alt+y"),
                sep(),
                k("Group", "groupLayers", "cmd+g"),
                k("Ungroup", "ungroupLayers", "cmd+shift+g"),
                sep(),
                k("Lock Selected Layers", "lockSelectedLayers", "cmd+l"),
                k("Unlock All Layers", "unlockAllLayers", "cmd+shift+l"),
                sep(),
                k("Select Previous Layer", "selectPreviousLayer", "cmd+up"),
                k("Select Next Layer", "selectNextLayer", "cmd+down"),
                k(
                    "Extend Selection to Previous Layer",
                    "extendSelectionPreviousLayer",
                    "cmd+shift+up",
                ),
                k(
                    "Extend Selection to Next Layer",
                    "extendSelectionNextLayer",
                    "cmd+shift+down",
                ),
                sep(),
                k("Trim In Point to Playhead", "trimIn", "alt+["),
                k("Trim Out Point to Playhead", "trimOut", "alt+]"),
                k("Move In Point to Playhead", "moveLayerIn", "["),
                k("Move Out Point to Playhead", "moveLayerOut", "]"),
                k(
                    "Move to Composition Start",
                    "moveLayerInToStart",
                    "alt+home",
                ),
                k("Move to Composition End", "moveLayerOutToEnd", "alt+end"),
                sep(),
                k("Center Anchor Point", "centerAnchor", "cmd+alt+home"),
                k("Edit Text", "text.editSelected", "enter"),
                k("Delete", "delete", "backspace"),
            ],
        ),
        (
            "Timeline",
            vec![
                k("Play/Pause", "play", "space"),
                sep(),
                k("Go to Start", "gotoStart", "home"),
                k("Go to End", "gotoEnd", "end"),
                k("Previous Frame", "prevFrame", "pageup"),
                k("Next Frame", "nextFrame", "pagedown"),
                k("Back 10 Frames", "stepFrames", "shift+pageup")
                    .with_args(serde_json::json!([-10])),
                k("Forward 10 Frames", "stepFrames", "shift+pagedown")
                    .with_args(serde_json::json!([10])),
                k(
                    "Previous Keyframe",
                    "timeline.adjacentKeyframe:prev",
                    "ctrl+left",
                ),
                k(
                    "Next Keyframe",
                    "timeline.adjacentKeyframe:next",
                    "ctrl+right",
                ),
                k("Previous Visible Event", "prevVisibleEvent", "j"),
                k("Next Visible Event", "nextVisibleEvent", "k"),
                sep(),
                k("Go to Layer In Point", "gotoLayerIn", "i"),
                k("Go to Layer Out Point", "gotoLayerOut", "o"),
                k("Go to Work Area Start", "gotoWorkIn", "shift+home"),
                k("Go to Work Area End", "gotoWorkOut", "shift+end"),
                k("Set Work Area Start", "workIn", "b"),
                k("Set Work Area End", "workOut", "n"),
                sep(),
                k("Nudge Keyframes Earlier", "nudgeKeyframes", "alt+left")
                    .with_args(serde_json::json!([-1])),
                k("Nudge Keyframes Later", "nudgeKeyframes", "alt+right")
                    .with_args(serde_json::json!([1])),
                k("Edit Value…", "keyframeValue", ""),
                k("Keyframe Velocity…", "keyframeVelocity", ""),
                k("Scale Keyframes…", "scaleKeyframes", ""),
                sep(),
                k("Easy Ease", "easyEase", "f9"),
                k("Easy Ease In", "easyEaseIn", "shift+f9"),
                k("Easy Ease Out", "easyEaseOut", "cmd+shift+f9"),
                sep(),
                k("Graph Editor", "graph", "shift+f3"),
                k(
                    "Reveal All Animated Properties",
                    "timeline.revealAll",
                    "cmd+`",
                ),
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
                Item::command("Composition Flowchart", "composition.flowchart", ""),
                sep(),
                Item::command("Tools", "tools", ""),
                sep(),
                Item::command("Show/Hide Sidebar", "toggleSidebar", ""),
                Item::command("Show/Hide Inspector", "toggleInspector", ""),
                sep(),
                k("Command Palette…", "palette", "Command+K"),
                Item::command("Field Menu…", "fieldMenu", ""),
                k("Agent", "agent", "Command+Shift+K"),
                sep(),
                Item::role(
                    "Enter Full Screen",
                    "toggleFullScreen:",
                    "Control+Command+F",
                ),
            ],
        ),
        (
            "Window",
            vec![
                Item::role("Minimize", "performMiniaturize:", "Command+M"),
                Item::role("Zoom", "performZoom:", ""),
                sep(),
                Item::command("Panels", "panels", ""),
                Item::command("Workspace", "workspaces", ""),
                Item::command("Maximize Panel", "maximizePanel", "`"),
                Item::command("Restore Panel", "restorePanel", ""),
                sep(),
                Item::command("New Tab", "newTab", ""),
                m("nextTab"),
                m("previousTab"),
                Item::command("Move Tab to New Window", "moveTabToNewWindow", ""),
                Item::role("Merge All Windows", "mergeAllWindows:", ""),
                Item::role("Show Tab Bar", "toggleTabBar:", ""),
                Item::role("Show All Tabs", "toggleTabOverview:", ""),
                sep(),
                Item::command("Error Log", "errorLog", ""),
                Item::command("Migration Report", "migrationReport", ""),
                sep(),
                Item::role("Bring All to Front", "arrangeInFront:", ""),
            ],
        ),
        ("Help", vec![Item::command("Powermove Help", "help", "")]),
    ]
}
/// AppKit's keyEquivalent uses the unmodified key and a separate modifier mask.
/// Accepts Electron accelerators (`Command+Shift+H`) and W1 chords (`cmd+shift+h`).
pub fn accelerator(s: &str) -> (String, u64) {
    let mut mods = 0;
    let mut key = String::new();
    // A literal plus is a trailing `+` (`cmd+shift++`).
    let (body, plus) = match s.strip_suffix("++") {
        Some(b) => (b, true),
        None => (s, false),
    };
    for part in body.split('+') {
        match part {
            "CommandOrControl" | "Command" | "cmd" => mods |= 1 << 20,
            "Control" | "ctrl" => mods |= 1 << 18,
            "Alt" | "alt" => mods |= 1 << 19,
            "Shift" | "shift" => mods |= 1 << 17,
            k => key = k.to_lowercase(),
        }
    }
    if plus {
        key = "+".into();
    }
    // NSFunctionKey code points; AppKit draws the matching glyphs.
    let special = match key.as_str() {
        "tab" => Some('\t'),
        "space" => Some(' '),
        "enter" => Some('\r'),
        "backspace" => Some('\u{8}'),
        "delete" => Some('\u{F728}'),
        "escape" => Some('\u{1b}'),
        "up" => Some('\u{F700}'),
        "down" => Some('\u{F701}'),
        "left" => Some('\u{F702}'),
        "right" => Some('\u{F703}'),
        "home" => Some('\u{F729}'),
        "end" => Some('\u{F72B}'),
        "pageup" => Some('\u{F72C}'),
        "pagedown" => Some('\u{F72D}'),
        k if k.len() >= 2 && k.starts_with('f') => k[1..]
            .parse::<u32>()
            .ok()
            .filter(|n| (1..=35).contains(n))
            .and_then(|n| char::from_u32(0xF703 + n)),
        _ => None,
    };
    if let Some(c) = special {
        key = c.to_string();
    }
    (key, mods)
}
/// Human-readable shortcut (`⇧⌘K`) for a W1 chord or Electron accelerator.
pub fn glyphs(chord: &str) -> String {
    if chord.is_empty() {
        return String::new();
    }
    let (key, mods) = accelerator(chord);
    let mut s = String::new();
    for (bit, g) in [(18, "⌃"), (19, "⌥"), (17, "⇧"), (20, "⌘")] {
        if mods & (1 << bit) != 0 {
            s.push_str(g);
        }
    }
    let k = match key.chars().next() {
        Some('\t') => "⇥".into(),
        Some(' ') => "Space".into(),
        Some('\r') => "↩".into(),
        Some('\u{8}') => "⌫".into(),
        Some('\u{F728}') => "⌦".into(),
        Some('\u{1b}') => "⎋".into(),
        Some('\u{F700}') => "↑".into(),
        Some('\u{F701}') => "↓".into(),
        Some('\u{F702}') => "←".into(),
        Some('\u{F703}') => "→".into(),
        Some('\u{F729}') => "↖".into(),
        Some('\u{F72B}') => "↘".into(),
        Some('\u{F72C}') => "⇞".into(),
        Some('\u{F72D}') => "⇟".into(),
        Some(c) if ('\u{F704}'..='\u{F726}').contains(&c) => {
            format!("F{}", c as u32 - 0xF703)
        }
        _ => key.to_uppercase(),
    };
    s.push_str(&k);
    s
}
