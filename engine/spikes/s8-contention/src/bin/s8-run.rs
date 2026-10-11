//! Orchestrator: runs S8 scenarios and collects results.
//!
//! For each scenario: sample idle GPU utilization (other work on the machine), start
//! the job process B and wait until it has calibrated, start the interactive process A
//! for the measured window, then stop B and collect both JSON summaries. A and B talk
//! over a FIFO (A writes deadline signals, B reads them), standing in for main's relay.
//!
//! Usage: `s8-run [--seconds 60] [--only a,b,c] [--out results/<dir>] [--list]`

use s8_contention::{Args, signal, stats};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

struct Scenario {
    name: &'static str,
    what: &'static str,
    a: Vec<&'static str>,
    b: Option<Vec<&'static str>>,
}

fn sc(name: &'static str, what: &'static str, a: &[&'static str], b: Option<&[&'static str]>) -> Scenario {
    Scenario {
        name,
        what,
        a: a.to_vec(),
        b: b.map(|b| b.to_vec()),
    }
}

fn scenarios() -> Vec<Scenario> {
    vec![
        sc("baseline", "A alone", &[], None),
        sc(
            "full",
            "B unsliced: one 4K blur export frame per command buffer, 2 in flight, default QoS",
            &[],
            Some(&["--mode", "full"]),
        ),
        sc(
            "heavy",
            "B one long kernel per command buffer (foreign renderer stand-in), 2 in flight",
            &[],
            Some(&["--mode", "heavy"]),
        ),
        sc(
            "sliced-2ms",
            "B sliced 2 ms, 1 in flight, utility QoS, yield 100 ms on miss",
            &[],
            Some(&[
                "--mode",
                "sliced",
                "--slice-ms",
                "2",
                "--in-flight",
                "1",
                "--qos",
                "utility",
                "--yield-on",
                "miss",
                "--yield-ms",
                "100",
            ]),
        ),
        sc(
            "sliced-2ms-noyield",
            "B sliced 2 ms, 1 in flight, utility QoS, no yield",
            &[],
            Some(&[
                "--mode",
                "sliced",
                "--slice-ms",
                "2",
                "--in-flight",
                "1",
                "--qos",
                "utility",
                "--yield-on",
                "none",
            ]),
        ),
        sc(
            "sliced-2ms-risk",
            "B sliced 2 ms, 1 in flight, utility QoS, yield 100 ms on at-risk or miss",
            &[],
            Some(&[
                "--mode",
                "sliced",
                "--slice-ms",
                "2",
                "--in-flight",
                "1",
                "--qos",
                "utility",
                "--yield-on",
                "risk",
                "--yield-ms",
                "100",
            ]),
        ),
        sc(
            "sliced-2ms-bg",
            "B sliced 2 ms, 1 in flight, background QoS + Darwin background process, yield on miss",
            &[],
            Some(&[
                "--mode",
                "sliced",
                "--slice-ms",
                "2",
                "--in-flight",
                "1",
                "--qos",
                "background",
                "--darwin-bg",
                "--yield-on",
                "miss",
            ]),
        ),
        sc(
            "sliced-2ms-if2",
            "B sliced 2 ms, 2 in flight, utility QoS, yield on miss",
            &[],
            Some(&[
                "--mode",
                "sliced",
                "--slice-ms",
                "2",
                "--in-flight",
                "2",
                "--qos",
                "utility",
                "--yield-on",
                "miss",
            ]),
        ),
        sc(
            "sliced-1ms",
            "B sliced 1 ms, 1 in flight, utility QoS, yield on miss",
            &[],
            Some(&[
                "--mode",
                "sliced",
                "--slice-ms",
                "1",
                "--in-flight",
                "1",
                "--qos",
                "utility",
                "--yield-on",
                "miss",
            ]),
        ),
        sc(
            "sliced-4ms",
            "B sliced 4 ms, 1 in flight, utility QoS, yield on miss",
            &[],
            Some(&[
                "--mode",
                "sliced",
                "--slice-ms",
                "4",
                "--in-flight",
                "1",
                "--qos",
                "utility",
                "--yield-on",
                "miss",
            ]),
        ),
        sc(
            "sliced-8ms",
            "B sliced 8 ms, 1 in flight, utility QoS, yield on miss",
            &[],
            Some(&[
                "--mode",
                "sliced",
                "--slice-ms",
                "8",
                "--in-flight",
                "1",
                "--qos",
                "utility",
                "--yield-on",
                "miss",
            ]),
        ),
        sc(
            "sliced-2ms-align",
            "B sliced 2 ms, frame-aligned (submit only between A's frame completion and next tick minus 1 ms), utility QoS, yield on miss",
            &[],
            Some(&[
                "--mode",
                "sliced",
                "--slice-ms",
                "2",
                "--in-flight",
                "1",
                "--qos",
                "utility",
                "--yield-on",
                "miss",
                "--align",
                "--align-guard-ms",
                "1",
            ]),
        ),
        sc(
            "sliced-1ms-align",
            "B sliced 1 ms, frame-aligned, guard 1 ms, utility QoS, yield on miss",
            &[],
            Some(&[
                "--mode",
                "sliced",
                "--slice-ms",
                "1",
                "--in-flight",
                "1",
                "--qos",
                "utility",
                "--yield-on",
                "miss",
                "--align",
                "--align-guard-ms",
                "1",
            ]),
        ),
        sc(
            "sliced-2ms-align-120",
            "A at 120 fps; B sliced 2 ms, frame-aligned, guard 1 ms",
            &["--fps", "120"],
            Some(&[
                "--mode",
                "sliced",
                "--slice-ms",
                "2",
                "--in-flight",
                "1",
                "--qos",
                "utility",
                "--yield-on",
                "miss",
                "--align",
                "--align-guard-ms",
                "1",
            ]),
        ),
        sc(
            "sliced-2ms-120",
            "A at 120 fps; B sliced 2 ms, 1 in flight, utility QoS, yield on miss",
            &["--fps", "120"],
            Some(&[
                "--mode",
                "sliced",
                "--slice-ms",
                "2",
                "--in-flight",
                "1",
                "--qos",
                "utility",
                "--yield-on",
                "miss",
            ]),
        ),
        sc(
            "full-bg",
            "B unsliced, background QoS + Darwin background process",
            &[],
            Some(&["--mode", "full", "--qos", "background", "--darwin-bg"]),
        ),
        sc(
            "full-prio2",
            "B unsliced, private Metal queue priority 2 (experiment)",
            &[],
            Some(&["--mode", "full", "--gpu-priority", "2"]),
        ),
        sc(
            "heavy-prio2",
            "B long kernels, private Metal queue priority 2 (experiment)",
            &[],
            Some(&["--mode", "heavy", "--gpu-priority", "2"]),
        ),
        sc(
            "full-a-prio0",
            "B unsliced; A private Metal queue priority 0 (experiment)",
            &["--gpu-priority", "0"],
            Some(&["--mode", "full"]),
        ),
        sc(
            "vt-hevc10-x1",
            "1 VideoToolbox HEVC Main10 4K session, no B GPU work",
            &[],
            Some(&["--mode", "none", "--vt-sessions", "1", "--vt-codec", "hevc10"]),
        ),
        sc(
            "vt-hevc10-x2",
            "2 concurrent HEVC Main10 4K sessions",
            &[],
            Some(&["--mode", "none", "--vt-sessions", "2", "--vt-codec", "hevc10"]),
        ),
        sc(
            "vt-hevc10-x4",
            "4 concurrent HEVC Main10 4K sessions",
            &[],
            Some(&["--mode", "none", "--vt-sessions", "4", "--vt-codec", "hevc10"]),
        ),
        sc(
            "vt-h264-x2",
            "2 concurrent H.264 High 4K sessions",
            &[],
            Some(&["--mode", "none", "--vt-sessions", "2", "--vt-codec", "h264"]),
        ),
        sc(
            "vt-prores-x1",
            "1 ProRes 422 HQ 4K session",
            &[],
            Some(&["--mode", "none", "--vt-sessions", "1", "--vt-codec", "prores422hq"]),
        ),
        sc(
            "vt-prores-x2",
            "2 concurrent ProRes 422 HQ 4K sessions",
            &[],
            Some(&["--mode", "none", "--vt-sessions", "2", "--vt-codec", "prores422hq"]),
        ),
        sc(
            "vt-prores-x4",
            "4 concurrent ProRes 422 HQ 4K sessions",
            &[],
            Some(&["--mode", "none", "--vt-sessions", "4", "--vt-codec", "prores422hq"]),
        ),
        sc(
            "export-sliced+hevc10",
            "Full export stand-in: B sliced 2 ms (utility, yield on miss) plus 1 HEVC Main10 session",
            &[],
            Some(&[
                "--mode",
                "sliced",
                "--slice-ms",
                "2",
                "--in-flight",
                "1",
                "--qos",
                "utility",
                "--yield-on",
                "miss",
                "--vt-sessions",
                "1",
                "--vt-codec",
                "hevc10",
            ]),
        ),
        sc(
            "export-full+hevc10",
            "Unsliced export stand-in: B full plus 1 HEVC Main10 session",
            &[],
            Some(&["--mode", "full", "--vt-sessions", "1", "--vt-codec", "hevc10"]),
        ),
    ]
}

/// GPU "Device Utilization %" from the IOAccelerator performance statistics.
fn gpu_util() -> Option<f64> {
    let out = Command::new("/usr/sbin/ioreg")
        .args(["-r", "-c", "IOAccelerator", "-d", "1"])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let key = "\"Device Utilization %\"=";
    let i = s.find(key)? + key.len();
    let digits: String = s[i..].chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

fn sample_util(stop: Arc<AtomicBool>) -> std::thread::JoinHandle<Vec<f64>> {
    std::thread::spawn(move || {
        let mut v = Vec::new();
        while !stop.load(Ordering::Relaxed) {
            if let Some(u) = gpu_util() {
                v.push(u);
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        v
    })
}

fn parse_result(text: &str) -> Option<Value> {
    text.lines()
        .rev()
        .find_map(|l| l.strip_prefix("S8RESULT "))
        .and_then(|j| serde_json::from_str(j).ok())
}

fn run(s: &Scenario, bin: &std::path::Path, seconds: f64, out: &std::path::Path) -> Value {
    // Idle baseline: what else is using the GPU right now.
    let idle: Vec<f64> = (0..6)
        .filter_map(|_| {
            std::thread::sleep(Duration::from_millis(500));
            gpu_util()
        })
        .collect();
    let fifo = std::env::temp_dir().join(format!("s8-{}-{}.fifo", std::process::id(), s.name));
    let fifo = fifo.to_str().unwrap().to_string();
    signal::make_fifo(&fifo).expect("mkfifo");

    let mut b_child = None;
    let b_out = Arc::new(Mutex::new(String::new()));
    let mut b_reader = None;
    if let Some(b_args) = &s.b {
        let mut child = Command::new(bin.join("s8-job"))
            .args(b_args)
            .args(["--signal-fifo", &fifo, "--label", s.name])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn s8-job");
        let stderr = child.stderr.take().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if line.contains("S8READY") {
                    let _ = tx.send(());
                } else {
                    eprintln!("  [b] {line}");
                }
            }
        });
        let mut stdout = child.stdout.take().unwrap();
        let sink = b_out.clone();
        b_reader = Some(std::thread::spawn(move || {
            let mut t = String::new();
            let _ = stdout.read_to_string(&mut t);
            *sink.lock().unwrap() = t;
        }));
        rx.recv_timeout(Duration::from_secs(60))
            .expect("s8-job never became ready");
        std::thread::sleep(Duration::from_secs(2));
        b_child = Some(child);
    }

    let stop = Arc::new(AtomicBool::new(false));
    let sampler = sample_util(stop.clone());
    let raw = out.join("raw").join(format!("{}-a.jsonl", s.name));
    let t = Instant::now();
    let a_out = Command::new(bin.join("s8-interactive"))
        .args(&s.a)
        .args([
            "--seconds",
            &seconds.to_string(),
            "--signal-fifo",
            &fifo,
            "--label",
            s.name,
            "--raw",
            raw.to_str().unwrap(),
        ])
        .stderr(Stdio::inherit())
        .output()
        .expect("spawn s8-interactive");
    let a_secs = t.elapsed().as_secs_f64();
    stop.store(true, Ordering::Relaxed);
    let during = sampler.join().unwrap();

    let b = if let Some(mut child) = b_child {
        // SAFETY: plain signal to our own child.
        unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
        let _ = child.wait();
        b_reader.unwrap().join().unwrap();
        parse_result(&b_out.lock().unwrap())
    } else {
        None
    };
    let _ = std::fs::remove_file(&fifo);
    let a = parse_result(&String::from_utf8_lossy(&a_out.stdout));
    json!({
        "scenario": s.name,
        "what": s.what,
        "a_args": s.a,
        "b_args": s.b,
        "a_wall_seconds": stats::r(a_secs, 2),
        "gpu_util_idle_before": stats::dist(&idle),
        "gpu_util_during": stats::dist(&during),
        "a": a,
        "b": b,
    })
}

fn row(v: &Value) -> String {
    let a = &v["a"];
    let b = &v["b"];
    let f = |x: &Value| x.as_f64().map(|n| format!("{n:.2}")).unwrap_or("-".into());
    let mut vt_fps = String::from("-");
    if let Some(arr) = b["vt"].as_array()
        && !arr.is_empty()
    {
        let fps: Vec<String> = arr
            .iter()
            .map(|s| format!("{:.1}", s["fps"].as_f64().unwrap_or(0.0)))
            .collect();
        vt_fps = fps.join("/");
    }
    format!(
        "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
        v["scenario"].as_str().unwrap_or("?"),
        a["frames_measured"],
        a["drops"]["deadline_1_frame"],
        a["drops"]["deadline_2_frames"],
        a["drops"]["deadline_3_frames"],
        f(&a["latency_ms"]["p50"]),
        f(&a["latency_ms"]["p99"]),
        f(&a["latency_ms"]["max"]),
        f(&a["completion_jitter_ms"]["p99"]),
        f(&a["gpu_start_delay_ms"]["p99"]),
        f(&b["export_fps"]),
        f(&b["cb_gpu_ms"]["p99"]),
        vt_fps,
    )
}

fn main() {
    let args = Args::from_env();
    let all = scenarios();
    if args.flag("list") {
        for s in &all {
            println!("{:24} {}", s.name, s.what);
        }
        return;
    }
    let seconds: f64 = args.num("seconds", 60.0);
    let only: Option<Vec<String>> = args.str("only").map(|o| o.split(',').map(str::to_string).collect());
    let bin = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
    let out = std::path::PathBuf::from(args.string_or("out", "results/latest"));
    std::fs::create_dir_all(out.join("raw")).unwrap();
    let header = "| scenario | frames | drops 1f | drops 2f | drops 3f | lat p50 | lat p99 | lat max | jitter p99 | gpu start delay p99 | B fps | B cb p99 ms | VT fps |\n|---|---|---|---|---|---|---|---|---|---|---|---|---|";
    println!("{header}");
    let mut results = Vec::new();
    for s in all
        .iter()
        .filter(|s| only.as_ref().is_none_or(|o| o.iter().any(|n| n == s.name)))
    {
        eprintln!("== {} ({})", s.name, s.what);
        let v = run(s, &bin, seconds, &out);
        std::fs::write(
            out.join(format!("{}.json", s.name)),
            serde_json::to_string_pretty(&v).unwrap(),
        )
        .unwrap();
        println!("{}", row(&v));
        results.push(v);
    }
    let path = out.join("summary.json");
    let mut merged: Vec<Value> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    for r in results {
        merged.retain(|m| m["scenario"] != r["scenario"]);
        merged.push(r);
    }
    std::fs::write(&path, serde_json::to_string_pretty(&merged).unwrap()).unwrap();
}
