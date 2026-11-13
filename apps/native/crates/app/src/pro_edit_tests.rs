//! Editing without the mouse: ripple trim, extend edit, slip, slide, lift,
//! extract, time remap, freeze frame and reverse, each from its key or menu
//! row into a real session, one Undo step, saved and opened again.
use crate::layer_commands::{command_plan, disabled};
use crate::parity_tests::{Desk, layer_of};
use pm_model::Value;
use serde_json::json;

const FRAME: f64 = 1. / 30.;

fn clip(id: &str, from: f64, dur: f64, trim: f64) -> serde_json::Value {
    json!({
        "id": id, "type": "video", "name": id, "from": from, "dur": dur, "on": true,
        "d": { "asset": "v1", "fit": "cover", "trim": trim, "speed": 1, "w": 64, "h": 36 },
        "p": { "opacity": { "v": 100, "kf": [{ "i": format!("k{id}"), "t": 1, "v": 50 }] } }
    })
}
fn shape(id: &str, from: f64, dur: f64) -> serde_json::Value {
    json!({
        "id": id, "type": "shape", "name": id, "from": from, "dur": dur, "on": true,
        "d": { "shape": "rect", "w": 100, "h": 100 }, "p": {}
    })
}
/// a [0,2] | b [2,5] | c [5,7] on one track, an overlay over all of it, and
/// a work area of 3 s to 4 s.
fn sequence() -> Desk {
    Desk::new(json!({
        "id": "desk", "name": "Synthetic", "compId": "c", "w": 640, "h": 360, "fps": 30,
        "dur": 10, "work": [3, 4], "bg": "#000000", "comps": {},
        "assets": { "v1": { "id": "v1", "kind": "video", "name": "clip.mov", "dur": 20 } },
        "layers": [shape("title", 0., 10.), clip("a", 0., 2., 0.), clip("b", 2., 3., 5.), shape("c", 5., 2.)]
    }))
}
fn spans(desk: &Desk) -> Vec<(String, f64, f64)> {
    desk.layers()
        .into_iter()
        .map(|l| (l.id, l.in_time, l.out_time))
        .collect()
}
fn span(desk: &Desk, id: &str) -> (f64, f64) {
    let l = desk.layers().into_iter().find(|l| l.id == id).unwrap();
    (l.in_time, l.out_time)
}
fn content(layer: &Value, key: &str) -> Value {
    layer
        .get("d")
        .and_then(|d| d.get(key))
        .cloned()
        .unwrap_or(Value::Null)
}
fn number(value: &Value) -> f64 {
    value
        .as_f64()
        .or_else(|| value.get("v").and_then(Value::as_f64))
        .unwrap()
}
fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}
fn label(desk: &Desk) -> Option<String> {
    desk.published().history.undo_label.clone()
}
/// Runs `step`, checks one Undo restores the document and Redo brings the
/// edit back.
fn one_step(desk: &Desk, step: impl Fn(&Desk)) {
    let before = desk.document();
    step(desk);
    let after = desk.document();
    assert_ne!(after, before, "the edit changed the project");
    desk.undo();
    assert_eq!(desk.document(), before, "one Undo step");
    desk.redo();
    assert_eq!(desk.document(), after, "Redo");
}

#[test]
fn ripple_trim_keys_take_the_part_before_or_after_the_playhead_and_close_up() {
    let desk = sequence();
    desk.select(&["b"]);
    one_step(&desk, |d| d.press("alt+shift+[", 3.));
    assert_eq!(label(&desk).as_deref(), Some("Ripple Trim"));
    assert_eq!(span(&desk, "b"), (2., 4.));
    assert_eq!(
        number(&content(&layer_of(&desk.published(), "b"), "trim")),
        6.
    );
    assert_eq!(span(&desk, "c"), (4., 6.), "c closes up");
    assert_eq!(span(&desk, "title"), (0., 10.), "the overlay stays");
    desk.undo();
    one_step(&desk, |d| d.press("alt+shift+]", 3.));
    assert_eq!(span(&desk, "b"), (2., 3.));
    assert_eq!(span(&desk, "c"), (3., 5.));
    let closed = spans(&desk);
    let again = desk.reopened();
    let reopened: Vec<_> =
        pm_kernel::model::list_layers(again.published.snapshot.root().unwrap(), None)
            .into_iter()
            .map(|l| (l.id, l.in_time, l.out_time))
            .collect();
    assert_eq!(reopened, closed, "saved and reopened");
}

#[test]
fn extend_edit_rolls_the_nearer_edit_to_the_playhead() {
    let desk = sequence();
    desk.select(&["b"]);
    one_step(&desk, |d| d.press("shift+x", 4.5));
    assert_eq!(label(&desk).as_deref(), Some("Extend Edit"));
    assert_eq!(span(&desk, "b"), (2., 4.5));
    assert_eq!(span(&desk, "c"), (4.5, 7.), "c takes the other side");
    desk.press("shift+x", 1.5);
    assert_eq!(span(&desk, "a"), (0., 1.5));
    assert_eq!(span(&desk, "b"), (1.5, 4.5));
    let b = layer_of(&desk.published(), "b");
    assert_eq!(number(&content(&b, "trim")), 4.5, "b's media stays put");
    let again = desk.reopened();
    let b = layer_of(&again.published, "b");
    assert_eq!(number(&content(&b, "trim")), 4.5, "saved and reopened");
}

#[test]
fn slip_and_slide_keys_move_by_frames() {
    let desk = sequence();
    desk.select(&["b"]);
    // Slip left: the content moves left, later source frames show.
    one_step(&desk, |d| d.press("cmd+alt+left", 0.));
    let trim = |d: &Desk| number(&content(&layer_of(&d.published(), "b"), "trim"));
    assert!(close(trim(&desk), 5. + FRAME));
    assert_eq!(span(&desk, "b"), (2., 5.), "the clip stays");
    desk.press("cmd+alt+shift+right", 0.);
    assert!(close(trim(&desk), 5. - 9. * FRAME));
    // Slide right a frame, then ten.
    one_step(&desk, |d| d.press("alt+.", 0.));
    assert_eq!(label(&desk).as_deref(), Some("Slide Layer"));
    let (from, to) = span(&desk, "b");
    assert!(close(from, 2. + FRAME) && close(to, 5. + FRAME));
    assert!(close(span(&desk, "a").1, 2. + FRAME));
    assert!(close(span(&desk, "c").0, 5. + FRAME) && close(span(&desk, "c").1, 7.));
    desk.press("alt+shift+,", 0.);
    assert!(close(span(&desk, "b").0, 2. - 9. * FRAME));
    let slid = span(&desk, "b");
    let again = desk.reopened();
    let b = layer_of(&again.published, "b");
    assert!(close(b.get("from").unwrap().as_f64().unwrap(), slid.0));
    assert!(
        close(number(&content(&b, "trim")), 5. - 9. * FRAME),
        "saved and reopened"
    );
    // The held keys repeat; a shape has no source to slip.
    assert!(
        pm_shell::keyboard::shipped()
            .iter()
            .filter(|b| ["slipLayers", "slideLayer"].contains(&b.command.as_str()))
            .all(|b| b.repeat)
    );
    desk.select(&["c"]);
    let published = desk.published();
    assert!(command_plan("slipLayers", &[json!(-1)], &published, 0.).is_none());
    assert!(disabled(Some(&published), &desk.layers(), &Default::default()).contains("slipLayers"));
}

#[test]
fn lift_and_extract_work_area_remove_the_range_from_the_selection_or_everything() {
    // Nothing selected: every layer.
    let desk = sequence();
    one_step(&desk, |d| d.run("liftWorkArea", &[], 0.));
    assert_eq!(label(&desk).as_deref(), Some("Lift Work Area"));
    let lifted = spans(&desk);
    assert_eq!(lifted.len(), 6, "title and b leave tails");
    assert!(lifted.contains(&("title".into(), 0., 3.)));
    assert!(
        lifted
            .iter()
            .any(|(id, from, to)| id != "title" && *from == 4. && *to == 10.)
    );
    assert_eq!(span(&desk, "c"), (5., 7.), "lift leaves the gap");
    desk.undo();
    one_step(&desk, |d| d.run("extractWorkArea", &[], 0.));
    assert_eq!(label(&desk).as_deref(), Some("Extract Work Area"));
    assert_eq!(span(&desk, "c"), (4., 6.), "extract closes it");
    assert_eq!(span(&desk, "b"), (2., 3.));
    let extracted = spans(&desk);
    let again = desk.reopened();
    let reopened: Vec<_> =
        pm_kernel::model::list_layers(again.published.snapshot.root().unwrap(), None)
            .into_iter()
            .map(|l| (l.id, l.in_time, l.out_time))
            .collect();
    assert_eq!(reopened, extracted, "saved and reopened");
    // With a selection only those layers change.
    let desk = sequence();
    desk.select(&["c"]);
    desk.run("extractWorkArea", &[], 0.);
    assert_eq!(span(&desk, "c"), (4., 6.));
    assert_eq!(span(&desk, "title"), (0., 10.));
    assert_eq!(span(&desk, "b"), (2., 5.));
}

#[test]
fn time_remap_freeze_and_reverse_keys_write_the_source_time_channel() {
    let desk = sequence();
    desk.select(&["b", "c"]);
    let keys = |d: &Desk| -> Vec<(f64, f64)> {
        content(&layer_of(&d.published(), "b"), "sourceTime")
            .get("kf")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .map(|k| {
                (
                    k.get("t").unwrap().as_f64().unwrap(),
                    k.get("v").unwrap().as_f64().unwrap(),
                )
            })
            .collect()
    };
    one_step(&desk, |d| d.press("cmd+alt+t", 0.));
    assert_eq!(label(&desk).as_deref(), Some("Enable Time Remapping"));
    let remap = keys(&desk);
    assert_eq!(remap[0], (0., 5.));
    assert!(close(remap[1].1, 8. - FRAME));
    desk.undo();
    one_step(&desk, |d| d.press("cmd+alt+r", 0.));
    assert_eq!(label(&desk).as_deref(), Some("Time-Reverse Layer"));
    let reverse = keys(&desk);
    assert!(close(reverse[0].1, 8. - FRAME) && reverse[1].1 == 5.);
    desk.undo();
    one_step(&desk, |d| d.press("alt+f", 3.));
    assert_eq!(label(&desk).as_deref(), Some("Freeze Frame"));
    assert_eq!(keys(&desk), [(1., 6.)]);
    let b = layer_of(&desk.published(), "b");
    assert_eq!(content(&b, "timeRemap").get("v"), Some(&Value::Bool(true)));
    let again = desk.reopened();
    let b = layer_of(&again.published, "b");
    let held = content(&b, "sourceTime")
        .get("kf")
        .and_then(Value::as_array)
        .unwrap()[0]
        .clone();
    assert_eq!(
        held.get("hold"),
        Some(&Value::Bool(true)),
        "saved and reopened"
    );
    // The shape in the selection is left alone; a shape alone cannot.
    assert!(content(&layer_of(&desk.published(), "c"), "sourceTime").is_null());
    desk.select(&["c"]);
    let published = desk.published();
    for name in ["timeRemapLayers", "freezeFrame", "reverseLayers"] {
        assert!(command_plan(name, &[], &published, 0.).is_none());
        assert!(disabled(Some(&published), &desk.layers(), &Default::default()).contains(name));
    }
}

#[test]
fn the_pro_edits_are_in_the_menus_with_their_keys_and_in_the_palette() {
    let menus = pm_shell::menu::menus();
    let row = |menu: &str, command: &str, args: serde_json::Value| {
        let (_, items) = menus.iter().find(|(title, _)| *title == menu).unwrap();
        let args = args.as_array().cloned().unwrap();
        items
            .iter()
            .find(|i| i.command == command && i.args == args)
            .unwrap_or_else(|| panic!("{menu} has no {command}"))
            .chord
            .clone()
    };
    for (command, args, chord) in [
        ("rippleTrimIn", json!([]), "alt+shift+["),
        ("rippleTrimOut", json!([]), "alt+shift+]"),
        ("extendEdit", json!([]), "shift+x"),
        ("slipLayers", json!([-1]), "cmd+alt+left"),
        ("slipLayers", json!([1]), "cmd+alt+right"),
        ("slideLayer", json!([-1]), "alt+,"),
        ("slideLayer", json!([1]), "alt+."),
        ("timeRemapLayers", json!([]), "cmd+alt+t"),
        ("reverseLayers", json!([]), "cmd+alt+r"),
        ("freezeFrame", json!([]), "alt+f"),
    ] {
        assert_eq!(row("Layer", command, args), chord, "{command}");
    }
    // After Effects keeps Lift and Extract in Edit, without keys.
    assert_eq!(row("Edit", "liftWorkArea", json!([])), "");
    assert_eq!(row("Edit", "extractWorkArea", json!([])), "");
    for tool in ["selection", "ripple", "roll", "slip", "slide"] {
        assert_eq!(row("View", &format!("timeline.tool:{tool}"), json!([])), "");
    }
    let palette = pm_shell::menu::palette_rows();
    for command in [
        pm_shell::menu::palette_command("slipLayers", &[json!(-10)]),
        pm_shell::menu::palette_command("slideLayer", &[json!(10)]),
        "liftWorkArea".into(),
        "timeline.tool:ripple".into(),
    ] {
        assert!(
            palette.iter().any(|(_, c, _)| *c == command),
            "{command} is in the palette"
        );
    }
}
