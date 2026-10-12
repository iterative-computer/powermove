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
//! Other processes on the machine also use the GPU. The orchestrator measures them with
//! the per-process `accumulatedGPUTime` counters of the AGX user clients in the
//! IORegistry, but only while none of its own processes run: on this machine those
//! counters misattribute GPU time while A or B is running (A's own GPU work was charged
//! to an unrelated long-lived process, and A itself was charged about 0.5% of its
//! timestamp-measured GPU time), so a during-run reading is not evidence of external
//! load. Instead each attempt is bracketed: before it, the orchestrator waits until
//! external GPU time over a 3 s window is below `--max-external-pct` of wall time; after
//! it, it samples another 3 s window.
//!
//! Load from other processes also changes over minutes, so every scenario with a B
//! first runs A alone for `--control-s` seconds (the paired control) with the same A
//! arguments. An attempt is rerun (up to `--retries`) when its after-window exceeds the
//! limit or its control dropped frames; the cleanest attempt is kept, every attempt is
//! recorded, and a scenario is marked `contaminated` when no attempt was clean.
//!
//! Blender scenarios need `--blender PATH` (the Blender executable); without it they are
//! skipped and listed as skipped.
//!
//! Usage: `s8-run [--seconds 60] [--only a,b,c] [--out results/<dir>] [--list]
//!         [--max-external-pct 10] [--quiet-wait-s 900] [--retries 2] [--blender PATH]
//!         [--control-s 20] [--desk "<s8-interactive args>" --desk-n 3]`; `s8-run --reattribute results/<dir>` recomputes the
//!         late-frame attribution of finished scenarios from their raw logs.

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
/// A 16 x 9 pt click-through window in the bottom-left corner: the smallest window the
/// compositor still presents (an offscreen window gets no presented times).
const PRESENT: &[&str] = &["--present", "corner", "--window-width", "16", "--window-height", "9"];
/// A heavier interactive frame: 8 layers and a radius-40 separable blur.
const AHEAVY: &[&str] = &["--layers", "8", "--blur-radius", "40"];
/// Blender EEVEE worker, 4K, 16 samples, 16-bit PNG output; `{blender}` is replaced by
/// the `--blender` path.
const BLENDER: &[&str] = &["--blender", "{blender}"];

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
        // Kernel-length sweep: B submits one compute dispatch per command buffer, 2 in
        // flight, no slicing; the blur radius sets how long one dispatch runs.
        s(
            "kernel-2ms",
            "B one dispatch of about 2 ms per command buffer, 2 in flight, unsliced",
            &[],
            b(&[&["--mode", "heavy", "--heavy-radius", "22"]]),
        ),
        s(
            "kernel-4ms",
            "B one dispatch of about 4 ms per command buffer, 2 in flight, unsliced",
            &[],
            b(&[&["--mode", "heavy", "--heavy-radius", "44"]]),
        ),
        s(
            "kernel-8ms",
            "B one dispatch of about 8 ms per command buffer, 2 in flight, unsliced",
            &[],
            b(&[&["--mode", "heavy", "--heavy-radius", "88"]]),
        ),
        s(
            "kernel-16ms",
            "B one dispatch of about 16 ms per command buffer, 2 in flight, unsliced",
            &[],
            b(&[&["--mode", "heavy", "--heavy-radius", "176"]]),
        ),
        s(
            "heavy-sliced",
            "B the heavy (radius 384) kernel cut into 2 ms bands, frame-aligned, utility QoS",
            &[],
            b(&[
                SLICED,
                ALIGN,
                &[
                    "--slice-ms",
                    "2",
                    "--yield-on",
                    "miss",
                    "--radius",
                    "384",
                    "--iters",
                    "1",
                ],
            ]),
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
        // Blender EEVEE worker (foreign renderer: process-level policies only).
        s(
            "blender",
            "Blender EEVEE 4K 16 samples rendering continuously, no policy",
            &[],
            b(&[&["--mode", "none"], BLENDER]),
        ),
        s(
            "blender-bg",
            "Blender in the Darwin background state for the whole run",
            &[],
            b(&[&["--mode", "none"], BLENDER, &["--blender-policy", "bg"]]),
        ),
        s(
            "blender-yield",
            "Blender SIGSTOP for 100 ms after each deadline miss",
            &[],
            b(&[
                &["--mode", "none"],
                BLENDER,
                &["--blender-policy", "yield", "--yield-on", "miss", "--yield-ms", "100"],
            ]),
        ),
        s(
            "blender-throttle",
            "Blender duty-cycled during playback: 10 ms running, 40 ms stopped",
            &[],
            b(&[
                &["--mode", "none"],
                BLENDER,
                &[
                    "--blender-policy",
                    "throttle",
                    "--blender-run-ms",
                    "10",
                    "--blender-stop-ms",
                    "40",
                ],
            ]),
        ),
        s(
            "blender-pause",
            "Blender SIGSTOP on PlaybackStart; A measured from its first frame",
            &["--warmup", "0"],
            b(&[&["--mode", "none"], BLENDER, &["--blender-policy", "pause"]]),
        ),
        // The S8 row: 4K playback while the job engine exports 4K blur (plus its encoder)
        // and Blender EEVEE renders, with contract 13 in force.
        s(
            "gate-pause",
            "S8 row: B sliced 2 ms frame-aligned + HEVC Main10 session + Blender paused on playback; A from its first frame",
            &["--warmup", "0"],
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
                BLENDER,
                &["--blender-policy", "pause"],
            ]),
        ),
        s(
            "gate-pause-vt30",
            "As gate-pause with the HEVC session paced at 30 fps (an export's encode rate, not flat out)",
            &["--warmup", "0"],
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
                    "--vt-fps",
                    "30",
                ],
                BLENDER,
                &["--blender-policy", "pause"],
            ]),
        ),
        s(
            "export-align+hevc10-vt30",
            "B sliced 2 ms frame-aligned plus 1 HEVC Main10 session paced at 30 fps",
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
                    "--vt-fps",
                    "30",
                ],
            ]),
        ),
        s(
            "gate-throttle",
            "S8 row: as gate-pause but Blender duty-cycled 10/40 ms during playback",
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
                BLENDER,
                &[
                    "--blender-policy",
                    "throttle",
                    "--blender-run-ms",
                    "10",
                    "--blender-stop-ms",
                    "40",
                ],
            ]),
        ),
        s(
            "gate-run",
            "S8 row without a Blender policy: B sliced 2 ms frame-aligned + HEVC session + Blender running",
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
                BLENDER,
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
        s(
            "present-blender",
            "As present-baseline with Blender EEVEE rendering, no policy",
            PRESENT,
            b(&[&["--mode", "none"], BLENDER]),
        ),
        s(
            "present-gate-pause",
            "As gate-pause, presenting on screen",
            &[
                "--present",
                "corner",
                "--window-width",
                "16",
                "--window-height",
                "9",
                "--warmup",
                "0",
            ],
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
                BLENDER,
                &["--blender-policy", "pause"],
            ]),
        ),
        // A with less headroom: 8 layers and a radius-40 blur, so its frame costs
        // several times the default (closer to a base machine's share of the frame).
        s(
            "aheavy-baseline",
            "A heavy (8 layers, blur radius 40) alone",
            AHEAVY,
            None,
        ),
        s("aheavy-full", "A heavy; B unsliced", AHEAVY, b(&[&["--mode", "full"]])),
        s(
            "aheavy-kernel-4ms",
            "A heavy; B one 4 ms dispatch per command buffer, unsliced",
            AHEAVY,
            b(&[&["--mode", "heavy", "--heavy-radius", "44"]]),
        ),
        s(
            "aheavy-sliced-2ms",
            "A heavy; B sliced 2 ms, utility QoS, yield on miss",
            AHEAVY,
            b(&[SLICED, &["--slice-ms", "2", "--yield-on", "miss"]]),
        ),
        s(
            "aheavy-sliced-2ms-align",
            "A heavy; B sliced 2 ms frame-aligned",
            AHEAVY,
            b(&[SLICED, ALIGN, &["--slice-ms", "2", "--yield-on", "miss"]]),
        ),
        s(
            "aheavy-gate-pause",
            "A heavy; S8 row policy (sliced 2 ms aligned + HEVC + Blender paused)",
            &["--layers", "8", "--blur-radius", "40", "--warmup", "0"],
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
                BLENDER,
                &["--blender-policy", "pause"],
            ]),
        ),
        // Drift check: A alone again at the end of the run.
        s("baseline-end", "A alone, after every other scenario", &[], None),
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

/// GPU time used by every process over `window`, as a fraction of wall time. Only
/// meaningful while none of the spike's processes run (see the module comment).
fn external_fraction(window: Duration) -> f64 {
    let before = gpu_time_by_pid();
    std::thread::sleep(window);
    let after = gpu_time_by_pid();
    let ext: f64 = after
        .iter()
        .filter_map(|(pid, (_, t))| before.get(pid).map(|b| t.saturating_sub(b.1) as f64 / 1e6))
        .sum();
    ext / (window.as_secs_f64() * 1e3)
}

/// Gate on a quiet GPU, run, sample the GPU again afterwards, and retry when the
/// attempt was not clean: the after-window was busy, or the paired control (A alone just
/// before B started) already dropped frames. Keeps the cleanest attempt (fewest control
/// drops, then quietest bracket) and records every attempt.
fn run(s: &Scenario, bin: &std::path::Path, seconds: f64, out: &std::path::Path, gate: &Gate) -> Value {
    let mut attempts = Vec::new();
    let mut best: Option<((u64, f64), Value)> = None;
    for attempt in 0..=gate.retries {
        let t = Instant::now();
        let mut before = external_fraction(Duration::from_secs(3));
        while before > gate.max_ext && t.elapsed().as_secs_f64() < gate.wait_s {
            eprintln!("   waiting for a quiet GPU (external {:.1}%)", before * 100.0);
            std::thread::sleep(Duration::from_secs(5));
            before = external_fraction(Duration::from_secs(3));
        }
        let mut v = run_once(s, bin, seconds, out, gate);
        let after = external_fraction(Duration::from_secs(3));
        let worst = before.max(after);
        let control_drops = v["control"]["drops_1f"].as_u64().unwrap_or(0);
        v["external_gpu_before"] = json!(stats::r(before, 4));
        v["external_gpu_after"] = json!(stats::r(after, 4));
        v["quiet_wait_s"] = json!(stats::r(t.elapsed().as_secs_f64() - seconds, 1));
        attempts.push(json!({
            "attempt": attempt,
            "external_gpu_before": stats::r(before, 4),
            "external_gpu_after": stats::r(after, 4),
            "control_drops_1f": v["control"]["drops_1f"],
            "drops_1f": v["a"]["drops"]["deadline_1_frame"],
            "drops_2f": v["a"]["drops"]["deadline_2_frames"],
        }));
        eprintln!(
            "   attempt {attempt}: external GPU {:.1}% before, {:.1}% after; control drops {control_drops}; drops {}",
            before * 100.0,
            after * 100.0,
            v["a"]["drops"]["deadline_1_frame"],
        );
        let key = (control_drops, worst);
        if best.as_ref().is_none_or(|(b, _)| key < *b) {
            best = Some((key, v));
        }
        if worst <= gate.max_ext && control_drops == 0 {
            break;
        }
    }
    let ((control_drops, worst), mut v) = best.unwrap();
    v["attempts"] = json!(attempts);
    v["contaminated"] = json!(worst > gate.max_ext || control_drops > 0);
    v
}

struct Gate {
    max_ext: f64,
    wait_s: f64,
    retries: usize,
    blender: Option<String>,
    /// Seconds of A alone run just before each scenario that has a B (0: none).
    control_s: f64,
    /// Arguments for unmeasured `s8-interactive` processes run alongside every scenario
    /// (desktop-load stand-in); `None` for none.
    desk: Option<String>,
    /// How many desk-load processes (`--desk-n`, default 1).
    desk_n: usize,
}

fn run_once(s: &Scenario, bin: &std::path::Path, seconds: f64, out: &std::path::Path, gate: &Gate) -> Value {
    // Idle baseline: what else is using the GPU right now.
    let idle: Vec<f64> = (0..6)
        .filter_map(|_| {
            std::thread::sleep(Duration::from_millis(500));
            gpu_util()
        })
        .collect();
    let display_asleep = display::main_display_asleep();
    // Optional extra GPU clients standing in for an active desktop (WindowServer
    // compositing, browsers playing video, other apps): `desk_n` interactive-style
    // processes with their own devices, started before the control and stopped after A.
    // Their frames are not measured.
    let mut desk: Vec<std::process::Child> = Vec::new();
    for i in 0..gate.desk_n {
        let Some(d) = gate.desk.as_ref() else { break };
        let child = Command::new(bin.join("s8-interactive"))
            .args(d.split_whitespace())
            // Long enough to outlast the control, B's start-up (Blender may take minutes),
            // and A; it is killed after A anyway.
            .args(["--seconds", &(gate.control_s + seconds + 300.0).to_string()])
            .args(["--warmup", "0", "--label", &format!("desk{i}"), "--no-rt"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn desk load");
        desk.push(child);
    }
    if !desk.is_empty() {
        std::thread::sleep(Duration::from_secs(3));
    }
    let screen_locked = display::screen_locked();
    let fifo = std::env::temp_dir().join(format!("s8-{}-{}.fifo", std::process::id(), s.name));
    let fifo = fifo.to_str().unwrap().to_string();
    signal::make_fifo(&fifo).expect("mkfifo");

    let control = (s.b.is_some() && gate.control_s > 0.0)
        .then(|| run_a(s, bin, gate.control_s, &["--label", "control"]))
        .flatten()
        .map(|a| control_summary(&a));

    let mut b_child = None;
    let b_out = Arc::new(Mutex::new(String::new()));
    let mut b_reader = None;
    if let Some(b_args) = &s.b {
        let b_args: Vec<&str> = b_args
            .iter()
            .map(|a| {
                if *a == "{blender}" {
                    gate.blender.as_deref().unwrap()
                } else {
                    a
                }
            })
            .collect();
        let b_raw = out.join("raw").join(format!("{}-b.jsonl", s.name));
        let mut child = Command::new(bin.join("s8-job"))
            .args(&b_args)
            .args(["--signal-fifo", &fifo, "--label", s.name])
            .args(["--raw", b_raw.to_str().unwrap()])
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
        rx.recv_timeout(Duration::from_secs(180))
            .expect("s8-job never became ready");
        std::thread::sleep(Duration::from_secs(2));
        b_child = Some(child);
    }

    let stop = Arc::new(AtomicBool::new(false));
    let sampler = sample_util(stop.clone());
    let raw = out.join("raw").join(format!("{}-a.jsonl", s.name));
    let t = Instant::now();
    let a = run_a(
        s,
        bin,
        seconds,
        &[
            "--signal-fifo",
            &fifo,
            "--label",
            s.name,
            "--raw",
            raw.to_str().unwrap(),
        ],
    );
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
    for mut d in desk {
        let _ = d.kill();
        let _ = d.wait();
    }
    let _ = std::fs::remove_file(&fifo);
    let attribution = a
        .as_ref()
        .and_then(|a| a["fps"].as_f64())
        .filter(|_| s.b.is_some())
        .map(|fps| {
            attribute(
                &out.join("raw").join(format!("{}-a.jsonl", s.name)),
                &out.join("raw").join(format!("{}-b.jsonl", s.name)),
                1e9 / fps,
            )
        });
    json!({
        "scenario": s.name,
        "what": s.what,
        "a_args": s.a,
        "b_args": s.b,
        "a_wall_seconds": stats::r(a_secs, 2),
        "display_asleep": display_asleep,
        "desk_load": gate.desk,
        "desk_processes": if gate.desk.is_some() { gate.desk_n } else { 0 },
        "screen_locked": screen_locked,
        "gpu_util_idle_before": stats::dist(&idle),
        "gpu_util_during": stats::dist(&during),
        "control": control,
        "attribution": attribution,
        "a": a,
        "b": b,
    })
}

/// Union of spans, sorted and disjoint (B's command buffers overlap with 2 in flight).
fn merge(mut spans: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    spans.sort_by(|x, y| x.partial_cmp(y).unwrap());
    let mut out: Vec<(f64, f64)> = Vec::with_capacity(spans.len());
    for (x, y) in spans {
        match out.last_mut() {
            Some(last) if x <= last.1 => last.1 = last.1.max(y),
            _ => out.push((x, y)),
        }
    }
    out
}

/// Overlap (ns) of `[a, b)` with sorted, disjoint spans.
fn overlap(spans: &[(f64, f64)], a: f64, b: f64) -> f64 {
    let start = spans.partition_point(|s| s.1 <= a);
    let mut sum = 0.0;
    for &(x, y) in &spans[start..] {
        if x >= b {
            break;
        }
        sum += (y.min(b) - x.max(a)).max(0.0);
    }
    sum
}

/// Attribute A's late frames: for each frame that missed the one-frame deadline, how
/// much of its GPU start delay (submit to first GPU work) and of its GPU execution B's
/// command buffers covered, and whether B was inside a yield window at submit. Classes:
/// `backlog` (A's previous frame was still on the GPU at submit), `b-start` (the start
/// delay exceeded 1 ms and B's command buffers covered at least half of it),
/// `b-concurrent` (A started promptly but ran alongside B for at least half of its
/// execution and was slowed), and `other` (B was mostly idle: the encoder, WindowServer,
/// other apps, or A's CPU side).
fn attribute(a_raw: &std::path::Path, b_raw: &std::path::Path, period_ns: f64) -> Value {
    let read = |p: &std::path::Path| -> Vec<Value> {
        std::fs::read_to_string(p)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    };
    let a = read(a_raw);
    let b = read(b_raw);
    let cbs = merge(
        b.iter()
            .filter(|v| v["kind"] == "cb")
            .filter_map(|v| Some((v["gpu_begin_ns"].as_f64()?, v["gpu_end_ns"].as_f64()?)))
            .collect(),
    );
    let yields: Vec<(f64, f64)> = b
        .iter()
        .filter(|v| v["kind"] == "yield")
        .filter_map(|v| Some((v["start_ns"].as_f64()?, v["end_ns"].as_f64()?)))
        .collect();
    let f = |v: &Value, k: &str| v[k].as_f64();
    let mut rows = Vec::new();
    let mut prev_end = 0.0f64;
    let (mut late, mut b_delay, mut b_conc, mut backlog, mut other, mut skipped) = (0, 0, 0, 0, 0, 0);
    let mut in_yield_n = 0;
    let mut b_share_all = Vec::new();
    for v in &a {
        let Some(tick) = f(v, "tick_ns") else { continue };
        let (Some(submit), Some(begin), Some(end), Some(done)) = (
            f(v, "submit_ns"),
            f(v, "gpu_begin_ns"),
            f(v, "gpu_end_ns"),
            f(v, "done_ns"),
        ) else {
            if v["skipped"] == true {
                skipped += 1;
            }
            continue;
        };
        let delay = (begin - submit).max(0.0);
        let b_in_delay = overlap(&cbs, submit, begin);
        if delay > 0.0 {
            b_share_all.push(b_in_delay / delay);
        }
        // A's previous frame still on the GPU when this one was submitted.
        let behind_self = prev_end > submit;
        prev_end = prev_end.max(end);
        if done - tick <= period_ns {
            continue;
        }
        late += 1;
        let in_yield = yields.iter().any(|&(y0, y1)| submit >= y0 && submit < y1);
        if in_yield {
            in_yield_n += 1;
        }
        let b_in_exec = overlap(&cbs, begin, end);
        let class = if behind_self {
            backlog += 1;
            "backlog"
        } else if delay > 1e6 && b_in_delay >= 0.5 * delay {
            b_delay += 1;
            "b-start"
        } else if b_in_exec >= 0.5 * (end - begin) {
            b_conc += 1;
            "b-concurrent"
        } else {
            other += 1;
            "other"
        };
        if rows.len() < 60 {
            rows.push(json!({
                "t_s": v["t_s"],
                "class": class,
                "latency_ms": stats::r((done - tick) / 1e6, 2),
                "start_delay_ms": stats::r(delay / 1e6, 2),
                "b_in_delay_ms": stats::r(b_in_delay / 1e6, 2),
                "exec_ms": stats::r((end - begin) / 1e6, 2),
                "b_in_exec_ms": stats::r(b_in_exec / 1e6, 2),
                "b_in_yield": in_yield,
            }));
        }
    }
    json!({
        "late_frames": late,
        "skipped_frames": skipped,
        "late_b_held_start": b_delay,
        "late_b_concurrent": b_conc,
        "late_behind_own_backlog": backlog,
        "late_other": other,
        "late_submitted_in_b_yield": in_yield_n,
        "b_share_of_start_delay_all_frames": stats::dist(&b_share_all).rounded(3),
        "late": rows,
    })
}

/// Run process A with the scenario's arguments for `seconds` and parse its summary.
fn run_a(s: &Scenario, bin: &std::path::Path, seconds: f64, extra: &[&str]) -> Option<Value> {
    let out = Command::new(bin.join("s8-interactive"))
        .args(&s.a)
        .args(["--seconds", &seconds.to_string()])
        .args(extra)
        .stderr(Stdio::inherit())
        .output()
        .expect("spawn s8-interactive");
    parse_result(&String::from_utf8_lossy(&out.stdout))
}

/// The paired control: A alone, with the scenario's A arguments, immediately before B
/// starts. Machine load from other processes changes over minutes, so a scenario's
/// drops are attributed to B only when the control just before it was clean.
fn control_summary(a: &Value) -> Value {
    json!({
        "frames": a["frames_measured"],
        "drops_1f": a["drops"]["deadline_1_frame"],
        "drops_2f": a["drops"]["deadline_2_frames"],
        "drops_3f": a["drops"]["deadline_3_frames"],
        "latency_ms": a["latency_ms"],
        "gpu_exec_ms": a["gpu_exec_ms"],
        "presentation": a["presentation"],
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
        "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
        v["scenario"].as_str().unwrap_or("?"),
        if v["control"].is_null() {
            "-".to_string()
        } else {
            v["control"]["drops_1f"].to_string()
        },
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
    if let Some(dir) = args.str("reattribute") {
        // Recompute the attribution of every scenario JSON in `dir` from its raw logs.
        let dir = std::path::Path::new(dir);
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|n| n.ends_with(".json") && n != "summary.json")
            .collect();
        names.sort();
        for n in names {
            let path = dir.join(&n);
            let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            let name = v["scenario"].as_str().unwrap_or_default().to_string();
            let (Some(fps), false) = (v["a"]["fps"].as_f64(), v["b"].is_null()) else {
                continue;
            };
            v["attribution"] = attribute(
                &dir.join("raw").join(format!("{name}-a.jsonl")),
                &dir.join("raw").join(format!("{name}-b.jsonl")),
                1e9 / fps,
            );
            std::fs::write(&path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
            eprintln!("{name}: {}", v["attribution"]["late_frames"]);
        }
        return;
    }
    let seconds: f64 = args.num("seconds", 60.0);
    let gate = Gate {
        max_ext: args.num::<f64>("max-external-pct", 10.0) / 100.0,
        wait_s: args.num("quiet-wait-s", 900.0),
        retries: args.num("retries", 2),
        blender: args.str("blender").map(str::to_string),
        control_s: args.num("control-s", 20.0),
        desk: args.str("desk").map(str::to_string),
        desk_n: args.num("desk-n", 1),
    };
    if let Some(p) = &gate.blender {
        assert!(std::path::Path::new(p).is_file(), "--blender {p}: no such file");
    }
    let only: Option<Vec<String>> = args.str("only").map(|o| o.split(',').map(str::to_string).collect());
    let bin = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
    let out = std::path::PathBuf::from(args.string_or("out", "results/latest"));
    std::fs::create_dir_all(out.join("raw")).unwrap();
    let header = "| scenario | ctl 1f | frames | drops 1f | drops 2f | drops 3f | lat p50 | lat p99 | lat max | jitter p99 | gpu start delay p99 | B fps | B cb p99 ms | VT fps |\n|---|---|---|---|---|---|---|---|---|---|---|---|---|---|";
    println!("{header}");
    let mut results = Vec::new();
    for s in all
        .iter()
        .filter(|s| only.as_ref().is_none_or(|o| o.iter().any(|n| n == s.name)))
    {
        eprintln!("== {} ({})", s.name, s.what);
        let needs_blender = s.b.as_ref().is_some_and(|b| b.contains(&"{blender}"));
        if needs_blender && gate.blender.is_none() {
            eprintln!("   skipped: needs --blender");
            continue;
        }
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
    fn overlap_sums_intersections() {
        let spans = merge(vec![(20.0, 30.0), (0.0, 10.0), (25.0, 40.0), (2.0, 5.0)]);
        assert_eq!(spans, vec![(0.0, 10.0), (20.0, 40.0)]);
        assert_eq!(overlap(&spans, 5.0, 22.0), 7.0);
        assert_eq!(overlap(&spans, 10.0, 20.0), 0.0);
        assert_eq!(overlap(&spans, 26.0, 28.0), 2.0);
        assert_eq!(overlap(&spans, 35.0, 60.0), 5.0);
    }

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
    }
}
