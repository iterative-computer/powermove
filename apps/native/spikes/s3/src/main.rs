mod color;
mod native;
mod policy;
mod render;
mod worker;
use gpui::{prelude::*, *};
use std::{cell::RefCell, rc::Rc, time::Duration};
struct Viewer {
    native: Rc<RefCell<native::Native>>,
    popout: bool,
}
impl Render for Viewer {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let host = self.native.clone();
        div()
            .size_full()
            .text_color(rgb(0xe7e9ee))
            .font_family(".AppleSystemUIFont")
            .child(
                canvas(
                    |_, _, _| (),
                    move |b, _, _, _| {
                        host.borrow_mut().publish(
                            f32::from(b.size.width) as f64,
                            f32::from(b.size.height) as f64,
                        )
                    },
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .w_full()
                    .h(px(46.))
                    .bg(rgb(0x17191d))
                    .p_3()
                    .child(if self.popout {
                        "Powermove · Viewer popout"
                    } else {
                        "Powermove Native · S3 Viewer"
                    }),
            )
            .child(
                div()
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .w_full()
                    .h(px(28.))
                    .bg(rgb(0x17191d))
                    .px_3()
                    .child("Engine overlays · linear P3 · native timecode"),
            )
            // Paused/static chrome remains GPUI above the below-slot viewer.
            .child(
                div()
                    .absolute()
                    .top(px(54.))
                    .right(px(34.))
                    .text_size(px(11.))
                    .child("SAFE AREA"),
            )
    }
}
fn arg(name: &str) -> Option<String> {
    let prefix = format!("--{name}=");
    std::env::args().find_map(|a| a.strip_prefix(&prefix).map(str::to_owned))
}
fn main() -> anyhow::Result<()> {
    anyhow::ensure!(
        std::fs::read_to_string("/tmp/pm-shots/gui.lock").is_ok_and(|s| s.trim() == "s3"),
        "Acquire S3 GUI lock with run.sh before opening windows"
    );
    let doc = arg("document")
        .ok_or_else(|| anyhow::anyhow!("--document=<corpus document.json> required; see run.sh"))?;
    let scene = render::Scene::load(&doc)?;
    let pq = arg("pq").map(|p| render::Scene::pq(&p)).transpose()?;
    let seconds = arg("seconds").unwrap_or("60".into()).parse::<f64>()?;
    let hz = arg("hz").unwrap_or("120".into()).parse::<f64>()?;
    anyhow::ensure!(
        hz > 0. && hz <= 240. && seconds > 0. && seconds <= 300.,
        "invalid run duration/rate"
    );
    let verify = std::env::args().any(|a| a == "--verify");
    let exercise = std::env::args().any(|a| a == "--exercise");
    let report = arg("report").unwrap_or("/tmp/s3-report.json".into());
    let gpu = pm_gpu::Gpu::new("S3 engine-owned GPU")?;
    let device = render::metal_device(&gpu);
    eprintln!(
        "S3 engine and GPUI share Metal registry {}",
        device.registry_id()
    );
    let popout_gpu = exercise.then(|| pm_gpu::Gpu {
        device: gpu.device.clone(),
        queue: gpu.queue.clone(),
        info: gpu.info.clone(),
        registry_id: gpu.registry_id,
        limits: gpu.limits.clone(),
    });
    let popout_scene = exercise.then(|| render::Scene::load(&doc)).transpose()?;
    let popout_device = device.clone();
    let failed = Rc::new(std::cell::Cell::new(false));
    let outcome = failed.clone();
    let platform = Rc::new(gpui_macos::MacPlatform::new_with_metal_device(
        false,
        device.clone(),
    ));
    Application::with_platform(platform).run(move |cx| {
        let entity = cx
            .open_window(
                WindowOptions::new()
                    .macos_window_background(MacosWindowBackground::Transparent)
                    .window_bounds(Some(WindowBounds::Windowed(Bounds::new(
                        point(px(100.), px(100.)),
                        size(px(688.), px(510.)),
                    )))),
                move |window, cx| {
                    window.set_window_title("Powermove Native S3 Viewer");
                    let native = native::Native::install(window, &device);
                    // Initial placement is applied again at the first GPUI present hook.
                    native.borrow_mut().publish(688., 510.);
                    native.borrow_mut().place();
                    native.borrow().identity();
                    let host = worker::Host::start(
                        gpu,
                        native.borrow().layer.clone(),
                        native.borrow().output(),
                        worker::Options {
                            scene,
                            pq,
                            seconds,
                            hz,
                            verify,
                            report,
                        },
                    );
                    let updates = host.updates.clone();
                    native.borrow_mut().host = Some(host);
                    let failed = failed.clone();
                    let viewer = cx.new(|_| Viewer {
                        native: native.clone(),
                        popout: false,
                    });
                    cx.spawn(async move |cx| {
                        while let Ok(update) = updates.recv().await {
                            if let Some(error) = &update.error {
                                failed.set(true);
                                eprintln!("S3 FAILED: {error}");
                            }
                            let done = update.finished;
                            native.borrow_mut().update(&update);
                            if done {
                                let _ = cx.update(|cx| cx.quit());
                                break;
                            }
                        }
                    })
                    .detach();
                    viewer
                },
            )
            .unwrap();
        if let (Some(gpu), Some(scene)) = (popout_gpu, popout_scene) {
            cx.open_window(
                WindowOptions::new()
                    .macos_window_background(MacosWindowBackground::Transparent)
                    .window_bounds(Some(WindowBounds::Windowed(Bounds::new(
                        point(px(800.), px(180.)),
                        size(px(480.), px(420.)),
                    )))),
                move |window, cx| {
                    window.set_window_title("Powermove Native S3 Popout");
                    let native = native::Native::install(window, &popout_device);
                    native.borrow_mut().publish(480., 420.);
                    native.borrow_mut().place();
                    native.borrow().identity();
                    let host = worker::Host::start(
                        gpu,
                        native.borrow().layer.clone(),
                        native.borrow().output(),
                        worker::Options {
                            scene,
                            pq: None,
                            seconds: seconds.min(10.),
                            hz,
                            verify: false,
                            report: "/tmp/s3-popout.json".into(),
                        },
                    );
                    let updates = host.updates.clone();
                    native.borrow_mut().host = Some(host);
                    let view = cx.new(|_| Viewer {
                        native: native.clone(),
                        popout: true,
                    });
                    cx.spawn(async move |_| {
                        while let Ok(update) = updates.recv().await {
                            native.borrow_mut().update(&update);
                            if update.finished {
                                break;
                            }
                        }
                    })
                    .detach();
                    view
                },
            )
            .unwrap();
        }
        if exercise {
            let view = entity.read(cx).unwrap().native.clone();
            cx.spawn(async move |cx| {
                cx.background_executor().timer(Duration::from_secs(2)).await;
                view.borrow_mut().simulate();
                cx.background_executor().timer(Duration::from_secs(2)).await;
                view.borrow_mut().simulate();
                view.borrow().resize();
                // Synthetic Rust seek events; each latency starts at request enqueue.
                for i in 0..120 {
                    view.borrow_mut().seek(((i * 37) % 250) as f64 / 100.);
                    cx.background_executor()
                        .timer(Duration::from_millis(35))
                        .await;
                }
            })
            .detach();
        }
        cx.activate(true);
    });
    anyhow::ensure!(!outcome.get(), "S3 viewer run failed; see stderr");
    Ok(())
}
