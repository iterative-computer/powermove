//! N3/N4: synthetic v1.2 and gzip v1.3 library inputs, edited natively and
//! reopened with the actual tagged project/history readers (development only).
mod common;
use pm_kernel::{
    commands::comps::CompPatch,
    library::LibraryStore,
    persistence::{DiskFormat, encode_disk_value},
    session::{SessionSource, ShellCall, ShellReply},
    *,
};
use pm_model::json::{Value, parse, to_canonical_string};
use std::{fs, path::Path, process::Command as Process};

fn oracle(root: &Path, tag: &str, expected: &Value) {
    fs::write(root.join("expected.json"), to_canonical_string(expected)).unwrap();
    let runtime = std::env::var_os("PM_TEST_JS_RUNTIME").unwrap_or_else(|| "node".into());
    let result = Process::new(runtime)
        .arg(common::crate_dir().join("tests/library_roundtrip_oracle.mjs"))
        .arg(root)
        .arg(tag)
        .current_dir(common::crate_dir().join("../../../.."))
        .output()
        .expect("Node with stripTypeScriptTypes (or Bun) is required for tagged parity");
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
fn expected(root: &Path) -> Value {
    let p = LibraryStore::open(root).load("P").unwrap().unwrap();
    let mut m = pm_model::Map::new();
    m.insert("project", p.project);
    m.insert("history", p.history.unwrap());
    Value::Object(m)
}
#[test]
fn v12_v13_library_roundtrip_through_tagged_readers_keeps_authored_values() {
    for (format, tag) in [(DiskFormat::V1_2, "v1.2.0"), (DiskFormat::V1_3, "v1.3.0")] {
        let dir = common::TempDir::new("library-roundtrip");
        fs::create_dir(dir.path("store")).unwrap();
        let mut project = parse(r##"{"id":"P","name":"Synthetic","compId":"C","compName":"Main","w":640,"h":360,"fps":23.976,"dur":7.123456789012345,"bg":"#123456","assets":{},"comps":{},"layers":[{"id":"L","type":"shape","from":0.1234567890123456,"dur":2.987654321098765,"p":{"position.x":{"v":0.1234567890123456,"kf":[],"expr":null}},"d":{"text":"Unicode: 🧭 é"}}],"future":{"kept":true}}"##).unwrap();
        project
            .as_object_mut()
            .unwrap()
            .get_mut("future")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("opaque", Value::from("synthetic".repeat(6000)));
        if format == DiskFormat::V1_3 {
            project
                .as_object_mut()
                .unwrap()
                .insert("revision", Value::Number(100.0));
        }
        let mut slot = pm_model::Map::new();
        slot.insert("v", Value::from(tag.trim_start_matches('v')));
        slot.insert("proj", project.clone());
        if format == DiskFormat::V1_3 {
            slot.insert("recoverySequence", Value::Number(7.0));
            // A stale sequence must not replay, while a newer sequence with a
            // rewound edit revision must apply. The tagged v1.3 reader is oracle.
            fs::write(dir.path("store/projectJournal.P.json"), r#"[
                {"sequence":6,"revision":101,"patches":[{"path":["fps"],"exists":true,"value":120}]},
                {"sequence":8,"revision":1,"patches":[{"path":["fps"],"exists":true,"value":23.976}]}
            ]"#).unwrap();
        }
        let input = encode_disk_value(&Value::Object(slot), format).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&input).contains("__powermoveDiskValue"),
            format == DiskFormat::V1_3
        );
        fs::write(dir.path("store/project.P.json"), input).unwrap();
        fs::write(
            dir.path("store/projects.json"),
            r#"[{"id":"P","name":"Synthetic"}]"#,
        )
        .unwrap();
        fs::write(
            dir.path("store/projectHistory.P.json"),
            r#"{"version":1,"paths":[],"index":0,"entries":[]}"#,
        )
        .unwrap();
        // Verify the old format before editing, independently in the tagged code.
        let loaded = LibraryStore::open(&dir.0).load("P").unwrap().unwrap();
        assert_eq!(loaded.project.get("fps"), Some(&Value::Number(23.976)));
        if format == DiskFormat::V1_3 {
            assert_eq!((loaded.journal_applied, loaded.journal_skipped), (1, 1));
        }
        oracle(&dir.0, tag, &expected(&dir.0));
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
        k.handle(
            "P",
            Request::new(
                Caller::native(),
                Call::Transact {
                    label: Some("Exact rate".into()),
                    body: vec![
                        CompCommand::Set {
                            patch: Box::new(CompPatch {
                                fps: Some(29.97),
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
        let saved = expected(&dir.0);
        let p = saved.get("project").unwrap();
        assert_eq!(p.get("fps"), Some(&Value::Number(29.97)));
        assert_eq!(p.get("dur"), project.get("dur"));
        assert_eq!(p.get("layers"), project.get("layers"));
        assert_eq!(p.get("future"), project.get("future"));
        oracle(&dir.0, tag, &saved);
        // Rollback-compatible v1.2 is also read by v1.3. Gzip is intentionally
        // opt-in: v1.2 has no gzip decoder (N4).
        if format == DiskFormat::V1_2 {
            oracle(&dir.0, "v1.3.0", &saved);
        }
        let history = fs::read(dir.path("store/projectHistory.P.json")).unwrap();
        let slot_bytes = fs::read(dir.path("store/project.P.json")).unwrap();
        // Canonical history export is byte stable across a native reopen/save.
        k.shell(ShellCall::Close(session)).unwrap();
        drop(k);
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
        assert_eq!(
            k.project("P")
                .unwrap()
                .store()
                .root()
                .unwrap()
                .get("comps")
                .unwrap()
                .get("C")
                .unwrap()
                .get("fps")
                .unwrap()
                .to_value(),
            Value::Number(29.97)
        );
        k.shell(ShellCall::Save(session.clone())).unwrap();
        assert_eq!(
            fs::read(dir.path("store/project.P.json")).unwrap(),
            slot_bytes
        );
        assert_eq!(
            fs::read(dir.path("store/projectHistory.P.json")).unwrap(),
            history
        );
        assert_eq!(expected(&dir.0), saved);
        k.handle("P", Request::new(Caller::native(), HistoryCall::Undo))
            .unwrap();
        k.shell(ShellCall::Save(session.clone())).unwrap();
        let undone = expected(&dir.0);
        assert_eq!(
            undone.get("project").unwrap().get("fps"),
            Some(&Value::Number(23.976))
        );
        oracle(&dir.0, tag, &undone);
        k.handle("P", Request::new(Caller::native(), HistoryCall::Redo))
            .unwrap();
        k.shell(ShellCall::Save(session)).unwrap();
        assert_eq!(expected(&dir.0), saved);
        assert_eq!(
            fs::read(dir.path("store/projectHistory.P.json")).unwrap(),
            history
        );
    }
}
