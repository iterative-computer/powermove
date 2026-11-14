//! Headless save timing on a disposable project copy.
//! Usage: save_timing COPY.pmv LAYER_ID [SAVES]
//! Opens the copy as a file session (journals and backups beside it), then
//! repeats: rename LAYER_ID, save in place; prints each save's wall time.
//! `save_timing --compare COPY.pmv LAYER_ID`: each save's plan is written by
//! the writer that rebuilt the whole history and by step texts, timed, and
//! the two files compared byte for byte (the converting save, a second
//! save, a save after an edit, after an undo, after a redo).
//! Set PM_SAVE_TRACE=1 for the writer's phase times. Never run on a user file.
use pm_kernel::session::{SessionSource, ShellCall, ShellReply};
use pm_kernel::{Call, Caller, Command, KernelActor, KernelConfig, Request, WriteBody};
use std::time::Instant;

fn main() {
    // The kernel refuses blocking waits on the main thread.
    std::thread::spawn(run).join().unwrap();
}
fn run() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--compare") {
        return compare(std::path::Path::new(&args[2]), &args[3]);
    }
    let (file, layer) = (std::path::PathBuf::from(&args[1]), args[2].clone());
    let saves: usize = args.get(3).and_then(|n| n.parse().ok()).unwrap_or(3);
    let dir = file.parent().unwrap().join("save-timing");
    let actor = KernelActor::spawn(KernelConfig {
        journal_root: Some(dir.join("journals")),
        backup_root: Some(dir.join("backups")),
        ..Default::default()
    });
    let k = actor.handle();
    let started = Instant::now();
    let session = match k
        .shell(ShellCall::Open {
            id: "P".into(),
            source: SessionSource::File(file.clone()),
            electron_profile: None,
        })
        .wait()
        .unwrap()
        .unwrap()
    {
        ShellReply::Opened { session, .. } => session,
        other => panic!("{other:?}"),
    };
    println!("open: {:.0} ms", started.elapsed().as_secs_f64() * 1000.);
    let mut session = session;
    for i in 0..saves {
        k.call(
            "P",
            Request::new(
                Caller::native(),
                Call::Transact {
                    label: Some(format!("Rename {i}")),
                    body: WriteBody::Commands(vec![Command::SetLayerName {
                        layer_id: layer.clone(),
                        name: format!("Save timing {i}"),
                    }]),
                    expected_revision: None,
                },
            ),
        )
        .wait()
        .unwrap()
        .unwrap();
        let t = Instant::now();
        let plan = k
            .prepare_save(session.clone(), None)
            .wait()
            .unwrap()
            .unwrap();
        let prepared = t.elapsed();
        let completion = plan.run();
        let ran = t.elapsed();
        match k.finish_save(completion).wait().unwrap().unwrap() {
            ShellReply::Saved { session: s, .. } => session = s,
            other => panic!("{other:?}"),
        }
        println!(
            "save {}: {:.0} ms (prepare {:.0}, run {:.0}, finish {:.0}); file {} bytes",
            i + 1,
            t.elapsed().as_secs_f64() * 1000.,
            prepared.as_secs_f64() * 1000.,
            (ran - prepared).as_secs_f64() * 1000.,
            (t.elapsed() - ran).as_secs_f64() * 1000.,
            std::fs::metadata(&file).map_or(0, |m| m.len())
        );
    }
}

fn compare(file: &std::path::Path, layer: &str) {
    use pm_kernel::project_io::save::ReferenceWriter;
    use pm_kernel::project_io::{OpenOptions, SaveOptions, SaveTarget, open_project};
    use pm_kernel::{DocumentStore, Origin};
    let twin = file.with_extension("twin.pmv");
    let plain = file.with_extension("plain.pmv");
    let witness = std::env::var_os("PM_HISTORY_WITNESS").map(std::path::PathBuf::from);
    let t = Instant::now();
    let mut store = DocumentStore::open(open_project(file, OpenOptions::default()).unwrap());
    println!(
        "open (as given): {:.0} ms",
        t.elapsed().as_secs_f64() * 1000.
    );
    // What the app runs on another thread after an open.
    let t = Instant::now();
    store.save_warmup()();
    println!(
        "warm-up after open: {:.0} ms",
        t.elapsed().as_secs_f64() * 1000.
    );
    let mut round = 0;
    let mut step = |store: &mut DocumentStore, what: &str| {
        let plan = store
            .prepare_save(SaveTarget::InPlace, SaveOptions::default())
            .unwrap();
        for other in [&twin, &plain] {
            let _ = std::fs::remove_file(other);
            std::fs::copy(plan.destination(), other).unwrap();
        }
        let ours = plan.retargeted(&twin).with_reference_writer(None);
        let unpacked = plan
            .retargeted(&plain)
            .with_reference_writer(Some(ReferenceWriter::Unpacked));
        let reference = plan.with_reference_writer(Some(ReferenceWriter::Packed));
        let timed = |label: &str, plan: pm_kernel::project_io::SavePlan| {
            eprintln!("-- {what}: {label}");
            let t = Instant::now();
            let receipt = plan.run().unwrap();
            (receipt, t.elapsed().as_secs_f64() * 1000.)
        };
        let (_, before) = timed("unpacked writer (before)", unpacked);
        let (receipt, reference_ms) = timed("pack(to_saved()) rebuilt", reference);
        let (_, after) = timed("packed writer (after)", ours);
        let same = std::fs::read(file).unwrap() == std::fs::read(&twin).unwrap();
        let size = |p: &std::path::Path| std::fs::metadata(p).map_or(0, |m| m.len());
        println!(
            "{what}: before {before:.0} ms ({} bytes), after {after:.0} ms ({} bytes), rebuilt reference {reference_ms:.0} ms; written == pack(to_saved()): {same}",
            size(&plain),
            size(&twin)
        );
        assert!(same, "{what}: the files differ");
        if let Some(dir) = &witness {
            round += 1;
            std::fs::create_dir_all(dir).unwrap();
            let member = |p: &std::path::Path| {
                pm_model::container::read_container(
                    &std::fs::File::open(p).unwrap(),
                    pm_model::container::ReadOptions::default(),
                )
                .unwrap()
                .document
                .get("history")
                .cloned()
                .unwrap_or(pm_model::Value::Null)
            };
            let write = |name: String, v: &pm_model::Value| {
                std::fs::write(dir.join(name), pm_model::json::to_canonical_string(v)).unwrap()
            };
            write(format!("{round}-packed.json"), &member(&twin));
            write(format!("{round}-plain.json"), &member(&plain));
        }
        store.finish_save(receipt).unwrap();
    };
    let rename = |store: &mut DocumentStore, name: &str| {
        store
            .execute(
                Command::SetLayerName {
                    layer_id: layer.to_owned(),
                    name: name.to_owned(),
                },
                Origin::Interface,
            )
            .unwrap();
    };
    step(&mut store, "converting save");
    step(&mut store, "second save");
    rename(&mut store, "Compare one");
    step(&mut store, "first save after an edit");
    rename(&mut store, "Compare two");
    step(&mut store, "a later save");
    rename(&mut store, "Compare three");
    step(&mut store, "another later save");
    store.undo().unwrap();
    step(&mut store, "after an undo");
    store.redo().unwrap();
    step(&mut store, "after a redo");
    // Remove one embedded picture: its media is then only the history's, and
    // the history is written plain (the old app restores media from it).
    let embedded: Vec<String> = store
        .media()
        .iter()
        .map(|m| m.id.to_string_lossy())
        .collect();
    let listed = store
        .root()
        .and_then(|r| r.get("assets"))
        .map(|a| a.to_value())
        .unwrap_or(pm_model::Value::Null);
    if let Some(id) = embedded.iter().find(|id| listed.get(id.as_str()).is_some()) {
        store
            .execute(
                pm_kernel::MediaCommand::RemoveAsset {
                    asset_id: id.clone(),
                }
                .into(),
                Origin::Interface,
            )
            .unwrap();
        step(&mut store, "a save that falls back to the plain history");
        rename(&mut store, "Compare four");
        step(&mut store, "a later save, still plain");
    }
    drop(store);
    for (label, path) in [("packed", file), ("unpacked", plain.as_path())] {
        let t = Instant::now();
        let reopened = DocumentStore::open(open_project(path, OpenOptions::default()).unwrap());
        println!(
            "reopen {label}: {:.0} ms, {} steps",
            t.elapsed().as_secs_f64() * 1000.,
            reopened.history().records().len()
        );
    }
    let _ = std::fs::remove_file(&twin);
}
