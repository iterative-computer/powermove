//! The agent's route to the commands a person has: `list_commands` lists
//! every menu, palette and key command with its keys and state, and
//! `run_command` runs one the way its menu row runs it. Commands that open a
//! file panel or a sheet (which waits unanswered in the background), quit or
//! close a window are refused, each naming the agent tool that does that job.
use crate::layer_commands::Plan;
use pm_kernel::Published;
use pm_kernel::commands::comps::{CompCommand, NewComp};
use serde_json::{Value, json};

/// Commands only the agent runs (a person has the same through a key, a
/// toggle or a sheet): id, label, menu, and its arguments.
pub const AGENT: [(&str, &str, &str, &str); 9] = [
    ("transport.play", "Play", "Playback", ""),
    ("transport.pause", "Pause", "Playback", ""),
    (
        "transport.seek",
        "Move the Playhead",
        "Playback",
        "[seconds]",
    ),
    (
        "transport.loop",
        "Loop Playback On or Off",
        "Playback",
        "[on]",
    ),
    (
        "workArea.set",
        "Set the Work Area",
        "Playback",
        "[start, end] in seconds",
    ),
    (
        "precompose.selection",
        "Pre-compose the Selected Layers",
        "Composition",
        "[{name?, leaveAttributes?, openAfter?}]",
    ),
    (
        "composition.new",
        "New Composition",
        "Composition",
        "[{name?, w?, h?, fps?, dur?, background?}]",
    ),
    (
        "color.importCdlFile",
        "Import a CDL onto the Selected Layer's Color Wheels",
        "Color",
        "[path of a .cc, .cdl or .ccc]",
    ),
    (
        "color.exportCdlFile",
        "Export the Selected Layer's Color Wheels as a CDL",
        "Color",
        "[path of a new .cc or .cdl]",
    ),
];

/// What a command refused here is, and the agent's way to it: `Some(tool)`
/// does the same job; `None` is not an edit (windows, the app, the agent's
/// own panel).
pub fn refused(command: &str) -> Option<Option<&'static str>> {
    Some(match command {
        "quit" | "closeWindow" | "closeTab" | "newWindow" | "newTab" | "moveTabToNewWindow"
        | "nextTab" | "previousTab" | "checkForUpdates" | "help" | "palette" | "fieldMenu"
        | "agent" | "bringWorkspace" | "bringSignIns" | "panelLibrary" => None,
        "open" | "openRecent" => Some("open_project"),
        "projects" => Some("list_projects"),
        "newProject" => Some("create_project"),
        "save" | "saveAs" => Some("save_project"),
        "import" | "importSequence" | "importFolder" => Some("import_media"),
        "importCaptions" | "addCaption" => Some("add_captions (apply_commands)"),
        "generateCaptions" => Some("generate_captions"),
        "export" => Some("export_video"),
        "exportCaptions" => Some("export_captions"),
        "compositionSettings" => Some("set_composition (apply_commands)"),
        "newComposition" => Some("run_command composition.new"),
        "precompose" => Some("run_command precompose.selection"),
        "keyframeValue" | "scaleKeyframes" => Some("replace_keyframes (apply_commands)"),
        "keyframeVelocity" => Some("set_easing (apply_commands)"),
        "viewer.applyLut" => Some("manage_media"),
        "color.importCdl" => Some("run_command color.importCdlFile"),
        "color.exportCdl" => Some("run_command color.exportCdlFile"),
        "store" => Some("store_search"),
        _ => return None,
    })
}

/// Whether `command` reads as on: its own toggle, or the one it sets.
pub fn is_checked(command: &str, checked: &std::collections::BTreeSet<String>) -> bool {
    checked.contains(command) || (command == "transport.loop" && checked.contains("toggleLoop"))
}
/// Every command a person can run here, and the agent's commands, with keys,
/// whether each can run now and whether a toggle is on.
pub fn list(
    disabled: &std::collections::BTreeSet<String>,
    checked: &std::collections::BTreeSet<String>,
    query: &str,
) -> Value {
    let table = pm_shell::keymap::current();
    let query = query.trim().to_lowercase();
    let mut rows: Vec<Value> = crate::shortcuts::commands(&table)
        .into_iter()
        .filter(|(_, _, _, command, _)| refused(command).is_none())
        .map(|(menu, _, label, command, args)| {
            let keys: Vec<String> = table
                .iter()
                .filter(|b| b.command == command && b.args == args)
                .map(|b| pm_shell::menu::glyphs(&b.chord))
                .collect();
            json!({"command": command, "args": args, "label": label, "menu": menu, "keys": keys,
                "enabled": !disabled.contains(&command), "checked": checked.contains(&command)})
        })
        .collect();
    rows.extend(AGENT.iter().map(|(command, label, menu, args)| {
        json!({"command": command, "args": args, "label": label, "menu": menu, "keys": [],
            "enabled": true, "checked": is_checked(command, checked)})
    }));
    rows.push(json!({"command": "shortcuts", "args": "[{op: \"set\", value: {preset, bindings: [{command, args, keys}]}}]",
        "label": "Set the Keyboard Shortcuts", "menu": "Settings", "keys": [], "enabled": true, "checked": false}));
    if !query.is_empty() {
        rows.retain(|r| {
            [&r["label"], &r["menu"], &r["command"]].iter().any(|v| {
                v.as_str()
                    .is_some_and(|s| s.to_lowercase().contains(&query))
            })
        });
    }
    json!({"commands": rows,
        "note": "Run one with run_command {command, args}. Commands on the selection act on the project's selection (select_layers) and the playhead (transport.seek)."})
}

/// Whether the agent may run `command` now: a person's command (or the
/// agent's own), not refused, not unavailable in this state.
pub fn check(command: &str, disabled: &std::collections::BTreeSet<String>) -> Result<(), String> {
    if let Some(alternative) = refused(command) {
        return Err(match alternative {
            Some(tool) => format!("{command} opens a panel or sheet; use {tool} instead."),
            None => format!("{command} is not available to the agent."),
        });
    }
    let known = AGENT.iter().any(|(c, ..)| *c == command)
        || command == "shortcuts"
        || crate::shortcuts::commands(&pm_shell::keymap::current())
            .iter()
            .any(|(_, _, _, c, _)| c == command);
    if !known {
        return Err(format!(
            "Unknown command {command}. Read list_commands for ids."
        ));
    }
    if disabled.contains(command) {
        return Err(format!(
            "{command} cannot run now (it needs a project, a selection or selected keyframes). Read list_commands."
        ));
    }
    Ok(())
}

/// The agent's edits that a person makes through a sheet.
pub fn plan(name: &str, args: &[Value], published: &Published, time: f64) -> Option<Plan> {
    let options = args.first().cloned().unwrap_or(Value::Null);
    let number = |key: &str| options.get(key).and_then(Value::as_f64);
    let text = |key: &str| options.get(key).and_then(Value::as_str).map(str::to_owned);
    match name {
        "workArea.set" => {
            let at = |i: usize| args.get(i).and_then(Value::as_f64);
            Some(Plan {
                label: "Set Work Area".into(),
                commands: vec![
                    CompCommand::SetWorkArea {
                        comp: None,
                        start: at(0),
                        end: at(1),
                    }
                    .into(),
                ],
                select: None,
                count_keys: None,
            })
        }
        "precompose.selection" => {
            let mut draft = crate::layer_commands::precompose_draft(published)?;
            if let Some(name) = text("name") {
                draft.name = name;
            }
            if let Some(leave) = options.get("leaveAttributes").and_then(Value::as_bool) {
                draft.leave = leave;
            }
            if let Some(open) = options.get("openAfter").and_then(Value::as_bool) {
                draft.open_after = open;
            }
            Some(crate::layer_commands::precompose_plan(
                published, &draft, time,
            ))
        }
        "composition.new" => {
            let comp_id = pm_kernel::new_id("C");
            Some(Plan {
                label: "New Composition".into(),
                commands: vec![
                    CompCommand::Create {
                        comp_id: comp_id.clone(),
                        settings: NewComp {
                            name: text("name"),
                            width: number("w"),
                            height: number("h"),
                            fps: number("fps"),
                            duration: number("dur"),
                            background: text("background"),
                        },
                    }
                    .into(),
                    CompCommand::Open { comp_id }.into(),
                ],
                select: Some(vec![]),
                count_keys: None,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parity_tests::{Desk, project};

    /// Every command a person can reach (menus, the palette, keys) has an
    /// agent route: run_command, or a named tool that does the same job;
    /// only commands that are not edits (windows, the app) have none.
    #[test]
    fn every_person_reachable_command_has_an_agent_route() {
        let table = pm_shell::keymap::current();
        let mut missing = vec![];
        let mut not_edits = vec![];
        for (_, _, _, command, _) in crate::shortcuts::commands(&table) {
            match refused(&command) {
                None => {
                    // run_command reaches it: it is known and passes the check.
                    if let Err(e) = check(&command, &Default::default()) {
                        missing.push(format!("{command}: {e}"));
                    }
                }
                Some(Some(_)) => {}
                Some(None) => not_edits.push(command),
            }
        }
        assert!(missing.is_empty(), "no agent route: {missing:?}");
        // What has none is window and app control, never an edit.
        for command in &not_edits {
            assert!(
                [
                    "quit",
                    "closeWindow",
                    "closeTab",
                    "newWindow",
                    "newTab",
                    "moveTabToNewWindow",
                    "nextTab",
                    "previousTab",
                    "checkForUpdates",
                    "help",
                    "palette",
                    "fieldMenu",
                    "agent",
                    "bringWorkspace",
                    "bringSignIns",
                    "panelLibrary"
                ]
                .contains(&command.as_str()),
                "{command} is an edit with no agent route"
            );
        }
        // The editing families the audit named are among the runnable ones.
        for command in [
            "play",
            "toggleLoop",
            "workIn",
            "workOut",
            "resetWorkArea",
            "rippleDelete",
            "rippleTrimIn",
            "extendEdit",
            "slipLayers",
            "slideLayer",
            "liftWorkArea",
            "timeRemapLayers",
            "freezeFrame",
            "graph",
            "graph.value",
            "timeline.tool:ripple",
            "viewer.grid",
            "viewer.rulers",
            "viewer.showAlpha",
        ] {
            assert!(check(command, &Default::default()).is_ok(), "{command}");
        }
        for command in [
            "transport.seek",
            "workArea.set",
            "precompose.selection",
            "shortcuts",
        ] {
            assert!(check(command, &Default::default()).is_ok(), "{command}");
        }
        assert!(check("quit", &Default::default()).is_err());
        assert!(
            check("export", &Default::default())
                .unwrap_err()
                .contains("export_video")
        );
        assert!(check("noSuchCommand", &Default::default()).is_err());
        let off = std::collections::BTreeSet::from(["rippleDelete".to_owned()]);
        assert!(
            check("rippleDelete", &off)
                .unwrap_err()
                .contains("cannot run now")
        );
    }

    #[test]
    fn the_list_carries_keys_state_and_the_agents_own_commands() {
        let checked = std::collections::BTreeSet::from(["toggleLoop".to_owned()]);
        let off = std::collections::BTreeSet::from(["rippleDelete".to_owned()]);
        let list = list(&off, &checked, "");
        let rows = list["commands"].as_array().unwrap();
        let row = |command: &str| rows.iter().find(|r| r["command"] == command).cloned();
        assert_eq!(row("rippleTrimIn").unwrap()["keys"], json!(["⌥⇧["]));
        assert_eq!(row("rippleDelete").unwrap()["enabled"], json!(false));
        assert_eq!(row("toggleLoop").unwrap()["checked"], json!(true));
        assert!(row("transport.seek").is_some() && row("shortcuts").is_some());
        assert!(
            row("quit").is_none() && row("export").is_none(),
            "refused rows are not offered"
        );
        let found = super::list(&off, &checked, "ripple");
        assert!(
            found["commands"]
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r.to_string().to_lowercase().contains("ripple"))
        );
    }

    /// The agent's sheet-free edits through a real session: one undo each.
    #[test]
    fn the_agents_work_area_precompose_and_new_composition() {
        let layer = |id: &str| {
            json!({"id": id, "type": "shape", "name": id, "from": 0, "dur": 4, "on": true,
                "p": {"opacity": {"v": 100, "kf": []}}, "d": {"shape": "rect", "w": 10, "h": 10}})
        };
        let desk = Desk::new(project(vec![layer("a"), layer("b")]));
        let run = |name: &str, args: Value| {
            let args = args.as_array().cloned().unwrap();
            let published = desk.published();
            let plan = plan(name, &args, &published, 0.).unwrap_or_else(|| panic!("{name}"));
            desk.call(crate::layer_commands::transaction(&plan, None).unwrap());
        };
        run("workArea.set", json!([1, 3]));
        let work = |d: &Desk| {
            let p = d.published();
            let root = p.snapshot.root().unwrap();
            let open = pm_kernel::model::open_comp_id(root);
            pm_kernel::model::comp_node(root, open.as_deref())
                .unwrap()
                .get("work")
                .unwrap()
                .to_value()
        };
        assert_eq!(pm_model::json::to_canonical_string(&work(&desk)), "[1,3]");
        desk.select(&["a"]);
        run("precompose.selection", json!([{"name": "Agent Pre-comp"}]));
        let names: Vec<_> = desk.layers().into_iter().map(|l| l.kind).collect();
        assert!(names.contains(&"precomp".to_owned()), "{names:?}");
        assert!(
            desk.layers().iter().all(|l| l.id != "a"),
            "a moved into the pre-comp"
        );
        desk.undo();
        assert!(desk.layers().iter().any(|l| l.id == "a"), "one undo step");
        run(
            "composition.new",
            json!([{"name": "Agent Comp", "w": 320, "h": 240}]),
        );
        let p = desk.published();
        let comps = p.snapshot.root().unwrap().get("comps").unwrap().to_value();
        assert!(pm_model::json::to_canonical_string(&comps).contains("Agent Comp"));
    }
}
