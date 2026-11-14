//! Headless save timing on a disposable project copy.
//! Usage: save_timing COPY.pmv LAYER_ID [SAVES]
//! Opens the copy as a file session (journals and backups beside it), then
//! repeats: rename LAYER_ID, save in place; prints each save's wall time.
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
