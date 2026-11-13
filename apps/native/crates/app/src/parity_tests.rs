//! Features of the shipped app, each through the route a person uses (a
//! chord through the shipped resolver, a command, a menu row) into a real
//! session, then saved to a project file and opened again by a second kernel.
use crate::layer_commands::{Route, command_plan, route, transaction};
use pm_kernel::session::{SessionHandle, SessionSource, ShellCall, ShellReply};
use pm_kernel::{
    Call, Caller, HistoryCall, KernelActor, KernelConfig, KernelHandle, Published, Request,
    SelectionCall,
};
use pm_model::Value;
use serde_json::json;
use std::sync::Arc;

pub(crate) struct Desk {
    dir: tempfile::TempDir,
    actor: KernelActor,
    pub session: SessionHandle,
}
fn open(dir: &std::path::Path, id: &str, source: SessionSource) -> (KernelActor, SessionHandle) {
    let actor = KernelActor::spawn(KernelConfig {
        journal_root: Some(dir.join("journals")),
        ..Default::default()
    });
    crate::mcp::attach(&actor.handle(), dir).unwrap();
    let ShellReply::Opened { session, .. } = actor
        .handle()
        .shell(ShellCall::Open {
            id: id.into(),
            source,
            electron_profile: None,
        })
        .wait()
        .unwrap()
        .unwrap()
    else {
        panic!("the project opens")
    };
    (actor, session)
}
impl Desk {
    pub fn new(project: serde_json::Value) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let source = SessionSource::New(pm_model::json::parse(&project.to_string()).unwrap());
        let (actor, session) = open(dir.path(), "desk", source);
        Self {
            dir,
            actor,
            session,
        }
    }
    pub fn kernel(&self) -> KernelHandle {
        self.actor.handle()
    }
    pub fn published(&self) -> Arc<Published> {
        self.kernel().published(&self.session.project).unwrap()
    }
    pub fn call(&self, call: impl Into<Call>) {
        let ack = self
            .kernel()
            .call_blocking(
                &self.session.project,
                Request::new(Caller::native(), call.into()),
            )
            .unwrap();
        assert!(
            !matches!(ack.reply, pm_kernel::Reply::Throw(_)),
            "the kernel refused: {:?}",
            ack.reply
        );
    }
    pub fn select(&self, ids: &[&str]) {
        let ids = ids.iter().map(|id| (*id).to_owned()).collect();
        self.call(SelectionCall::Layers { ids, add: false });
    }
    /// A planned command as `Controller::command` runs it: one transaction,
    /// then the selection it leads to.
    pub fn run(&self, command: &str, args: &[serde_json::Value], time: f64) {
        let Some(Route::Plan(name)) = route(command, args) else {
            panic!("{command} is not a planned command")
        };
        let published = self.published();
        let plan = command_plan(name, args, &published, time)
            .unwrap_or_else(|| panic!("{name} had nothing to do"));
        if let Some(call) = transaction(&plan, Some(published.revision())) {
            self.call(call);
        }
        if let Some(ids) = plan.select {
            self.call(SelectionCall::Layers { ids, add: false });
        }
    }
    /// A chord through the shipped keymap.
    pub fn press(&self, chord: &str, time: f64) {
        let (command, args) = crate::layer_commands::commands_tests::dispatch(chord);
        self.run(&command, &args, time);
    }
    pub fn undo(&self) {
        self.call(Call::History(HistoryCall::Undo));
    }
    pub fn redo(&self) {
        self.call(Call::History(HistoryCall::Redo));
    }
    pub fn selected(&self) -> Vec<String> {
        self.published().selection.layers().to_vec()
    }
    pub fn layers(&self) -> Vec<pm_kernel::LayerInfo> {
        pm_kernel::model::list_layers(self.published().snapshot.root().unwrap(), None)
    }
    /// The whole stored project, to compare states exactly.
    pub fn document(&self) -> Value {
        self.published().snapshot.document.to_value().unwrap()
    }
    /// Saves to a project file and opens that file with a second kernel.
    pub fn reopened(&self) -> Reopened {
        let file = self.dir.path().join("saved.pmv");
        let _ = std::fs::remove_file(&file);
        self.kernel()
            .shell(ShellCall::SaveAs {
                session: self.session.clone(),
                path: file.clone(),
                replace: false,
            })
            .wait()
            .unwrap()
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let (actor, session) = open(dir.path(), "again", SessionSource::File(file));
        let published = actor.handle().published(&session.project).unwrap();
        Reopened {
            published,
            _actor: actor,
            _dir: dir,
        }
    }
}
pub(crate) struct Reopened {
    pub published: Arc<Published>,
    _actor: KernelActor,
    _dir: tempfile::TempDir,
}
pub(crate) fn layer_of(published: &Published, id: &str) -> Value {
    let root = published.snapshot.root().unwrap();
    let at = pm_kernel::model::find_layer(root, id).unwrap_or_else(|| panic!("no layer {id}"));
    pm_kernel::model::layer_at(root, &at).unwrap().to_value()
}
pub(crate) fn evaluate(published: &Published, id: &str, channel: &str, time: f64) -> f64 {
    pm_kernel::QueryMirror::new(published)
        .queries()
        .evaluate(None, id, channel, time)
        .unwrap()
        .as_f64()
        .unwrap()
}
fn shape(id: &str) -> serde_json::Value {
    json!({
        "id": id, "type": "shape", "name": id, "from": 0, "dur": 4, "on": true,
        "p": {
            "opacity": { "v": 100, "kf": [] },
            "position.x": { "v": 320, "kf": [] }, "position.y": { "v": 180, "kf": [] },
            "scale.x": { "v": 100, "kf": [] }, "scale.y": { "v": 100, "kf": [] },
            "rotation": { "v": 0, "kf": [] }, "skew": { "v": 0, "kf": [] },
            "anchor.x": { "v": 0, "kf": [] }, "anchor.y": { "v": 0, "kf": [] }
        },
        "d": { "shape": "rect", "w": 100, "h": 100 }, "fx": []
    })
}
pub(crate) fn project(layers: Vec<serde_json::Value>) -> serde_json::Value {
    json!({
        "id": "desk", "name": "Synthetic", "compId": "c", "w": 640, "h": 360, "fps": 30,
        "dur": 10, "bg": "#000000", "assets": {}, "comps": {}, "layers": layers
    })
}

#[test]
fn the_null_chord_makes_a_null_object_as_the_shipped_app_made_it() {
    let desk = Desk::new(project(vec![shape("a")]));
    let before = desk.document();
    desk.press("cmd+alt+shift+y", 2.);
    let id = desk.selected().pop().expect("the new null is selected");
    assert_eq!(desk.selected().len(), 1);
    let published = desk.published();
    let null = layer_of(&published, &id);
    let text = |key: &str| null.get(key).and_then(Value::as_str).map(str::to_owned);
    let number = |value: Option<&Value>| value.and_then(Value::as_f64);
    assert_eq!(text("type").as_deref(), Some("null"));
    assert_eq!(text("name").as_deref(), Some("Null"));
    // From the playhead to the composition's end.
    assert_eq!(number(null.get("from")), Some(2.));
    assert_eq!(number(null.get("dur")), Some(8.));
    let content = null.get("d").unwrap();
    assert_eq!(number(content.get("w")), Some(100.));
    assert_eq!(number(content.get("h")), Some(100.));
    // At the composition's centre, anchored at its own centre, drawing nothing.
    for (channel, value) in [
        ("position.x", 320.),
        ("position.y", 180.),
        ("anchor.x", 50.),
        ("anchor.y", 50.),
        ("opacity", 0.),
    ] {
        assert_eq!(evaluate(&published, &id, channel, 2.), value, "{channel}");
    }
    assert_eq!(
        desk.layers().first().map(|l| l.id.clone()),
        Some(id.clone())
    );
    assert_eq!(
        published.history.undo_label.as_deref(),
        Some("New Null Object")
    );
    let made = desk.document();

    let again = desk.reopened();
    assert_eq!(layer_of(&again.published, &id), null, "saved and reopened");

    desk.undo();
    assert_eq!(desk.document(), before, "one Undo removes it");
    desk.redo();
    assert_eq!(desk.document(), made, "Redo makes the same null");
}

#[test]
fn null_object_is_in_the_layer_menu_the_add_menu_and_the_palette_with_its_shortcut() {
    let menus = pm_shell::menu::menus();
    let (_, layer) = menus.iter().find(|(title, _)| *title == "Layer").unwrap();
    let row = layer
        .iter()
        .find(|item| item.command == "newNull")
        .expect("Layer > New Null Object");
    assert_eq!(row.label, "New Null Object");
    assert_eq!(pm_shell::menu::glyphs(&row.chord), "⌥⇧⌘Y");
    assert!(
        pm_shell::ADD
            .iter()
            .any(|(command, _)| *command == "newNull")
    );
    // Every row of the toolbar's Add menu is a command something runs.
    let session = include_str!("session.rs");
    for (command, _) in pm_shell::ADD.iter().filter(|(c, _)| !c.is_empty()) {
        assert!(
            route(command, &[]).is_some() || session.contains(&format!("\"{command}\" =>")),
            "{command}"
        );
    }
}

/// Properties ▸ Add Expression…: the sheet opens on the shipped starting
/// text, and Apply writes it to every channel of the row as one Undo step.
#[test]
fn add_expression_applies_the_sheets_text_to_the_rows_channels() {
    use crate::dialogs::{DialogId, WindowId};
    use crate::expression_sheet::Target;
    let desk = Desk::new(project(vec![shape("a")]));
    desk.select(&["a"]);
    let before = desk.document();
    let expression = |published: &Published, channel: &str| {
        layer_of(published, "a")
            .get("p")
            .and_then(|p| p.get(channel))
            .and_then(|c| c.get("expr"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    // What the Properties row's menu action sends (builtins `expressions` tests).
    let target = Target::read(&json!({
        "document": desk.published().snapshot.doc_id, "layer": "a", "layers": 1,
        "property": "Position", "source": null,
        "targets": [["a", "position.x"], ["a", "position.y"]],
    }))
    .unwrap();
    let sheet = target.surface(DialogId("d".into()), WindowId("w".into()));
    let typed = sheet.fields[0].values[0].clone();
    assert_eq!(typed, "value + wiggle(2, 20)");
    let call = target
        .apply(&typed, Some(desk.published().revision()))
        .unwrap()
        .expect("an edit");
    desk.call(call);
    let published = desk.published();
    for channel in ["position.x", "position.y"] {
        assert_eq!(
            expression(&published, channel).as_deref(),
            Some(typed.as_str())
        );
    }
    assert_eq!(expression(&published, "opacity"), None);
    assert_eq!(
        published.history.undo_label.as_deref(),
        Some("Set Expression")
    );
    let made = desk.document();

    let again = desk.reopened();
    assert_eq!(layer_of(&again.published, "a"), layer_of(&published, "a"));

    desk.undo();
    assert_eq!(desk.document(), before, "one Undo removes both");
    desk.redo();
    assert_eq!(desk.document(), made);
}

/// Properties ▸ Show in Graph Editor, as the command the row sends: the
/// timeline shows its graph on that property and the layer's row opens.
#[gpui::test]
fn show_in_graph_editor_opens_the_timelines_graph_on_the_property(cx: &mut gpui::TestAppContext) {
    use pm_panel_timeline::{GraphDisplay, RowId};
    let mut s = crate::timeline_tests::stage(cx, 2);
    let model = |s: &crate::timeline_tests::Stage| s.surface.read_with(&s.vcx, |t, _| t.model());
    assert_eq!(model(&s).graph_display, GraphDisplay::DopeSheet);
    s.surface.update(&mut s.vcx, |t, cx| {
        t.show_graph("l1".into(), "opacity".into(), vec!["opacity".into()], cx)
    });
    s.idle(cx);
    let shown = model(&s);
    assert_eq!(shown.graph_display, GraphDisplay::Graph);
    assert_eq!(
        shown.graph_focus,
        Some(("l1".to_owned(), "opacity".to_owned()))
    );
    assert!(shown.rows().iter().any(|row| row.id
        == RowId::Property {
            layer_id: "l1".into(),
            channel: "opacity".into()
        }));
    let published = s
        .rig
        .kernel
        .published(crate::v4_view_tests::PROJECT)
        .unwrap();
    assert_eq!(published.selection.channel(), Some("opacity"));
    // Asked again, the graph stays up (it is not a toggle).
    s.surface.update(&mut s.vcx, |t, cx| {
        t.show_graph("l1".into(), "opacity".into(), vec!["opacity".into()], cx)
    });
    s.idle(cx);
    assert_eq!(model(&s).graph_display, GraphDisplay::Graph);
}

// The layer right-click menu (timeline rows and the viewer).
fn layer(id: &str, kind: &str, content: serde_json::Value) -> serde_json::Value {
    let mut layer = shape(id);
    layer["type"] = json!(kind);
    layer["d"] = content;
    layer
}
fn menu_project() -> serde_json::Value {
    let mut member = shape("c");
    member["group"] = json!("g");
    let mut locked = shape("locked");
    locked["lock"] = json!(true);
    let mut keyed = shape("keyed");
    // A held value with a key in its middle that changes nothing.
    keyed["p"]["opacity"]["kf"] = json!([
        {"i":"k0","t":0,"v":0}, {"i":"k1","t":1,"v":50}, {"i":"k2","t":1.5,"v":50},
        {"i":"k3","t":2,"v":50}, {"i":"k4","t":3,"v":100}
    ]);
    let mut project = project(vec![
        layer(
            "t",
            "text",
            json!({"text":"One two","font":"Helvetica","size":40,"color":"#FFFFFF"}),
        ),
        shape("a"),
        shape("b"),
        layer("g", "group", json!({})),
        member,
        layer(
            "v",
            "video",
            json!({"asset":"A","embeddedAudio":true,"trim":1.5,"speed":1,"w":64,"h":64}),
        ),
        locked,
        keyed,
    ]);
    project["assets"] = json!({"A":{"id":"A","name":"A","kind":"video","dur":100}});
    project
}
fn labels(rows: &[serde_json::Value]) -> Vec<String> {
    rows.iter()
        .map(|row| match row["label"].as_str() {
            Some(label) if row["disabled"] == true => format!("({label})"),
            Some(label) => label.to_owned(),
            None => "-".to_owned(),
        })
        .collect()
}
fn child_labels(rows: &[serde_json::Value], parent: &str) -> Vec<String> {
    let row = rows
        .iter()
        .find(|row| row["label"] == parent)
        .unwrap_or_else(|| panic!("no {parent} row"));
    labels(row["children"].as_array().unwrap())
}
fn menu(desk: &Desk, pressed: &str, time: f64) -> Vec<serde_json::Value> {
    crate::layer_commands::layer_menu(&desk.published(), pressed, time)
}
/// Chooses a row as the menu does: by its preorder position.
fn choose(desk: &Desk, pressed: &str, time: f64, path: &[&str]) {
    let rows = menu(desk, pressed, time);
    fn find(rows: &[serde_json::Value], path: &[&str], next: &mut usize) -> Option<usize> {
        for row in rows {
            let index = *next;
            *next += 1;
            let children = row["children"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_default();
            if row["label"] == path[0] {
                if path.len() == 1 {
                    assert_ne!(row["disabled"], true, "{} is disabled", path[0]);
                    return Some(index);
                }
                return find(children, &path[1..], next);
            }
            let mut skipped = 0;
            fn count(rows: &[serde_json::Value], n: &mut usize) {
                for row in rows {
                    *n += 1;
                    if let Some(children) = row["children"].as_array() {
                        count(children, n);
                    }
                }
            }
            count(children, &mut skipped);
            *next += skipped;
        }
        None
    }
    let index = find(&rows, path, &mut 0).unwrap_or_else(|| panic!("no row {path:?}"));
    let (command, args) = crate::layer_commands::layer_menu_choice(&rows, index).unwrap();
    desk.run(&command, &args, time);
}

#[test]
fn the_layer_menu_has_the_shipped_rows_in_the_shipped_order() {
    let desk = Desk::new(menu_project());
    desk.select(&["a"]);
    assert_eq!(
        labels(&menu(&desk, "a", 1.)),
        [
            "Duplicate",
            "Group",
            "Simplify Keyframes",
            "Pre-compose…",
            "Move to Group",
            "Parent",
            "Timing",
            "-",
            "Hide",
            "Solo",
            "Lock",
            "-",
            "Delete"
        ]
    );
    assert_eq!(
        child_labels(&menu(&desk, "a", 1.), "Move to Group"),
        ["Outside Groups", "g"]
    );
    assert_eq!(
        child_labels(&menu(&desk, "a", 1.), "Timing"),
        [
            "Split at Playhead",
            "Trim In to Playhead",
            "Trim Out to Playhead",
            "Fit to Composition"
        ]
    );
    // Outside the layer's time only Fit to Composition has something to do.
    assert_eq!(
        child_labels(&menu(&desk, "a", 6.), "Timing"),
        [
            "(Split at Playhead)",
            "(Trim In to Playhead)",
            "(Trim Out to Playhead)",
            "Fit to Composition"
        ]
    );
    // A parent can be any layer that would not make a cycle; never itself.
    let parents = child_labels(&menu(&desk, "a", 1.), "Parent");
    assert_eq!(parents[0], "None");
    assert!(parents.contains(&"b".to_owned()) && !parents.contains(&"a".to_owned()));

    // Two layers: the lower can be the upper's matte.
    desk.select(&["a", "b"]);
    assert_eq!(labels(&menu(&desk, "a", 1.))[4], "Use Bottom Layer as Mask");
    // Text layers split; nothing else offers it.
    desk.select(&["t"]);
    assert_eq!(labels(&menu(&desk, "t", 1.))[4], "Split Text into Layers");
    assert_eq!(
        child_labels(&menu(&desk, "t", 1.), "Split Text into Layers"),
        ["By Word", "By Character", "By Line"]
    );
    // A group ungroups and has no timing of its own.
    desk.select(&["g"]);
    let group = labels(&menu(&desk, "g", 1.));
    assert!(group.contains(&"Ungroup".to_owned()) && !group.contains(&"Timing".to_owned()));
    // A video that carries its soundtrack.
    desk.select(&["v"]);
    let video = labels(&menu(&desk, "v", 1.));
    let at = video
        .iter()
        .position(|l| l == "Mute Audio")
        .expect("Mute Audio");
    assert_eq!(video[at + 1], "Separate Audio");
    assert_eq!(video[at + 2], "-");
    // A locked layer: only Unlock is live.
    desk.select(&["locked"]);
    assert_eq!(
        labels(&menu(&desk, "locked", 1.)),
        [
            "(Duplicate)",
            "(Group)",
            "(Simplify Keyframes)",
            "(Pre-compose…)",
            "(Move to Group)",
            "(Parent)",
            "(Timing)",
            "-",
            "(Hide)",
            "(Solo)",
            "Unlock",
            "-",
            "(Delete)"
        ]
    );
    // A row pressed outside the selection is the menu's only layer.
    desk.select(&["a", "b"]);
    assert!(!labels(&menu(&desk, "v", 1.)).contains(&"Use Bottom Layer as Mask".to_owned()));
}

#[test]
fn layer_menu_rows_change_the_project_as_the_shipped_app_did_with_one_undo_each() {
    let desk = Desk::new(menu_project());
    let get = |id: &str| layer_of(&desk.published(), id);
    let number = |id: &str, key: &str| get(id).get(key).and_then(Value::as_f64);
    let text = |id: &str, key: &str| get(id).get(key).and_then(Value::as_str).map(str::to_owned);
    let flag = |id: &str, key: &str| get(id).get(key).and_then(Value::as_bool);
    // Each step: do it, check it, check Undo and Redo restore both states.
    let step = |pressed: &str, path: &[&str], label: &str, check: &dyn Fn()| {
        let before = desk.document();
        choose(&desk, pressed, 1., path);
        check();
        assert_eq!(
            desk.published().history.undo_label.as_deref(),
            Some(label),
            "{path:?}"
        );
        let after = desk.document();
        desk.undo();
        assert_eq!(desk.document(), before, "undo {path:?}");
        desk.redo();
        assert_eq!(desk.document(), after, "redo {path:?}");
    };

    desk.select(&["a"]);
    step(
        "a",
        &["Timing", "Fit to Composition"],
        "Fit to Composition",
        &|| {
            assert_eq!(
                (number("a", "from"), number("a", "dur")),
                (Some(0.), Some(10.))
            );
        },
    );
    step("a", &["Parent", "b"], "Parent Layers", &|| {
        assert_eq!(text("a", "parent").as_deref(), Some("b"));
    });
    assert!(!child_labels(&menu(&desk, "b", 1.), "Parent").contains(&"a".to_owned()));
    step("a", &["Parent", "None"], "Remove Parent", &|| {
        assert_eq!(text("a", "parent"), None);
    });
    step("a", &["Move to Group", "g"], "Move to Group", &|| {
        assert_eq!(text("a", "group").as_deref(), Some("g"));
    });
    step(
        "a",
        &["Move to Group", "Outside Groups"],
        "Move out of Group",
        &|| {
            assert_eq!(text("a", "group"), None);
        },
    );
    step("a", &["Hide"], "Hide Layers", &|| {
        assert_eq!(flag("a", "on"), Some(false))
    });
    assert_eq!(labels(&menu(&desk, "a", 1.))[8], "Show");
    step("a", &["Show"], "Show Layers", &|| {
        assert_eq!(flag("a", "on"), Some(true))
    });
    step("a", &["Solo"], "Solo Layers", &|| {
        assert_eq!(flag("a", "solo"), Some(true))
    });
    step("a", &["Unsolo"], "Solo Layers", &|| {
        assert_ne!(flag("a", "solo"), Some(true))
    });
    step("a", &["Lock"], "Lock Layers", &|| {
        assert_eq!(flag("a", "lock"), Some(true))
    });
    step("a", &["Unlock"], "Unlock Layers", &|| {
        assert_ne!(flag("a", "lock"), Some(true))
    });

    desk.select(&["a", "b"]);
    // The lower of the two in the stack becomes the upper one's matte.
    let order: Vec<String> = desk
        .layers()
        .into_iter()
        .filter(|l| l.id == "a" || l.id == "b")
        .map(|l| l.id)
        .collect();
    step(
        "b",
        &["Use Bottom Layer as Mask"],
        "Use Bottom Layer as Mask",
        &|| {
            assert_eq!(
                text(&order[0], "matteSource").as_deref(),
                Some(order[1].as_str())
            );
            assert_eq!(text(&order[1], "matteSource"), None);
        },
    );

    desk.select(&["keyed"]);
    let keys = || crate::layer_commands::key_count(&desk.published(), &["keyed".to_owned()]);
    assert_eq!(keys(), 5);
    step(
        "keyed",
        &["Simplify Keyframes"],
        "Simplify Keyframes",
        &|| {
            assert_eq!(keys(), 4, "the key in the middle of the hold goes");
        },
    );

    desk.select(&["v"]);
    step("v", &["Mute Audio"], "Mute Audio", &|| {
        let muted = get("v")
            .get("d")
            .and_then(|d| d.get("audioMuted"))
            .and_then(Value::as_bool);
        assert_eq!(muted, Some(true));
    });
    step("v", &["Unmute Audio"], "Unmute Audio", &|| {});
    let video_at = desk.layers().iter().position(|l| l.id == "v").unwrap();
    step("v", &["Separate Audio"], "Separate Audio", &|| {
        let layers = desk.layers();
        let audio = &layers[video_at];
        assert_eq!(
            (audio.kind.as_str(), audio.name.as_str()),
            ("audio", "v Audio")
        );
        assert_eq!(
            layers[video_at + 1].id,
            "v",
            "the soundtrack sits above its picture"
        );
        assert_eq!((audio.in_time, audio.out_time), (0., 4.));
        let content = get(&audio.id).get("d").cloned().unwrap();
        assert_eq!(content.get("asset").and_then(Value::as_str), Some("A"));
        let trim = content.get("trim").unwrap();
        assert_eq!(
            trim.as_f64()
                .or_else(|| trim.get("v").and_then(Value::as_f64)),
            Some(1.5)
        );
        let embedded = get("v")
            .get("d")
            .and_then(|d| d.get("embeddedAudio"))
            .and_then(Value::as_bool);
        assert_eq!(embedded, Some(false));
        assert_eq!(desk.selected(), std::slice::from_ref(&audio.id));
    });
    assert!(!labels(&menu(&desk, "v", 1.)).contains(&"Separate Audio".to_owned()));

    // Everything above is in the saved file.
    let again = desk.reopened();
    for layer in desk.layers() {
        assert_eq!(
            layer_of(&again.published, &layer.id),
            get(&layer.id),
            "{}",
            layer.id
        );
    }
}

#[test]
fn a_layer_in_a_locked_group_offers_unlock_group() {
    let mut project = menu_project();
    project["layers"][3]["lock"] = json!(true);
    let desk = Desk::new(project);
    desk.select(&["c"]);
    let rows = menu(&desk, "c", 1.);
    assert_eq!(labels(&rows)[labels(&rows).len() - 3], "Unlock Group");
    choose(&desk, "c", 1., &["Unlock Group"]);
    assert_ne!(
        layer_of(&desk.published(), "g")
            .get("lock")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        desk.published().history.undo_label.as_deref(),
        Some("Unlock Group")
    );
}

/// The layer commands with no shipped chord are menu-bar rows that send
/// their own command (the Layer menu lists keymap bindings only), and
/// choosing each one changes the project.
#[test]
fn chordless_layer_commands_run_from_the_menu_bar() {
    let desk = Desk::new(menu_project());
    // `install_menus` sends a chosen row's command and arguments as listed.
    let rows: Vec<(&str, pm_shell::menu::Item)> = pm_shell::menu::menus()
        .into_iter()
        .flat_map(|(title, items)| items.into_iter().map(move |item| (title, item)))
        .collect();
    for (command, select) in [
        ("toggleVisibility", "a"),
        ("toggleSolo", "a"),
        ("fitToComposition", "a"),
        ("simplifyKeyframes", "keyed"),
        ("separateAudio", "v"),
    ] {
        let (menu, item) = rows
            .iter()
            .find(|(_, item)| item.command == command)
            .unwrap_or_else(|| panic!("{command} has no menu row"));
        assert!(
            !pm_shell::menu::DISPLAY_ONLY.contains(menu),
            "{command} sits in {menu}, which lists keymap bindings only"
        );
        assert!(item.role.is_none() && item.chord.is_empty(), "{command}");
        desk.select(&[select]);
        let before = desk.document();
        desk.run(&item.command, &item.args, 1.);
        assert_ne!(desk.document(), before, "{command} changed nothing");
        desk.undo();
        assert_eq!(desk.document(), before, "{command} is one Undo step");
    }
}

/// A shape layer from `from` for `dur` seconds.
fn span(id: &str, from: f64, dur: f64) -> serde_json::Value {
    let mut layer = shape(id);
    layer["from"] = json!(from);
    layer["dur"] = json!(dur);
    layer
}
fn spans(desk: &Desk) -> Vec<(String, f64, f64)> {
    desk.layers()
        .into_iter()
        .map(|l| (l.id, l.in_time, l.out_time))
        .collect()
}
fn at(id: &str, from: f64, to: f64) -> (String, f64, f64) {
    (id.to_owned(), from, to)
}

#[test]
fn shift_delete_removes_the_selection_and_closes_the_time_it_took() {
    // The chord, and the same command for the forward Delete key.
    for chord in ["shift+backspace", "shift+delete"] {
        let (command, args) = crate::layer_commands::commands_tests::dispatch(chord);
        assert_eq!(
            (command.as_str(), args.len()),
            ("rippleDelete", 0),
            "{chord}"
        );
    }
    let mut locked = span("locked", 5., 2.);
    locked["lock"] = json!(true);
    let desk = Desk::new(project(vec![
        span("a", 0., 2.),
        span("b", 2., 2.),
        span("c", 4., 2.),
        span("background", 0., 10.),
        span("overlapping", 1., 2.),
        span("later", 5., 2.),
        locked,
    ]));
    let before = desk.document();
    desk.select(&["b"]);
    desk.press("shift+backspace", 0.);
    // Layers that start at or after b's end close up by its two seconds;
    // ones that overlap it, and a locked one, stay.
    let closed = vec![
        at("a", 0., 2.),
        at("c", 2., 4.),
        at("background", 0., 10.),
        at("overlapping", 1., 3.),
        at("later", 3., 5.),
        at("locked", 5., 7.),
    ];
    assert_eq!(spans(&desk), closed);
    assert_eq!(
        desk.published().history.undo_label.as_deref(),
        Some("Ripple Delete")
    );
    let after = desk.document();
    let again = desk.reopened();
    let reopened: Vec<_> =
        pm_kernel::model::list_layers(again.published.snapshot.root().unwrap(), None)
            .into_iter()
            .map(|l| (l.id, l.in_time, l.out_time))
            .collect();
    assert_eq!(reopened, closed, "saved and reopened");
    desk.undo();
    assert_eq!(desk.document(), before, "one Undo step");
    desk.redo();
    assert_eq!(desk.document(), after);
}

#[test]
fn ripple_delete_closes_each_removed_span_and_a_moving_group_carries_its_members() {
    let mut member = span("member", 6.5, 1.);
    member["group"] = json!("group");
    let mut group = span("group", 6., 2.);
    group["type"] = json!("group");
    let desk = Desk::new(project(vec![
        span("a", 0., 2.),
        span("b", 2., 2.),
        span("c", 4., 2.),
        group,
        member,
    ]));
    desk.select(&["a", "c"]);
    desk.press("shift+backspace", 0.);
    // b closes a's two seconds; the group, after both spans, closes four,
    // and its member moves with it (once).
    assert_eq!(
        spans(&desk),
        [at("b", 0., 2.), at("group", 2., 4.), at("member", 2.5, 3.5)]
    );
    desk.undo();
    assert_eq!(spans(&desk).len(), 5);
    // A locked layer in the selection refuses the whole command.
    desk.call(Call::Transact {
        label: None,
        expected_revision: None,
        body: pm_kernel::WriteBody::Commands(vec![
            pm_kernel::LayerCommand::Set {
                layer_id: "b".into(),
                patch: pm_kernel::commands::layers::LayerPatch {
                    locked: Some(true),
                    ..Default::default()
                },
            }
            .into(),
        ]),
    });
    desk.select(&["a", "b"]);
    let published = desk.published();
    assert!(command_plan("rippleDelete", &[], &published, 0.).is_none());
    assert!(
        crate::layer_commands::disabled(Some(&published), &desk.layers(), &Default::default())
            .contains("rippleDelete")
    );
}

#[test]
fn option_page_keys_move_the_selected_layers_by_frames() {
    let desk = Desk::new(project(vec![span("a", 1., 2.), span("b", 3., 2.)]));
    let before = desk.document();
    desk.select(&["a"]);
    let frame = 1. / 30.;
    for (chord, frames) in [
        ("alt+pagedown", 1.),
        ("alt+shift+pagedown", 10.),
        ("alt+pageup", -1.),
        ("alt+shift+pageup", -10.),
    ] {
        let from = desk.layers()[0].in_time;
        desk.press(chord, 0.);
        let layer = &desk.layers()[0];
        assert!(
            (layer.in_time - (from + frames * frame)).abs() < 1e-9,
            "{chord}: {from} -> {}",
            layer.in_time
        );
        assert!(
            (layer.out_time - layer.in_time - 2.).abs() < 1e-9,
            "{chord}"
        );
        assert_eq!(
            desk.published().history.undo_label.as_deref(),
            Some("Move Layers")
        );
    }
    assert_eq!(desk.layers()[1].in_time, 3., "the unselected layer stays");
    // Earlier than the composition's start is allowed, as a drag allows it.
    desk.select(&["b"]);
    for _ in 0..4 {
        desk.undo();
    }
    assert_eq!(desk.document(), before);
    desk.select(&["a"]);
    desk.press("alt+pagedown", 0.);
    let moved = desk.layers()[0].in_time;
    let again = desk.reopened();
    assert_eq!(
        pm_kernel::model::list_layers(again.published.snapshot.root().unwrap(), None)[0].in_time,
        moved,
        "saved and reopened"
    );
    // The keys repeat while held.
    let held = pm_shell::keyboard::shipped()
        .into_iter()
        .filter(|b| b.command == "nudgeLayerTime")
        .all(|b| b.repeat);
    assert!(held);
}

#[test]
fn the_new_editing_rows_are_in_the_menus_with_their_keys() {
    let menus = pm_shell::menu::menus();
    let row = |menu: &str, command: &str, args: serde_json::Value| {
        let (_, items) = menus.iter().find(|(title, _)| *title == menu).unwrap();
        let args = args.as_array().cloned().unwrap();
        items
            .iter()
            .find(|i| i.command == command && i.args == args)
            .unwrap_or_else(|| panic!("{menu} has no {command}"))
            .clone()
    };
    // Edit lists Ripple Delete without a key equivalent (AppKit would take
    // Shift-Delete from a text field); the Layer menu shows the key.
    assert_eq!(row("Edit", "rippleDelete", json!([])).chord, "");
    assert_eq!(
        row("Layer", "rippleDelete", json!([])).chord,
        "shift+backspace"
    );
    for (frames, chord) in [
        (-1, "alt+pageup"),
        (1, "alt+pagedown"),
        (-10, "alt+shift+pageup"),
        (10, "alt+shift+pagedown"),
    ] {
        assert_eq!(row("Layer", "nudgeLayerTime", json!([frames])).chord, chord);
    }
    assert_eq!(
        row("Timeline", "timeline.addKeyframe", json!([])).chord,
        "alt+k"
    );
    for property in ["p", "s", "r", "t", "a"] {
        let item = row(
            "Timeline",
            &format!("timeline.keyProperty:{property}"),
            json!([]),
        );
        assert_eq!(item.chord, format!("alt+shift+{property}"));
    }
}

#[test]
fn the_palette_lists_commands_with_arguments_and_commands_outside_the_menus() {
    let rows = pm_shell::menu::palette_rows();
    let find = |label: &str| {
        rows.iter()
            .find(|(_, _, l)| l == label)
            .unwrap_or_else(|| panic!("no palette row {label}"))
            .clone()
    };
    // A row with arguments carries them to the command it sends.
    for (label, menu, command, args) in [
        ("Forward 10 Frames", "Timeline", "stepFrames", json!([10])),
        ("Back 10 Frames", "Timeline", "stepFrames", json!([-10])),
        (
            "Nudge Keyframes Later",
            "Timeline",
            "nudgeKeyframes",
            json!([1]),
        ),
        (
            "Move Ten Frames Earlier",
            "Layer",
            "nudgeLayerTime",
            json!([-10]),
        ),
        ("Align Left", "Layer", "alignLayers", json!(["left"])),
        (
            "Distribute Vertically",
            "Layer",
            "distributeLayers",
            json!(["vertical"]),
        ),
    ] {
        let (category, row, _) = find(label);
        assert_eq!(category, menu, "{label}");
        let (name, sent) = pm_shell::menu::split_palette_command(&row);
        assert_eq!(
            (name, serde_json::Value::from(sent)),
            (command, args),
            "{label}"
        );
    }
    // The viewer's toggles and Skimming have no row in the shell's menus.
    for (label, command) in [
        ("Show Safe Areas", "viewer.safeAreas"),
        ("Show Guides", "viewer.guides"),
        ("Popout Viewer", "viewer.popout"),
        ("Toggle Proxies", "viewer.toggleProxies"),
        ("Skimming", "toggleSkimming"),
        ("Audio Skimming", "toggleAudioSkimming"),
    ] {
        let (category, row, _) = find(label);
        assert_eq!((category, row.as_str()), ("View", command));
        if let Some(action) = crate::viewer2::command_action(command) {
            let _ = action;
        } else {
            assert!(
                command.starts_with("toggle"),
                "{command} has no viewer action"
            );
        }
    }
    // Safe areas and guides have After Effects' keys.
    let keys = pm_shell::keyboard::shipped();
    let chord = |command: &str| {
        keys.iter()
            .find(|b| b.command == command)
            .map(|b| b.chord.as_str())
    };
    assert_eq!(chord("viewer.safeAreas"), Some("'"));
    assert_eq!(chord("viewer.guides"), Some("cmd+;"));
    // No row is listed twice.
    let mut commands: Vec<&String> = rows.iter().map(|(_, c, _)| c).collect();
    let all = commands.len();
    commands.sort();
    commands.dedup();
    assert_eq!(commands.len(), all);
}

#[test]
fn align_and_distribute_run_on_the_selection_as_one_undo_step() {
    let at = |id: &str, x: f64| {
        let mut layer = shape(id);
        layer["p"]["position.x"]["v"] = json!(x);
        layer
    };
    let desk = Desk::new(project(vec![at("a", 100.), at("b", 150.), at("c", 500.)]));
    let before = desk.document();
    let x = |desk: &Desk, id: &str| evaluate(&desk.published(), id, "position.x", 0.);
    desk.select(&["a", "b", "c"]);
    // Equal gaps: the outer two stay and the middle one goes halfway.
    desk.run("distributeLayers", &[json!("horizontal")], 0.);
    assert_eq!(
        (x(&desk, "a"), x(&desk, "b"), x(&desk, "c")),
        (100., 300., 500.)
    );
    assert_eq!(
        desk.published().history.undo_label.as_deref(),
        Some("Distribute Layers")
    );
    desk.undo();
    assert_eq!(desk.document(), before);
    // Fewer than three layers: nothing to distribute, and the row is off.
    desk.select(&["a", "b"]);
    let published = desk.published();
    assert!(command_plan("distributeLayers", &[json!("horizontal")], &published, 0.).is_none());
    let off =
        crate::layer_commands::disabled(Some(&published), &desk.layers(), &Default::default());
    assert!(off.contains("distributeLayers") && !off.contains("alignLayers"));
    // Align: the selection's box meets the composition's edge, each layer
    // moving by the same amount.
    desk.run("alignLayers", &[json!("left")], 0.);
    let (a, b) = (x(&desk, "a"), x(&desk, "b"));
    assert!(a < 100., "moved left: {a}");
    assert_eq!(b - a, 50., "the pair keeps its spacing");
    assert_eq!(
        desk.published().history.undo_label.as_deref(),
        Some("Align Layers")
    );
    desk.run("alignLayers", &[json!("right")], 0.);
    assert!(x(&desk, "b") > 150.);
    desk.undo();
    desk.undo();
    assert_eq!(desk.document(), before);
}

#[test]
fn import_folder_is_live_in_the_file_menu_and_keeps_the_folders_as_media_folders() {
    // File ▸ Import Folder… is a live row whenever a project is open.
    let (_, file) = pm_shell::menu::menus()
        .into_iter()
        .find(|(menu, _)| *menu == "File")
        .unwrap();
    assert!(
        file.iter()
            .any(|i| i.command == "importFolder" && i.label == "Import Folder…")
    );
    let desk = Desk::new(project(vec![shape("a")]));
    let off = |published: Option<&Published>| {
        crate::layer_commands::disabled(published, &[], &Default::default())
    };
    assert!(!off(Some(&desk.published())).contains("importFolder"));
    assert!(off(None).contains("importFolder"));

    // What the folder sheet hands the importer: one folder with media, a
    // subfolder with media and a subfolder with none.
    let source = tempfile::tempdir().unwrap();
    let shots = source.path().join("Shots");
    std::fs::create_dir_all(shots.join("Plates")).unwrap();
    std::fs::create_dir_all(shots.join("Notes")).unwrap();
    let still = crate::mcp_services::png(&[200, 40, 40, 255], 1, 1).unwrap();
    std::fs::write(shots.join("wide.png"), &still).unwrap();
    std::fs::write(shots.join("Plates/close.png"), &still).unwrap();
    std::fs::write(shots.join("Notes/read.txt"), b"not media").unwrap();
    // The project without the kernel's record of retained bytes, which an
    // undone import keeps so that Redo still has its files.
    let authored = |desk: &Desk| {
        let mut document = desk.document();
        document
            .as_object_mut()
            .unwrap()
            .remove("nativeMediaVersions");
        document
    };
    let before = authored(&desk);
    let panels = pm_builtins::Builtins::new(desk.kernel(), &desk.session.project, || {}).unwrap();
    let outcome = panels
        .import_media(
            desk.published().snapshot.doc_id,
            desk.dir.path(),
            pm_builtins::media_import::ImportRequest {
                paths: vec![shots],
                keep_folders: true,
                ..Default::default()
            },
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(outcome.imported, ["wide.png", "close.png"]);
    let read = |published: &Published| {
        let root = published.snapshot.root().unwrap().to_value();
        let named = |map: &str, name: &str| {
            root.get(map)
                .and_then(Value::as_object)
                .and_then(|m| {
                    m.iter()
                        .map(|(_, v)| v)
                        .find(|v| v.get("name").and_then(Value::as_str) == Some(name))
                })
                .cloned()
        };
        let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).map(str::to_owned);
        let shots = named("assetFolders", "Shots").expect("the chosen folder is a media folder");
        let plates = named("assetFolders", "Plates").expect("its subfolder is one too");
        assert!(
            named("assetFolders", "Notes").is_none(),
            "no media, no folder"
        );
        assert_eq!(text(&shots, "parent"), None);
        assert_eq!(text(&plates, "parent"), text(&shots, "id"));
        let wide = named("assets", "wide.png").unwrap();
        let close = named("assets", "close.png").unwrap();
        assert_eq!(text(&wide, "folder"), text(&shots, "id"));
        assert_eq!(text(&close, "folder"), text(&plates, "id"));
    };
    read(&desk.published());
    // Media lands in the Project panel only: no layer per file.
    assert_eq!(desk.layers().len(), 1);
    read(&desk.reopened().published);
    // One undo step for the folders and their media.
    desk.undo();
    assert_eq!(authored(&desk), before);
    desk.redo();
    read(&desk.published());
}

#[test]
fn pasting_a_copied_picture_or_file_makes_a_layer_at_the_playhead_stored_with_the_project() {
    // Command-V reaches Paste, which asks the pasteboard for media before
    // the app's own clipboard (`Controller::paste_clipboard`).
    let (command, args) = crate::layer_commands::commands_tests::dispatch("cmd+v");
    assert_eq!(route(&command, &args), Some(Route::Paste));
    let desk = Desk::new(project(vec![shape("a")]));
    let authored = |desk: &Desk| {
        let mut document = desk.document();
        document
            .as_object_mut()
            .unwrap()
            .remove("nativeMediaVersions");
        document
    };
    let before = authored(&desk);
    let panels = pm_builtins::Builtins::new(desk.kernel(), &desk.session.project, || {}).unwrap();
    let import = |request: pm_builtins::media_import::ImportRequest| {
        panels
            .import_media(
                desk.published().snapshot.doc_id,
                desk.dir.path(),
                request,
                &std::sync::atomic::AtomicBool::new(false),
            )
            .unwrap()
    };
    // A screenshot on the pasteboard: staged as a PNG, imported at 2 s.
    let picture = crate::mcp_services::png(&[9; 4 * 32 * 18], 32, 18).unwrap();
    let (dir, path) = crate::media_import::stage_pasted_image(picture, true).unwrap();
    let outcome = import(pm_builtins::media_import::ImportRequest {
        paths: vec![path],
        place_at: Some(2.),
        staged: Some(dir),
        ..Default::default()
    });
    assert_eq!(outcome.imported, ["Pasted Image.png"]);
    let read = |published: &Published, id: &str, asset_name: &str| {
        let layer = layer_of(published, id);
        let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).map(str::to_owned);
        assert_eq!(text(&layer, "type").as_deref(), Some("image"));
        assert_eq!(layer.get("from").and_then(Value::as_f64), Some(2.));
        let asset = text(layer.get("d").unwrap(), "asset").unwrap();
        let root = published.snapshot.root().unwrap().to_value();
        let record = root.get("assets").and_then(|a| a.get(&asset)).unwrap();
        assert_eq!(text(record, "name").as_deref(), Some(asset_name));
        // The bytes are kept by the project, as an import keeps them.
        assert!(
            published.media.get(&asset).is_some(),
            "{asset} has its bytes"
        );
        text(record, "sourcePath")
    };
    let pasted = desk.selected().pop().expect("the pasted layer is selected");
    // It came from the pasteboard: there is no file on disk to relink to.
    assert_eq!(read(&desk.published(), &pasted, "Pasted Image.png"), None);
    assert_eq!(desk.layers().len(), 2);
    read(&desk.reopened().published, &pasted, "Pasted Image.png");
    // One undo step removes the layer and its media; redo brings both back.
    desk.undo();
    assert_eq!(authored(&desk), before);
    desk.redo();
    read(&desk.published(), &pasted, "Pasted Image.png");

    // A file copied in Finder: the same, from the file where it lies.
    let source = tempfile::tempdir().unwrap();
    let still = source.path().join("Plate.png");
    std::fs::write(
        &still,
        crate::mcp_services::png(&[200, 40, 40, 255], 1, 1).unwrap(),
    )
    .unwrap();
    let notes = source.path().join("notes.txt");
    std::fs::write(&notes, b"words").unwrap();
    assert!(crate::media_import::pasted_files_hold_media(&[
        notes.clone(),
        still.clone()
    ]));
    // Copied files with no media among them are left to the app's own paste.
    assert!(!crate::media_import::pasted_files_hold_media(&[notes]));
    import(pm_builtins::media_import::ImportRequest {
        paths: vec![still.clone()],
        place_at: Some(2.),
        ..Default::default()
    });
    let pasted = desk.selected().pop().unwrap();
    assert_eq!(
        read(&desk.published(), &pasted, "Plate.png").as_deref(),
        Some(&*still.to_string_lossy())
    );
}
