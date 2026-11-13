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
