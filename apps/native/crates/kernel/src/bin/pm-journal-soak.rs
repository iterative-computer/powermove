//! Headless P1 process-crash probe. Only generated synthetic documents are used.
//! Run: cargo run -p pm-kernel --bin pm-journal-soak -- --rounds 1000 --seed 71
use pm_kernel::{
    Call, Caller, CompCommand, HistoryCall, Kernel, KernelConfig, Origin, Reply, Request,
    commands::comps::CompPatch,
    journal::{self, JournalConfig, Recovery},
    library::LibraryStore,
    model::to_canonical,
    persistence::DiskFormat,
    session::{SessionSource, ShellCall, ShellReply},
};
use pm_model::json::{Value, parse, to_canonical_string};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::process::ExitStatusExt,
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const BASE: &str = r##"{"id":"p","name":"Synthetic","compId":"c","w":320,"h":180,"fps":30,"dur":2,"bg":"#000000","assets":{},"layers":[],"comps":{},"future":{"exact":0.1234567890123456}}"##;

#[derive(Clone, Copy)]
enum Step {
    Edit(usize),
    Begin,
    Foreign,
    Commit,
    Autosave,
    Discard,
}
impl Step {
    fn phase(self) -> &'static str {
        match self {
            Self::Edit(_) => "transaction",
            Self::Begin => "begin",
            Self::Foreign => "fold-foreign",
            Self::Commit => "fold-commit",
            Self::Autosave => "autosave",
            Self::Discard => "discard",
        }
    }
}
fn steps() -> Vec<Step> {
    let mut s: Vec<_> = (0..20).map(Step::Edit).collect();
    s.extend([
        Step::Begin,
        Step::Edit(20),
        Step::Foreign,
        Step::Commit,
        Step::Autosave,
        Step::Edit(21),
        Step::Discard,
    ]);
    s
}
fn document(k: &Kernel) -> Value {
    k.project("p")
        .unwrap()
        .store()
        .snapshot()
        .document
        .to_value()
        .unwrap()
}
fn edit(k: &mut Kernel, i: usize, foreign: bool) {
    let caller = if foreign {
        Caller::realm(7, 1, Origin::Extension)
    } else {
        Caller::native()
    };
    let ack = k
        .handle(
            "p",
            Request::new(
                caller,
                Call::Transact {
                    label: Some(format!("Synthetic {i}")),
                    body: vec![
                        CompCommand::Set {
                            patch: Box::new(CompPatch {
                                name: Some(format!("Edit {i} {}", "x".repeat(16384))),
                                fps: Some(if i.is_multiple_of(2) { 29.97 } else { 23.976 }),
                                width: Some(320.0 + i as f64),
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
        .unwrap()
        .reply
        .unwrap();
    assert!(matches!(ack.reply, Reply::Applied { changed: true, .. }));
    assert!(ack.notice.is_none(), "{ack:?}");
}
fn emit(kind: &str, n: usize, phase: &str, value: &Value) {
    let mut out = std::io::stdout().lock();
    writeln!(out, "{kind} {n} {phase} {}", to_canonical_string(value)).unwrap();
    out.flush().unwrap();
}
fn child(root: &Path) {
    let mut k = Kernel::new(KernelConfig {
        journal_root: Some(root.join("journals")),
        backup_root: None,
        journal: JournalConfig {
            checkpoint_revisions: 3,
            max_segment_bytes: 32768,
            ..Default::default()
        },
    });
    k.shell(ShellCall::AttachLibrary {
        profile: root.into(),
        format: DiskFormat::V1_2,
        electron_profile: None,
    })
    .unwrap();
    let session = match k
        .shell(ShellCall::Open {
            id: "p".into(),
            source: SessionSource::New(parse(BASE).unwrap()),
            electron_profile: None,
        })
        .unwrap()
    {
        ShellReply::Opened { session, .. } => session,
        other => panic!("{other:?}"),
    };
    k.shell(ShellCall::Save(session.clone())).unwrap();
    k.durability_barrier().unwrap();
    let mut input = std::io::stdin().lock();
    for (n, step) in steps().into_iter().enumerate() {
        let before = document(&k);
        // Compute the only permitted next document independently of journal replay.
        let candidate = match step {
            Step::Edit(_) | Step::Foreign => {
                let i = match step {
                    Step::Edit(i) => i,
                    _ => 99,
                };
                let mut next = before.clone();
                let m = next.as_object_mut().unwrap();
                m.insert(
                    "name",
                    Value::from(format!("Edit {i} {}", "x".repeat(16384))),
                );
                let comp = m
                    .get_mut("comps")
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .get_mut("c")
                    .unwrap()
                    .as_object_mut()
                    .unwrap();
                comp.insert(
                    "fps",
                    Value::Number(if i % 2 == 0 { 29.97 } else { 23.976 }),
                );
                comp.insert("w", Value::Number(320.0 + i as f64));
                next
            }
            _ => before.clone(),
        };
        let phase = if n == 21 { "fold-owner" } else { step.phase() };
        emit("PRE", n, phase, &candidate);
        let mut line = String::new();
        assert!(input.read_line(&mut line).unwrap() > 0);
        assert_eq!(line.trim(), "go");
        match step {
            Step::Edit(i) => edit(&mut k, i, false),
            Step::Foreign => edit(&mut k, 99, true),
            Step::Begin => {
                k.handle(
                    "p",
                    Request::new(
                        Caller::native(),
                        HistoryCall::Begin {
                            label: "Fold".into(),
                        },
                    ),
                )
                .unwrap();
            }
            Step::Commit => {
                k.handle(
                    "p",
                    Request::new(Caller::native(), HistoryCall::Commit { label: None }),
                )
                .unwrap();
            }
            Step::Autosave => {
                k.shell(ShellCall::Save(session.clone())).unwrap();
            }
            Step::Discard => {
                k.shell(ShellCall::DiscardClose(session.clone())).unwrap();
            }
        }
        let after = if matches!(step, Step::Discard) {
            before
        } else {
            document(&k)
        };
        assert_eq!(after, candidate, "candidate oracle at {n}");
        emit("ACK", n, phase, &after);
    }
    // Stay alive until SIGKILL even after the final acknowledgment.
    let mut line = String::new();
    let _ = input.read_line(&mut line);
}
struct Reap(Child);
impl Drop for Reap {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn next(seed: &mut u64) -> u64 {
    *seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    *seed
}
fn recovered(root: &Path) -> Value {
    let store = LibraryStore::open(root);
    let base = to_canonical(store.load("p").unwrap().unwrap().project);
    let project = root.join("journals/p");
    // DiscardClose archives by rename. Its rejected document remains recoverable.
    let location = if journal::sessions(&project).is_empty() && project.join("discarded").exists() {
        project.join("discarded")
    } else {
        project
    };
    match journal::recover(&location, None, Some(&base)) {
        Recovery::Recovered(r) | Recovery::Copy { recovered: r, .. } => {
            r.document.to_value().unwrap()
        }
        Recovery::Nothing => base,
        other => panic!("recovery failed: {other:?}"),
    }
}
fn run(rounds: usize, mut seed: u64) {
    let initial_seed = seed;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("pm-journal-soak-{}-{stamp}", std::process::id()));
    fs::create_dir(&root).unwrap();
    for round in 0..rounds {
        let dir = root.join(round.to_string());
        fs::create_dir(&dir).unwrap();
        let targets = [(next(&mut seed) % 20) as usize, 21, 22, 23, 24, 26];
        let target = targets[round % targets.len()];
        let timing = (round / targets.len()) % 3; // before, in-flight, acknowledged
        let delay = next(&mut seed) % 2500;
        let mut child = Reap(
            Command::new(std::env::current_exe().unwrap())
                .arg("--child")
                .arg(&dir)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        let stdout = child.0.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 {
                    break;
                }
                // A killed process may leave a partial protocol line. It never
                // constitutes a delivered acknowledgment.
                if !line.ends_with('\n') {
                    break;
                }
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let mut last = to_canonical(parse(BASE).unwrap());
        let mut candidate = last.clone();
        let mut phase = String::new();
        let mut saved_before_discard = None;
        loop {
            let line = rx
                .recv_timeout(Duration::from_secs(15))
                .expect("child progress timeout");
            let parts: Vec<_> = line.trim_end().splitn(4, ' ').collect();
            assert_eq!(parts.len(), 4, "{line}");
            let n: usize = parts[1].parse().unwrap();
            let value = parse(parts[3]).unwrap();
            if parts[0] == "ACK" {
                last = value;
                if n == target && timing == 2 {
                    break;
                }
            } else {
                assert_eq!(parts[0], "PRE");
                if n == target {
                    candidate = value;
                    phase = parts[2].into();
                    if target == 26 {
                        saved_before_discard =
                            Some(fs::read(dir.join("store/project.p.json")).unwrap());
                    }
                    if timing == 0 {
                        break;
                    }
                }
                child.0.stdin.as_mut().unwrap().write_all(b"go\n").unwrap();
                if n == target && timing == 1 {
                    std::thread::sleep(Duration::from_micros(delay));
                    break;
                }
            }
        }
        child.0.kill().unwrap();
        let status = child.0.wait().unwrap();
        assert_eq!(
            status.signal(),
            Some(libc::SIGKILL),
            "child must be killed, not fail on its own"
        );
        reader.join().unwrap();
        // Include every complete acknowledgment emitted before death.
        for line in rx.try_iter() {
            let p: Vec<_> = line.trim_end().splitn(4, ' ').collect();
            if p[0] == "ACK" {
                last = parse(p[3]).unwrap();
            }
        }
        let actual = recovered(&dir);
        if let Some(bytes) = saved_before_discard {
            assert_eq!(
                fs::read(dir.join("store/project.p.json")).unwrap(),
                bytes,
                "DiscardClose changed the saved base"
            );
        }
        assert!(
            actual == last || (timing == 1 && actual == candidate),
            "seed={initial_seed} round={round} phase={phase} timing={timing} delay_us={delay}; preserved fixture: {}",
            dir.display()
        );
        if timing == 2 && target == 26 {
            assert!(journal::sessions(&dir.join("journals/p")).is_empty());
            assert!(!journal::sessions(&dir.join("journals/p/discarded")).is_empty());
        }
        fs::remove_dir_all(&dir).unwrap();
        println!("PASS round={round} phase={phase} timing={timing} delay_us={delay}");
    }
    fs::remove_dir(&root).unwrap();
    println!("PASS rounds={rounds} seed={initial_seed}");
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--child") {
        child(Path::new(&args[2]));
        return;
    }
    let mut rounds = 18;
    let mut seed = 71;
    for pair in args[1..].chunks(2) {
        assert_eq!(
            pair.len(),
            2,
            "usage: pm-journal-soak [--rounds N] [--seed N]"
        );
        match pair[0].as_str() {
            "--rounds" => rounds = pair[1].parse().unwrap(),
            "--seed" => seed = pair[1].parse().unwrap(),
            other => panic!("unknown option {other}"),
        }
    }
    assert!(rounds > 0);
    run(rounds, seed);
}
