//! Hostile project files never crash or hang the kernel (stability item 2).
//!
//! Each case goes through the app's open path (`ShellCall::PrepareOpen` on a
//! `KernelActor` with the app's journal and backup roots). A project that
//! opens is then used the way the editor uses it: every property of every
//! layer evaluated at the start, middle and end, an edit, undo, redo, Save As
//! and close. The verdict per case is one of: opened, or refused with a
//! readable message. A dead kernel (a panic) or a call that never answers
//! (a hang) fails the test, and afterwards the same kernel must still open a
//! good project.

mod hostile;

use pm_kernel::session::{SessionSource, ShellCall, ShellReply};
use pm_kernel::{
    Call, Caller, Command, HistoryCall, KernelActor, KernelConfig, KernelHandle, Pending,
    QueryMirror, Request, WriteBody,
};
use pm_model::NodeKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long one kernel answer may take (`PM_HOSTILE_ANSWER_S` overrides).
fn answer_limit() -> Duration {
    std::env::var("PM_HOSTILE_ANSWER_S")
        .ok()
        .and_then(|s| s.parse().ok())
        .map_or(Duration::from_secs(60), Duration::from_secs)
}

#[derive(Debug)]
#[allow(dead_code)] // fields are read through Debug in the failure list
enum Outcome {
    Opened,
    Refused(String),
    Died(&'static str),
    Hung(&'static str),
}

/// Waits for a kernel answer on a helper thread, so a hang is a verdict.
fn answer<T: Send + 'static>(p: Pending<T>, what: &'static str) -> Result<T, Outcome> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(p.wait());
    });
    match rx.recv_timeout(answer_limit()) {
        Ok(Some(v)) => Ok(v),
        Ok(None) => Err(Outcome::Died(what)),
        Err(_) => Err(Outcome::Hung(what)),
    }
}

fn spawn(profile: &Path) -> KernelActor {
    KernelActor::spawn(KernelConfig {
        journal_root: Some(profile.join("journals")),
        backup_root: Some(profile.join("backups")),
        ..Default::default()
    })
}

/// Every `p` channel of every layer in every composition.
fn channels(root: &pm_model::Node) -> Vec<(Option<String>, String, String)> {
    let mut out = vec![];
    let mut visit = |comp: Option<String>, layers: Option<&pm_model::Node>| {
        let Some(NodeKind::Array(layers)) = layers.map(|l| l.kind()) else {
            return;
        };
        for layer in layers.iter() {
            let id = match layer.get("id").map(|n| n.kind()) {
                Some(NodeKind::String(s)) => s.as_str().unwrap_or_default().to_owned(),
                _ => continue,
            };
            if let Some(NodeKind::Object(p)) = layer.get("p").map(|n| n.kind()) {
                for key in p.keys() {
                    if let Some(k) = key.as_str() {
                        out.push((comp.clone(), id.clone(), k.to_owned()));
                    }
                }
            }
        }
    };
    visit(None, root.get("layers"));
    if let Some(NodeKind::Object(comps)) = root.get("comps").map(|n| n.kind()) {
        for (id, comp) in comps.iter() {
            visit(id.as_str().map(str::to_owned), comp.get("layers"));
        }
    }
    out
}

fn exercise(kernel: &KernelHandle, path: &Path, out_dir: &Path) -> Outcome {
    let t = std::time::Instant::now();
    let stage = |name: &str| {
        if std::env::var_os("PM_HOSTILE_STAGES").is_some() {
            eprintln!("  {name} at {:?}", t.elapsed());
        }
    };
    let reply = match answer(
        kernel.shell(ShellCall::PrepareOpen {
            id: "H".into(),
            source: SessionSource::File(path.to_owned()),
            electron_profile: None,
        }),
        "open",
    ) {
        Ok(r) => r,
        Err(o) => return o,
    };
    let session = match reply {
        Ok(ShellReply::Opened { session, .. }) => session,
        Ok(other) => return Outcome::Refused(format!("unexpected reply {other:?}")),
        Err(e) => return Outcome::Refused(e.to_string()),
    };
    stage("opened");
    let Some(published) = kernel.published("H") else {
        return Outcome::Died("publish");
    };
    // What the editor does on the main thread: mirror and evaluate.
    let (tx, rx) = std::sync::mpsc::channel();
    let p = published.clone();
    std::thread::spawn(move || {
        let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut mirror = QueryMirror::new(&p);
            let root = p.snapshot.root().cloned();
            let _ = p.snapshot.layers(None);
            let _ = p.snapshot.render_comp();
            if let Some(root) = root {
                for (comp, layer, channel) in channels(&root).into_iter().take(4000) {
                    for t in [0.0, 1.0, 1e9, -1.0] {
                        let _ = mirror
                            .queries()
                            .evaluate(comp.as_deref(), &layer, &channel, t);
                    }
                }
            }
        }));
        let _ = tx.send(ok.is_ok());
    });
    match rx.recv_timeout(answer_limit()) {
        Ok(true) => {}
        Ok(false) => return Outcome::Died("evaluate (UI query mirror panicked)"),
        Err(_) => return Outcome::Hung("evaluate"),
    }
    stage("evaluated");
    let first = published
        .snapshot
        .layers(None)
        .into_iter()
        .find(|l| !l.locked);
    if let Some(layer) = first {
        let edit = Call::Transact {
            label: None,
            body: WriteBody::Commands(vec![Command::SetStaticOpacity {
                layer_id: layer.id.clone(),
                value: 50.0,
            }]),
            expected_revision: None,
        };
        for call in [
            Request::new(Caller::native(), edit),
            Request::new(Caller::native(), HistoryCall::Undo),
            Request::new(Caller::native(), HistoryCall::Redo),
        ] {
            if let Err(o) = answer(kernel.call("H", call), "edit/undo/redo") {
                return o;
            }
        }
    }
    stage("edited");
    let save = kernel.shell(ShellCall::SaveAs {
        session: session.clone(),
        path: out_dir.join("saved.pmv"),
        replace: true,
    });
    if let Err(o) = answer(save, "save as") {
        return o;
    }
    let session = kernel
        .published("H")
        .and_then(|p| p.session.clone())
        .map(|s| s.handle)
        .unwrap_or(session);
    stage("saved");
    if let Err(o) = answer(kernel.shell(ShellCall::DiscardClose(session)), "close") {
        return o;
    }
    stage("closed");
    Outcome::Opened
}

fn good_project(dir: &Path) -> PathBuf {
    let path = dir.join("good.pmv");
    let seed = &hostile::seeds()[0].1;
    std::fs::write(&path, serde_json::to_vec(seed).unwrap()).unwrap();
    path
}

/// A refusal a person can act on: words, not a debug dump.
fn readable(message: &str) -> bool {
    !message.trim().is_empty()
        && !message.contains("unexpected reply")
        && !message.contains("panicked")
        && message.len() < 600
}

#[test]
fn hostile_projects_open_or_refuse_without_crashing_or_hanging() {
    // `PM_HOSTILE_ONLY=<substring>` runs the matching cases only.
    let only = std::env::var("PM_HOSTILE_ONLY").ok();
    let cases: Vec<_> = hostile::cases(12)
        .into_iter()
        .filter(|c| only.as_ref().is_none_or(|o| c.name.contains(o.as_str())))
        .collect();
    let root = std::env::temp_dir().join(format!("pm-hostile-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let mut failures = vec![];
    let (mut opened, mut refused) = (0, 0);
    let mut messages = std::collections::BTreeMap::new();
    for case in &cases {
        let dir = root.join(&case.name);
        std::fs::create_dir_all(&dir).unwrap();
        let path = hostile::write(case, &dir);
        let profile = dir.join("profile");
        let actor = spawn(&profile);
        let kernel = actor.handle();
        let started = std::time::Instant::now();
        let outcome = exercise(&kernel, &path, &dir);
        if started.elapsed() > Duration::from_secs(2) {
            eprintln!("slow: {} took {:?}", case.name, started.elapsed());
        }
        match &outcome {
            Outcome::Opened => opened += 1,
            Outcome::Refused(m) if readable(m) => {
                refused += 1;
                *messages.entry(m.clone()).or_insert(0usize) += 1;
            }
            other => failures.push(format!("{}: {other:?}", case.name)),
        }
        // The same kernel still works afterwards.
        if !matches!(outcome, Outcome::Died(_) | Outcome::Hung(_)) {
            let good = good_project(&dir);
            match answer(
                kernel.shell(ShellCall::PrepareOpen {
                    id: "G".into(),
                    source: SessionSource::File(good),
                    electron_profile: None,
                }),
                "open after",
            ) {
                Ok(Ok(ShellReply::Opened { .. })) => {}
                other => failures.push(format!("{}: kernel unusable after: {other:?}", case.name)),
            }
        }
        if matches!(outcome, Outcome::Hung(_)) {
            // A wedged kernel thread can't be joined; leave it behind.
            std::mem::forget(actor);
        } else {
            drop(actor);
        }
    }
    let _ = std::fs::remove_dir_all(&root);
    eprintln!(
        "hostile cases: {} total, {opened} opened, {refused} refused readably, {} failed",
        cases.len(),
        failures.len()
    );
    for (message, n) in &messages {
        eprintln!("  refused ×{n}: {message}");
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Writes the corpus for the real-app pass. Opt-in:
/// `PM_HOSTILE_OUT=<dir> cargo test -p pm-kernel --test hostile_open write_corpus -- --ignored`
#[test]
#[ignore]
fn write_corpus() {
    let out = PathBuf::from(std::env::var("PM_HOSTILE_OUT").expect("PM_HOSTILE_OUT"));
    std::fs::create_dir_all(&out).unwrap();
    let cases = hostile::cases(12);
    for case in &cases {
        hostile::write(case, &out);
    }
    eprintln!("{} cases in {}", cases.len(), out.display());
}
