//! `pm-glsl-fuzz`: the fuzz target of the GLSL pipeline (section 11).
//!
//! ```text
//! pm-glsl-fuzz --seeds seeds.json [--seed S] [--iterations N] [--deadline SECONDS] [--out results.jsonl]
//! pm-glsl-fuzz --bombs [--deadline SECONDS] [--out results.jsonl]
//! ```
//!
//! `seeds.json` is an array of `{id, frag, lineOffset}` (wrapped programs,
//! for example the built-in definitions or the coverage probes). The parent
//! never translates anything: a worker process (this binary with `--worker`)
//! prints `begin <i>` before each input and one JSON line after it, so a
//! crash in glslang or naga is attributed to the exact input and a hang is
//! cut off by the deadline. Exit status 1 when any input crashed, hung, or
//! panicked.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use pm_glsl::fuzz::{self, Outcome, Seed};

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

enum Inputs {
    Mutants { seeds: Vec<Seed>, seed: u64 },
    Bombs,
}

impl Inputs {
    fn from_args(args: &[String]) -> Result<Inputs, String> {
        if args.iter().any(|a| a == "--bombs") {
            return Ok(Inputs::Bombs);
        }
        let path = arg(args, "--seeds").ok_or("--seeds <file> or --bombs is required")?;
        let bytes = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
        let seeds: Vec<Seed> =
            serde_json::from_slice(&bytes).map_err(|e| format!("{path}: {e}"))?;
        if seeds.is_empty() {
            return Err(format!("{path}: no seeds"));
        }
        let seed = arg(args, "--seed").map_or(Ok(1), |s| s.parse().map_err(|_| "bad --seed"))?;
        Ok(Inputs::Mutants { seeds, seed })
    }

    fn count(&self, iterations: u64) -> u64 {
        match self {
            Inputs::Mutants { .. } => iterations,
            Inputs::Bombs => fuzz::bombs().len() as u64,
        }
    }

    fn get(&self, index: u64) -> (String, String, u32) {
        match self {
            Inputs::Mutants { seeds, seed } => fuzz::mutant(seeds, *seed, index),
            Inputs::Bombs => {
                let (name, source) = fuzz::bombs().swap_remove(index as usize);
                (source, name, 0)
            }
        }
    }
}

fn worker(args: &[String]) -> Result<(), String> {
    let inputs = Inputs::from_args(args)?;
    let start: u64 = arg(args, "--start").map_or(0, |s| s.parse().unwrap_or(0));
    let count: u64 = arg(args, "--count").map_or(1, |s| s.parse().unwrap_or(1));
    std::panic::set_hook(Box::new(|_| {}));
    let stdout = std::io::stdout();
    for index in start..start + count {
        let (source, _, offset) = inputs.get(index);
        let mut out = stdout.lock();
        let _ = writeln!(out, "begin {index}");
        let _ = out.flush();
        drop(out);
        let started = Instant::now();
        // The pipeline itself runs on a large stack, as the engine runs it.
        let outcome = std::thread::Builder::new()
            .stack_size(pm_glsl::cache::TRANSLATION_STACK)
            .spawn(move || fuzz::outcome(&source, offset))
            .map_err(|e| e.to_string())?
            .join()
            .unwrap_or(Outcome {
                result: "panic".into(),
                detail: String::new(),
            });
        let line = serde_json::json!({
            "index": index,
            "result": outcome.result,
            "detail": outcome.detail,
            "ms": started.elapsed().as_secs_f64() * 1000.0,
        });
        let mut out = stdout.lock();
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }
    Ok(())
}

fn run(args: &[String]) -> Result<bool, String> {
    let inputs = Inputs::from_args(args)?;
    let iterations: u64 = arg(args, "--iterations").map_or(2000, |s| s.parse().unwrap_or(2000));
    let deadline =
        Duration::from_secs(arg(args, "--deadline").map_or(30, |s| s.parse().unwrap_or(30)));
    let total = inputs.count(iterations);
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut results: Box<dyn Write> = match arg(args, "--out") {
        Some(path) => Box::new(std::fs::File::create(&path).map_err(|e| format!("{path}: {e}"))?),
        None => Box::new(std::io::stdout()),
    };
    let passthrough: Vec<String> = ["--seeds", "--seed"]
        .iter()
        .filter_map(|name| arg(args, name).map(|v| [name.to_string(), v]))
        .flatten()
        .chain(args.iter().filter(|a| *a == "--bombs").cloned())
        .collect();
    let mut tally: std::collections::BTreeMap<String, u64> = Default::default();
    let mut abnormal = false;
    let mut next = 0u64;
    while next < total {
        // One worker per bomb (peak memory per input); mutants share one.
        let count = if matches!(inputs, Inputs::Bombs) {
            1
        } else {
            total - next
        };
        let mut child = Command::new(&exe)
            .arg("--worker")
            .args(&passthrough)
            .args(["--start", &next.to_string(), "--count", &count.to_string()])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        let stdout = child.stdout.take().ok_or("no worker stdout")?;
        let (tx, rx) = mpsc::channel::<String>();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if tx.send(line.unwrap_or_default()).is_err() {
                    break;
                }
            }
        });
        let mut begun: Option<u64> = None;
        let mut begun_at = Instant::now();
        loop {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(line) => {
                    if let Some(i) = line.strip_prefix("begin ") {
                        begun = i.parse().ok();
                        begun_at = Instant::now();
                    } else if !line.is_empty() {
                        let result = serde_json::from_str::<serde_json::Value>(&line)
                            .ok()
                            .and_then(|v| v["result"].as_str().map(str::to_owned))
                            .unwrap_or_default();
                        abnormal |= result == "panic";
                        *tally.entry(result).or_default() += 1;
                        let _ = writeln!(results, "{line}");
                        begun = None;
                        next += 1;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if let Some(i) = begun.filter(|_| begun_at.elapsed() > deadline) {
                        let _ = child.kill();
                        let _ = child.wait();
                        let (_, note, _) = inputs.get(i);
                        let _ = writeln!(
                            results,
                            "{}",
                            serde_json::json!({"index": i, "result": "timeout", "note": note})
                        );
                        *tally.entry("timeout".into()).or_default() += 1;
                        abnormal = true;
                        next = i + 1;
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    let status = child.wait().map_err(|e| e.to_string())?;
                    if let Some(i) = begun {
                        let (_, note, _) = inputs.get(i);
                        let _ = writeln!(
                            results,
                            "{}",
                            serde_json::json!({"index": i, "result": format!("crash ({status})"), "note": note})
                        );
                        *tally.entry("crash".into()).or_default() += 1;
                        abnormal = true;
                        next = i + 1;
                    } else if next < total && !matches!(inputs, Inputs::Bombs) {
                        return Err(format!("worker exited without output ({status})"));
                    }
                    break;
                }
            }
        }
    }
    eprintln!("{total} inputs: {tally:?}");
    Ok(!abnormal)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = if args.iter().any(|a| a == "--worker") {
        worker(&args).map(|_| true)
    } else {
        run(&args)
    };
    match result {
        Ok(true) => {}
        Ok(false) => std::process::exit(1),
        Err(e) => {
            eprintln!("pm-glsl-fuzz: {e}");
            std::process::exit(2);
        }
    }
}
