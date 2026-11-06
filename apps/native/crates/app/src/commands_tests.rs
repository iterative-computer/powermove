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
fn dispatch(chord: &str) -> (String, Vec<serde_json::Value>) {
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
