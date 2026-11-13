//! W079–W083 domain evidence through the production actor. No decoder or UI.
use crate::fixture::{Scratch, layer, without_revision};
use pm_kernel::commands::{
    pro_edit::{Edge, ProEditCommand as Edit, SplitTail, TrackTarget},
    pro_links::ProLinkCommand as Link,
    pro_source::*,
};
use pm_kernel::session::{SessionHandle, SessionSource, ShellCall, ShellReply};
use pm_kernel::{
    Call, Caller, Command, HistoryCall, KernelActor, Origin, Reply, Request, WriteBody,
};
use pm_mcp::ActorAutomation;
use serde_json::{Value, json};
type Check = Result<(), String>;
struct Project {
    host: ActorAutomation,
    session: SessionHandle,
    _actor: KernelActor,
    scratch: Scratch,
}
fn clip(id: &str, from: f64, track: i32, kind: &str) -> Value {
    json!({"id":id,"type":kind,"name":id,"from":from,"dur":4,"track":track,"on":true,
      "d":{"asset":"src","trim":{"v":from+2.0,"kf":[],"expr":null},"speed":1},
      "p":{"opacity":{"v":100,"kf":[{"i":format!("{id}-key"),"t":1,"v":25}],"expr":null}},
      "fx":[],"masks":[],"extensionData":{"retain":"authored"}})
}
impl Project {
    fn new(locked: bool, audio: bool) -> Result<Self, String> {
        let scratch = Scratch::new().map_err(|e| e.to_string())?;
        let actor = KernelActor::spawn(Default::default());
        let handle = actor.handle();
        handle
            .shell(ShellCall::AttachLibrary {
                profile: scratch.0.join("profile"),
                format: pm_kernel::persistence::DiskFormat::V1_2,
                electron_profile: None,
            })
            .wait()
            .ok_or("actor stopped")?
            .map_err(|e| format!("{e:?}"))?;
        let mut c = clip("c", 8., 0, "video");
        c["lock"] = json!(locked);
        let mut layers = vec![
            clip("a", 0., 0, "video"),
            clip("b", 4., 0, "video"),
            c,
            clip("other", 4., 2, "video"),
        ];
        if audio {
            layers.push(clip("sound", 4., 1, "audio"));
        }
        // A video asset without `timing` is a v1 one: opening it adds the Electron
        // video reading marker (`videoColor`), so the document is shaped as loaded.
        let doc = json!({"id":"P-pro","name":"Synthetic pro acceptance","compId":"C1","fps":30,"dur":40,"w":320,"h":180,"assets":{"src":{"id":"src","kind":"video","dur":100,"hasAudio":true,"videoColor":"electron"}},"layers":layers});
        let opened = handle
            .shell(ShellCall::Open {
                id: "P-pro".into(),
                source: SessionSource::New(pm_model::json::parse(&doc.to_string()).unwrap()),
                electron_profile: None,
            })
            .wait()
            .ok_or("actor stopped")?
            .map_err(|e| format!("{e:?}"))?;
        let ShellReply::Opened { session, .. } = opened else {
            return Err("project did not open".into());
        };
        Ok(Self {
            host: ActorAutomation::new(handle),
            session,
            _actor: actor,
            scratch,
        })
    }
    fn document(&self) -> Result<Value, String> {
        self.host.read_document("P-pro").map_err(|e| e.to_string())
    }
    fn call(&self, call: impl Into<Call>) -> Check {
        let ack = self
            .host
            .kernel
            .call(
                "P-pro",
                Request::new(Caller::realm(120, 1, Origin::Interface), call),
            )
            .wait()
            .ok_or("actor stopped")?
            .map_err(|e| format!("{e:?}"))?;
        if let Reply::Throw(e) = ack.reply {
            return Err(format!("{e:?}"));
        }
        Ok(())
    }
    fn edit(&self, command: impl Into<Command>) -> Check {
        self.call(Call::Transact {
            label: Some("Pro acceptance".into()),
            body: WriteBody::Commands(vec![command.into()]),
            expected_revision: Some(
                self.host
                    .inspect_project("P-pro")
                    .map_err(|e| e.to_string())?
                    .revision(),
            ),
        })
    }
    fn reject(&self, command: impl Into<Command>) -> Check {
        let before = self.document()?;
        let history = self.host.inspect_project("P-pro").unwrap().history.clone();
        assert!(self.edit(command).is_err());
        assert_eq!(self.document()?, before);
        assert_eq!(self.host.inspect_project("P-pro").unwrap().history, history);
        Ok(())
    }
    fn roundtrip(&self, command: impl Into<Command>) -> Check {
        let before = without_revision(self.document()?);
        self.edit(command)?;
        let after = without_revision(self.document()?);
        assert_ne!(after, before);
        self.call(HistoryCall::Undo)?;
        assert_eq!(without_revision(self.document()?), before);
        self.call(HistoryCall::Redo)?;
        assert_eq!(without_revision(self.document()?), after);
        let path = self
            .scratch
            .0
            .join(format!("{}.pmv", pm_kernel::new_id("synthetic-pro")));
        self.host
            .kernel
            .shell(ShellCall::SaveAs {
                session: self.session.clone(),
                path: path.clone(),
                replace: false,
            })
            .wait()
            .ok_or("actor stopped")?
            .map_err(|e| format!("{e:?}"))?;
        let opened = pm_kernel::project_io::open_project(&path, Default::default())
            .map_err(|e| e.to_string())?;
        let mut saved = pm_kernel::DocumentStore::open(opened);
        let realize = |s: &pm_kernel::DocumentStore| -> Value {
            serde_json::from_str(&pm_model::json::to_canonical_string(
                &pm_model::canonical::realize_project(s.snapshot().document.to_value().unwrap()),
            ))
            .unwrap()
        };
        assert_eq!(without_revision(realize(&saved)), after);
        saved.undo().map_err(|e| e.to_string())?;
        assert_eq!(without_revision(realize(&saved)), before);
        saved.redo().map_err(|e| e.to_string())?;
        assert_eq!(without_revision(realize(&saved)), after);
        Ok(())
    }
}
fn tracks() -> Vec<TrackTarget> {
    vec![TrackTarget::new(0)]
}
pub fn ripple_delete() -> Check {
    let p = Project::new(false, false)?;
    let other = layer(&p.document()?, "other").clone();
    p.roundtrip(Edit::RippleDelete {
        layer_ids: vec!["b".into()],
        tracks: tracks(),
        linked: true,
    })?;
    let doc = p.document()?;
    assert!(
        !doc["layers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["id"] == "b")
    );
    assert_eq!(layer(&doc, "c")["from"], 4);
    assert_eq!(layer(&doc, "other"), &other);
    Project::new(true, false)?.reject(Edit::RippleDelete {
        layer_ids: vec!["b".into()],
        tracks: tracks(),
        linked: true,
    })
}
pub fn ripple_trim() -> Check {
    for delta in [-1., 1.] {
        let p = Project::new(false, false)?;
        p.roundtrip(Edit::RippleTrim {
            layer_id: "a".into(),
            edge: Edge::Out,
            delta,
            tracks: tracks(),
            linked: true,
        })?;
        assert_eq!(
            layer(&p.document()?, "b")["from"].as_f64(),
            Some(4. + delta)
        );
        Project::new(true, false)?.reject(Edit::RippleTrim {
            layer_id: "a".into(),
            edge: Edge::Out,
            delta,
            tracks: tracks(),
            linked: true,
        })?;
    }
    Ok(())
}
pub fn roll() -> Check {
    let p = Project::new(false, false)?;
    p.roundtrip(Edit::Roll {
        left_id: "a".into(),
        right_id: "b".into(),
        delta: 1.,
        linked: true,
    })?;
    let d = p.document()?;
    assert_eq!(layer(&d, "a")["dur"], 5);
    assert_eq!(layer(&d, "b")["dur"], 3);
    assert_eq!(layer(&d, "b")["d"]["trim"]["v"], 7);
    assert_eq!(layer(&d, "b")["p"]["opacity"]["kf"][0]["t"], 0);
    p.reject(Edit::Roll {
        left_id: "a".into(),
        right_id: "b".into(),
        delta: 100.,
        linked: true,
    })
}
pub fn slide() -> Check {
    let p = Project::new(false, false)?;
    let b = layer(&p.document()?, "b").clone();
    p.roundtrip(Edit::Slide {
        previous_id: "a".into(),
        layer_id: "b".into(),
        next_id: "c".into(),
        delta: 1.,
        linked: true,
    })?;
    let d = p.document()?;
    assert_eq!(layer(&d, "b")["from"], 5);
    assert_eq!(layer(&d, "b")["dur"], 4);
    assert_eq!(layer(&d, "b")["d"], b["d"]);
    assert_eq!(layer(&d, "b")["p"], b["p"]);
    assert_eq!(layer(&d, "a")["dur"], 5);
    assert_eq!(layer(&d, "c")["from"], 9);
    Project::new(true, false)?.reject(Edit::Slide {
        previous_id: "a".into(),
        layer_id: "b".into(),
        next_id: "c".into(),
        delta: 1.,
        linked: true,
    })
}
pub fn source() -> Check {
    for mode in [SourceEditMode::Insert, SourceEditMode::Overwrite] {
        let p = Project::new(false, false)?;
        let other = layer(&p.document()?, "other").clone();
        let mut c = ProSourceCommand {
            comp_id: None,
            asset_id: "src".into(),
            points: EditPoints {
                source_in: Some(10.),
                source_out: Some(12.),
                timeline_in: Some(5.),
                ..Default::default()
            },
            mode,
            targets: vec![SourceTarget {
                media: SourceMedia::Video,
                track: TrackTarget::new(0),
                layer_id: "new".into(),
            }],
            link_id: None,
            splits: vec![SourceSplit {
                layer_id: "b".into(),
                tail_id: "tail".into(),
                tail_link_id: None,
            }],
        };
        p.roundtrip(c.clone())?;
        let d = p.document()?;
        assert_eq!(layer(&d, "new")["d"]["trim"]["v"], 10);
        assert_eq!(layer(&d, "other"), &other);
        assert_eq!(layer(&d, "tail")["from"], 7);
        assert_eq!(
            layer(&d, "tail")["d"]["trim"]["v"],
            if mode == SourceEditMode::Insert { 7 } else { 9 }
        );
        c.asset_id = "missing".into();
        p.reject(c)?;
    }
    Ok(())
}
pub fn links() -> Check {
    let p = Project::new(false, true)?;
    p.roundtrip(Link::Link {
        video_id: "b".into(),
        audio_id: "sound".into(),
        link_id: "pair".into(),
    })?;
    p.roundtrip(Edit::Move {
        layer_ids: vec!["b".into()],
        delta: 1.,
        linked: true,
    })?;
    assert_eq!(layer(&p.document()?, "sound")["from"], 5);
    p.roundtrip(Edit::Slip {
        layer_ids: vec!["b".into()],
        delta: 1.,
        linked: false,
    })?;
    assert_eq!(layer(&p.document()?, "sound")["d"]["trim"]["v"], 6);
    p.roundtrip(Edit::Split {
        layer_id: "b".into(),
        time: 7.,
        tails: vec![
            SplitTail {
                layer_id: "b".into(),
                tail_id: "bt".into(),
            },
            SplitTail {
                layer_id: "sound".into(),
                tail_id: "st".into(),
            },
        ],
        linked: true,
        tail_link_id: Some("tails".into()),
    })?;
    assert_eq!(layer(&p.document()?, "bt")["editLink"]["id"], "tails");
    assert_eq!(layer(&p.document()?, "st")["editLink"]["id"], "tails");
    p.roundtrip(Edit::Trim {
        layer_ids: vec!["b".into()],
        edge: Edge::Out,
        delta: -0.5,
        linked: true,
    })?;
    let before_cancel = p.document()?;
    p.call(pm_kernel::EditCall::Begin {
        label: "Cancelled linked gesture".into(),
    })?;
    p.edit(Edit::Move {
        layer_ids: vec!["b".into()],
        delta: 1.,
        linked: true,
    })?;
    p.call(pm_kernel::EditCall::Cancel)?;
    assert_eq!(
        without_revision(p.document()?),
        without_revision(before_cancel)
    );
    p.roundtrip(Link::Unlink {
        layer_ids: vec!["b".into()],
    })?;
    assert!(layer(&p.document()?, "sound").get("editLink").is_none());
    p.roundtrip(Edit::Delete {
        layer_ids: vec!["bt".into()],
        linked: true,
    })?;
    assert!(
        !p.document()?["layers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["id"] == "bt" || l["id"] == "st")
    );
    Ok(())
}
pub fn shuttle() -> Check {
    use pm_kernel::commands::pro_transport::*;
    let p = Project::new(false, false)?;
    let before = p.host.inspect_project("P-pro").unwrap();
    let mut state = ShuttleState {
        available: true,
        time: 4.,
        rate: ShuttleRate::Forward1,
        ..Default::default()
    };
    for (input, rate) in [
        (ShuttleInput::L, 2),
        (ShuttleInput::L, 2),
        (ShuttleInput::J, -1),
        (ShuttleInput::J, -2),
        (ShuttleInput::K, 0),
    ] {
        let ShuttleOutcome::Handled(next) = state.dispatch(input).map_err(|e| e.to_string())?
        else {
            return Err("shuttle declined while playing".into());
        };
        state = next;
        assert_eq!(state.rate.factor(), rate);
    }
    for key in [ShuttleInput::J, ShuttleInput::K, ShuttleInput::L] {
        assert_eq!(
            state.dispatch(key).map_err(|e| e.to_string())?,
            ShuttleOutcome::Declined
        );
    }
    let ShuttleOutcome::Handled(state) = state
        .dispatch(ShuttleInput::Reverse)
        .map_err(|e| e.to_string())?
    else {
        return Err("reverse declined".into());
    };
    let ui = pm_kernel::state::UiState {
        transport: state
            .transport_value(&before.ui_state.transport)
            .map_err(|e| e.to_string())?,
        ..before.ui_state.clone()
    };
    p.host
        .kernel
        .state_transaction(pm_kernel::state::StateTransaction {
            ops: vec![pm_kernel::state::StateOp::Ui {
                project: "P-pro".into(),
                next: ui,
            }],
            ..Default::default()
        })
        .wait()
        .ok_or("actor stopped")?
        .map_err(|e| format!("{e:?}"))?;
    let after = p.host.inspect_project("P-pro").unwrap();
    assert_eq!(after.revision(), before.revision());
    assert_eq!(after.history, before.history);
    assert_eq!(after.dirty, before.dirty);
    assert_eq!(
        ShuttleState::from_transport(&after.ui_state.transport, true, 30.)
            .map_err(|e| e.to_string())?,
        state
    );
    Ok(())
}
