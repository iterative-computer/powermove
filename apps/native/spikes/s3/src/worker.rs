use crate::{
    color,
    policy::{Output, Policy, Stamp, percentile},
    render::{Engine, Scene},
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
    pub seek: Option<(f64, f64, Stamp)>,
}
#[derive(Clone, Debug)]
pub struct Update {
    pub time: f64,
    pub finished: bool,
    pub error: Option<String>,
}
pub struct Host {
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
}
impl Host {
    pub fn start(
        gpu: pm_gpu::Gpu,
        layer: metal::MetalLayer,
        output: Output,
        clock: std::sync::mpsc::Receiver<crate::pacing::Tick>,
        layer_updates: Arc<std::sync::atomic::AtomicU64>,
        options: Options,
    ) -> Self {
        let control = Arc::new(Mutex::new(Control {
            policy: Policy::new(output),
            rect: [0.1, 0.1, 0.9, 0.9],
            stop: false,
            appkit_inputs: 0,
            seek: None,
        }));
        let renders = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let (tx, updates) = async_channel::bounded(1);
        let c = control.clone();
        let redraws = renders.clone();
        let presents = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let commits = presents.clone();
        std::thread::Builder::new()
            .name("s3-engine-viewer".into())
            .spawn(move || {
                let result = run(
                    gpu,
                    layer,
                    clock,
                    &c,
                    &tx,
                    &redraws,
                    &commits,
                    &layer_updates,
                    options,
                );
                let update = Update {
                    time: 0.,
                    finished: true,
                    error: result.err().map(|e| format!("{e:#}")),
                };
                let _ = tx.send_blocking(update);
            })
            .unwrap();
        Self {
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
    layer: &metal::MetalLayer,
    output: Output,
) -> anyhow::Result<serde_json::Value> {
    let t = ticks(0.13);
    let reference = engine.export(scene, t, Size::new(output.width, output.height))?;
    let sub = engine
        .encode(scene, t, output, layer, [0.; 4], 0, true)?
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
    let report = json!({"pixels":pixels.len(),"deltaE2000_p50":percentile(&errors,0.5),"deltaE2000_p99":percentile(&errors,0.99),"deltaE2000_max":percentile(&errors,1.),"export_peak_linear_P3":maximum_linear,"passes_deltaE_gate":percentile(&errors,0.99)<=1. && percentile(&errors,1.)<=2.,"overlays":false,"readback":"actual drawable blit before present; pm-export::render_frame independent render"});
    Ok(report)
}
fn run(
    gpu: pm_gpu::Gpu,
    layer: metal::MetalLayer,
    clock: std::sync::mpsc::Receiver<crate::pacing::Tick>,
    c: &Mutex<Control>,
    tx: &async_channel::Sender<Update>,
    redraws: &std::sync::atomic::AtomicU64,
    gpui_commits: &std::sync::atomic::AtomicU64,
    layer_updates: &std::sync::atomic::AtomicU64,
    options: Options,
) -> anyhow::Result<()> {
    let mut engine = Engine::new(gpu)?;
    // Wait for the first committed GPUI slot geometry. No worker→main waits.
    std::thread::sleep(Duration::from_millis(1500));
    let output = c.lock().unwrap().policy.output;
    let mut accuracy_rows = json!({});
    if options.verify {
        accuracy_rows["edr_drawable"] = edr_probe(&mut engine, &options.scene, &layer, output)?;
        accuracy_rows["sdr"] = accuracy(&mut engine, &options.scene, &layer, output)?;
        if let Some(pq) = &options.pq {
            accuracy_rows["pq"] = accuracy(&mut engine, pq, &layer, output)?;
        }
    }
    let interval = 1. / options.hz;
    let start = Instant::now();
    while clock.try_recv().is_ok() {}
    let mut first_sequence = None;
    let mut last_sequence = None;
    let mut tick_gaps = Vec::new();
    let mut last_timestamp = None;
    let mut missed_ticks = 0;
    let mut work = Vec::new();
    let mut stale = 0;
    let mut no_drawable = 0;
    let mut late = 0;
    let presented = Arc::new(Mutex::new(Vec::<(u64, f64, f64, f64)>::new()));
    let mut gpui_at_start = None;
    let mut commits_at_start = None;
    let mut layers_at_start = None;
    let mut overlay_rows = Vec::new();
    while start.elapsed().as_secs_f64() < options.seconds + 1. {
        let tick = clock.recv_timeout(Duration::from_secs(2))?;
        let first = *first_sequence.get_or_insert(tick.sequence);
        let index = tick.sequence - first;
        if let Some(last) = last_sequence {
            missed_ticks += tick.sequence - last - 1;
        }
        last_sequence = Some(tick.sequence);
        if let Some(last) = last_timestamp {
            tick_gaps.push((tick.timestamp - last) * 1000.);
        }
        last_timestamp = Some(tick.timestamp);
        let due = tick.target;
        let began = Instant::now();
        let (output, stamp, rect, seek) = {
            let mut ctl = c.lock().unwrap();
            if ctl.stop {
                break;
            }
            let stamp = if let Some((_, _, stamp)) = ctl.seek {
                stamp
            } else {
                ctl.policy.request(options.scene.doc.revision())
            };
            (ctl.policy.output, stamp, ctl.rect, ctl.seek.take())
        };
        let time = seek.map_or((index as f64 * interval) % 2.5, |s| s.0);
        let input = seek.map_or(now(), |s| s.1);
        let Some(sub) = engine.encode(
            &options.scene,
            ticks(time),
            output,
            &layer,
            rect,
            1,
            options.measure_overlays,
        )?
        else {
            no_drawable += 1;
            continue;
        };
        let current = c.lock().unwrap();
        if !current.policy.accepts(stamp) {
            stale += 1;
            continue;
        }
        let samples = presented.clone();
        let measured = index;
        let scrub = seek.is_some();
        let handler = block::ConcreteBlock::new(move |d: &metal::DrawableRef| {
            let t = d.presented_time();
            samples
                .lock()
                .unwrap()
                .push((measured, t, due, if scrub { input } else { 0. }));
        })
        .copy();
        sub.drawable.add_presented_handler(&handler);
        // Present at the next refresh target. Queue is bounded to one GPU buffer.
        unsafe {
            use foreign_types::ForeignTypeRef;
            use objc2::{msg_send, runtime::AnyObject};
            let _: () = msg_send![sub.command.as_ptr() as *mut AnyObject,presentDrawable:sub.drawable.as_ptr() as *mut AnyObject,atTime:due];
        }
        sub.command.commit();
        drop(current);
        sub.complete()?;
        if options.measure_overlays {
            overlay_rows.push(measure_overlay(&sub, rect, index, time));
        }
        // Retained surface lives through completion, including errors and resize.
        debug_assert!(Arc::strong_count(&sub.surface) >= 2);
        if index >= options.hz as u64 {
            layers_at_start.get_or_insert_with(|| layer_updates.load(Ordering::Acquire));
            commits_at_start.get_or_insert_with(|| gpui_commits.load(Ordering::Acquire));
            gpui_at_start.get_or_insert_with(|| redraws.load(Ordering::Acquire));
            let elapsed = began.elapsed().as_secs_f64() * 1000.;
            work.push(elapsed);
            if elapsed > interval * 1000. {
                late += 1
            }
        }
        let _ = tx.try_send(Update {
            time,
            finished: false,
            error: None,
        });
    }
    std::thread::sleep(Duration::from_millis(300));
    let samples = presented.lock().unwrap();
    let measured: Vec<_> = samples
        .iter()
        .filter(|s| s.0 >= options.hz as u64)
        .collect();
    let scanout: Vec<_> = measured.iter().filter(|s| s.1 > 0.).map(|s| s.1).collect();
    let gaps: Vec<_> = scanout.windows(2).map(|w| (w[1] - w[0]) * 1000.).collect();
    let scrub: Vec<_> = samples
        .iter()
        .filter(|s| s.3 > 0. && s.1 > 0.)
        .map(|s| (s.1 - s.3) * 1000.)
        .collect();
    let dropped = measured.iter().filter(|s| s.1 == 0.).count();
    let report = json!({"pacing":"NSView CADisplayLink, preferredFrameRateRange", "display_link_gap_ms":{"p50":percentile(&tick_gaps,0.5),"p95":percentile(&tick_gaps,0.95)},"display_link_ticks_missed":missed_ticks,"requested_hz":options.hz,"requested_seconds":options.seconds,"measured_frames":work.len(),"presented_callbacks":measured.len(),"not_presented":dropped,"missing_presented_callbacks":work.len().saturating_sub(measured.len()),"scanout_gaps_over_1_5_intervals":gaps.iter().filter(|g| **g > interval*1500.).count(),"presentation_late_ms_p95":percentile(&measured.iter().filter(|s| s.1>0.).map(|s| (s.1-s.2)*1000.).collect::<Vec<_>>(),0.95),"late_or_missed_schedule":late,"no_drawable":no_drawable,"stale_rejected":stale,"frame_ms":{"p50":percentile(&work,0.5),"p95":percentile(&work,0.95)},"scanout_gap_ms":{"p50":percentile(&gaps,0.5),"p95":percentile(&gaps,0.95)},"appkit_input_events":c.lock().unwrap().appkit_inputs,"scrub_ms":{"samples":scrub.len(),"p50":percentile(&scrub,0.5),"p95":percentile(&scrub,0.95)},"headroom_budget_ms":interval*500.,"native_layer_updates_during_measurement":layer_updates.load(Ordering::Acquire)-layers_at_start.unwrap_or(0),"overlay_measurements":overlay_rows,"gpui_presents_during_measurement":gpui_commits.load(Ordering::Acquire)-commits_at_start.unwrap_or(0),"gpui_paints_during_measurement":redraws.load(Ordering::Acquire)-gpui_at_start.unwrap_or(0),"accuracy":accuracy_rows,"scope":"one public corpus project; silent transport; scrub origin is NSEvent timestamp for AppKit clicks, enqueue time for exercise seeks; no paired Electron baseline"});
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
    layer: &metal::MetalLayer,
    output: Output,
) -> anyhow::Result<serde_json::Value> {
    let sub = engine
        .encode(scene, ticks(0.), output, layer, [0.; 4], 2, true)?
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
        json!({"current_headroom":output.headroom,"pixel_format":"RGBA16Float","colorspace":"kCGColorSpaceExtendedLinearDisplayP3","edr_enabled":true,"requested_linear_patches":[0.5,1.,2.,4.,8.],"drawable_linear_patches":patches,"limit":"pre-compositor drawable values; no photometer measurement"}),
    )
}
