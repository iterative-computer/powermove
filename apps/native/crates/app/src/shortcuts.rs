//! Settings › Shortcuts: what each command's keys are, which keys two
//! commands share, and the person's changes, kept in the profile as the
//! `keymap` setting (`pm_shell::keymap`'s plain data, which the agent and
//! extensions can write as well).
use pm_shell::keyboard::Binding;
use pm_shell::keymap::{self, Assign, Keymap};
use pm_shell::menu;
use serde_json::{Value, json};

/// The profile setting that holds the person's keymap.
pub const SETTING: &str = "keymap";

/// The person's keymap as stored (the default when absent or unreadable).
pub fn stored(state: &pm_kernel::state::StateSnapshot) -> Keymap {
    state
        .settings
        .get(SETTING)
        .and_then(|v| serde_json::from_str(&pm_model::json::to_canonical_string(v)).ok())
        .and_then(|v| keymap::parse(&v).ok())
        .unwrap_or_default()
}

/// A command's row id: its name, with its arguments as the palette writes them.
fn row_id(command: &str, args: &[Value]) -> String {
    menu::palette_command(command, args)
}

/// Names for commands that only a key reaches (no menu row or palette label).
fn key_only_label(command: &str, args: &[Value]) -> Option<String> {
    let number = |i: usize| args.get(i).and_then(Value::as_f64).unwrap_or(0.);
    Some(match command {
        "undo" => "Undo".into(),
        "redo" => "Redo".into(),
        "copyLayers" => "Copy".into(),
        "cutLayers" => "Cut".into(),
        "contextPaste" => "Paste".into(),
        "selectAll" => "Select All".into(),
        "blurField" => "Leave Text Field".into(),
        "toggleLayerStrips" => "Show Layer Bars".into(),
        "nudgeKeyframes" if number(0) < 0. => {
            format!("Nudge Keyframes {} Frames Earlier", -number(0))
        }
        "nudgeKeyframes" => format!("Nudge Keyframes {} Frames Later", number(0)),
        // Control-J: reverse from a stop, without holding K.
        "shuttleBackward" => "Shuttle Reverse from Stop".into(),
        "nudgeSelection" => {
            let (x, y) = (number(0), number(1));
            let (way, by) = match (x, y) {
                (x, _) if x < 0. => ("Left", -x),
                (x, _) if x > 0. => ("Right", x),
                (_, y) if y < 0. => ("Up", -y),
                _ => ("Down", y),
            };
            format!("Nudge {way} {by} px")
        }
        other => {
            let letter = other.strip_prefix("timeline.revealProperty:")?;
            let name = match letter {
                "p" => "Position",
                "s" => "Scale",
                "r" => "Rotation",
                "t" => "Opacity",
                "a" => "Anchor Point",
                "u" => "Animated Properties",
                "e" => "Effects",
                "m" => "Masks",
                "f" => "Mask Feather",
                "l" => "Audio Levels",
                _ => return None,
            };
            if args.first() == Some(&json!(true)) {
                format!("Reveal {name} Too")
            } else {
                format!("Reveal {name}")
            }
        }
    })
}

/// Every command a person can give a key: the menus' and the palette's
/// commands, and those only a key reaches. Menu, id, label, command, args.
pub fn commands(table: &[Binding]) -> Vec<(String, String, String, String, Vec<Value>)> {
    let mut out: Vec<(String, String, String, String, Vec<Value>)> = menu::palette_rows()
        .into_iter()
        .map(|(title, id, label)| {
            let (command, args) = menu::split_palette_command(&id);
            let command = command.to_owned();
            (title.to_owned(), id, label, command, args)
        })
        .collect();
    for b in table {
        let id = row_id(&b.command, &b.args);
        if out.iter().any(|row| row.1 == id) {
            continue;
        }
        let label = key_only_label(&b.command, &b.args).unwrap_or_else(|| b.command.clone());
        out.push(("Keys".into(), id, label, b.command.clone(), b.args.clone()));
    }
    out
}

/// The Shortcuts page: the preset, each command's keys (as glyphs and as
/// chords, for search), whether the person changed it, whether the menu bar
/// owns it, and the keys two commands share. `recording` is the row whose
/// next key is being taken.
pub fn fact(map: &Keymap, recording: Option<&str>, asked: Option<&str>) -> Value {
    let table = keymap::effective(map);
    let entries = commands(&table);
    let keys_of = |command: &str, args: &[Value]| -> Vec<String> {
        let mut keys: Vec<String> = table
            .iter()
            .filter(|b| b.command == command && b.args == args)
            .map(|b| b.chord.clone())
            .collect();
        // A Control twin of a Command key is the same shortcut on another keyboard.
        let twins: Vec<String> = keys
            .iter()
            .filter_map(|k| k.strip_prefix("ctrl+").map(|rest| format!("cmd+{rest}")))
            .filter(|cmd| keys.contains(cmd))
            .collect();
        keys.retain(|k| {
            !k.strip_prefix("ctrl+")
                .is_some_and(|rest| twins.contains(&format!("cmd+{rest}")))
        });
        keys
    };
    let label_of = |command: &str, args: &[Value]| {
        entries
            .iter()
            .find(|e| e.3 == command && e.4 == args)
            .map(|e| e.2.clone())
            .unwrap_or_else(|| command.to_owned())
    };
    let rows: Vec<Value> = entries
        .iter()
        .map(|(title, id, label, command, args)| {
            let keys = keys_of(command, args);
            json!({
                "id": id,
                "menu": title,
                "label": label,
                "keys": keys.iter().map(|k| menu::glyphs(k)).collect::<Vec<_>>(),
                "chords": keys,
                "changed": map.bindings.iter().any(|a| a.command == *command && a.args == *args),
                "fixed": menu::is_equivalent(command),
                "recording": recording == Some(id.as_str()),
                // The key the recording row asked about, and who has it.
                "asked": asked
                    .filter(|_| recording == Some(id.as_str()))
                    .and_then(|chord| {
                        let (_, other, menu_owned) = holder(map, id, chord)?;
                        Some(json!({"keys": menu::glyphs(chord), "other": other, "menu": menu_owned}))
                    }),
            })
        })
        .collect();
    let conflicts: Vec<Value> = keymap::conflicts(&table)
        .into_iter()
        .map(|clash| {
            let mut labels: Vec<String> = clash
                .commands
                .iter()
                .map(|(command, args)| label_of(command, args))
                .collect();
            if let Some(menu) = clash.menu {
                labels.push(menu);
            }
            json!({"keys": menu::glyphs(&clash.chord), "chord": clash.chord, "labels": labels})
        })
        .collect();
    json!({
        "preset": map.preset,
        "presets": keymap::PRESETS.iter().map(|(id, label)| json!({"v": id, "label": label})).collect::<Vec<_>>(),
        "rows": rows,
        "conflicts": conflicts,
        "changed": !map.bindings.is_empty(),
    })
}

/// What a Shortcuts action asks of the app.
#[derive(Debug, PartialEq)]
pub enum Act {
    /// Store this keymap.
    Store(Keymap),
    /// Take the next key for this row.
    Record(String),
    /// Stop taking a key.
    StopRecording,
    /// Read a keymap file / write one.
    Import,
    Export,
}

/// The command and arguments a row id names.
fn target(id: &str) -> (String, Vec<Value>) {
    let (command, args) = menu::split_palette_command(id);
    (command.to_owned(), args)
}

/// `map` with `command` given exactly `keys`.
pub fn assign(mut map: Keymap, id: &str, keys: Vec<String>) -> Result<Keymap, String> {
    let (command, args) = target(id);
    if menu::is_equivalent(&command) {
        return Err("This key belongs to the menu bar.".into());
    }
    map.bindings
        .retain(|a| !(a.command == command && a.args == args));
    map.bindings.push(Assign {
        command,
        args,
        keys,
    });
    Ok(map)
}

/// One Shortcuts action (`{op, id?, value?}`) over the stored keymap.
pub fn act(map: Keymap, action: &Value) -> Result<Act, String> {
    let id = || action["id"].as_str().ok_or("Shortcut row");
    Ok(match action["op"].as_str() {
        Some("record") => Act::Record(id()?.to_owned()),
        Some("stop") => Act::StopRecording,
        // Remove Shortcut: the command keeps no key.
        Some("clear") => Act::Store(assign(map, id()?, vec![])?),
        // Back to the preset's keys for this command.
        Some("reset") => {
            let (command, args) = target(id()?);
            let mut map = map;
            map.bindings
                .retain(|a| !(a.command == command && a.args == args));
            Act::Store(map)
        }
        Some("resetAll") => Act::Store(Keymap {
            bindings: vec![],
            ..map
        }),
        Some("preset") => {
            let preset = action["value"].as_str().ok_or("Preset")?;
            if !keymap::PRESETS.iter().any(|(p, _)| *p == preset) {
                return Err("Unknown preset".into());
            }
            Act::Store(Keymap {
                preset: preset.into(),
                ..map
            })
        }
        // A whole keymap as data (the agent, a script): checked as stored.
        Some("set") => Act::Store(keymap::parse(&action["value"])?),
        Some("import") => Act::Import,
        Some("export") => Act::Export,
        _ => return Err("Unknown shortcut action".into()),
    })
}

/// A recorded key for `id`: Escape cancels, anything else is its new key.
/// What a key pressed while a row records does.
#[derive(Debug, PartialEq)]
pub enum Recorded {
    /// The key is the row's now (free, or moved from the command that had it).
    Store(Keymap),
    /// Another command has the key: the row asks first ("press it again to
    /// move it here"), still recording. `chord` is the key asked about.
    Ask { chord: String },
    /// Escape: the row keeps its keys.
    Cancel,
    Error(String),
}

/// Who has `chord` besides row `id`: its row id, its name, and whether the
/// menu bar owns it (such a key never moves).
pub fn holder(map: &Keymap, id: &str, chord: &str) -> Option<(String, String, bool)> {
    let table = keymap::effective(map);
    let own = target(id);
    let b = table
        .iter()
        .find(|b| b.chord == chord && (b.command.as_str(), &b.args) != (own.0.as_str(), &own.1))?;
    let other = row_id(&b.command, &b.args);
    let label = commands(&table)
        .into_iter()
        .find(|e| e.1 == other)
        .map_or_else(|| b.command.clone(), |e| e.2);
    Some((other, label, menu::is_equivalent(&b.command)))
}

/// The key pressed while row `id` records. `asked` is the key the row last
/// asked about: pressed again, it moves here from the command that had it.
pub fn recorded(map: Keymap, id: &str, chord: &str, asked: Option<&str>) -> Recorded {
    if chord == "escape" {
        return Recorded::Cancel;
    }
    // Delete takes the row's key away, as System Settings does.
    if matches!(chord, "backspace" | "delete") {
        return match assign(map, id, vec![]) {
            Ok(next) => Recorded::Store(next),
            Err(error) => Recorded::Error(error),
        };
    }
    let Some(chord) = keymap::normalize(chord) else {
        return Recorded::Error(format!("{chord} cannot be a shortcut."));
    };
    let Some((other, _, menu_owned)) = holder(&map, id, &chord) else {
        return match assign(map, id, vec![chord]) {
            Ok(next) => Recorded::Store(next),
            Err(error) => Recorded::Error(error),
        };
    };
    if menu_owned || asked != Some(chord.as_str()) {
        return Recorded::Ask { chord };
    }
    // Pressed again: the key moves here, and the other command keeps the
    // rest of its keys.
    let (command, args) = target(&other);
    let rest: Vec<String> = keymap::effective(&map)
        .into_iter()
        .filter(|b| b.command == command && b.args == args && b.chord != chord)
        .map(|b| b.chord)
        .collect();
    match assign(map, id, vec![chord]).and_then(|next| assign(next, &other, rest)) {
        Ok(next) => Recorded::Store(next),
        Err(error) => Recorded::Error(error),
    }
}

/// An exported file: the keymap with a marker naming what it is.
pub fn export(map: &Keymap) -> String {
    let mut value = keymap::to_value(map);
    value["format"] = json!("powermove-shortcuts");
    value["version"] = json!(1);
    serde_json::to_string_pretty(&value).expect("plain data")
}
/// An imported file, checked as a stored keymap is.
pub fn import(text: &str) -> Result<Keymap, String> {
    let value: Value = serde_json::from_str(text)
        .map_err(|_| "This is not a Powermove keyboard shortcuts file.".to_owned())?;
    keymap::parse(&value)
}

/// Settings › Shortcuts in a window: its keys follow the profile's keymap,
/// and the page's actions land here.
impl crate::session::Controller {
    /// Keys, menus and the palette follow the stored keymap (the Settings
    /// page, an import, an extension or the agent may have changed it).
    /// Cheap when nothing changed: one revision compare.
    pub(crate) fn sync_keymap(&mut self) {
        let state = self.kernel.state();
        if self.keymap_revision == Some(state.revision) && !self.shortcuts_page.is_null() {
            return;
        }
        self.keymap_revision = Some(state.revision);
        let value = state.settings.get(SETTING).cloned();
        if value == self.keymap_seen && !self.shortcuts_page.is_null() {
            return;
        }
        let map = stored(&state);
        if value != self.keymap_seen {
            self.host.borrow_mut().set_keymap(keymap::effective(&map));
        }
        self.keymap_seen = value;
        self.rebuild_shortcuts_page(&map);
    }
    fn rebuild_shortcuts_page(&mut self, map: &Keymap) {
        let stamp = self.shortcuts_page["stamp"].as_u64().unwrap_or(0) + 1;
        let mut page = fact(
            map,
            self.shortcut_recording.as_deref(),
            self.shortcut_asked.as_deref(),
        );
        page["stamp"] = json!(stamp);
        self.shortcuts_page = page;
    }
    fn store_keymap(&mut self, map: Keymap, cx: &mut gpui::Context<Self>) {
        let Ok(value) = pm_model::json::parse(&keymap::to_value(&map).to_string()) else {
            return;
        };
        let pending = self
            .kernel
            .state_transaction(pm_kernel::state::StateTransaction {
                ops: vec![pm_kernel::state::StateOp::Setting {
                    key: SETTING.into(),
                    value: Some(value),
                }],
                ..Default::default()
            });
        cx.spawn(async move |this, cx| {
            let landed = pending.await;
            let _ = this.update(cx, |c, cx| {
                match landed {
                    Some(Ok(_)) => {}
                    Some(Err(error)) => c.message = format!("Shortcuts were not saved: {error:?}"),
                    None => c.message = "Shortcuts were not saved: the profile closed".into(),
                }
                c.sync_keymap();
                c.update_builtins(cx);
                cx.notify();
            });
        })
        .detach();
    }
    pub(crate) fn shortcuts_action(
        &mut self,
        action: &Value,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let map = stored(&self.kernel.state());
        let recording = self.shortcut_recording.take();
        self.shortcut_asked = None;
        self.host.borrow_mut().record_next_key(false);
        match act(map.clone(), action) {
            Ok(Act::Store(next)) => self.store_keymap(next, cx),
            Ok(Act::Record(id)) => {
                // Pressing Change… again on the row that is listening stops it.
                if recording.as_deref() != Some(id.as_str()) {
                    self.shortcut_recording = Some(id);
                    self.host.borrow_mut().record_next_key(true);
                }
            }
            Ok(Act::StopRecording) => {}
            Ok(Act::Import) => {
                let rx = crate::platform::paths(&self.host.borrow().window, false);
                cx.spawn_in(window, async move |this, cx| {
                    let Ok(Some(path)) = rx.recv().await else {
                        return;
                    };
                    let read = std::fs::read_to_string(&path)
                        .map_err(|e| e.to_string())
                        .and_then(|text| import(&text));
                    let _ = this.update(cx, |c, cx| match read {
                        Ok(map) => c.store_keymap(map, cx),
                        Err(error) => {
                            c.message = error;
                            cx.notify();
                        }
                    });
                })
                .detach();
            }
            Ok(Act::Export) => {
                let rx = crate::platform::paths_named(
                    &self.host.borrow().window,
                    true,
                    "Powermove Shortcuts.json",
                );
                let text = export(&map);
                cx.spawn_in(window, async move |this, cx| {
                    let Ok(Some(path)) = rx.recv().await else {
                        return;
                    };
                    if let Err(error) = std::fs::write(&path, text) {
                        let _ = this.update(cx, |c, cx| {
                            c.message = format!("The shortcuts file was not written: {error}");
                            cx.notify();
                        });
                    }
                })
                .detach();
            }
            Err(error) => self.message = error,
        }
        self.rebuild_shortcuts_page(&map);
        self.update_builtins(cx);
        cx.notify();
    }
    /// The key Settings › Shortcuts was waiting for.
    pub(crate) fn shortcut_recorded(&mut self, chord: &str, cx: &mut gpui::Context<Self>) {
        let map = stored(&self.kernel.state());
        let Some(id) = self.shortcut_recording.take() else {
            return;
        };
        let asked = self.shortcut_asked.take();
        match recorded(map.clone(), &id, chord, asked.as_deref()) {
            Recorded::Store(next) => self.store_keymap(next, cx),
            Recorded::Cancel => {}
            // Still recording: the row says who has the key; pressed again
            // it moves, Escape keeps the old one.
            Recorded::Ask { chord } => {
                self.shortcut_recording = Some(id);
                self.shortcut_asked = Some(chord);
                self.host.borrow_mut().record_next_key(true);
            }
            Recorded::Error(error) => {
                self.message = error;
                self.shortcut_recording = Some(id);
                self.host.borrow_mut().record_next_key(true);
            }
        }
        self.rebuild_shortcuts_page(&map);
        self.update_builtins(cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row<'a>(page: &'a Value, id: &str) -> &'a Value {
        page["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == id)
            .unwrap_or_else(|| panic!("no row {id}"))
    }

    #[test]
    fn every_command_has_a_name_and_its_keys_as_glyphs() {
        let page = fact(&Keymap::default(), None, None);
        let unnamed: Vec<&str> = page["rows"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| {
                let id = r["id"].as_str().unwrap();
                r["label"] == menu::split_palette_command(id).0
            })
            .filter_map(|r| r["id"].as_str())
            .collect();
        assert!(unnamed.is_empty(), "no names for {unnamed:?}");
        assert_eq!(row(&page, "rippleTrimIn")["keys"], json!(["⌥⇧["]));
        assert_eq!(row(&page, "fitView")["keys"], json!(["⇧\\"]));
        // The viewer's keys are listed too, under View.
        for (id, glyph) in [
            ("viewer.grid", "⌘'"),
            ("viewer.rulers", "⌘R"),
            ("viewer.zoom200", "⌘2"),
            ("viewer.showRed", "⌥1"),
            ("viewer.showAlphaOverlay", "⌥5"),
        ] {
            assert_eq!(row(&page, id)["keys"], json!([glyph]), "{id}");
            assert_eq!(row(&page, id)["menu"], "View", "{id}");
        }
        // One shortcut per key: the Control twin of Command-S is not listed.
        assert_eq!(row(&page, "save")["chords"], json!(["cmd+s"]));
        assert_eq!(row(&page, "save")["fixed"], json!(true));
        assert_eq!(page["preset"], "after-effects");
        assert_eq!(page["presets"].as_array().unwrap().len(), 4);
        let shared: Vec<&str> = page["conflicts"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|c| c["chord"].as_str())
            .collect();
        assert!(shared.contains(&"alt+shift+s") && shared.contains(&"cmd+alt+h"));
        let hold = page["conflicts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["chord"] == "cmd+alt+h")
            .unwrap();
        assert_eq!(
            hold["labels"],
            json!(["Toggle Hold Keyframe", "Hide Others"])
        );
    }

    #[test]
    fn a_recorded_key_assigns_escape_cancels_and_reset_returns_to_the_preset() {
        let map = Keymap::default();
        assert_eq!(recorded(map.clone(), "rippleTrimIn", "escape", None), Recorded::Cancel);
        // Q is the Shape tool's: it asks, and pressed again it moves here.
        assert!(matches!(
            recorded(map.clone(), "rippleTrimIn", "q", None),
            Recorded::Ask { .. }
        ));
        let Recorded::Store(map) = recorded(map, "rippleTrimIn", "q", Some("q")) else {
            panic!()
        };
        let page = fact(&map, None, None);
        assert_eq!(row(&page, "rippleTrimIn")["chords"], json!(["q"]));
        assert_eq!(row(&page, "rippleTrimIn")["changed"], json!(true));
        assert_eq!(row(&page, "toolShape")["chords"], json!(["shift+q"]));
        let Ok(Act::Store(map)) = act(map, &json!({"op":"reset","id":"rippleTrimIn"})) else {
            panic!()
        };
        // Ripple Trim In is back to its preset key; the Shape tool keeps the
        // change the move made to it (it lost Q).
        assert!(!map.bindings.iter().any(|a| a.command == "rippleTrimIn"));
        assert_eq!(map.bindings.len(), 1);
        // The menu bar's keys stay its own.
        assert!(matches!(
            recorded(map.clone(), "save", "cmd+shift+x", None),
            Recorded::Error(_)
        ));
        let Ok(Act::Store(map)) = act(map, &json!({"op":"clear","id":"slipLayers\u{1f}[-1]"}))
        else {
            panic!()
        };
        let page = fact(&map, Some("play"), None);
        assert_eq!(row(&page, "slipLayers\u{1f}[-1]")["chords"], json!([]));
        assert_eq!(row(&page, "play")["recording"], json!(true));
    }

    #[test]
    fn a_key_another_command_has_asks_first_and_moves_when_pressed_again() {
        let map = Keymap::default();
        let page = fact(&map, None, None);
        let taken = row(&page, "play")["chords"][0].as_str().unwrap().to_owned();
        let play = row(&page, "play")["label"].clone();
        // The first press asks; the row still records and says who has it.
        assert_eq!(
            recorded(map.clone(), "rippleTrimIn", &taken, None),
            Recorded::Ask { chord: taken.clone() }
        );
        let page = fact(&map, Some("rippleTrimIn"), Some(&taken));
        assert_eq!(row(&page, "rippleTrimIn")["asked"]["other"], play);
        assert_eq!(row(&page, "rippleTrimIn")["asked"]["menu"], json!(false));
        assert!(row(&page, "play")["asked"].is_null());
        // Escape keeps both as they were.
        assert_eq!(
            recorded(map.clone(), "rippleTrimIn", "escape", Some(&taken)),
            Recorded::Cancel
        );
        // Pressed again, the key moves here and leaves the other command.
        let Recorded::Store(next) = recorded(map.clone(), "rippleTrimIn", &taken, Some(&taken))
        else {
            panic!()
        };
        let page = fact(&next, None, None);
        assert_eq!(row(&page, "rippleTrimIn")["chords"], json!([taken]));
        assert!(!row(&page, "play")["chords"].as_array().unwrap().contains(&json!(taken)));
        // A key the menu bar owns is never moved, however often it is pressed.
        assert!(matches!(
            recorded(map.clone(), "rippleTrimIn", "cmd+s", Some("cmd+s")),
            Recorded::Ask { .. }
        ));
        let page = fact(&map, Some("rippleTrimIn"), Some("cmd+s"));
        assert_eq!(row(&page, "rippleTrimIn")["asked"]["menu"], json!(true));
        // Delete takes the row's key away.
        let Recorded::Store(cleared) = recorded(map, "play", "backspace", None) else {
            panic!()
        };
        assert_eq!(row(&fact(&cleared, None, None), "play")["chords"], json!([]));
    }

    #[test]
    fn presets_switch_and_a_file_round_trips() {
        let Ok(Act::Store(map)) = act(
            Keymap::default(),
            &json!({"op":"preset","value":"final-cut-pro"}),
        ) else {
            panic!()
        };
        let map = assign(map, "extendEdit", vec!["e".into()]).unwrap();
        let text = export(&map);
        assert!(text.contains("powermove-shortcuts"));
        assert_eq!(import(&text).unwrap(), map);
        assert!(import("{}").is_ok_and(|m| m == Keymap::default()));
        assert!(import("not json").is_err());
        assert!(act(map.clone(), &json!({"op":"preset","value":"avid"})).is_err());
        let Ok(Act::Store(set)) = act(
            Keymap::default(),
            &json!({"op":"set","value": keymap::to_value(&map)}),
        ) else {
            panic!()
        };
        assert_eq!(set, map);
        assert!(act(map, &json!({"op":"set","value":{"preset":"avid"}})).is_err());
    }

    /// The keymap the page stores is what keys resolve against and what the
    /// menus show (the palette reads the same table).
    #[test]
    fn a_stored_keymap_drives_keys_and_menus() {
        let Ok(Act::Store(map)) = act(
            Keymap::default(),
            &json!({"op":"preset","value":"premiere-pro"}),
        ) else {
            panic!()
        };
        // As the profile keeps it, and read back.
        let stored = pm_model::json::parse(&keymap::to_value(&map).to_string()).unwrap();
        let back: Value =
            serde_json::from_str(&pm_model::json::to_canonical_string(&stored)).unwrap();
        assert_eq!(keymap::parse(&back).unwrap(), map);
        keymap::set_current(keymap::effective(&map));
        let dispatch = crate::layer_commands::commands_tests::dispatch;
        assert_eq!(dispatch("q").0, "rippleTrimIn");
        assert_eq!(dispatch("i").0, "workIn");
        let menus = menu::menus();
        let layer = &menus.iter().find(|(title, _)| *title == "Layer").unwrap().1;
        let chord = |command: &str| {
            layer
                .iter()
                .find(|i| i.command == command)
                .map(|i| i.chord.clone())
        };
        assert_eq!(chord("rippleTrimIn").as_deref(), Some("q"));
        keymap::set_current(keymap::effective(&Keymap::default()));
        assert_eq!(dispatch("alt+shift+[").0, "rippleTrimIn");
    }

    /// Edit ▸ Keyboard Shortcuts… opens the page; each preset gives it its
    /// editor's key (the palette shows it).
    #[test]
    fn keyboard_shortcuts_is_in_the_edit_menu_on_each_editors_key() {
        let menus = menu::menus();
        let edit = &menus.iter().find(|(title, _)| *title == "Edit").unwrap().1;
        assert!(edit.iter().any(|i| i.command == "keyboardShortcuts"));
        for (preset, chord) in [
            ("after-effects", "cmd+alt+'"),
            ("premiere-pro", "cmd+alt+k"),
            ("final-cut-pro", "cmd+alt+k"),
            ("davinci-resolve", "cmd+alt+k"),
        ] {
            let table = keymap::effective(&Keymap {
                preset: preset.into(),
                bindings: vec![],
            });
            let keys: Vec<_> = table
                .iter()
                .filter(|b| b.command == "keyboardShortcuts")
                .map(|b| b.chord.as_str())
                .collect();
            assert_eq!(keys, [chord], "{preset}");
        }
    }
}
