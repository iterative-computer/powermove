//! A project's saves as the app makes them, timed. Never run on a user file.
//! Usage: save_sequence COPY.pmv LAYER_ID
//! Opens the copy through the kernel actor (journals and backups beside it,
//! the post-open warm-up as the app runs it), saves it with no edit (the
//! converting save of an old file), then renames LAYER_ID and saves, four
//! times; then reopens the saved file. PM_SAVE_TRACE=1 adds the writer's
//! phases.
use pm_kernel::session::{SessionSource, ShellCall, ShellReply};
use pm_kernel::{Call, Caller, Command, KernelActor, KernelConfig, Request, WriteBody};
use std::time::Instant;

fn main() {
    // The kernel refuses blocking waits on the main thread.
    std::thread::spawn(run).join().unwrap();
}

fn run() {
    let args: Vec<String> = std::env::args().collect();
    let (file, layer) = (std::path::PathBuf::from(&args[1]), args[2].clone());
    let dir = file.parent().unwrap().join("save-sequence");
    let actor = KernelActor::spawn(KernelConfig {
        journal_root: Some(dir.join("journals")),
        backup_root: Some(dir.join("backups")),
        ..Default::default()
    });
    let k = actor.handle();
    let ms = |t: Instant| t.elapsed().as_secs_f64() * 1000.;
    let t = Instant::now();
    let mut session = match k
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
    println!("open: {:.0} ms", ms(t));
    let size = || std::fs::metadata(&file).map_or(0, |m| m.len());
    let mut save = |what: &str| {
        eprintln!("-- {what}");
        let t = Instant::now();
        let plan = k
            .prepare_save(session.clone(), None)
            .wait()
            .unwrap()
            .unwrap();
        let prepared = ms(t);
        let completion = plan.run();
        match k.finish_save(completion).wait().unwrap().unwrap() {
            ShellReply::Saved { session: s, .. } => session = s,
            other => panic!("{other:?}"),
        }
        println!(
            "{what}: {:.0} ms (prepare {prepared:.0}); file {} bytes",
            ms(t),
            size()
        );
    };
    save("converting save, no edit");
    for (i, what) in [
        "first save after an edit",
        "later save 1",
        "later save 2",
        "later save 3",
    ]
    .into_iter()
    .enumerate()
    {
        k.call(
            "P",
            Request::new(
                Caller::native(),
                Call::Transact {
                    label: Some(format!("Rename {i}")),
                    body: WriteBody::Commands(vec![Command::SetLayerName {
                        layer_id: layer.clone(),
                        name: format!("Save sequence {i}"),
                    }]),
                    expected_revision: None,
                },
            ),
        )
        .wait()
        .unwrap()
        .unwrap();
        save(what);
    }
    drop(k);
    drop(actor);
    let t = Instant::now();
    let reopened = pm_kernel::DocumentStore::open(
        pm_kernel::project_io::open_project(&file, Default::default()).unwrap(),
    );
    println!(
        "reopen saved file: {:.0} ms, {} steps",
        ms(t),
        reopened.history().records().len()
    );
}
