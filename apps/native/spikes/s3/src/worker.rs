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
    pub seek: Option<(f64, Instant, Stamp)>,
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
}
pub struct Options {
    pub scene: Scene,
    pub pq: Option<Scene>,
    pub seconds: f64,
    pub hz: f64,
    pub verify: bool,
    pub report: String,
}
impl Host {
    pub fn start(
        gpu: pm_gpu::Gpu,
        layer: metal::MetalLayer,
        output: Output,
        options: Options,
    ) -> Self {
        let control = Arc::new(Mutex::new(Control {
            policy: Policy::new(output),
            rect: [0.1, 0.1, 0.9, 0.9],
            stop: false,
            seek: None,
        }));
        let renders = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let (tx, updates) = async_channel::bounded(1);
        let c = control.clone();
        let redraws = renders.clone();
        std::thread::Builder::new()
            .name("s3-engine-viewer".into())
            .spawn(move || {
                let result = run(gpu, layer, &c, &tx, &redraws, options);
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
        .encode(scene, t, output, layer, [0.; 4], false, true)?
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
    let report = json!({"pixels":pixels.len(),"deltaE2000_p50":percentile(&errors,0.5),"deltaE2000_p99":percentile(&errors,0.99),"deltaE2000_max":percentile(&errors,1.),"export_peak_linear_P3":maximum_linear,"overlays":false,"readback":"actual drawable blit before present; pm-export::render_frame independent render"});
    anyhow::ensure!(
        percentile(&errors, 0.99) <= 1. && percentile(&errors, 1.) <= 2.,
        "accuracy gate failed: {report}"
    );
    Ok(report)
}
fn run(
    gpu: pm_gpu::Gpu,
    layer: metal::MetalLayer,
    c: &Mutex<Control>,
    tx: &async_channel::Sender<Update>,
    redraws: &std::sync::atomic::AtomicU64,
    options: Options,
) -> anyhow::Result<()> {
    let mut engine = Engine::new(gpu)?;
    // Wait for the first committed GPUI slot geometry. No worker→main waits.
    std::thread::sleep(Duration::from_millis(500));
    let output = c.lock().unwrap().policy.output;
    let mut accuracy_rows = json!({});
    if options.verify {
        accuracy_rows["sdr"] = accuracy(&mut engine, &options.scene, &layer, output)?;
        if let Some(pq) = &options.pq {
            accuracy_rows["pq"] = accuracy(&mut engine, pq, &layer, output)?;
        }
    }
    let interval = 1. / options.hz;
    let start = Instant::now();
    let base = now();
    let mut index = 0_u64;
    let mut work = Vec::new();
    let mut stale = 0;
    let mut no_drawable = 0;
    let mut late = 0;
    let presented = Arc::new(Mutex::new(Vec::<(u64, f64, f64, f64)>::new()));
    let mut gpui_at_start = None;
    while start.elapsed().as_secs_f64() < options.seconds + 1. {
        let due = base + (index as f64 + 1.) * interval;
        let delay = due - interval - now();
        if delay > 0. {
            std::thread::sleep(Duration::from_secs_f64(delay));
        }
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
        let input = seek.map_or(now(), |s| now() - s.1.elapsed().as_secs_f64());
        let Some(sub) = engine.encode(
            &options.scene,
            ticks(time),
            output,
            &layer,
            rect,
            true,
            false,
        )?
        else {
            no_drawable += 1;
            index += 1;
            continue;
        };
        let current = c.lock().unwrap();
        if !current.policy.accepts(stamp) {
            stale += 1;
            index += 1;
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
        // Retained surface lives through completion, including errors and resize.
        debug_assert!(Arc::strong_count(&sub.surface) >= 2);
        if index >= options.hz as u64 {
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
        index += 1;
        // Recover instead of accumulating ever-growing presentation latency.
        let behind = now() - (base + index as f64 * interval);
        if behind > interval {
            let missed = (behind / interval).floor() as u64;
            index += missed;
            late += missed;
        }
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
    let report = json!({"requested_hz":options.hz,"requested_seconds":options.seconds,"measured_frames":work.len(),"presented_callbacks":measured.len(),"not_presented":dropped,"late_or_missed_schedule":late,"no_drawable":no_drawable,"stale_rejected":stale,"frame_ms":{"p50":percentile(&work,0.5),"p95":percentile(&work,0.95)},"scanout_gap_ms":{"p50":percentile(&gaps,0.5),"p95":percentile(&gaps,0.95)},"scrub_ms":{"samples":scrub.len(),"p50":percentile(&scrub,0.5),"p95":percentile(&scrub,0.95)},"headroom_budget_ms":interval*500.,"gpui_paints_during_measurement":redraws.load(Ordering::Acquire)-gpui_at_start.unwrap_or(0),"accuracy":accuracy_rows,"scope":"one public corpus project; silent host-clock transport; synthetic Rust seek inputs; no paired Electron baseline"});
    std::fs::write(&options.report, serde_json::to_vec_pretty(&report)?)?;
    eprintln!("S3 report {report}");
    Ok(())
}
