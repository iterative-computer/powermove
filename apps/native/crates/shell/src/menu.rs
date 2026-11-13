//! One menu model for AppKit and table verification.
use serde::{Deserialize, Serialize};
use serde_json::json;
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
    /// A registry command, showing the shipped keymap's chord for it.
    fn registered(
        keymap: &[crate::keyboard::Binding],
        label: &str,
        command: &str,
        args: serde_json::Value,
    ) -> Self {
        let args = args.as_array().cloned().unwrap_or_default();
        let chord = keymap
            .iter()
            .find(|b| b.command == command && b.args == args)
            .map(|b| b.chord.as_str())
            .unwrap_or_default();
        Self {
            args,
            ..Self::command(label, command, chord)
        }
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
    let keymap = crate::keyboard::shipped();
    [
        ("Select", "toolSelect"),
        ("Hand", "toolHand"),
        ("Zoom", "toolZoom"),
        ("Rotate", "toolRotate"),
        ("Anchor Point", "toolAnchor"),
        ("Shape", "toolShape"),
        ("Pen", "toolPen"),
        ("Text", "toolText"),
    ]
    .iter()
    .map(|(l, c)| Item::registered(&keymap, l, c, json!([])))
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
    let keymap = crate::keyboard::shipped();
    let r = |label: &str, command: &str| Item::registered(&keymap, label, command, json!([]));
    let r_args = |label: &str, command: &str, args| Item::registered(&keymap, label, command, args);
    let k = Item::command;
    vec![
        (
            "Powermove",
            vec![
                Item::role("About Powermove", "orderFrontStandardAboutPanel:", ""),
                Item::command("Check for Updates…", "checkForUpdates", ""),
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
                r("Projects…", "projects"),
                sep(),
                m("closeTab"),
                sep(),
                m("save"),
                m("saveAs"),
                sep(),
                m("import"),
                Item::command("Import Image Sequence…", "importSequence", ""),
                Item::command("Import Folder…", "importFolder", ""),
                Item::command("Import Captions…", "importCaptions", ""),
                sep(),
                m("export"),
                Item::command("Export Captions…", "exportCaptions", ""),
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
                r("Deselect All", "deselect"),
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
                k("Add Caption at Playhead", "addCaption", ""),
                k("Generate Captions", "generateCaptions", ""),
                sep(),
                m("precompose"),
                k("Release Pre-comp", "releasePrecomp", ""),
                k("Continuously Rasterize", "toggleContinuousRaster", ""),
                sep(),
                k("Split Text by Word", "splitTextWords", ""),
                k("Split Text by Character", "splitTextCharacters", ""),
                k("Split Text by Line", "splitTextLines", ""),
                sep(),
                // Layer commands the shipped keymap has no chord for stand
                // here: the Layer menu lists registry bindings only.
                k("Hide/Show Selected Layers", "toggleVisibility", ""),
                k("Solo Selected Layers", "toggleSolo", ""),
                k("Fit to Composition", "fitToComposition", ""),
                k("Separate Audio", "separateAudio", ""),
                sep(),
                k("Edit Keyframe Value…", "keyframeValue", ""),
                k("Keyframe Velocity…", "keyframeVelocity", ""),
                k("Scale Keyframes…", "scaleKeyframes", ""),
                k("Simplify Keyframes", "simplifyKeyframes", ""),
            ],
        ),
        (
            "Layer",
            vec![
                r("New Solid", "newSolid"),
                r("New Shape", "newShape"),
                r("New Adjustment Layer", "newAdjustment"),
                r("New Null Object", "newNull"),
                sep(),
                r("Group", "groupLayers"),
                r("Ungroup", "ungroupLayers"),
                sep(),
                r("Lock Selected Layers", "lockSelectedLayers"),
                r("Unlock All Layers", "unlockAllLayers"),
                sep(),
                r("Select Previous Layer", "selectPreviousLayer"),
                r("Select Next Layer", "selectNextLayer"),
                r(
                    "Extend Selection to Previous Layer",
                    "extendSelectionPreviousLayer",
                ),
                r("Extend Selection to Next Layer", "extendSelectionNextLayer"),
                sep(),
                r("Trim In Point to Playhead", "trimIn"),
                r("Trim Out Point to Playhead", "trimOut"),
                r("Move In Point to Playhead", "moveLayerIn"),
                r("Move Out Point to Playhead", "moveLayerOut"),
                r("Move to Composition Start", "moveLayerInToStart"),
                r("Move to Composition End", "moveLayerOutToEnd"),
                sep(),
                r("Center Anchor Point", "centerAnchor"),
                r("Edit Text", "text.editSelected"),
                r("Delete", "delete"),
            ],
        ),
        (
            "Timeline",
            vec![
                r("Play/Pause", "play"),
                r("Shuttle Reverse", "shuttleBackward"),
                r("Shuttle Stop", "shuttleStop"),
                r("Shuttle Forward", "shuttleForward"),
                sep(),
                r("Go to Start", "gotoStart"),
                r("Go to End", "gotoEnd"),
                r("Previous Frame", "prevFrame"),
                r("Next Frame", "nextFrame"),
                r_args("Back 10 Frames", "stepFrames", json!([-10])),
                r_args("Forward 10 Frames", "stepFrames", json!([10])),
                r("Previous Keyframe", "timeline.adjacentKeyframe:prev"),
                r("Next Keyframe", "timeline.adjacentKeyframe:next"),
                r("Previous Visible Event", "prevVisibleEvent"),
                r("Next Visible Event", "nextVisibleEvent"),
                sep(),
                r("Go to Layer In Point", "gotoLayerIn"),
                r("Go to Layer Out Point", "gotoLayerOut"),
                r("Go to Work Area Start", "gotoWorkIn"),
                r("Go to Work Area End", "gotoWorkOut"),
                r("Set Work Area Start", "workIn"),
                r("Set Work Area End", "workOut"),
                sep(),
                r_args("Nudge Keyframes Earlier", "nudgeKeyframes", json!([-1])),
                r_args("Nudge Keyframes Later", "nudgeKeyframes", json!([1])),
                r("Easy Ease", "easyEase"),
                r("Easy Ease In", "easyEaseIn"),
                r("Easy Ease Out", "easyEaseOut"),
                sep(),
                r("Graph Editor", "graph"),
                r("Reveal All Animated Properties", "timeline.revealAll"),
            ],
        ),
        ("Playback", vec![k("Loop Playback", "toggleLoop", "")]),
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
                r("Command Palette…", "palette"),
                Item::command("Field Menu…", "fieldMenu", ""),
                r("Agent", "agent"),
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
                Item::command("Panel Library", "panelLibrary", ""),
                Item::command("Maximize Panel", "maximizePanel", "`"),
                Item::command("Restore Panel", "restorePanel", ""),
                sep(),
                Item::command("New Tab", "newTab", ""),
                m("nextTab"),
                m("previousTab"),
                Item::command("Move Tab to New Window", "moveTabToNewWindow", ""),
                Item::role("Merge All Windows", "mergeAllWindows:", ""),
                Item::role("Show All Tabs", "toggleTabOverview:", ""),
                sep(),
                Item::command("Store", "store", ""),
                Item::command("Background Tasks", "backgroundTasks", "cmd+9"),
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
