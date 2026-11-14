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
    use pm_kernel::project_io::{OpenOptions, SaveOptions, SaveTarget, open_project};
    use pm_kernel::{DocumentStore, Origin};
    let twin = file.with_extension("twin.pmv");
    let mut store = DocumentStore::open(open_project(file, OpenOptions::default()).unwrap());
    // What the app runs on another thread after an open.
    let t = Instant::now();
    store.save_warmup()();
    println!(
        "warm-up after open: {:.0} ms",
        t.elapsed().as_secs_f64() * 1000.
    );
    let step = |store: &mut DocumentStore, what: &str| {
        let plan = store
            .prepare_save(SaveTarget::InPlace, SaveOptions::default())
            .unwrap();
        let _ = std::fs::remove_file(&twin);
        std::fs::copy(plan.destination(), &twin).unwrap();
        let ours = plan.retargeted(&twin).with_reference_writer(false);
        let reference = plan.with_reference_writer(true);
        eprintln!("-- {what}: reference writer");
        let t = Instant::now();
        let receipt = reference.run().unwrap();
        let old_ms = t.elapsed().as_secs_f64() * 1000.;
        eprintln!("-- {what}: step texts");
        let t = Instant::now();
        ours.run().unwrap();
        let new_ms = t.elapsed().as_secs_f64() * 1000.;
        let same = std::fs::read(file).unwrap() == std::fs::read(&twin).unwrap();
        println!(
            "{what}: reference {old_ms:.0} ms, step texts {new_ms:.0} ms, files identical: {same}"
        );
        assert!(same, "{what}: the files differ");
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
    step(&mut store, "after an edit");
    rename(&mut store, "Compare two");
    step(&mut store, "after one more edit");
    store.undo().unwrap();
    step(&mut store, "after an undo");
    store.redo().unwrap();
    step(&mut store, "after a redo");
    let _ = std::fs::remove_file(&twin);
}
