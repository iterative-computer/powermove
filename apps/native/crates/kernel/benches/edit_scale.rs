//! Typical edits on synthetic large projects, timed on the kernel thread as
//! the app runs it: a `KernelActor` with a journal, the project opened from a
//! saved `.pmv`. Prints open time and, per edit kind, the median and worst
//! `Published::actor_work` (handle + publish) and the caller's round trip.
//!
//! cargo bench -p pm-kernel --features synthetic --bench edit_scale [-- 5000 20000]
//!
//! Target: a typical edit under one 60 Hz frame (16.7 ms) on the kernel thread.
use std::time::{Duration, Instant};

use pm_kernel::commands::properties::SetProperty;
use pm_kernel::commands::{KeyframeCommand, PropertyCommand};
use pm_kernel::project_io::{OpenOptions, SaveOptions, SaveTarget, open_project};
use pm_kernel::*;

const FRAME: Duration = Duration::from_micros(16_667);
const RUNS: usize = 25;

fn edit(kind: &str, layer: &str, n: usize) -> Command {
    match kind {
        "move" => PropertyCommand::Set(SetProperty::new(
            layer,
            "position.x",
            pm_model::Value::Number(100.0 + n as f64),
            1.0,
        ))
        .into(),
        "keyframe" => KeyframeCommand::Add {
            layer_id: layer.into(),
            channel: "rotation".into(),
            time: 0.5 + n as f64 * 0.1,
            key_id: format!("bench{n}"),
            value: Some(pm_model::Value::Number(n as f64)),
        }
        .into(),
        "rename" => Command::SetLayerName {
            layer_id: layer.into(),
            name: format!("Renamed {n}"),
        },
        "toggle" => Command::SetLayerVisible {
            layer_id: layer.into(),
            visible: n % 2 == 1,
        },
        _ => unreachable!(),
    }
}

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

fn run(layers: usize) -> bool {
    let dir = std::env::temp_dir().join(format!("pm-edit-scale-{}-{layers}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("scale.pmv");
    let mut store = DocumentStore::new_untitled(pm_kernel::synthetic::project(layers));
    let plan = store
        .prepare_save(
            SaveTarget::SaveAs {
                path: path.clone(),
                replace: true,
            },
            SaveOptions::default(),
        )
        .unwrap();
    store.finish_save(plan.run().unwrap()).unwrap();
    drop(store);
    let size = std::fs::metadata(&path).unwrap().len();

    let actor = KernelActor::spawn(KernelConfig {
        journal_root: Some(dir.join("journals")),
        backup_root: Some(dir.join("backups")),
        ..Default::default()
    });
    let kernel = actor.handle();
    let started = Instant::now();
    let opened = open_project(&path, OpenOptions::default()).unwrap();
    let read = started.elapsed();
    kernel
        .open("scale", OpenSource::File(opened))
        .wait()
        .unwrap()
        .unwrap();
    let open = started.elapsed();
    println!(
        "{layers} layers ({:.1} MB): read {:.0} ms, open {:.0} ms",
        size as f64 / 1e6,
        read.as_secs_f64() * 1e3,
        open.as_secs_f64() * 1e3
    );

    let ids = pm_kernel::synthetic::main_layer_ids(layers);
    let targets = [ids[ids.len() / 2].clone(), "c7L3".to_owned()];
    let mut ok = true;
    for kind in ["move", "keyframe", "rename", "toggle"] {
        for target in &targets {
            let (mut actor_work, mut round) = (vec![], vec![]);
            for n in 0..RUNS {
                let t = Instant::now();
                let ack = kernel.call_blocking(
                    "scale",
                    Request::new(
                        Caller::native(),
                        Call::Transact {
                            label: Some(kind.into()),
                            body: WriteBody::Commands(vec![edit(kind, target, n)]),
                            expected_revision: None,
                        },
                    ),
                );
                round.push(t.elapsed());
                assert!(
                    !matches!(ack.as_ref().map(|a| &a.reply), Ok(Reply::Throw(_)) | Err(_)),
                    "{kind} {target}: {ack:?}"
                );
                actor_work.push(kernel.published("scale").unwrap().actor_work);
            }
            let worst = *actor_work.iter().max().unwrap();
            let med = median(actor_work);
            ok &= med < FRAME;
            println!(
                "  {kind:<8} {target:<6} kernel median {:>7.2} ms  worst {:>7.2} ms  round trip median {:>7.2} ms{}",
                med.as_secs_f64() * 1e3,
                worst.as_secs_f64() * 1e3,
                median(round).as_secs_f64() * 1e3,
                if med < FRAME { "" } else { "  OVER A FRAME" }
            );
        }
    }
    actor.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
    ok
}

fn main() {
    let sizes: Vec<usize> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    let sizes = if sizes.is_empty() {
        vec![5000, 20_000]
    } else {
        sizes
    };
    // `wait` refuses the process's main thread (the app's rule): bench on a worker.
    let ok = std::thread::spawn(move || sizes.into_iter().filter(|n| !run(*n)).count() == 0)
        .join()
        .unwrap();
    if !ok {
        std::process::exit(1);
    }
}
