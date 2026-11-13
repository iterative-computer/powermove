//! The keys a person works with: the shipped table, a preset over it (After
//! Effects, the default, Premiere Pro, Final Cut Pro or DaVinci Resolve), and
//! the person's own changes over that.
//!
//! A preset and a person's keymap are the same plain data, so an extension
//! or the agent can write either:
//! `{"preset": "premiere-pro", "bindings": [{"command": "rippleTrimIn",
//! "args": [], "keys": ["q"]}]}`. An entry gives a command (with its
//! arguments) exactly the keys listed, none when the list is empty, and takes
//! each of those keys from whatever had it, as the other editors' shortcut
//! editors do.
use crate::keyboard::{Binding, shipped};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cell::RefCell;

/// One command's keys.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Assign {
    pub command: String,
    #[serde(default)]
    pub args: Vec<Value>,
    pub keys: Vec<String>,
}
/// A preset's id and the person's changes over it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Keymap {
    #[serde(default = "default_preset")]
    pub preset: String,
    #[serde(default)]
    pub bindings: Vec<Assign>,
}
fn default_preset() -> String {
    PRESETS[0].0.into()
}
impl Default for Keymap {
    fn default() -> Self {
        Self {
            preset: default_preset(),
            bindings: vec![],
        }
    }
}
/// Every preset, the default first: id and name.
pub const PRESETS: [(&str, &str); 4] = [
    ("after-effects", "After Effects"),
    ("premiere-pro", "Premiere Pro"),
    ("final-cut-pro", "Final Cut Pro"),
    ("davinci-resolve", "DaVinci Resolve"),
];
/// A preset's entries over the shipped table.
pub fn preset(id: &str) -> Option<Vec<Assign>> {
    let text = match id {
        "after-effects" => include_str!("../data/presets/after-effects.json"),
        "premiere-pro" => include_str!("../data/presets/premiere-pro.json"),
        "final-cut-pro" => include_str!("../data/presets/final-cut-pro.json"),
        "davinci-resolve" => include_str!("../data/presets/davinci-resolve.json"),
        _ => return None,
    };
    let map: Keymap = serde_json::from_str(text).expect("shipped preset");
    Some(map.bindings)
}
/// A keymap as stored or imported: a known preset, commands named, every key
/// a valid chord (written the way `normalize` reads them).
pub fn parse(value: &Value) -> Result<Keymap, String> {
    let mut map: Keymap = serde_json::from_value(value.clone())
        .map_err(|_| "This is not a Powermove keyboard shortcuts file.".to_owned())?;
    if !PRESETS.iter().any(|(id, _)| *id == map.preset) {
        return Err(format!("Unknown shortcut preset “{}”.", map.preset));
    }
    for assign in &mut map.bindings {
        if assign.command.trim().is_empty() {
            return Err("A shortcut names no command.".into());
        }
        let mut keys = vec![];
        for key in &assign.keys {
            let chord = normalize(key).ok_or_else(|| format!("“{key}” is not a shortcut."))?;
            if !keys.contains(&chord) {
                keys.push(chord);
            }
        }
        assign.keys = keys;
    }
    Ok(map)
}
/// The stored form (what Export writes and the agent reads back).
pub fn to_value(map: &Keymap) -> Value {
    serde_json::to_value(map).expect("plain data")
}
/// A chord as the table writes it (`cmd+ctrl+alt+shift+key`), from that form,
/// from glyphs (`⌥⇧[`) or from words (`Option-Shift-[`, `opt+shift+[`).
pub fn normalize(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let mut mods = [false; 4];
    let mut rest = text;
    // Glyph modifiers lead (⌃⌥⇧⌘ in any order).
    loop {
        let Some(c) = rest.chars().next() else { break };
        let at = match c {
            '⌘' => 0,
            '⌃' => 1,
            '⌥' => 2,
            '⇧' => 3,
            _ => break,
        };
        mods[at] = true;
        rest = &rest[c.len_utf8()..];
    }
    // Word modifiers, each followed by + or - (a trailing + or - is the key).
    loop {
        let lower = rest.to_ascii_lowercase();
        let found = [
            ("command", 0),
            ("cmd", 0),
            ("control", 1),
            ("ctrl", 1),
            ("option", 2),
            ("opt", 2),
            ("alt", 2),
            ("shift", 3),
        ]
        .into_iter()
        .find(|(word, _)| {
            lower.starts_with(word)
                && matches!(lower.as_bytes().get(word.len()), Some(b'+' | b'-'))
                && lower.len() > word.len() + 1
        });
        let Some((word, at)) = found else { break };
        mods[at] = true;
        rest = &rest[word.len() + 1..];
    }
    let key = key_name(rest)?;
    let mut chord = String::new();
    for (on, name) in mods.iter().zip(["cmd+", "ctrl+", "alt+", "shift+"]) {
        if *on {
            chord.push_str(name);
        }
    }
    chord.push_str(&key);
    Some(chord)
}
fn key_name(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let named = match lower.as_str() {
        "return" | "enter" | "↩" | "⏎" => "enter",
        "tab" | "⇥" => "tab",
        "space" | "␣" => "space",
        "backspace" | "⌫" => "backspace",
        "delete" | "forward delete" | "⌦" => "delete",
        "escape" | "esc" | "⎋" => "escape",
        "up" | "↑" => "up",
        "down" | "↓" => "down",
        "left" | "←" => "left",
        "right" | "→" => "right",
        "home" | "↖" => "home",
        "end" | "↘" => "end",
        "pageup" | "page up" | "⇞" => "pageup",
        "pagedown" | "page down" | "⇟" => "pagedown",
        other => {
            if let Some(n) = other.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()) {
                return (1..=12).contains(&n).then(|| format!("f{n}"));
            }
            let mut chars = other.chars();
            let c = chars.next()?;
            return (chars.next().is_none() && !c.is_whitespace()).then(|| c.to_string());
        }
    };
    Some(named.into())
}
/// The table with a preset and a person's changes applied.
pub fn effective(map: &Keymap) -> Vec<Binding> {
    let mut table = shipped();
    apply(
        &mut table,
        &preset(&map.preset).unwrap_or_default(),
        "preset",
    );
    apply(&mut table, &map.bindings, "user");
    table
}
fn apply(table: &mut Vec<Binding>, entries: &[Assign], tag: &str) {
    for entry in entries {
        let same = |b: &Binding| b.command == entry.command && b.args == entry.args;
        // The command's own first row keeps how it behaves (its priority,
        // whether it repeats while held, whether it works in a text field).
        let template = table.iter().find(|b| same(b)).cloned();
        table.retain(|b| !same(b) && !entry.keys.contains(&b.chord));
        for (i, chord) in entry.keys.iter().enumerate() {
            table.push(Binding {
                id: format!(
                    "{tag}.{}{}.{i}",
                    entry.command,
                    serde_json::Value::from(entry.args.clone())
                ),
                chord: chord.clone(),
                command: entry.command.clone(),
                args: entry.args.clone(),
                priority: template.as_ref().map_or(100, |b| b.priority),
                in_fields: template.as_ref().is_some_and(|b| b.in_fields),
                repeat: template.as_ref().is_some_and(|b| b.repeat),
                loose: false,
            });
        }
    }
}
/// Two or more commands on one key, in the order the key tries them: the
/// first that applies runs (Option-Shift-S adds a Scale key with a layer
/// selected and turns Audio Skimming on otherwise). `menu` is a key the menu
/// bar answers after the table declines it (Option-Command-H, Hide Others).
#[derive(Clone, Debug, PartialEq)]
pub struct Clash {
    pub chord: String,
    pub commands: Vec<(String, Vec<Value>)>,
    pub menu: Option<String>,
}
pub fn conflicts(table: &[Binding]) -> Vec<Clash> {
    let menu_keys = menu_bar_keys();
    let mut chords: Vec<&str> = table.iter().map(|b| b.chord.as_str()).collect();
    chords.sort_unstable();
    chords.dedup();
    let mut out = vec![];
    for chord in chords {
        let mut rows: Vec<&Binding> = table.iter().filter(|b| b.chord == chord).collect();
        rows.sort_by_key(|b| b.priority);
        let mut commands: Vec<(String, Vec<Value>)> = vec![];
        for b in rows {
            let pair = (b.command.clone(), b.args.clone());
            if !commands.contains(&pair) {
                commands.push(pair);
            }
        }
        let menu = menu_keys
            .iter()
            .find(|(c, command, _)| c == chord && !commands.iter().any(|(own, _)| own == command))
            .map(|(_, _, label)| label.clone());
        if commands.len() > 1 || (menu.is_some() && !commands.is_empty()) {
            out.push(Clash {
                chord: chord.into(),
                commands,
                menu,
            });
        }
    }
    out
}
/// Keys the menu bar itself owns (AppKit roles and its key equivalents):
/// chord, command and label.
fn menu_bar_keys() -> Vec<(String, String, String)> {
    crate::menu::menus()
        .into_iter()
        .flat_map(|(_, items)| items)
        .filter(|i| i.role.is_some() || crate::menu::is_equivalent(&i.command))
        .filter_map(|i| Some((from_accelerator(&i.chord)?, i.command, i.label)))
        .collect()
}
/// `Command+Alt+H` (an Electron accelerator) as a table chord.
fn from_accelerator(text: &str) -> Option<String> {
    if text.is_empty() {
        return None;
    }
    // A literal plus is the key after the last separator (`Command+Shift++`).
    let (mods, key) = match text.strip_suffix("++") {
        Some(mods) => (mods, "+"),
        None => text.rsplit_once('+').unwrap_or(("", text)),
    };
    let words: Vec<String> = mods
        .split('+')
        .filter(|w| !w.is_empty())
        .chain(std::iter::once(key))
        .map(|w| match w.to_ascii_lowercase().as_str() {
            "command" | "cmdorctrl" | "commandorcontrol" | "cmd" => "Command".into(),
            "control" | "ctrl" => "Control".into(),
            "alt" | "option" => "Option".into(),
            "shift" => "Shift".into(),
            other => other.to_owned(),
        })
        .collect();
    normalize(&words.join("-"))
}

thread_local! {
    static CURRENT: RefCell<Option<Vec<Binding>>> = const { RefCell::new(None) };
}
/// The table in use on this thread (the default keymap until one is set).
pub fn current() -> Vec<Binding> {
    CURRENT.with(|c| {
        c.borrow_mut()
            .get_or_insert_with(|| effective(&Keymap::default()))
            .clone()
    })
}
/// Makes `table` the one menus, the palette and new resolvers read.
pub fn set_current(table: Vec<Binding>) {
    CURRENT.with(|c| *c.borrow_mut() = Some(table));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn keys(table: &[Binding], command: &str, args: Value) -> Vec<String> {
        let args = args.as_array().cloned().unwrap();
        table
            .iter()
            .filter(|b| b.command == command && b.args == args)
            .map(|b| b.chord.clone())
            .collect()
    }

    #[test]
    fn chords_read_from_the_table_form_glyphs_and_words() {
        for (text, chord) in [
            ("alt+shift+[", "alt+shift+["),
            ("⌥⇧[", "alt+shift+["),
            ("⇧⌥[", "alt+shift+["),
            ("Option-Shift-[", "alt+shift+["),
            ("opt+shift+[", "alt+shift+["),
            ("Command-Option-T", "cmd+alt+t"),
            ("⌘-", "cmd+-"),
            ("cmd+-", "cmd+-"),
            ("Control-J", "ctrl+j"),
            ("Q", "q"),
            ("Space", "space"),
            ("F9", "f9"),
            ("⇧F3", "shift+f3"),
            ("⌥↑", "alt+up"),
        ] {
            assert_eq!(normalize(text).as_deref(), Some(chord), "{text}");
        }
        for text in ["", "cmd+", "Option-", "f13", "ab"] {
            assert_eq!(normalize(text), None, "{text}");
        }
    }

    #[test]
    fn the_default_is_after_effects_with_keys_for_the_commands_that_had_none() {
        let table = effective(&Keymap::default());
        assert_eq!(keys(&table, "fitView", json!([])), ["shift+\\"]);
        assert_eq!(keys(&table, "prevEdge", json!([])), ["alt+up"]);
        assert_eq!(keys(&table, "nextEdge", json!([])), ["alt+down"]);
        assert_eq!(
            keys(&table, "prevSelectedEvent", json!([])),
            ["alt+shift+up"]
        );
        assert_eq!(
            keys(&table, "nextSelectedEvent", json!([])),
            ["alt+shift+down"]
        );
        // Everything else is the shipped table.
        assert_eq!(table.len(), shipped().len() + 5);
        assert_eq!(keys(&table, "workIn", json!([])), ["b", "shift+b"]);
    }

    #[test]
    fn each_preset_names_real_commands_and_takes_the_keys_it_gives() {
        let shipped = shipped();
        let known = |command: &str| {
            shipped.iter().any(|b| b.command == command)
                || crate::menu::palette_rows()
                    .iter()
                    .any(|(_, c, _)| crate::menu::split_palette_command(c).0 == command)
        };
        for (id, _) in PRESETS {
            let entries = preset(id).unwrap();
            for e in &entries {
                assert!(known(&e.command), "{id}: {} is not a command", e.command);
                for key in &e.keys {
                    assert_eq!(normalize(key).as_deref(), Some(key.as_str()), "{id}: {key}");
                }
            }
            // Every preset covers the commands that had no key.
            let table = effective(&Keymap {
                preset: id.into(),
                bindings: vec![],
            });
            for command in [
                "fitView",
                "prevEdge",
                "nextEdge",
                "prevSelectedEvent",
                "nextSelectedEvent",
            ] {
                assert!(
                    !keys(&table, command, json!([])).is_empty(),
                    "{id}: {command}"
                );
            }
        }
        let premiere = effective(&Keymap {
            preset: "premiere-pro".into(),
            bindings: vec![],
        });
        assert_eq!(keys(&premiere, "workIn", json!([])), ["i"]);
        assert_eq!(keys(&premiere, "rippleTrimIn", json!([])), ["q"]);
        assert_eq!(keys(&premiere, "nextFrame", json!([])), ["right"]);
        // I marks In: Go to Layer In Point gave its key up, and Q is no longer
        // the Shape tool's.
        assert!(keys(&premiere, "gotoLayerIn", json!([])).is_empty());
        assert_eq!(keys(&premiere, "toolShape", json!([])), ["shift+q"]);
        // J is the shuttle alone, keeping its priority.
        let j: Vec<_> = premiere.iter().filter(|b| b.chord == "j").collect();
        assert_eq!(j.len(), 1);
        assert_eq!(
            (j[0].command.as_str(), j[0].priority),
            ("shuttleBackward", 90)
        );
        // The palette keeps Command-K in every preset.
        assert!(
            premiere
                .iter()
                .any(|b| b.chord == "cmd+k" && b.command == "palette")
        );
    }

    #[test]
    fn a_persons_change_takes_its_key_and_an_empty_list_unbinds() {
        let map = parse(&json!({"preset": "after-effects", "bindings": [
            {"command": "rippleTrimIn", "keys": ["Q"]},
            {"command": "trimOut", "args": [], "keys": []},
        ]}))
        .unwrap();
        let table = effective(&map);
        assert_eq!(keys(&table, "rippleTrimIn", json!([])), ["q"]);
        assert!(keys(&table, "toolShape", json!([])) == ["shift+q"]);
        assert!(keys(&table, "trimOut", json!([])).is_empty());
        // It round-trips as plain data.
        assert_eq!(parse(&to_value(&map)).unwrap(), map);
        assert!(parse(&json!({"preset": "avid"})).is_err());
        assert!(parse(&json!({"bindings": [{"command": "play", "keys": ["cmd+"]}]})).is_err());
        assert!(parse(&json!("not a keymap")).is_err());
    }

    #[test]
    fn conflicts_list_each_key_with_its_commands_in_the_order_they_are_tried() {
        let table = effective(&Keymap::default());
        let clashes = conflicts(&table);
        let at = |chord: &str| clashes.iter().find(|c| c.chord == chord).cloned();
        // Option-Shift-S: the Scale key with a layer selected, else Audio Skimming.
        let s = at("alt+shift+s").expect("⌥⇧S is shared");
        assert_eq!(s.commands[0].0, "timeline.keyProperty:s");
        assert_eq!(s.commands[1].0, "toggleAudioSkimming");
        // Option-Command-H: Toggle Hold Keyframe, then the menu bar's Hide Others.
        let h = at("cmd+alt+h").expect("⌥⌘H is shared");
        assert_eq!(h.commands, [("timeline.toggleHold".to_owned(), vec![])]);
        assert_eq!(h.menu.as_deref(), Some("Hide Others"));
        // A Control alias of the same command is not a conflict.
        assert!(at("ctrl+s").is_none());
    }

    /// Shift with comma, period, backslash, semicolon or quote names the key
    /// (AppKit gives "<", ">", "|", ":" and a double quote): Command-Shift-Comma
    /// and Option-Shift-Period reach their commands.
    #[test]
    fn shifted_punctuation_keeps_its_key_name() {
        use crate::keyboard::physical_fallback;
        for (typed, code, key) in [
            ("<", 43, ","),
            (">", 47, "."),
            ("|", 42, "\\"),
            (":", 41, ";"),
            ("\"", 39, "'"),
            ("{", 33, "["),
        ] {
            assert_eq!(physical_fallback(typed.into(), code, true), key, "{typed}");
        }
        // Shift-= stays "+" (Command-Shift-+ zooms in).
        assert_eq!(physical_fallback("+".into(), 24, true), "+");
    }
}
