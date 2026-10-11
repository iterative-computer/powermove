//! Orchestrator: runs S8 scenarios and collects results.
//!
//! For each scenario: sample idle GPU utilization (other work on the machine), start
//! the job process B and wait until it has calibrated, start the interactive process A
//! for the measured window, then stop B and collect both JSON summaries. A and B talk
//! over a FIFO (A writes deadline signals, B reads them), standing in for main's relay.
//!
//! Each scenario JSON also records whether the main display was asleep or the screen
//! locked: in either state WindowServer composites nothing, so the GPU has less
//! competing work than on an active desktop, and `present-*` scenarios are skipped.
//!
//! Other processes on the machine also use the GPU. Before each scenario the
//! orchestrator waits until external GPU time (per-process `accumulatedGPUTime` from the
//! IORegistry) is below `--max-external-pct` of wall time, and it reruns a scenario (up to
//! `--retries`) when external use during A's run exceeded that; the least contaminated
//! attempt is kept and every attempt is recorded.
//!
//! Usage: `s8-run [--seconds 60] [--only a,b,c] [--out results/<dir>] [--list]
//!         [--max-external-pct 10] [--quiet-wait-s 900] [--retries 2]`

use s8_contention::{Args, display, signal, stats};
use serde_json::{Value, json};
use std::collections::HashMap;
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

/// Job-process flags shared by the sliced scenarios.
const SLICED: &[&str] = &["--mode", "sliced", "--in-flight", "1", "--qos", "utility"];
/// Frame-aligned slicing: submit only after A's current frame is done and only if the
/// slice is predicted to end 1 ms before A's next tick.
const ALIGN: &[&str] = &["--align", "--align-guard-ms", "1"];
const PRESENT: &[&str] = &["--present", "corner", "--window-width", "48", "--window-height", "27"];

fn b(parts: &[&[&'static str]]) -> Option<Vec<&'static str>> {
    Some(parts.iter().flat_map(|p| p.iter().copied()).collect())
}

fn scenarios() -> Vec<Scenario> {
    let s = |name, what, a: &[&'static str], b: Option<Vec<&'static str>>| Scenario {
        name,
        what,
        a: a.to_vec(),
        b,
    };
    vec![
        s("baseline", "A alone", &[], None),
        // No slicing.
        s(
            "full",
            "B unsliced: one 4K blur export frame per command buffer, 2 in flight, default QoS",
            &[],
            b(&[&["--mode", "full"]]),
        ),
        s(
            "heavy",
            "B one long kernel per command buffer (foreign renderer stand-in), 2 in flight",
            &[],
            b(&[&["--mode", "heavy"]]),
        ),
        s(
            "full-bg",
            "B unsliced, background QoS + Darwin background process",
            &[],
            b(&[&["--mode", "full", "--qos", "background", "--darwin-bg"]]),
        ),
        // Time slicing with reactive yields.
        s(
            "sliced-2ms",
            "B sliced 2 ms, 1 in flight, utility QoS, yield 100 ms on miss",
            &[],
            b(&[SLICED, &["--slice-ms", "2", "--yield-on", "miss", "--yield-ms", "100"]]),
        ),
        s(
            "sliced-2ms-noyield",
            "B sliced 2 ms, 1 in flight, utility QoS, no yield",
            &[],
            b(&[SLICED, &["--slice-ms", "2", "--yield-on", "none"]]),
        ),
        s(
            "sliced-2ms-risk",
            "B sliced 2 ms, utility QoS, yield 100 ms on at-risk or miss",
            &[],
            b(&[SLICED, &["--slice-ms", "2", "--yield-on", "risk", "--yield-ms", "100"]]),
        ),
        s(
            "sliced-2ms-bg",
            "B sliced 2 ms, background QoS + Darwin background process, yield on miss",
            &[],
            b(&[&[
                "--mode",
                "sliced",
                "--in-flight",
                "1",
                "--qos",
                "background",
                "--darwin-bg",
                "--slice-ms",
                "2",
                "--yield-on",
                "miss",
            ]]),
        ),
        s(
            "sliced-2ms-if2",
            "B sliced 2 ms, 2 in flight, utility QoS, yield on miss",
            &[],
            b(&[&[
                "--mode",
                "sliced",
                "--in-flight",
                "2",
                "--qos",
                "utility",
                "--slice-ms",
                "2",
                "--yield-on",
                "miss",
            ]]),
        ),
        s(
            "sliced-1ms",
            "B sliced 1 ms, utility QoS, yield on miss",
            &[],
            b(&[SLICED, &["--slice-ms", "1", "--yield-on", "miss"]]),
        ),
        s(
            "sliced-4ms",
            "B sliced 4 ms, utility QoS, yield on miss",
            &[],
            b(&[SLICED, &["--slice-ms", "4", "--yield-on", "miss"]]),
        ),
        s(
            "sliced-8ms",
            "B sliced 8 ms, utility QoS, yield on miss",
            &[],
            b(&[SLICED, &["--slice-ms", "8", "--yield-on", "miss"]]),
        ),
        // Frame-aligned slicing.
        s(
            "sliced-2ms-align",
            "B sliced 2 ms, frame-aligned, guard 1 ms, utility QoS, yield on miss",
            &[],
            b(&[SLICED, ALIGN, &["--slice-ms", "2", "--yield-on", "miss"]]),
        ),
        s(
            "sliced-1ms-align",
            "B sliced 1 ms, frame-aligned, guard 1 ms",
            &[],
            b(&[SLICED, ALIGN, &["--slice-ms", "1", "--yield-on", "miss"]]),
        ),
        s(
            "sliced-4ms-align",
            "B sliced 4 ms, frame-aligned, guard 1 ms",
            &[],
            b(&[SLICED, ALIGN, &["--slice-ms", "4", "--yield-on", "miss"]]),
        ),
        s(
            "sliced-2ms-align-relay1",
            "As sliced-2ms-align, signals take effect 1 ms after A sends them (main relay stand-in)",
            &[],
            b(&[
                SLICED,
                ALIGN,
                &["--slice-ms", "2", "--yield-on", "miss", "--relay-delay-ms", "1"],
            ]),
        ),
        s(
            "sliced-2ms-align-relay4",
            "As sliced-2ms-align, signals take effect 4 ms after A sends them",
            &[],
            b(&[
                SLICED,
                ALIGN,
                &["--slice-ms", "2", "--yield-on", "miss", "--relay-delay-ms", "4"],
            ]),
        ),
        s(
            "sliced-2ms-120",
            "A at 120 fps; B sliced 2 ms, utility QoS, yield on miss",
            &["--fps", "120"],
            b(&[SLICED, &["--slice-ms", "2", "--yield-on", "miss"]]),
        ),
        s(
            "sliced-2ms-align-120",
            "A at 120 fps; B sliced 2 ms, frame-aligned, guard 1 ms",
            &["--fps", "120"],
            b(&[SLICED, ALIGN, &["--slice-ms", "2", "--yield-on", "miss"]]),
        ),
        // Foreign renderer pause (Blender stand-in).
        s(
            "baseline-cold",
            "A alone, no warm-up window (reference for heavy-pause)",
            &["--warmup", "0"],
            None,
        ),
        s(
            "heavy-pause",
            "B long kernels, paused on PlaybackStart (Blender pause stand-in); A measured from its first frame",
            &["--warmup", "0"],
            b(&[&["--mode", "heavy", "--pause-on-playback"]]),
        ),
        // Private Metal queue priority (experiment only).
        s(
            "full-prio2",
            "B unsliced, private Metal queue priority 2 (experiment)",
            &[],
            b(&[&["--mode", "full", "--gpu-priority", "2"]]),
        ),
        s(
            "heavy-prio2",
            "B long kernels, private Metal queue priority 2 (experiment)",
            &[],
            b(&[&["--mode", "heavy", "--gpu-priority", "2"]]),
        ),
        s(
            "full-a-prio0",
            "B unsliced; A private Metal queue priority 0 (experiment)",
            &["--gpu-priority", "0"],
            b(&[&["--mode", "full"]]),
        ),
        // VideoToolbox encode sessions.
        s(
            "vt-hevc10-x1",
            "1 VideoToolbox HEVC Main10 4K session, no B GPU work",
            &[],
            b(&[&["--mode", "none", "--vt-sessions", "1", "--vt-codec", "hevc10"]]),
        ),
        s(
            "vt-hevc10-x2",
            "2 concurrent HEVC Main10 4K sessions",
            &[],
            b(&[&["--mode", "none", "--vt-sessions", "2", "--vt-codec", "hevc10"]]),
        ),
        s(
            "vt-hevc10-x4",
            "4 concurrent HEVC Main10 4K sessions",
            &[],
            b(&[&["--mode", "none", "--vt-sessions", "4", "--vt-codec", "hevc10"]]),
        ),
        s(
            "vt-h264-x2",
            "2 concurrent H.264 High 4K sessions",
            &[],
            b(&[&["--mode", "none", "--vt-sessions", "2", "--vt-codec", "h264"]]),
        ),
        s(
            "vt-prores-x1",
            "1 ProRes 422 HQ 4K session",
            &[],
            b(&[&["--mode", "none", "--vt-sessions", "1", "--vt-codec", "prores422hq"]]),
        ),
        s(
            "vt-prores-x2",
            "2 concurrent ProRes 422 HQ 4K sessions",
            &[],
            b(&[&["--mode", "none", "--vt-sessions", "2", "--vt-codec", "prores422hq"]]),
        ),
        s(
            "vt-prores-x4",
            "4 concurrent ProRes 422 HQ 4K sessions",
            &[],
            b(&[&["--mode", "none", "--vt-sessions", "4", "--vt-codec", "prores422hq"]]),
        ),
        // Export stand-ins: GPU work plus an encoder.
        s(
            "export-full+hevc10",
            "Unsliced export stand-in: B full plus 1 HEVC Main10 session",
            &[],
            b(&[&["--mode", "full", "--vt-sessions", "1", "--vt-codec", "hevc10"]]),
        ),
        s(
            "export-sliced+hevc10",
            "B sliced 2 ms (utility, yield on miss) plus 1 HEVC Main10 session",
            &[],
            b(&[
                SLICED,
                &[
                    "--slice-ms",
                    "2",
                    "--yield-on",
                    "miss",
                    "--vt-sessions",
                    "1",
                    "--vt-codec",
                    "hevc10",
                ],
            ]),
        ),
        s(
            "export-align+hevc10",
            "B sliced 2 ms frame-aligned plus 1 HEVC Main10 session",
            &[],
            b(&[
                SLICED,
                ALIGN,
                &[
                    "--slice-ms",
                    "2",
                    "--yield-on",
                    "miss",
                    "--vt-sessions",
                    "1",
                    "--vt-codec",
                    "hevc10",
                ],
            ]),
        ),
        s(
            "export-align+prores",
            "B sliced 2 ms frame-aligned plus 1 ProRes 422 HQ session",
            &[],
            b(&[
                SLICED,
                ALIGN,
                &[
                    "--slice-ms",
                    "2",
                    "--yield-on",
                    "miss",
                    "--vt-sessions",
                    "1",
                    "--vt-codec",
                    "prores422hq",
                ],
            ]),
        ),
        // On-screen presentation (needs an awake, unlocked display).
        s(
            "present-baseline",
            "A alone, presenting through a small on-screen CAMetalLayer window",
            PRESENT,
            None,
        ),
        s(
            "present-full",
            "As present-baseline with B unsliced",
            PRESENT,
            b(&[&["--mode", "full"]]),
        ),
        s(
            "present-align",
            "As present-baseline with B sliced 2 ms frame-aligned",
            PRESENT,
            b(&[SLICED, ALIGN, &["--slice-ms", "2", "--yield-on", "miss"]]),
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

/// Accumulated GPU time (ns) per process from the AGX user clients in the IORegistry
/// (`AppUsage` / `accumulatedGPUTime`, summed over a process's clients).
fn gpu_time_by_pid() -> HashMap<u32, (String, u64)> {
    let mut map: HashMap<u32, (String, u64)> = HashMap::new();
    let Ok(out) = Command::new("/usr/sbin/ioreg")
        .args(["-r", "-c", "AGXDeviceUserClient", "-d", "1", "-l"])
        .output()
    else {
        return map;
    };
    parse_gpu_time(&String::from_utf8_lossy(&out.stdout), &mut map);
    map
}

fn parse_gpu_time(text: &str, map: &mut HashMap<u32, (String, u64)>) {
    let mut current: Option<u32> = None;
    for line in text.lines() {
        if let Some(i) = line.find("\"IOUserClientCreator\" = \"pid ") {
            let rest = &line[i + "\"IOUserClientCreator\" = \"pid ".len()..];
            let (pid, name) = rest.split_once(", ").unwrap_or((rest, ""));
            current = pid.trim().parse().ok();
            if let Some(pid) = current {
                map.entry(pid)
                    .or_insert_with(|| (name.trim_end_matches('"').to_string(), 0));
            }
        } else if line.contains("\"AppUsage\"") {
            let Some(pid) = current else { continue };
            let key = "\"accumulatedGPUTime\"=";
            let mut sum = 0u64;
            for (i, _) in line.match_indices(key) {
                let digits: String = line[i + key.len()..]
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect();
                sum += digits.parse::<u64>().unwrap_or(0);
            }
            if let Some(e) = map.get_mut(&pid) {
                e.1 += sum;
            }
        }
    }
}

/// GPU time other processes used between two snapshots (ms), excluding `ours`.
///
/// Only the total, WindowServer's share, and the number of processes are recorded:
/// result files are committed to a public repository and the names of the apps running
/// on the machine are not. The top consumers are printed to stderr for the operator.
fn external_gpu(before: &HashMap<u32, (String, u64)>, after: &HashMap<u32, (String, u64)>, ours: &[u32]) -> Value {
    let mut rows: Vec<(String, f64)> = after
        .iter()
        .filter(|(pid, _)| !ours.contains(pid))
        .filter_map(|(pid, (name, t))| {
            let t0 = before.get(pid).map(|b| b.1)?;
            let d = t.saturating_sub(t0) as f64 / 1e6;
            (d > 0.0).then(|| (name.clone(), d))
        })
        .collect();
    rows.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    let total: f64 = rows.iter().map(|r| r.1).sum();
    let window_server: f64 = rows.iter().filter(|r| r.0 == "WindowServer").map(|r| r.1).sum();
    for (name, ms) in rows.iter().take(4) {
        eprintln!("   external GPU: {name} {ms:.1} ms");
    }
    json!({
        "total_ms": stats::r(total, 1),
        "window_server_ms": stats::r(window_server, 1),
        "processes": rows.len(),
    })
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

/// External GPU time over `window` as a fraction of wall time.
fn external_fraction(window: Duration) -> f64 {
    let before = gpu_time_by_pid();
    std::thread::sleep(window);
    let after = gpu_time_by_pid();
    let ext = external_gpu_quiet(&before, &after);
    ext / (window.as_secs_f64() * 1e3)
}

fn external_gpu_quiet(before: &HashMap<u32, (String, u64)>, after: &HashMap<u32, (String, u64)>) -> f64 {
    after
        .iter()
        .filter_map(|(pid, (_, t))| before.get(pid).map(|b| t.saturating_sub(b.1) as f64 / 1e6))
        .sum()
}

/// Gate on a quiet GPU, run, and retry when other processes used more than
/// `max_ext` of the GPU during A's run. Keeps the least contaminated attempt and records
/// every attempt.
fn run(s: &Scenario, bin: &std::path::Path, seconds: f64, out: &std::path::Path, gate: &Gate) -> Value {
    let mut attempts = Vec::new();
    let mut best: Option<(f64, Value)> = None;
    for attempt in 0..=gate.retries {
        let t = Instant::now();
        let mut quiet = external_fraction(Duration::from_secs(3));
        while quiet > gate.max_ext && t.elapsed().as_secs_f64() < gate.wait_s {
            eprintln!("   waiting for a quiet GPU (external {:.1}%)", quiet * 100.0);
            std::thread::sleep(Duration::from_secs(5));
            quiet = external_fraction(Duration::from_secs(3));
        }
        let mut v = run_once(s, bin, seconds, out);
        let a_ms = v["a_wall_seconds"].as_f64().unwrap_or(seconds) * 1e3;
        let ext = v["external_gpu_during_a"]["total_ms"].as_f64().unwrap_or(f64::NAN) / a_ms;
        v["external_gpu_fraction"] = json!(stats::r(ext, 4));
        v["quiet_before"] = json!(stats::r(quiet, 4));
        v["quiet_wait_s"] = json!(stats::r(t.elapsed().as_secs_f64(), 1));
        attempts.push(json!({
            "attempt": attempt,
            "external_gpu_fraction": stats::r(ext, 4),
            "drops_1f": v["a"]["drops"]["deadline_1_frame"],
            "drops_2f": v["a"]["drops"]["deadline_2_frames"],
        }));
        eprintln!("   attempt {attempt}: external GPU {:.1}% of A's run", ext * 100.0);
        let clean = ext <= gate.max_ext;
        if best.as_ref().is_none_or(|(b, _)| ext < *b) {
            best = Some((ext, v));
        }
        if clean {
            break;
        }
    }
    let (ext, mut v) = best.unwrap();
    v["attempts"] = json!(attempts);
    v["contaminated"] = json!(ext > gate.max_ext);
    v
}

struct Gate {
    max_ext: f64,
    wait_s: f64,
    retries: usize,
}

fn run_once(s: &Scenario, bin: &std::path::Path, seconds: f64, out: &std::path::Path) -> Value {
    // Idle baseline: what else is using the GPU right now.
    let idle: Vec<f64> = (0..6)
        .filter_map(|_| {
            std::thread::sleep(Duration::from_millis(500));
            gpu_util()
        })
        .collect();
    let display_asleep = display::main_display_asleep();
    let screen_locked = display::screen_locked();
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

    let b_pid = b_child.as_ref().map(|c| c.id());
    let gpu_before = gpu_time_by_pid();
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
    let gpu_after = gpu_time_by_pid();
    let ours: Vec<u32> = b_pid.into_iter().chain([std::process::id()]).collect();
    let external = external_gpu(&gpu_before, &gpu_after, &ours);

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
        "display_asleep": display_asleep,
        "screen_locked": screen_locked,
        "gpu_util_idle_before": stats::dist(&idle),
        "gpu_util_during": stats::dist(&during),
        "external_gpu_during_a": external,
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
    let gate = Gate {
        max_ext: args.num::<f64>("max-external-pct", 10.0) / 100.0,
        wait_s: args.num("quiet-wait-s", 900.0),
        retries: args.num("retries", 2),
    };
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
        if s.a.contains(&"--present") && (display::main_display_asleep() || display::screen_locked() != Some(false)) {
            // Nothing reaches the glass while the display sleeps or the lock screen is up.
            eprintln!("   skipped: display asleep or screen locked");
            continue;
        }
        let v = run(s, &bin, seconds, &out, &gate);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_agx_user_clients() {
        let text = r#"
    | {
      "IOUserClientCreator" = "pid 463, WindowServer"
      "AppUsage" = ({"API"="Metal","lastSubmittedTime"=1,"accumulatedGPUTime"=1000},{"API"="Metal","accumulatedGPUTime"=24})
    | {
      "IOUserClientCreator" = "pid 463, WindowServer"
      "AppUsage" = ({"API"="Metal","accumulatedGPUTime"=6})
    | {
      "IOUserClientCreator" = "pid 916, MenuBarAgent"
"#;
        let mut map = HashMap::new();
        parse_gpu_time(text, &mut map);
        assert_eq!(map[&463], ("WindowServer".to_string(), 1030));
        assert_eq!(map[&916], ("MenuBarAgent".to_string(), 0));
        let mut after = map.clone();
        after.get_mut(&463).unwrap().1 += 2_000_000;
        after.insert(1, ("new".into(), 5_000_000));
        let v = external_gpu(&map, &after, &[916]);
        assert_eq!(v["total_ms"], 2.0);
    }
}
