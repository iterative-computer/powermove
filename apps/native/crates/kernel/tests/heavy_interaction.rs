//! Interaction budgets for a synthetic document with deeply nested groups,
//! large vector paths, and large saved undo records.
mod common;

use std::time::{Duration, Instant};

use pm_kernel::commands::properties::SetProperty;
use pm_kernel::model::list_layers;
use pm_kernel::project_io::{OpenedProject, ProjectFile};
use pm_kernel::*;
use pm_model::{Document, Value, json::parse, patch::Patch, patch::patches_to_value, path};

const SIZE: usize = 2_000;
const GROUPS: usize = 200;
static FIXTURE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn project() -> OpenedProject {
    let geometry = format!("[{}0]", "12.34567,".repeat(27_778));
    let layer_metadata = "x".repeat(8_500);
    let mut layers = Vec::with_capacity(SIZE);
    for index in 0..SIZE {
        let group = if index == 0 {
            String::new()
        } else {
            format!(r#","group":"L{}""#, (index - 1).min(GROUPS - 1))
        };
        let shape = if index >= SIZE - 5 {
            format!(r#","d":{{"paths":[{{"id":"outline","vertices":{geometry}}}]}}"#)
        } else {
            String::new()
        };
        let kind = if index < GROUPS { "group" } else { "shape" };
        layers.push(format!(
            r#"{{"id":"L{index}","type":"{kind}","name":"Layer {index}","from":0,"dur":2,"on":true,"fixtureData":"{layer_metadata}","p":{{"position.x":{{"v":0,"kf":[]}}}}{group}{shape}}}"#,
        ));
    }
    let document = Document::new(Some(parse(&format!(
        r#"{{"id":"fixture","name":"Fixture","w":320,"h":180,"fps":30,"dur":2,"assets":{{}},"comps":{{}},"layers":[{}]}}"#,
        layers.join(",")
    )).unwrap()), 1);
    let record = |n| {
        let payload = Value::from(format!("{n:03}{}", "a".repeat(70_000)));
        patches_to_value(&[Patch::set(path!["testPayload"], payload)])
    };
    let entries = (0..100)
        .map(|n| {
            let mut entry = pm_model::Map::new();
            entry.insert("label", Value::from("Edit"));
            entry.insert("forward", record(n + 1));
            entry.insert("backward", record(n));
            Value::Object(entry)
        })
        .collect();
    let mut saved = pm_model::Map::new();
    saved.insert("version", Value::Number(1.));
    saved.insert("paths", Value::from("composition"));
    saved.insert("index", Value::Number(99.));
    saved.insert("entries", Value::Array(entries));
    OpenedProject {
        file: ProjectFile::untitled(),
        document,
        open_comp: None,
        history: Some(History::import(&Value::Object(saved), None).unwrap()),
    }
}

fn set(value: f64) -> WriteBody {
    WriteBody::Commands(vec![
        PropertyCommand::Set(SetProperty::new(
            "L1999",
            "position.x",
            Value::Number(value),
            1.,
        ))
        .into(),
    ])
}

fn call(kernel: &KernelHandle, call: impl Into<Call>) -> Reply {
    let reply = kernel
        .call_blocking("fixture", Request::new(Caller::native(), call.into()))
        .unwrap()
        .reply;
    assert!(!matches!(reply, Reply::Throw(_)), "{reply:?}");
    reply
}

fn select(kernel: &KernelHandle, index: usize) {
    call(
        kernel,
        SelectionCall::Layers {
            ids: vec![format!("L{index}")],
            add: false,
        },
    );
}

fn median(mut times: Vec<Duration>) -> Duration {
    times.sort_unstable();
    times[times.len() / 2]
}

#[test]
fn large_project_interaction_budgets() {
    let _fixture = FIXTURE.lock().unwrap();
    let actor = KernelActor::spawn(KernelConfig::default());
    let kernel = actor.handle();
    kernel
        .open("fixture", OpenSource::File(project()))
        .wait()
        .unwrap()
        .unwrap();
    let view = kernel.published("fixture").unwrap();
    assert_eq!(view.history.entries, 100);
    let root = view.snapshot.document.root().unwrap();

    let listing = median(
        (0..15)
            .map(|_| {
                let start = Instant::now();
                assert_eq!(std::hint::black_box(list_layers(root, None)).len(), SIZE);
                start.elapsed()
            })
            .collect(),
    );
    let selection = median(
        (0..15)
            .map(|n| {
                let start = Instant::now();
                select(&kernel, SIZE - n - 1);
                start.elapsed()
            })
            .collect(),
    );
    let edit = median(
        (1..16)
            .map(|n| {
                call(
                    &kernel,
                    Call::Transact {
                        label: None,
                        body: set(n as f64),
                        expected_revision: None,
                    },
                );
                kernel.published("fixture").unwrap().actor_work
            })
            .collect(),
    );
    let Reply::Gesture(id) = call(
        &kernel,
        GestureCall::Begin {
            label: "Drag".into(),
        },
    ) else {
        panic!("no drag lease");
    };
    let gesture = median(
        (17..32)
            .map(|n| {
                call(
                    &kernel,
                    GestureCall::Step {
                        gesture: id,
                        body: set(n as f64),
                        replace: true,
                    },
                );
                kernel.published("fixture").unwrap().actor_work
            })
            .collect(),
    );
    call(&kernel, GestureCall::Commit { gesture: id });
    let mut mirror = QueryMirror::new(&kernel.published("fixture").unwrap());
    let query = median(
        (32..47)
            .map(|n| {
                call(
                    &kernel,
                    EditCall::Dispatch {
                        body: set(n as f64),
                    },
                );
                let published = kernel.published("fixture").unwrap();
                let start = Instant::now();
                mirror.sync(&published);
                assert_eq!(
                    mirror
                        .queries()
                        .evaluate(None, "L1999", "position.x", 1.)
                        .unwrap(),
                    Value::Number(n as f64),
                );
                start.elapsed()
            })
            .collect(),
    );
    let undo_redo = median(
        (0..15)
            .map(|_| {
                let start = Instant::now();
                call(&kernel, HistoryCall::Undo);
                call(&kernel, HistoryCall::Redo);
                start.elapsed()
            })
            .collect(),
    );
    eprintln!(
        "median listing {listing:?}, selection {selection:?}, edit {edit:?}, gesture {gesture:?}, query {query:?}, undo+redo {undo_redo:?}"
    );
    actor.shutdown();

    assert!(
        selection < Duration::from_micros(500),
        "selection {selection:?}"
    );
    assert!(edit < Duration::from_millis(2), "edit {edit:?}");
    assert!(gesture < Duration::from_millis(2), "gesture {gesture:?}");
    assert!(listing < Duration::from_millis(2), "listing {listing:?}");
}

#[test]
fn file_save_and_checkpoint_leave_the_actor_available() {
    use std::sync::{Arc, Condvar, Mutex, mpsc};

    use pm_kernel::session::{SessionSource, ShellCall, ShellReply};

    let _fixture = FIXTURE.lock().unwrap();
    let dir = common::TempDir::new("heavy-interaction");
    let source = dir.path("project.pmv");
    let destination = dir.path("copy.pmv");
    let other = dir.path("other.pmv");
    let mut store = DocumentStore::open(project());
    common::save_as(&mut store, &source).unwrap();
    std::fs::copy(&source, &other).unwrap();
    drop(store);

    let checkpoints = Arc::new((Mutex::new(false), Condvar::new()));
    let (writing, reached) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let wait = Mutex::new(wait);
    let actor = KernelActor::spawn(KernelConfig {
        journal_root: Some(dir.path("journals")),
        lock_root: Some(dir.path("locks")),
        journal: JournalConfig {
            hold_checkpoints: Some(checkpoints.clone()),
            ..Default::default()
        },
        publish_hooks: Some(project_io::PublishHooks {
            before_install: Some(Arc::new(move |_| {
                let _ = writing.send(());
                let _ = wait.lock().unwrap().recv_timeout(Duration::from_secs(10));
            })),
            ..Default::default()
        }),
        ..Default::default()
    });
    let kernel = actor.handle();
    let ShellReply::Opened { session, .. } = kernel
        .shell(ShellCall::Open {
            id: "fixture".into(),
            source: SessionSource::File(source),
            electron_profile: None,
        })
        .wait()
        .unwrap()
        .unwrap()
    else {
        panic!("no open file session");
    };

    struct Hold(Arc<(Mutex<bool>, Condvar)>);
    impl Drop for Hold {
        fn drop(&mut self) {
            let (held, resume) = &*self.0;
            *held.lock().unwrap() = false;
            resume.notify_all();
        }
    }

    *checkpoints.0.lock().unwrap() = true;
    let hold = Hold(checkpoints);
    call(
        &kernel,
        Call::Transact {
            label: None,
            body: set(10.),
            expected_revision: None,
        },
    );
    let pending = kernel.prepare_save(session, Some(destination.clone()));
    let deadline = Instant::now() + Duration::from_secs(10);
    let save = loop {
        if let Some(save) = pending.try_take() {
            break save.unwrap();
        }
        assert!(
            Instant::now() < deadline,
            "save preparation stalled behind the checkpoint"
        );
        std::thread::sleep(Duration::from_millis(1));
    };
    let writer = std::thread::spawn(move || save.run());
    reached.recv_timeout(Duration::from_secs(10)).unwrap();
    // An open waiting for durability also leaves the actor free.
    let opening = kernel.shell(ShellCall::PrepareOpen {
        id: "other".into(),
        source: SessionSource::File(other),
        electron_profile: None,
    });

    let (answered, completion) = mpsc::channel();
    let during_save = kernel.clone();
    let editor = std::thread::spawn(move || {
        select(&during_save, SIZE - 1);
        let reply = call(&during_save, EditCall::Dispatch { body: set(20.) });
        answered.send(reply).unwrap();
    });
    let answer = completion.recv_timeout(Duration::from_millis(500));
    let _ = release.send(());
    drop(hold);
    editor.join().unwrap();
    assert!(matches!(
        answer.unwrap(),
        Reply::Edit(EditResult { ok: true, .. })
    ));
    assert!(matches!(
        opening.wait().unwrap().unwrap(),
        ShellReply::Opened { .. }
    ));
    kernel
        .finish_save(writer.join().unwrap())
        .wait()
        .unwrap()
        .unwrap();

    // The snapshot went to disk. The concurrent change remains live and dirty.
    let saved = open_project(&destination, OpenOptions::default()).unwrap();
    let mut queries = QueryContext::new(&saved.document);
    assert_eq!(
        queries.evaluate(None, "L1999", "position.x", 1.).unwrap(),
        Value::Number(10.)
    );
    let live = kernel.published("fixture").unwrap();
    assert!(live.dirty);
    let mut queries = QueryContext::new(&live.snapshot.document);
    assert_eq!(
        queries.evaluate(None, "L1999", "position.x", 1.).unwrap(),
        Value::Number(20.)
    );
    actor.shutdown();
}
