//! Keymap and menu commands: real chords through the shipped resolver, into the
//! same route the session runs, into the real kernel. Undo reverts each edit.
use super::*;
use crate::timeline_tests::{Stage, stage, stage_project};
use crate::v4_view_tests::{PROJECT, settle};
use gpui::TestAppContext;
use pm_kernel::{Call, Caller, HistoryCall, KernelHandle, Request, SelectionCall};
use pm_shell::keyboard::{Effect, Key, Resolver};
use std::sync::Arc;

fn published(kernel: &KernelHandle) -> Arc<Published> {
    kernel.published(PROJECT).expect("open project")
}
fn call(kernel: &KernelHandle, call: impl Into<Call>) {
    kernel
        .call(PROJECT, Request::new(Caller::native(), call.into()))
        .wait()
        .unwrap()
        .unwrap();
}
/// The publication reaches the surface as `Controller::refresh` delivers it.
fn delivered(s: &mut Stage, cx: &mut TestAppContext) {
    s.idle(cx);
    s.surface.update(&mut s.vcx, |t, cx| t.published(cx));
    s.idle(cx);
}
fn select(s: &mut Stage, cx: &mut TestAppContext, ids: &[&str]) {
    let ids = ids.iter().map(|i| (*i).to_owned()).collect();
    call(&s.rig.kernel, SelectionCall::Layers { ids, add: false });
    delivered(s, cx);
}
fn undo(s: &mut Stage, cx: &mut TestAppContext) {
    call(&s.rig.kernel, Call::History(HistoryCall::Undo));
    delivered(s, cx);
}
/// The first routed command a chord reaches; earlier bindings decline, as
/// `blurField` does outside a text field.
pub(crate) fn dispatch(chord: &str) -> (String, Vec<serde_json::Value>) {
    let mut resolver = Resolver::new();
    let mut effects = resolver.submit(Key {
        token: 1,
        chord: chord.into(),
        field: false,
        repeat: false,
        up: false,
        compat: false,
    });
    loop {
        match effects.into_iter().next() {
            Some(Effect::Dispatch { command, args, .. }) if route(&command, &args).is_some() => {
                return (command, args);
            }
            Some(Effect::Dispatch { .. }) => effects = resolver.resolve(1, false),
            other => panic!("{chord} resolved to {other:?}"),
        }
    }
}
/// `Controller::command` for routed commands, minus its AppKit window.
fn run(s: &mut Stage, cx: &mut TestAppContext, command: &str, args: &[serde_json::Value]) {
    let time = *s.recorder.time.borrow();
    match route(command, args).unwrap_or_else(|| panic!("{command} has no route")) {
        Route::Plan(name) => {
            let snapshot = published(&s.rig.kernel);
            let plan = command_plan(name, args, &snapshot, time)
                .unwrap_or_else(|| panic!("{name} had nothing to do"));
            if let Some(call_) = transaction(&plan, Some(snapshot.revision())) {
                call(&s.rig.kernel, call_);
            }
            if let Some(ids) = plan.select {
                call(&s.rig.kernel, SelectionCall::Layers { ids, add: false });
            }
            delivered(s, cx);
        }
        Route::Timeline { frames, shift } => {
            s.surface.update(&mut s.vcx, |t, cx| {
                t.command(command, time, frames, shift, cx)
            });
        }
        other => panic!("{other:?} is run by the session"),
    }
    delivered(s, cx);
}
fn press(s: &mut Stage, cx: &mut TestAppContext, chord: &str) {
    let (command, args) = dispatch(chord);
    run(s, cx, &command, &args);
}
fn layers(s: &Stage) -> Vec<pm_kernel::LayerInfo> {
    let p = published(&s.rig.kernel);
    pm_kernel::model::list_layers(p.snapshot.root().unwrap(), None)
}
fn layer(s: &Stage, id: &str) -> pm_kernel::LayerInfo {
    layers(s).into_iter().find(|l| l.id == id).unwrap()
}
fn selected(s: &Stage) -> Vec<String> {
    published(&s.rig.kernel).selection.layers().to_vec()
}

/// Whether `source` has a match arm for the quoted `name`: the name in a
/// pattern, followed by other alternatives or a guard, then `=>`.
fn match_arm(source: &str, name: &str) -> bool {
    let quoted = format!("\"{name}\"");
    source.match_indices(&quoted).any(|(at, _)| {
        let mut rest = source[at + quoted.len()..].trim_start();
        loop {
            if rest.starts_with("=>") || rest.starts_with("if ") {
                return true;
            }
            if let Some(next) = rest.strip_prefix('|') {
                rest = next.trim_start();
            } else if let Some(next) = rest.strip_prefix('"') {
                let Some(end) = next.find('"') else {
                    return false;
                };
                rest = next[end + 1..].trim_start();
            } else {
                return false;
            }
        }
    })
}
#[gpui::test]
fn every_keymap_and_menu_command_has_an_owner(_: &mut TestAppContext) {
    // Session and shell arms, by the command names their matches handle.
    let session = include_str!("session.rs");
    let view = include_str!("../../shell/src/view.rs");
    let native = include_str!("../../shell/src/native.rs");
    // `Controller::command` falls through to the surface routes, which hand
    // keyframe dialogs to `keyframe_sheets::Kind::of`.
    let surfaces = include_str!("surface_routes.rs");
    let keyframes = include_str!("keyframe_sheets.rs");
    let known = |name: &str| {
        route(name, &[]).is_some()
            || UNAVAILABLE.contains(&name)
            || [session, view, native, surfaces, keyframes]
                .iter()
                .any(|s| match_arm(s, name))
            || name.starts_with("tool")
    };
    let mut names: Vec<String> = pm_shell::keyboard::shipped()
        .into_iter()
        .map(|b| b.command)
        .collect();
    for (_, items) in pm_shell::menu::menus() {
        names.extend(
            items
                .into_iter()
                .filter(|i| i.role.is_none())
                .map(|i| i.command),
        );
    }
    // Palette rows that have no row in these menus.
    names.extend(pm_shell::menu::extras().into_iter().map(|(_, i)| i.command));
    names.retain(|n| !n.is_empty());
    assert!(match_arm(r#""a" | "zoomIn" => {}"#, "zoomIn"));
    assert!(!match_arm(r#"let label = "zoomIn";"#, "zoomIn"));
    let missing: Vec<_> = names.iter().filter(|n| !known(n)).collect();
    assert!(missing.is_empty(), "{missing:?}");
}
#[gpui::test]
fn a_command_that_cannot_run_is_disabled_and_lives_with_a_selection(cx: &mut TestAppContext) {
    let mut s = stage(cx, 2);
    let off = |s: &Stage| {
        let p = published(&s.rig.kernel);
        disabled(Some(&p), &layers(s), &Clipboard::Empty)
    };
    for name in ["trimIn", "split", "duplicate", "groupLayers", "easyEase"] {
        assert!(off(&s).contains(name), "{name}");
    }
    assert!(off(&s).contains("text.editSelected"));
    // Every viewer tool has a gesture: the toolbar, menu, palette and keys run them.
    for tool in ["toolPen", "toolText", "toolShape"] {
        assert!(!off(&s).contains(tool), "{tool}");
    }
    assert!(off(&s).contains("pasteLayers"));
    assert!(!off(&s).contains("newSolid"));
    select(&mut s, cx, &["l0"]);
    for name in ["trimIn", "split", "duplicate", "groupLayers"] {
        assert!(!off(&s).contains(name), "{name}");
    }
    assert!(off(&s).contains("ungroupLayers"));
    assert!(off(&s).contains("easyEase"));
    let none = disabled(None, &[], &Clipboard::Empty);
    assert!(none.contains("newSolid"));
    assert!(none.contains("workIn"));
}
#[gpui::test]
fn bracket_keys_move_and_trim_at_the_playhead_and_undo_restores(cx: &mut TestAppContext) {
    let mut s = stage(cx, 2);
    select(&mut s, cx, &["l0"]);
    *s.recorder.time.borrow_mut() = 1.;
    let before = layer(&s, "l0");
    press(&mut s, cx, "alt+[");
    assert_eq!(layer(&s, "l0").in_time, 1.);
    assert_eq!(layer(&s, "l0").out_time, before.out_time);
    undo(&mut s, cx);
    assert_eq!(layer(&s, "l0").in_time, before.in_time);
    press(&mut s, cx, "[");
    assert_eq!(layer(&s, "l0").in_time, 1.);
    assert_eq!(layer(&s, "l0").out_time, before.out_time + 1.);
    undo(&mut s, cx);
    assert_eq!(layer(&s, "l0").out_time, before.out_time);
}
#[gpui::test]
fn split_duplicate_and_delete_change_the_stack_and_undo_restores(cx: &mut TestAppContext) {
    let mut s = stage(cx, 2);
    select(&mut s, cx, &["l0"]);
    *s.recorder.time.borrow_mut() = 1.;
    for (chord, grown) in [("cmd+b", 3), ("cmd+d", 3), ("backspace", 1)] {
        press(&mut s, cx, chord);
        assert_eq!(layers(&s).len(), grown, "{chord}");
        undo(&mut s, cx);
        assert_eq!(layers(&s).len(), 2, "{chord} undo");
        select(&mut s, cx, &["l0"]);
    }
}
#[gpui::test]
fn new_layer_chords_add_and_select_and_undo_removes(cx: &mut TestAppContext) {
    let mut s = stage(cx, 1);
    for (chord, kind) in [
        ("cmd+y", "solid"),
        ("cmd+shift+y", "shape"),
        ("cmd+alt+y", "adjustment"),
        ("cmd+alt+shift+y", "null"),
    ] {
        press(&mut s, cx, chord);
        let id = selected(&s).pop().expect("the new layer is selected");
        assert_eq!(layer(&s, &id).kind, kind);
        undo(&mut s, cx);
        assert_eq!(layers(&s).len(), 1, "{chord}");
    }
}
#[gpui::test]
fn group_lock_and_stack_order_run_on_the_selection(cx: &mut TestAppContext) {
    let mut s = stage(cx, 3);
    select(&mut s, cx, &["l0", "l1"]);
    press(&mut s, cx, "cmd+g");
    let group = selected(&s).pop().unwrap();
    assert_eq!(layer(&s, &group).kind, "group");
    assert_eq!(layer(&s, "l0").group.as_deref(), Some(group.as_str()));
    press(&mut s, cx, "cmd+shift+g");
    assert_eq!(layer(&s, "l0").group, None);
    undo(&mut s, cx);
    undo(&mut s, cx);
    assert_eq!(layers(&s).len(), 3);

    select(&mut s, cx, &["l2"]);
    press(&mut s, cx, "cmd+shift+]");
    assert_eq!(layer(&s, "l2").index, 0);
    undo(&mut s, cx);
    assert_eq!(layer(&s, "l2").index, 2);

    press(&mut s, cx, "cmd+l");
    assert!(layer(&s, "l2").locked);
    press(&mut s, cx, "cmd+shift+l");
    assert!(!layer(&s, "l2").locked);
    undo(&mut s, cx);
    undo(&mut s, cx);
    assert!(!layer(&s, "l2").locked);
}
#[gpui::test]
fn arrow_keys_nudge_position_and_undo_restores(cx: &mut TestAppContext) {
    let mut s = stage_project(cx, crate::gizmo_tests::project());
    select(&mut s, cx, &["l0"]);
    *s.recorder.time.borrow_mut() = 0.5;
    let value = |s: &Stage, channel: &str| {
        let p = published(&s.rig.kernel);
        pm_kernel::QueryMirror::new(&p)
            .queries()
            .evaluate(None, "l0", channel, 0.5)
            .unwrap()
            .as_f64()
            .unwrap()
    };
    let before = value(&s, "position.x");
    press(&mut s, cx, "shift+right");
    assert_eq!(value(&s, "position.x"), before + 10.);
    undo(&mut s, cx);
    assert_eq!(value(&s, "position.x"), before);
}
#[gpui::test]
fn selection_chords_walk_the_stack(cx: &mut TestAppContext) {
    let mut s = stage(cx, 3);
    press(&mut s, cx, "cmd+a");
    assert_eq!(selected(&s).len(), 3);
    select(&mut s, cx, &["l1"]);
    press(&mut s, cx, "cmd+down");
    assert_eq!(selected(&s), ["l2"]);
    press(&mut s, cx, "cmd+up");
    assert_eq!(selected(&s), ["l1"]);
    press(&mut s, cx, "cmd+shift+up");
    assert_eq!(selected(&s).len(), 2);
    press(&mut s, cx, "escape");
    assert!(selected(&s).is_empty());
}
#[gpui::test]
fn work_area_keys_set_it_and_go_to_it_not_to_the_comp_ends(cx: &mut TestAppContext) {
    let mut s = stage(cx, 1);
    *s.recorder.time.borrow_mut() = 2.;
    press(&mut s, cx, "b");
    *s.recorder.time.borrow_mut() = 6.;
    press(&mut s, cx, "n");
    *s.recorder.time.borrow_mut() = 4.;
    press(&mut s, cx, "shift+home");
    assert_eq!(*s.recorder.time.borrow(), 2.);
    press(&mut s, cx, "shift+end");
    assert_eq!(*s.recorder.time.borrow(), 6.);
    undo(&mut s, cx);
    press(&mut s, cx, "shift+end");
    assert_eq!(*s.recorder.time.borrow(), 10., "undo reverted the end");
}
#[gpui::test]
fn layer_keys_step_the_playhead_and_reveal_keys_show_properties(cx: &mut TestAppContext) {
    let mut s = stage(cx, 2);
    select(&mut s, cx, &["l1"]);
    press(&mut s, cx, "i");
    assert_eq!(*s.recorder.time.borrow(), 0.5);
    press(&mut s, cx, "o");
    assert_eq!(*s.recorder.time.borrow(), 2.5);
    let rows = |s: &Stage| s.surface.read_with(&s.vcx, |t, _| t.model().rows().len());
    let collapsed = rows(&s);
    press(&mut s, cx, "t");
    assert!(rows(&s) > collapsed, "T reveals Opacity");
}
/// J and K, stopped: the shuttle declines and the keys walk every stop in
/// the timeline (the shipped `timelineEventTimes`), not only layer ends.
#[gpui::test]
fn j_and_k_stop_on_keyframes_markers_and_the_work_area(cx: &mut TestAppContext) {
    let project = r##"{"id":"integration","name":"Stops","compId":"c","w":640,"h":360,"fps":30,"dur":10,"bg":"#000000","assets":{},"comps":{},"work":[1,6],"markers":[{"id":"m1","t":4}],"layers":[{"id":"l0","type":"shape","name":"Box","from":0.5,"dur":8,"on":true,"p":{"opacity":{"v":100,"kf":[{"i":"k1","t":1.5,"v":0},{"i":"k2","t":2.5,"v":100}]},"x":{"v":10,"kf":[]},"y":{"v":20,"kf":[]},"sx":{"v":100,"kf":[]},"sy":{"v":100,"kf":[]},"r":{"v":0,"kf":[]}},"d":{"w":100,"h":100},"fx":[]}]}"##;
    let mut s = stage_project(cx, project.into());
    assert_eq!(dispatch("k").0, "nextVisibleEvent");
    assert_eq!(dispatch("j").0, "prevVisibleEvent");
    let mut stops = vec![];
    for _ in 0..8 {
        press(&mut s, cx, "k");
        stops.push(*s.recorder.time.borrow());
    }
    // Layer in, work-area start, a key (layer time 1.5 s), a key, the
    // marker, the work area's end, layer out, the composition's end.
    assert_eq!(stops, [0.5, 1., 2., 3., 4., 6., 8.5, 10.]);
    // K at the last stop stays; J walks them back to the start.
    press(&mut s, cx, "k");
    assert_eq!(*s.recorder.time.borrow(), 10.);
    let mut back = vec![];
    for _ in 0..8 {
        press(&mut s, cx, "j");
        back.push(*s.recorder.time.borrow());
    }
    assert_eq!(back, [8.5, 6., 4., 3., 2., 1., 0.5, 0.]);
}
fn key_times(s: &Stage, id: &str, channel: &str) -> Vec<f64> {
    let p = published(&s.rig.kernel);
    let root = p.snapshot.root().unwrap();
    let at = pm_kernel::model::find_layer(root, id).unwrap();
    let layer = pm_kernel::model::layer_at(root, &at).unwrap().to_value();
    layer
        .get("p")
        .and_then(|p| p.get(channel))
        .and_then(|c| c.get("kf"))
        .and_then(Value::as_array)
        .map(|keys| {
            keys.iter()
                .filter_map(|k| k.get("t").and_then(Value::as_f64))
                .collect()
        })
        .unwrap_or_default()
}
const KEYABLE: &str = r##"{"id":"integration","name":"Keys","compId":"c","w":640,"h":360,"fps":30,"dur":10,"bg":"#000000","assets":{},"comps":{},"layers":[{"id":"l0","type":"shape","name":"Box","from":1,"dur":8,"on":true,"p":{"opacity":{"v":100,"kf":[]},"position.x":{"v":10,"kf":[]},"position.y":{"v":20,"kf":[]},"scale.x":{"v":100,"kf":[]},"scale.y":{"v":100,"kf":[]},"rotation":{"v":0,"kf":[]},"anchor.x":{"v":0,"kf":[]},"anchor.y":{"v":0,"kf":[]}},"d":{"w":100,"h":100},"fx":[]}]}"##;
/// Option-Shift-P/S/R/T/A (After Effects): a key on that property at the
/// playhead, removed by the same chord there, each one Undo step.
#[gpui::test]
fn option_shift_property_keys_add_and_remove_a_keyframe_at_the_playhead(cx: &mut TestAppContext) {
    let mut s = stage_project(cx, KEYABLE.into());
    select(&mut s, cx, &["l0"]);
    *s.recorder.time.borrow_mut() = 3.;
    for (chord, channels) in [
        ("alt+shift+p", vec!["position.x", "position.y"]),
        ("alt+shift+s", vec!["scale.x", "scale.y"]),
        ("alt+shift+r", vec!["rotation"]),
        ("alt+shift+t", vec!["opacity"]),
        ("alt+shift+a", vec!["anchor.x", "anchor.y"]),
    ] {
        press(&mut s, cx, chord);
        for channel in &channels {
            // Layer time: the layer starts at 1 s.
            assert_eq!(key_times(&s, "l0", channel), [2.], "{chord} {channel}");
        }
    }
    assert_eq!(key_times(&s, "l0", "opacity"), [2.]);
    let rows = s.surface.read_with(&s.vcx, |t, _| t.model().rows().len());
    assert!(rows > 1, "the keyed properties show in the timeline");
    // A second key elsewhere, then the chord on the first removes only it.
    *s.recorder.time.borrow_mut() = 5.;
    press(&mut s, cx, "alt+shift+t");
    assert_eq!(key_times(&s, "l0", "opacity"), [2., 4.]);
    *s.recorder.time.borrow_mut() = 3.;
    press(&mut s, cx, "alt+shift+t");
    assert_eq!(key_times(&s, "l0", "opacity"), [4.]);
    undo(&mut s, cx);
    assert_eq!(key_times(&s, "l0", "opacity"), [2., 4.]);
    // One axis keyed: the chord completes the pair instead of removing.
    *s.recorder.time.borrow_mut() = 6.;
    let mut set = SetProperty::new("l0", "position.x", Value::Number(40.), 6.);
    set.mode = pm_kernel::commands::properties::KeyMode::Keyframe;
    call(
        &s.rig.kernel,
        Call::Transact {
            label: None,
            expected_revision: None,
            body: pm_kernel::WriteBody::Commands(vec![pm_kernel::PropertyCommand::Set(set).into()]),
        },
    );
    delivered(&mut s, cx);
    assert_eq!(key_times(&s, "l0", "position.x"), [2., 5.]);
    press(&mut s, cx, "alt+shift+p");
    assert_eq!(key_times(&s, "l0", "position.y"), [2., 5.]);
}
/// Option-K (Add Keyframe): the animated properties of the selected layers
/// get a key at the playhead; a selected property narrows it to that one.
#[gpui::test]
fn add_keyframe_keys_the_animated_properties_at_the_playhead(cx: &mut TestAppContext) {
    let mut s = stage_project(cx, KEYABLE.into());
    select(&mut s, cx, &["l0"]);
    assert_eq!(dispatch("alt+k").0, "timeline.addKeyframe");
    *s.recorder.time.borrow_mut() = 3.;
    // Nothing is animated yet: nothing to key.
    press(&mut s, cx, "alt+k");
    assert!(key_times(&s, "l0", "opacity").is_empty());
    press(&mut s, cx, "alt+shift+t");
    press(&mut s, cx, "alt+shift+r");
    *s.recorder.time.borrow_mut() = 5.;
    press(&mut s, cx, "alt+k");
    assert_eq!(key_times(&s, "l0", "opacity"), [2., 4.]);
    assert_eq!(key_times(&s, "l0", "rotation"), [2., 4.]);
    assert!(key_times(&s, "l0", "position.x").is_empty());
    // Again on the same frame changes nothing.
    press(&mut s, cx, "alt+k");
    assert_eq!(key_times(&s, "l0", "opacity"), [2., 4.]);
    undo(&mut s, cx);
    assert_eq!(key_times(&s, "l0", "opacity"), [2.], "one Undo step");
    assert_eq!(key_times(&s, "l0", "rotation"), [2.]);
}
/// With no layer selected Option-Shift-S has nothing to key and stays the
/// Audio Skimming toggle.
#[test]
fn a_property_key_chord_falls_through_to_its_other_meaning() {
    let mut resolver = Resolver::new();
    let mut effects = resolver.submit(Key {
        token: 1,
        chord: "alt+shift+s".into(),
        field: false,
        repeat: false,
        up: false,
        compat: false,
    });
    let mut offered = vec![];
    while let Some(Effect::Dispatch { command, .. }) = effects.into_iter().next() {
        offered.push(command);
        effects = resolver.resolve(1, false);
    }
    assert_eq!(offered, ["timeline.keyProperty:s", "toggleAudioSkimming"]);
}
/// The timeline's own keys: a marker at the playhead, the time scale, the
/// work area back to the whole composition, and the menu toggles.
#[gpui::test]
fn timeline_keys_add_a_marker_zoom_and_fit_and_the_menu_toggles_report_their_state(
    cx: &mut TestAppContext,
) {
    let mut s = stage(cx, 2);
    let markers = |s: &Stage| -> Vec<f64> {
        let p = published(&s.rig.kernel);
        let root = p.snapshot.root().unwrap();
        let open = pm_kernel::model::open_comp_id(root);
        let composition = pm_kernel::model::comp_node(root, open.as_deref())
            .unwrap()
            .to_value();
        composition
            .get("markers")
            .and_then(Value::as_array)
            .map(|m| {
                m.iter()
                    .filter_map(|m| m.get("t").and_then(Value::as_f64))
                    .collect()
            })
            .unwrap_or_default()
    };
    // The numeric keypad's * (After Effects), Shift-8 and Control-8.
    for chord in ["*", "shift+8", "ctrl+8"] {
        assert_eq!(dispatch(chord).0, "timeline.addMarker", "{chord}");
    }
    *s.recorder.time.borrow_mut() = 2.;
    press(&mut s, cx, "*");
    assert_eq!(markers(&s), [2.]);
    press(&mut s, cx, "*");
    assert_eq!(markers(&s), [2.], "one marker a frame");
    undo(&mut s, cx);
    assert!(markers(&s).is_empty());
    // = and - zoom the time scale (After Effects); \ fits the composition.
    let pps = |s: &Stage| s.surface.read_with(&s.vcx, |t, _| t.pps());
    press(&mut s, cx, "\\");
    let fitted = pps(&s);
    press(&mut s, cx, "=");
    let closer = pps(&s);
    assert!(closer > fitted, "{closer} > {fitted}");
    press(&mut s, cx, "-");
    press(&mut s, cx, "-");
    assert!(pps(&s) < fitted);
    press(&mut s, cx, "\\");
    assert!(
        (pps(&s) - fitted).abs() < 1e-6,
        "{} fits at {fitted}",
        pps(&s)
    );
    // Reset Work Area (Playback menu).
    *s.recorder.time.borrow_mut() = 2.;
    press(&mut s, cx, "b");
    run(&mut s, cx, "resetWorkArea", &[]);
    press(&mut s, cx, "shift+home");
    assert_eq!(*s.recorder.time.borrow(), 0.);
    // View > Timeline in Tracks and Snapping carry their state.
    let toggles = |s: &Stage| s.surface.read_with(&s.vcx, |t, _| t.toggles());
    assert_eq!(toggles(&s), (false, true));
    run(&mut s, cx, "timeline.toggleMode", &[]);
    run(&mut s, cx, "timeline.toggleSnapping", &[]);
    assert_eq!(toggles(&s), (true, false));
    run(&mut s, cx, "timeline.toggleMode", &[]);
    run(&mut s, cx, "timeline.toggleSnapping", &[]);
    assert_eq!(toggles(&s), (false, true));
    // Every new row is in a menu, the keyed ones with their keys.
    let menus = pm_shell::menu::menus();
    let row = |menu: &str, command: &str| {
        let (_, items) = menus.iter().find(|(title, _)| *title == menu).unwrap();
        items
            .iter()
            .find(|i| i.command == command)
            .unwrap_or_else(|| panic!("{menu} has no {command}"))
            .chord
            .clone()
    };
    assert_eq!(row("Timeline", "timeline.addMarker"), "*");
    assert_eq!(row("Timeline", "timeline.zoomIn"), "=");
    assert_eq!(row("Timeline", "timeline.zoomOut"), "-");
    assert_eq!(row("Timeline", "timeline.fit"), "\\");
    assert_eq!(row("Timeline", "timeline.toggleHold"), "cmd+alt+h");
    assert_eq!(row("Playback", "resetWorkArea"), "");
    for command in [
        "timeline.toggleMode",
        "timeline.toggleSnapping",
        "graph.value",
        "graph.speed",
    ] {
        assert_eq!(row("View", command), "");
    }
}
/// Option-Command-H (After Effects) holds the selected keyframes and lets
/// them go again.
#[gpui::test]
fn the_hold_key_toggles_hold_on_the_selected_keyframes(cx: &mut TestAppContext) {
    let mut s = stage_project(cx, KEYABLE.into());
    select(&mut s, cx, &["l0"]);
    *s.recorder.time.borrow_mut() = 3.;
    press(&mut s, cx, "alt+shift+t");
    *s.recorder.time.borrow_mut() = 5.;
    press(&mut s, cx, "alt+shift+t");
    let holds = |s: &Stage| -> Vec<bool> {
        let p = published(&s.rig.kernel);
        let root = p.snapshot.root().unwrap();
        let at = pm_kernel::model::find_layer(root, "l0").unwrap();
        let layer = pm_kernel::model::layer_at(root, &at).unwrap().to_value();
        layer
            .get("p")
            .and_then(|p| p.get("opacity"))
            .and_then(|c| c.get("kf"))
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .map(|k| k.get("outInterp").and_then(Value::as_str) == Some("hold"))
            .collect()
    };
    let ids: Vec<String> = s.surface.read_with(&s.vcx, |t, _| {
        t.model()
            .index
            .channel("l0", "opacity")
            .unwrap()
            .keys
            .iter()
            .map(|k| k.id.clone())
            .collect()
    });
    call(&s.rig.kernel, SelectionCall::Keys(ids));
    delivered(&mut s, cx);
    assert_eq!(dispatch("cmd+alt+h").0, "timeline.toggleHold");
    press(&mut s, cx, "cmd+alt+h");
    assert_eq!(holds(&s), [true, true]);
    press(&mut s, cx, "cmd+alt+h");
    assert_eq!(holds(&s), [false, false]);
    undo(&mut s, cx);
    assert_eq!(holds(&s), [true, true], "one Undo step");
}
#[gpui::test]
fn layer_paste_plans_a_copy_at_the_playhead_and_undo_removes_it(cx: &mut TestAppContext) {
    let mut s = stage(cx, 1);
    let snapshot = published(&s.rig.kernel);
    let root = snapshot.snapshot.root().unwrap();
    let at = pm_kernel::model::find_layer(root, "l0").unwrap();
    let copied = vec![pm_kernel::model::layer_at(root, &at).unwrap().to_value()];
    let plan = paste_plan(copied, 3.);
    call(
        &s.rig.kernel,
        transaction(&plan, Some(snapshot.revision())).unwrap(),
    );
    s.idle(cx);
    assert_eq!(layers(&s).len(), 2);
    let id = plan.select.unwrap().pop().unwrap();
    assert_eq!(layer(&s, &id).in_time, 3.);
    undo(&mut s, cx);
    assert_eq!(layers(&s).len(), 1);
}
#[gpui::test]
fn nudging_with_no_args_is_refused_instead_of_guessing(cx: &mut TestAppContext) {
    let mut s = stage(cx, 1);
    select(&mut s, cx, &["l0"]);
    let p = published(&s.rig.kernel);
    assert!(command_plan("nudgeSelection", &[], &p, 0.).is_none());
    let done = s.surface.clone();
    settle(cx, move |cx| done.read_with(cx, |t, _| t.idle()));
}
#[gpui::test]
fn copied_keys_paste_at_the_playhead_and_undo_removes_them(cx: &mut TestAppContext) {
    let mut s = stage_project(cx, crate::gizmo_tests::project());
    select(&mut s, cx, &["l0"]);
    let key = |time: f64| {
        let mut set = SetProperty::new("l0", "opacity", Value::Number(40.), time);
        set.mode = pm_kernel::commands::properties::KeyMode::Keyframe;
        pm_kernel::PropertyCommand::Set(set).into()
    };
    call(
        &s.rig.kernel,
        Call::Transact {
            label: None,
            expected_revision: None,
            body: pm_kernel::WriteBody::Commands(vec![key(0.5)]),
        },
    );
    delivered(&mut s, cx);
    let count = |s: &Stage| {
        s.surface.read_with(&s.vcx, |t, _| {
            t.model().index.layer("l0").map_or(0, |l| {
                l.channels.iter().map(|c| c.keys.len()).sum::<usize>()
            })
        })
    };
    assert_eq!(count(&s), 1);
    let ids: Vec<String> = s.surface.read_with(&s.vcx, |t, _| {
        let layer = t.model();
        let layer = layer.index.layer("l0").unwrap();
        layer
            .channels
            .iter()
            .flat_map(|c| c.keys.iter().map(|k| k.id.clone()))
            .collect()
    });
    call(&s.rig.kernel, SelectionCall::Keys(ids));
    delivered(&mut s, cx);
    let tracks = s.surface.read_with(&s.vcx, |t, _| t.copied_keys());
    assert_eq!(tracks.len(), 1);
    s.surface
        .update(&mut s.vcx, |t, cx| t.paste_keys(tracks, 1.5, cx));
    delivered(&mut s, cx);
    assert_eq!(count(&s), 2);
    undo(&mut s, cx);
    assert_eq!(count(&s), 1);
}
