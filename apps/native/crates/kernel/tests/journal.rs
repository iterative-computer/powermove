//! Journal, checkpoints and recovery, including the kill test (plan v6 P1
//! gate: "the journal kill test").

mod common;

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as Process, Stdio};
use std::time::Duration;

use common::TempDir;
use pm_kernel::journal::{self, JournalConfig, Recovery};
use pm_kernel::{
    Call, Caller, Command, HistoryCall, Kernel, KernelActor, KernelConfig, OpenSource, Reply,
    Request, WorkspaceCall, WriteBody, base_hash_of,
};
use pm_model::json::{Value, parse};

const PROJECT: &str = r##"{
  "id": "P", "name": "Fixture", "compId": "C1", "compName": "Main",
  "w": 200, "h": 100, "fps": 30, "dur": 2, "bg": "#000000", "assets": {},
  "layers": [
    {"id": "layer0", "type": "shape", "name": "Layer", "from": 0, "dur": 2, "on": true,
     "p": {"opacity": {"v": 100, "kf": [], "expr": null}}}
  ],
  "comps": {}
}"##;

fn config(root: &Path, checkpoint_revisions: u64) -> KernelConfig {
    KernelConfig {
        journal_root: Some(root.to_path_buf()),
        journal: JournalConfig {
            flush_interval: Duration::from_millis(50),
            checkpoint_revisions,
            max_segment_bytes: 4 * 1024 * 1024,
        },
        ..Default::default()
    }
}

fn rename(name: &str) -> Call {
    Call::Transact {
        label: Some(format!("Rename {name}")),
        body: WriteBody::Commands(vec![Command::SetLayerName {
            layer_id: "layer0".into(),
            name: name.into(),
        }]),
        expected_revision: None,
    }
}

fn handle(k: &mut Kernel, call: impl Into<Call>) -> Reply {
    k.handle("P", Request::new(Caller::native(), call))
        .unwrap()
        .reply
        .unwrap()
        .reply
}

fn layer_name(doc: &pm_model::Document) -> String {
    doc.to_value()
        .and_then(|v| {
            v.get("comps")?.get("C1")?.get("layers")?.as_array()?[0]
                .get("name")?
                .as_str()
                .map(str::to_owned)
        })
        .unwrap()
}

fn recovered(root: &Path, current_base: Option<&str>) -> pm_kernel::Recovered {
    match journal::recover(&root.join("P"), current_base, None) {
        Recovery::Recovered(r) => r,
        other => panic!("not recovered: {other:?}"),
    }
}

#[test]
fn recovery_restores_document_history_cursor_and_workspace() {
    let dir = TempDir::new("journal-roundtrip");
    let mut k = Kernel::new(config(&dir.0, 50));
    let base = parse(PROJECT).unwrap();
    let info = k.open("P", OpenSource::Untitled(base.clone())).unwrap();
    assert_eq!(info.base_hash, base_hash_of(&base));
    for name in ["a", "b", "c"] {
        handle(&mut k, rename(name));
    }
    assert!(matches!(
        handle(&mut k, HistoryCall::Undo),
        Reply::Bool(true)
    ));
    handle(
        &mut k,
        WorkspaceCall::Mutate {
            next: parse(r#"{"name":"W"}"#).unwrap(),
            threw: None,
        },
    );
    let expected_doc = k
        .project("P")
        .unwrap()
        .store()
        .snapshot()
        .document
        .to_value();
    let expected_history = k
        .project("P")
        .unwrap()
        .store()
        .history()
        .snapshot()
        .to_saved();
    k.sync_all();
    drop(k);

    let r = recovered(&dir.0, Some(&info.base_hash));
    assert!(!r.truncated);
    assert_eq!(r.document.to_value(), expected_doc);
    assert_eq!(layer_name(&r.document), "b");
    assert_eq!(r.history.snapshot().to_saved(), expected_history);
    assert_eq!(r.history.labels(), ["Rename a", "Rename b", "Rename c"]);
    assert_eq!(r.history.applied(), 2, "the cursor survives (redo kept)");
    assert_eq!(r.workspace, parse(r#"{"name":"W"}"#).unwrap());

    // Adopting the recovered work: undo and redo still work.
    let session = r.session.clone();
    let mut k = Kernel::new(config(&dir.0, 50));
    k.open(
        "P",
        OpenSource::Recovered {
            recovered: r,
            supersedes: vec![session.clone()],
        },
    )
    .unwrap();
    assert!(
        k.project("P").unwrap().store().is_dirty(),
        "recovered work is unsaved"
    );
    assert!(matches!(
        handle(&mut k, HistoryCall::Redo),
        Reply::Bool(true)
    ));
    assert_eq!(
        layer_name(&k.project("P").unwrap().store().snapshot().document),
        "c"
    );
    handle(&mut k, HistoryCall::Undo);
    handle(&mut k, HistoryCall::Undo);
    assert_eq!(
        layer_name(&k.project("P").unwrap().store().snapshot().document),
        "a"
    );
    k.sync_all();
    assert!(
        !session.exists(),
        "the superseded session goes once the new checkpoint is durable"
    );
}

#[test]
fn checkpoints_rotate_and_prune_and_recovery_matches() {
    let dir = TempDir::new("journal-rotate");
    let mut k = Kernel::new(config(&dir.0, 5));
    k.open("P", OpenSource::Untitled(parse(PROJECT).unwrap()))
        .unwrap();
    for i in 0..23 {
        handle(&mut k, rename(&format!("n{i}")));
    }
    k.sync_all();
    let session = journal::sessions(&dir.0.join("P")).pop().unwrap();
    let mut files: Vec<String> = std::fs::read_dir(&session)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    files.sort();
    assert_eq!(files, ["checkpoint-00000004.json", "journal-00000004.log"]);
    let expected = k
        .project("P")
        .unwrap()
        .store()
        .snapshot()
        .document
        .to_value();
    let r = recovered(&dir.0, None);
    assert_eq!(r.document.to_value(), expected);
    assert_eq!(r.history.len(), 23);
    assert_eq!(
        r.replayed,
        3 * 2,
        "3 renames after the last checkpoint (rev + push)"
    );
}

#[test]
fn a_torn_tail_is_ignored() {
    let dir = TempDir::new("journal-torn");
    let mut k = Kernel::new(config(&dir.0, 50));
    k.open("P", OpenSource::Untitled(parse(PROJECT).unwrap()))
        .unwrap();
    handle(&mut k, rename("kept"));
    k.sync_all();
    drop(k);
    let session = journal::sessions(&dir.0.join("P")).pop().unwrap();
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(session.join("journal-00000000.log"))
        .unwrap();
    f.write_all(b"0123456789abcdef {\"t\":\"rev\",\"b\":2,\"r\":3,\"op")
        .unwrap();
    let r = recovered(&dir.0, None);
    assert!(r.truncated);
    assert_eq!(layer_name(&r.document), "kept");
}

#[test]
fn recovery_refuses_a_different_base_and_offers_a_copy() {
    let dir = TempDir::new("journal-base");
    let mut k = Kernel::new(config(&dir.0, 50));
    k.open("P", OpenSource::Untitled(parse(PROJECT).unwrap()))
        .unwrap();
    handle(&mut k, rename("journalled"));
    k.sync_all();
    drop(k);
    // The library slot changed elsewhere since the session began.
    let mut changed = parse(PROJECT).unwrap();
    changed
        .as_object_mut()
        .unwrap()
        .insert("name", Value::from("Edited elsewhere"));
    match journal::recover(&dir.0.join("P"), Some(&base_hash_of(&changed)), None) {
        Recovery::Copy { recovered, .. } => {
            assert_eq!(layer_name(&recovered.document), "journalled")
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn without_a_checkpoint_only_the_true_base_replays() {
    let dir = TempDir::new("journal-nockpt");
    let mut k = Kernel::new(config(&dir.0, 50));
    let base = parse(PROJECT).unwrap();
    k.open("P", OpenSource::Untitled(base.clone())).unwrap();
    handle(&mut k, rename("x"));
    k.sync_all();
    drop(k);
    let session = journal::sessions(&dir.0.join("P")).pop().unwrap();
    std::fs::remove_file(session.join("checkpoint-00000000.json")).unwrap();
    let canonical = pm_kernel::model::to_canonical(base.clone());
    match journal::recover(&dir.0.join("P"), None, Some(&canonical)) {
        Recovery::Recovered(r) => assert_eq!(layer_name(&r.document), "x"),
        other => panic!("{other:?}"),
    }
    let mut wrong = canonical.clone();
    wrong
        .as_object_mut()
        .unwrap()
        .insert("name", Value::from("other"));
    assert!(matches!(
        journal::recover(&dir.0.join("P"), None, Some(&wrong)),
        Recovery::Unrecoverable { .. }
    ));
    assert!(session.exists(), "an unrecoverable journal is kept");
}

// ── the kill test ───────────────────────────────────────────────────────────

const CHILD_ENV: &str = "PM_KERNEL_KILL_CHILD";

/// The child: edits as fast as it can, printing each acknowledged revision.
fn child(dir: PathBuf) {
    let actor = KernelActor::spawn(config(&dir, 20));
    let kernel = actor.handle();
    kernel
        .open("P", OpenSource::Untitled(parse(PROJECT).unwrap()))
        .wait()
        .unwrap()
        .unwrap();
    let stdout = std::io::stdout();
    for i in 1.. {
        let ack = kernel
            .call_blocking(
                "P",
                Request::new(Caller::native(), rename(&format!("w{i}"))),
            )
            .unwrap();
        assert!(matches!(ack.reply, Reply::Applied { changed: true, .. }));
        let mut out = stdout.lock();
        writeln!(out, "ACK {i} {}", ack.revision).unwrap();
        out.flush().unwrap();
    }
}

#[test]
fn kill_test_sigkill_loses_no_acknowledged_write() {
    if let Ok(dir) = std::env::var(CHILD_ENV) {
        child(PathBuf::from(dir));
        return;
    }
    let dir = TempDir::new("journal-kill");
    let mut proc = Process::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "kill_test_sigkill_loses_no_acknowledged_write",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_ENV, &dir.0)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut last = 0u64;
    let mut lines = BufReader::new(proc.stdout.take().unwrap()).lines();
    while let Some(Ok(line)) = lines.next() {
        if let Some(rest) = line.strip_prefix("ACK ") {
            last = rest.split(' ').next().unwrap().parse().unwrap();
            if last >= 300 {
                break;
            }
        }
    }
    // Let the child keep writing (and checkpointing) at full speed, then
    // SIGKILL it wherever it is.
    std::thread::sleep(Duration::from_millis(7));
    proc.kill().unwrap();
    let status = proc.wait().unwrap();
    assert!(!status.success());
    // Drain whatever acks reached the pipe before the kill.
    for line in lines.map_while(Result::ok) {
        if let Some(rest) = line.strip_prefix("ACK ") {
            last = rest.split(' ').next().unwrap().parse().unwrap();
        }
    }
    assert!(last >= 300, "the child acknowledged {last} writes");

    let r = recovered(&dir.0, None);
    let name = layer_name(&r.document);
    let recovered_n: u64 = name.trim_start_matches('w').parse().unwrap();
    assert!(
        recovered_n >= last,
        "acknowledged w{last} but recovered only {name}"
    );
    assert_eq!(
        r.history.len() as u64,
        recovered_n,
        "every acknowledged step is undoable"
    );
    eprintln!(
        "kill test: {last} acknowledged, {recovered_n} recovered, {} lines replayed, torn tail: {}",
        r.replayed, r.truncated
    );

    // Undo after recovery.
    let mut k = Kernel::new(KernelConfig::default());
    k.open(
        "P",
        OpenSource::Recovered {
            recovered: r,
            supersedes: vec![],
        },
    )
    .unwrap();
    assert!(matches!(
        handle(&mut k, HistoryCall::Undo),
        Reply::Bool(true)
    ));
    assert_eq!(
        layer_name(&k.project("P").unwrap().store().snapshot().document),
        format!("w{}", recovered_n - 1)
    );
}
