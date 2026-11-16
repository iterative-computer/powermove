//! PM-392: opening a project is not an edit. Whatever an open writes to the
//! library (view state, a format upgrade) leaves the row's edit time (`at`)
//! as it was, so Home does not call a project restored at launch "Edited just
//! now"; a save after an edit moves it.
mod common;
use pm_kernel::{
    commands::comps::CompPatch,
    library::LibraryStore,
    persistence::{DiskFormat, encode_disk_value},
    session::{SessionSource, ShellCall, ShellReply},
    *,
};
use pm_model::json::{Value, parse};
use std::fs;

const EDITED: f64 = 1_000_000.0;

fn at(dir: &std::path::Path) -> Option<f64> {
    let rows = LibraryStore::open(dir).read("projects").unwrap().unwrap();
    rows.as_array()?
        .iter()
        .find(|r| r.get("id").and_then(Value::as_str) == Some("P"))?
        .get("at")
        .and_then(Value::as_f64)
}

#[test]
fn an_open_keeps_the_edit_time_and_a_save_after_an_edit_moves_it() {
    for format in [DiskFormat::V1_2, DiskFormat::V1_3] {
        let dir = common::TempDir::new("open-keeps-edit-time");
        fs::create_dir(dir.path("store")).unwrap();
        let project = parse(
            r##"{"id":"P","name":"Kept","compId":"C","compName":"Main","w":640,"h":360,"fps":30,"dur":5,"bg":"#000000","assets":{},"comps":{},"layers":[]}"##,
        )
        .unwrap();
        let mut slot = pm_model::Map::new();
        slot.insert("v", Value::from("1.2.0"));
        slot.insert("proj", project);
        fs::write(
            dir.path("store/project.P.json"),
            encode_disk_value(&Value::Object(slot), format).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.path("store/projects.json"),
            format!(r#"[{{"id":"P","name":"Kept","at":{EDITED}}}]"#),
        )
        .unwrap();
        fs::write(
            dir.path("store/projectHistory.P.json"),
            r#"{"version":1,"paths":[],"index":0,"entries":[]}"#,
        )
        .unwrap();
        let mut k = Kernel::new(KernelConfig::default());
        k.shell(ShellCall::AttachLibrary {
            profile: dir.0.clone(),
            format,
            electron_profile: None,
        })
        .unwrap();
        let session = match k
            .shell(ShellCall::Open {
                id: "P".into(),
                source: SessionSource::Library {
                    profile: dir.0.clone(),
                    project: "P".into(),
                },
                electron_profile: None,
            })
            .unwrap()
        {
            ShellReply::Opened { session, .. } => session,
            other => panic!("{other:?}"),
        };
        k.shell(ShellCall::Close(session.clone())).unwrap();
        assert_eq!(at(&dir.0), Some(EDITED), "{format:?}: opened and closed unchanged");
        // An edit saved is an edit: the time moves on.
        let session = match k
            .shell(ShellCall::Open {
                id: "P".into(),
                source: SessionSource::Library {
                    profile: dir.0.clone(),
                    project: "P".into(),
                },
                electron_profile: None,
            })
            .unwrap()
        {
            ShellReply::Opened { session, .. } => session,
            other => panic!("{other:?}"),
        };
        k.handle(
            "P",
            Request::new(
                Caller::native(),
                Call::Transact {
                    label: Some("Rate".into()),
                    body: vec![
                        CompCommand::Set {
                            patch: Box::new(CompPatch {
                                fps: Some(24.),
                                ..Default::default()
                            }),
                        }
                        .into(),
                    ]
                    .into(),
                    expected_revision: None,
                },
            ),
        )
        .unwrap();
        k.shell(ShellCall::Save(session.clone())).unwrap();
        k.shell(ShellCall::Close(session)).unwrap();
        assert!(at(&dir.0).is_some_and(|t| t > EDITED), "{format:?}: saved after an edit");
    }
}
