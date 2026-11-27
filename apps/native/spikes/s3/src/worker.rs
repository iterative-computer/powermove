use crate::{
    color,
    policy::{Output, Policy, Stamp, percentile},
    render::{Engine, Presentation, Scene},
};
use pm_protocol::common::{Size, Ticks};
use serde_json::json;
use std::{
    sync::{Arc, Mutex, atomic::Ordering},
    time::{Duration, Instant},
};

pub struct Control {
    pub policy: Policy,
    pub rect: [f32; 4],
    pub stop: bool,
    pub appkit_inputs: u64,
    pub superseded: u64,
    pub selected_time: Option<f64>,
    pub seek: Option<(f64, f64, f64, Stamp)>,
}
#[derive(Clone, Debug)]
pub struct Update {
    pub time: f64,
    pub finished: bool,
    pub error: Option<String>,
}
pub struct Host {
    pub clock: crate::mailbox::Clock,
    pub control: Arc<Mutex<Control>>,
    pub updates: async_channel::Receiver<Update>,
    pub renders: Arc<std::sync::atomic::AtomicU64>,
    pub presents: Arc<std::sync::atomic::AtomicU64>,
}
pub struct Options {
    pub scene: Scene,
    pub pq: Option<Scene>,
    pub seconds: f64,
    pub hz: f64,
    pub verify: bool,
    pub measure_overlays: bool,
    pub report: String,
    pub mode: String,
    pub legacy_sync: bool,
    pub synthetic_scrub: bool,
    pub cache: bool,
}
impl Host {
    pub fn start(
        gpu: pm_gpu::Gpu,
        layer: metal::MetalLayer,
        output: Output,
        clock: crate::mailbox::Clock,
        layer_updates: Arc<std::sync::atomic::AtomicU64>,
        options: Options,
    ) -> Self {
        let control = Arc::new(Mutex::new(Control {
            policy: Policy::new(output),
            rect: [0.1, 0.1, 0.9, 0.9],
            stop: false,
            appkit_inputs: 0,
            superseded: 0,
            selected_time: None,
            seek: None,
        }));
        let renders = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let (tx, updates) = async_channel::bounded(1);
        let c = control.clone();
        let redraws = renders.clone();
        let presents = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let commits = presents.clone();
        let input_clock = clock.clone();
        std::thread::Builder::new()
            .name("s3-engine-viewer".into())
            .spawn(move || {
                let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
                let timer = (options.mode == "hidden")
                    .then(|| timer(clock.clone(), options.hz, done.clone()));
                let result = run(
                    gpu,
                    Some(layer),
                    clock,
                    &c,
                    &tx,
                    Counters {
                        redraws: &redraws,
                        gpui_commits: &commits,
                        layer_updates: &layer_updates,
                    },
                    options,
                );
                done.store(true, Ordering::Release);
                if let Some(timer) = timer {
                    timer.join().unwrap();
                }
                let update = Update {
                    time: 0.,
                    finished: true,
                    error: result.err().map(|e| format!("{e:#}")),
                };
                let _ = tx.send_blocking(update);
            })
            .unwrap();
        Self {
            clock: input_clock,
            control,
            updates,
            renders,
            presents,
        }
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        self.control.lock().unwrap().stop = true;
    }
}
fn ticks(t: f64) -> Ticks {
    Ticks::new((t * pm_time::TICKS_PER_SECOND as f64).round() as i64).unwrap()
}
fn now() -> f64 {
    objc2_quartz_core::CACurrentMediaTime()
}
fn accuracy(
    engine: &mut Engine,
    scene: &Scene,
    layer: Option<&metal::MetalLayer>,
    output: Output,
) -> anyhow::Result<serde_json::Value> {
    let t = ticks(0.13);
    let reference = engine.export(scene, t, Size::new(output.width, output.height))?;
    let sub = engine
        .frame(
            scene,
            t,
            output,
            Presentation {
                layer,
                rect: [0.; 4],
                overlay: 0,
            },
            true,
            Arc::new(Mutex::new(Default::default())),
        )?
        .ok_or_else(|| anyhow::anyhow!("no drawable in accuracy run"))?;
    sub.commit();
    sub.complete()?;
    let pixels = sub.pixels();
    anyhow::ensure!(
        reference.len() == pixels.len() * 4 && !pixels.is_empty(),
        "drawable/export pixel count mismatch"
    );
    anyhow::ensure!(
        reference.iter().all(|v| v.is_finite()) && pixels.iter().flatten().all(|v| v.is_finite()),
        "non-finite accuracy pixels"
    );
    let mut errors = Vec::new();
    let mut maximum_linear = 0_f64;
    for (actual, reference) in pixels.iter().zip(reference.chunks_exact(4)) {
        let expected = color::p3(reference);
        maximum_linear = maximum_linear.max(expected.into_iter().fold(0_f64, f64::max));
        errors.push(color::delta(*actual, expected));
    }
    let report = json!({"pixels":pixels.len(),"deltaE2000_p50":percentile(&errors,0.5),"deltaE2000_p99":percentile(&errors,0.99),"deltaE2000_max":percentile(&errors,1.),"export_peak_linear_P3":maximum_linear,"passes_deltaE_gate":percentile(&errors,0.99)<=1. && percentile(&errors,1.)<=2.,"overlays":false,"readback":if layer.is_some() { "drawable blit; independent export render" } else { "offscreen texture blit; independent export render; NOT display accuracy" }});
    Ok(report)
}
struct Counters<'a> {
    redraws: &'a std::sync::atomic::AtomicU64,
    gpui_commits: &'a std::sync::atomic::AtomicU64,
    layer_updates: &'a std::sync::atomic::AtomicU64,
}

fn run(
    gpu: pm_gpu::Gpu,
    layer: Option<metal::MetalLayer>,
    clock: crate::mailbox::Clock,
    c: &Arc<Mutex<Control>>,
    tx: &async_channel::Sender<Update>,
    counters: Counters<'_>,
    options: Options,
) -> anyhow::Result<()> {
    let Counters {
        redraws,
        gpui_commits,
        layer_updates,
    } = counters;
    set_interactive();
    let mut engine = Engine::new(gpu)?;
    engine.legacy_sync = options.legacy_sync;
    engine.cache_enabled = options.cache;
    if options.mode != "offscreen" {
        std::thread::sleep(Duration::from_millis(1500));
    }
    let output = c.lock().unwrap().policy.output;
    let mut accuracy_rows = json!({});
    if options.verify {
        let target = layer.as_ref();
        accuracy_rows["edr_target"] = edr_probe(&mut engine, &options.scene, target, output)?;
        accuracy_rows["sdr"] = accuracy(&mut engine, &options.scene, target, output)?;
        if let Some(pq) = &options.pq {
            // Different immutable scene: do not let a matching revision/time
            // from the previous scene satisfy this independent oracle.
            engine.clear_cache();
            accuracy_rows["pq"] = accuracy(&mut engine, pq, target, output)?;
            engine.clear_cache();
        }
    }
    let interval = 1. / options.hz;
    let start = Instant::now();
    let mut last_sequence = None;
    let mut last_timestamp = None;
    let mut tick_gaps = Vec::with_capacity(8000);
    let mut callback_jitter = Vec::with_capacity(8000);
    let mut missed_ticks = 0;
    let mut no_drawable = 0;
    let mut stale = 0;
    let mut seek_retries = 0;
    let mut blocked = Vec::new();
    let mut traces = Vec::with_capacity(8000);
    let mut pending = std::collections::VecDeque::<crate::render::Submission>::with_capacity(2);
    let mut overlay_rows = Vec::new();
    let mut gpui_at_start = None;
    let mut commits_at_start = None;
    let mut layers_at_start = None;
    let mut paused = false;
    let mut stimulus = Stimulus::start(c.clone(), clock.clone(), options.synthetic_scrub);
    let mut ticks_received = 0;
    let mut schedule = crate::policy::PlaybackSchedule::default();
    let mut predicted_frames = 0;
    while start.elapsed().as_secs_f64() < options.seconds + 1. {
        let elapsed = start.elapsed().as_secs_f64();
        let foreground_clock = options.mode == "foreground" && !paused;
        let event = if foreground_clock {
            clock.next_for(schedule.wait(now(), interval))
        } else {
            clock.next()
        };
        if c.lock().unwrap().stop {
            break;
        }
        let (due, callback) = match event {
            crate::mailbox::Event::Tick(tick) => {
                ticks_received += 1;
                if let Some(last) = last_sequence {
                    missed_ticks += tick.sequence - last - 1;
                }
                last_sequence = Some(tick.sequence);
                if let Some(last) = last_timestamp {
                    tick_gaps.push((tick.timestamp - last) * 1000.);
                }
                last_timestamp = Some(tick.timestamp);
                callback_jitter.push((now() - tick.callback) * 1000.);
                if paused {
                    continue;
                }
                if foreground_clock {
                    schedule.observe(tick.target, interval);
                    let Some(due) = schedule.take(now(), interval) else {
                        continue;
                    };
                    (due, tick.callback)
                } else {
                    (tick.target, tick.callback)
                }
            }
            crate::mailbox::Event::Seek => (0., 0.),
            crate::mailbox::Event::Timeout => {
                if !foreground_clock {
                    continue;
                }
                let Some(due) = schedule.take(now(), interval) else {
                    continue;
                };
                predicted_frames += 1;
                (due, 0.)
            }
        };
        // Completion status retires surfaces without a per-frame CPU wait.
        while pending
            .front()
            .is_some_and(|s| s.command.status() == metal::MTLCommandBufferStatus::Completed)
        {
            pending.pop_front();
        }
        if pending.len() == 2 {
            let began = now();
            pending.pop_front().unwrap().complete()?;
            blocked.push((now() - began) * 1000.);
        }
        let (output, stamp, rect, seek) = {
            let mut ctl = c.lock().unwrap();
            let seek = ctl.seek.take();
            if let Some(s) = seek {
                ctl.selected_time = Some(s.0);
            }
            let stamp = if let Some((_, _, _, stamp)) = seek {
                stamp
            } else {
                ctl.policy.request(options.scene.doc.revision())
            };
            (ctl.policy.output, stamp, ctl.rect, seek)
        };
        if due == 0. && seek.is_none() {
            continue;
        }
        let time = seek.map_or(elapsed % 2.5, |s| s.0);
        // Seeking is a transport discontinuity. Keep the selected frame, rather
        // than immediately overwriting it with the next playback tick.
        paused |= seek.is_some();
        let trace = Arc::new(Mutex::new(crate::trace::Trace {
            id: traces.len() as u64,
            measured: elapsed >= 1.,
            scrub: seek.is_some_and(|s| s.1 > 0.),
            input: seek.map_or(0., |s| s.1),
            request: seek.map_or(callback, |s| s.2),
            worker_start: now(),
            link_callback: callback,
            target: due,
            ..Default::default()
        }));
        let sub = engine.frame(
            &options.scene,
            ticks(time),
            output,
            Presentation {
                layer: layer.as_ref(),
                rect,
                overlay: 1,
            },
            options.measure_overlays,
            trace.clone(),
        )?;
        let Some(sub) = sub else {
            no_drawable += 1;
            seek_retries += u64::from(retry_seek(c, &clock, seek));
            continue;
        };
        let current = c.lock().unwrap();
        if !current.policy.accepts(stamp) {
            // Producer work was already submitted. The source remains leased
            // until it finishes even when no consumer will present it.
            drop(current);
            sub.finish_producer()?;
            seek_retries += u64::from(retry_seek(c, &clock, seek));
            stale += 1;
            continue;
        }
        sub.submit((seek.is_none() && due > 0.).then_some(due));
        drop(current);
        if options.legacy_sync || options.measure_overlays {
            sub.complete()?;
        }
        if options.measure_overlays {
            overlay_rows.push(measure_overlay(&sub, rect, trace.lock().unwrap().id, time));
        }
        traces.push(trace);
        pending.push_back(sub);
        if elapsed >= 1. {
            layers_at_start.get_or_insert_with(|| layer_updates.load(Ordering::Acquire));
            commits_at_start.get_or_insert_with(|| gpui_commits.load(Ordering::Acquire));
            gpui_at_start.get_or_insert_with(|| redraws.load(Ordering::Acquire));
        }
        let _ = tx.try_send(Update {
            time,
            finished: false,
            error: None,
        });
    }
    stimulus.stop();
    for sub in pending {
        sub.complete()?;
    }
    // WindowServer may not present a hidden/locked layer. Leave missing/zero
    // callback counts visible instead of converting GPU completion to display.
    std::thread::sleep(Duration::from_millis(300));
    let rows: Vec<_> = traces.iter().map(|t| t.lock().unwrap().json()).collect();
    let mut report = crate::metrics::report(&rows, interval, &options.mode);
    report["predicted_phase_frames"] = json!(predicted_frames);
    report["expired_playback_targets"] = json!(schedule.skipped);
    report["requested_seconds"] = json!(options.seconds);
    report["requested_hz"] = json!(options.hz);
    report["legacy_sync"] = json!(options.legacy_sync);
    report["cache_enabled"] = json!(options.cache);
    report["synthetic_scrub_requests"] = json!(stimulus.count.load(Ordering::Acquire));
    report["display_link_ticks_received"] = json!(ticks_received);
    report["display_link_ticks_missed"] = json!(missed_ticks);
    report["display_link_gap_ms"] = crate::metrics::distribution(&tick_gaps);
    report["callback_to_worker_ms"] = crate::metrics::distribution(&callback_jitter);
    report["in_flight_bound_wait_ms"] = crate::metrics::distribution(&blocked);
    report["seek_retries"] = json!(seek_retries);
    report["no_drawable"] = json!(no_drawable);
    report["stale_rejected"] = json!(stale);
    report["appkit_input_events"] = json!(c.lock().unwrap().appkit_inputs);
    report["superseded_requests"] = json!(c.lock().unwrap().superseded);
    report["accuracy"] = accuracy_rows;
    report["overlay_measurements"] = json!(overlay_rows);
    report["native_layer_updates_during_measurement"] =
        json!(layer_updates.load(Ordering::Acquire) - layers_at_start.unwrap_or(0));
    report["gpui_presents_during_measurement"] =
        json!(gpui_commits.load(Ordering::Acquire) - commits_at_start.unwrap_or(0));
    report["gpui_paints_during_measurement"] =
        json!(redraws.load(Ordering::Acquire) - gpui_at_start.unwrap_or(0));
    std::fs::write(
        format!("{}.trace.json", options.report),
        serde_json::to_vec(&rows)?,
    )?;
    std::fs::write(&options.report, serde_json::to_vec_pretty(&report)?)?;
    eprintln!("S3 report {report}");
    // Collect every row and save the report, then preserve --verify failure
    // status. A complete measurement report must not turn a bad oracle green.
    let inaccurate = report["accuracy"].as_object().unwrap().values().any(|row| {
        row.get("passes_deltaE_gate")
            .and_then(serde_json::Value::as_bool)
            == Some(false)
    });
    anyhow::ensure!(
        !inaccurate,
        "drawable/export accuracy gate failed; see {}",
        options.report
    );
    Ok(())
}

// Compare the observed overlay mask against its content-viewport coordinates
// for EVERY captured frame. No screenshot or GPUI timestamp is used as a proxy.
fn measure_overlay(
    sub: &crate::render::Submission,
    rect: [f32; 4],
    frame: u64,
    time: f64,
) -> serde_json::Value {
    let pixels = sub.pixels();
    let mut mismatches = 0;
    let mut observed = [u32::MAX, u32::MAX, 0, 0];
    let mut expected = observed;
    let mut count = 0;
    for (i, p) in pixels.iter().enumerate() {
        let x = i as u32 % sub.width;
        let y = i as u32 / sub.width;
        let uv = [
            (x as f32 + 0.5) / sub.width as f32,
            (y as f32 + 0.5) / sub.height as f32,
        ];
        let dx = (uv[0] - (rect[0] + rect[2]) * 0.5).abs() - (rect[2] - rect[0]) * 0.5;
        let dy = (uv[1] - (rect[1] + rect[3]) * 0.5).abs() - (rect[3] - rect[1]) * 0.5;
        let wanted = dx.max(dy) < 0.002 && dx.max(dy) > -0.002;
        let actual = (p[0] - 0.2).abs() < 0.0003
            && (p[1] - 0.8).abs() < 0.0003
            && (p[2] - 0.45).abs() < 0.0003;
        if actual {
            observed = [
                observed[0].min(x),
                observed[1].min(y),
                observed[2].max(x),
                observed[3].max(y),
            ];
            count += 1;
        }
        if wanted {
            expected = [
                expected[0].min(x),
                expected[1].min(y),
                expected[2].max(x),
                expected[3].max(y),
            ];
        }
        if wanted != actual {
            mismatches += 1;
        }
    }
    let offset: Vec<i64> = observed
        .iter()
        .zip(expected)
        .map(|(a, b)| *a as i64 - b as i64)
        .collect();
    json!({"frame":frame,"content_time":time,"overlay_pixels":count,"mask_mismatches":mismatches,"edge_offset_px":offset})
}

fn edr_probe(
    engine: &mut Engine,
    scene: &Scene,
    layer: Option<&metal::MetalLayer>,
    output: Output,
) -> anyhow::Result<serde_json::Value> {
    let sub = engine
        .frame(
            scene,
            ticks(0.),
            output,
            Presentation {
                layer,
                rect: [0.; 4],
                overlay: 2,
            },
            true,
            Arc::new(Mutex::new(Default::default())),
        )?
        .ok_or_else(|| anyhow::anyhow!("no EDR probe drawable"))?;
    sub.commit();
    sub.complete()?;
    let pixels = sub.pixels();
    let patches: Vec<_> = (0..5)
        .map(|i| {
            let x = ((i as f64 + 0.5) / 5. * sub.width as f64) as usize;
            pixels[(sub.height as usize / 2) * sub.width as usize + x][0]
        })
        .collect();
    Ok(
        json!({"current_headroom":output.headroom,"pixel_format":"RGBA16Float","colorspace":"kCGColorSpaceExtendedLinearDisplayP3","edr_enabled":layer.is_some(),"requested_linear_patches":[0.5,1.,2.,4.,8.],"drawable_linear_patches":patches,"limit":if layer.is_some() {"pre-compositor drawable values; no photometer measurement"} else {"offscreen texture values; no compositor or display"}}),
    )
}

fn set_interactive() {
    unsafe extern "C" {
        fn pthread_set_qos_class_self_np(qos: u32, relative: i32) -> i32;
    }
    unsafe {
        pthread_set_qos_class_self_np(0x21, 0);
    }
}

/// No AppKit/WindowServer/display link at all; the clock is explicitly synthetic.
pub fn offscreen(gpu: pm_gpu::Gpu, options: Options) -> anyhow::Result<()> {
    let clock = crate::mailbox::Clock::default();
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let thread = timer(clock.clone(), options.hz, done.clone());
    let c = Arc::new(Mutex::new(Control {
        policy: Policy::new(Output {
            width: 1280,
            height: 720,
            display: 0,
            headroom: 1.,
        }),
        rect: [0.1, 0.1, 0.9, 0.9],
        stop: false,
        appkit_inputs: 0,
        superseded: 0,
        selected_time: None,
        seek: None,
    }));
    let (tx, _) = async_channel::bounded(1);
    let counter = std::sync::atomic::AtomicU64::new(0);
    let result = run(
        gpu,
        None,
        clock,
        &c,
        &tx,
        Counters {
            redraws: &counter,
            gpui_commits: &counter,
            layer_updates: &counter,
        },
        options,
    );
    done.store(true, Ordering::Release);
    thread.join().unwrap();
    result
}

fn timer(
    clock: crate::mailbox::Clock,
    hz: f64,
    stop: Arc<std::sync::atomic::AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        set_interactive();
        let began = Instant::now();
        let host = now();
        let mut sequence = 0;
        while !stop.load(Ordering::Acquire) {
            let due = Duration::from_secs_f64(sequence as f64 / hz);
            if let Some(wait) = due.checked_sub(began.elapsed()) {
                std::thread::sleep(wait);
            }
            let timestamp = host + sequence as f64 / hz;
            clock.tick(crate::mailbox::Tick {
                sequence,
                timestamp,
                target: timestamp + 1. / hz,
                callback: now(),
            });
            sequence += 1;
        }
    })
}

/// A separate synthetic input producer exercises the SAME queue/wake as AppKit.
/// It is explicitly not a measurement of HID/AppKit delivery.
struct Stimulus {
    stop: Arc<std::sync::atomic::AtomicBool>,
    count: Arc<std::sync::atomic::AtomicU64>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Stimulus {
    fn start(c: Arc<Mutex<Control>>, clock: crate::mailbox::Clock, enabled: bool) -> Self {
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let count = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let done = stop.clone();
        let sent = count.clone();
        let thread = enabled.then(|| {
            std::thread::spawn(move || {
                let began = Instant::now();
                for i in 0..120 {
                    let due = Duration::from_secs_f64(2. + i as f64 * 0.05);
                    while let Some(wait) = due.checked_sub(began.elapsed()) {
                        if done.load(Ordering::Acquire) {
                            return;
                        }
                        std::thread::sleep(wait.min(Duration::from_millis(50)));
                    }
                    if done.load(Ordering::Acquire) {
                        return;
                    }
                    let input = now();
                    let mut ctl = c.lock().unwrap();
                    let revision = ctl.policy.stamp.revision;
                    let stamp = ctl.policy.request(revision);
                    ctl.superseded += u64::from(ctl.seek.is_some());
                    ctl.seek = Some((((i * 3) % 8) as f64 / 8., input, now(), stamp));
                    drop(ctl);
                    clock.seek();
                    sent.fetch_add(1, Ordering::Release);
                }
            })
        });
        Self {
            stop,
            count,
            thread,
        }
    }
    fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            t.join().unwrap();
        }
    }
}
impl Drop for Stimulus {
    fn drop(&mut self) {
        self.stop();
    }
}

// A paused seek must survive drawable loss/resize, unless newer input replaced
// it. Keep the ORIGINAL input/request times so retries cannot hide latency.
fn retry_seek(
    c: &Mutex<Control>,
    clock: &crate::mailbox::Clock,
    seek: Option<(f64, f64, f64, Stamp)>,
) -> bool {
    let Some((time, input, request, _)) = seek else {
        return false;
    };
    let mut ctl = c.lock().unwrap();
    if ctl.seek.is_some() || ctl.stop {
        return false;
    }
    let revision = ctl.policy.stamp.revision;
    let stamp = ctl.policy.request(revision);
    ctl.seek = Some((time, input, request, stamp));
    drop(ctl);
    clock.seek();
    true
}
#[cfg(test)]
mod retry_tests {
    use super::*;
    #[test]
    fn drawable_retry_keeps_latency_origin_and_never_overwrites_new_input() {
        let mut policy = Policy::new(Output {
            width: 1280,
            height: 720,
            display: 0,
            headroom: 1.,
        });
        let stamp = policy.request(1);
        let original = Some((0.5, 10., 10.001, stamp));
        let c = Mutex::new(Control {
            policy,
            rect: [0.; 4],
            stop: false,
            appkit_inputs: 0,
            superseded: 0,
            selected_time: None,
            seek: None,
        });
        let clock = crate::mailbox::Clock::default();
        assert!(retry_seek(&c, &clock, original));
        let pending = c.lock().unwrap().seek.unwrap();
        assert_eq!((pending.0, pending.1, pending.2), (0.5, 10., 10.001));
        assert_ne!(pending.3, stamp);
        assert!(matches!(clock.next(), crate::mailbox::Event::Seek));
        assert!(!retry_seek(&c, &clock, Some((0.1, 9., 9.001, stamp))));
        assert_eq!(c.lock().unwrap().seek.unwrap(), pending);
    }
}

impl Control {
    /// Repaint a paused selection after resize/display retirement. This is not
    /// new input, so its zero input origin must stay out of scrub distributions.
    pub fn configure_output(&mut self, output: Output, request: f64) -> bool {
        let generation = self.policy.stamp.generation;
        self.policy.configure(output);
        if generation == self.policy.stamp.generation || self.seek.is_some() {
            return false;
        }
        let Some(time) = self.selected_time else {
            return false;
        };
        let revision = self.policy.stamp.revision;
        let stamp = self.policy.request(revision);
        self.seek = Some((time, 0., request, stamp));
        true
    }
}

#[cfg(test)]
mod output_tests {
    use super::*;
    #[test]
    fn paused_resize_requeues_selection_without_inventing_new_input() {
        let output = Output {
            width: 1280,
            height: 720,
            display: 0,
            headroom: 1.,
        };
        let mut c = Control {
            policy: Policy::new(output),
            rect: [0.; 4],
            stop: false,
            appkit_inputs: 0,
            superseded: 0,
            selected_time: Some(0.5),
            seek: None,
        };
        let resized = Output {
            width: 640,
            ..output
        };
        assert!(c.configure_output(resized, 12.));
        let s = c.seek.unwrap();
        assert_eq!((s.0, s.1, s.2), (0.5, 0., 12.));
        assert!(c.policy.accepts(s.3));
        assert!(!c.configure_output(resized, 13.));
        assert_eq!(c.seek.unwrap(), s);
    }
}
