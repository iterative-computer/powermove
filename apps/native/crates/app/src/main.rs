mod media;
mod panels;
mod perf_gate;
mod platform;
mod playback;
mod session;
mod smoke;
use anyhow::{Context, Result};
use gpui::AppContext;
use pm_node::fonts::{
    FontService,
    render::{FontBroker, RenderFonts},
};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
};

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|a| a == "--engine-job") {
        return pm_engine_service::jobs::run_child();
    }
    if args.first().is_some_and(|a| a == "--smoke") {
        return pm_shell::smoke();
    }
    if args.first().is_some_and(|a| a == "--gate-p1-smoke") {
        let manifest = args
            .get(1)
            .context("Usage: powermove-native --gate-p1-smoke <manifest.json>")?;
        return smoke::run(PathBuf::from(manifest));
    }
    let mut launch = None;
    let mut profile = std::env::var_os("PM_NATIVE_PROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("powermove-native-integration"));
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--profile" {
            i += 1;
            profile = PathBuf::from(args.get(i).context("--profile needs a directory")?);
        } else if args[i].to_string_lossy().starts_with('-') {
            anyhow::bail!("Unknown argument: {}", args[i].to_string_lossy());
        } else {
            anyhow::ensure!(launch.is_none(), "Open one launch path at a time");
            launch = Some(PathBuf::from(&args[i]));
        }
        i += 1;
    }
    std::fs::create_dir_all(&profile)?;
    // Set before any worker exists. This also disables shell NSUserDefaults restoration.
    unsafe {
        std::env::set_var("PM_NATIVE_SHELL_STATE", profile.join("shell-layouts.json"));
    }
    let broker = FontBroker::new();
    let (wake_tx, wake_rx) = async_channel::bounded(1);
    broker.set_wake(Arc::new(move || {
        let _ = wake_tx.try_send(());
    }));
    let fonts = RenderFonts::new(broker.clone());
    let worker_profile = profile.clone();
    let (actor, engine) = std::thread::spawn(move || -> Result<_> {
        let actor = pm_kernel::KernelActor::spawn(pm_kernel::KernelConfig {
            journal_root: Some(worker_profile.join("journals")),
            ..Default::default()
        });
        let gpu = pm_gpu::Gpu::new("powermove-native")?;
        let engine = Arc::new(pm_engine_service::EngineService::start_with_fonts(
            gpu, fonts,
        )?);
        Ok((actor, engine))
    })
    .join()
    .map_err(|_| anyhow::anyhow!("Engine initialization panicked"))??;
    let handle = actor.handle();
    let device = engine.metal_device().to_owned();
    let service = Rc::new(RefCell::new(
        FontService::new().map_err(anyhow::Error::msg)?,
    ));
    let started = Rc::new(Cell::new(false));
    let result = pm_shell::run_integrated(
        device,
        Rc::new({
            let engine = engine.clone();
            let broker = broker.clone();
            move |host, _window, cx| {
                if !started.replace(true) {
                    let rx = wake_rx.clone();
                    let broker = broker.clone();
                    let service = service.clone();
                    cx.spawn(async move |_| {
                        while rx.recv().await.is_ok() {
                            pm_node::fonts::render::serve(
                                &mut *service.borrow_mut(),
                                broker.take_asks(),
                            );
                        }
                    })
                    .detach();
                }
                let entity = cx.new(|cx| {
                    session::Controller::new(
                        handle.clone(),
                        engine.clone(),
                        profile.clone(),
                        host,
                        cx,
                    )
                });
                playback::install(&entity, cx);
                entity.update(cx, |v, cx| v.initialize(launch.clone(), cx));
                perf_gate::install(&entity, cx);
                Rc::new(session::Bridge(entity))
            }
        }),
    );
    broker.close();
    std::thread::spawn(move || {
        drop(engine);
        actor.shutdown();
    })
    .join()
    .map_err(|_| anyhow::anyhow!("Engine shutdown panicked"))?;
    result
}

fn node_number(node: &pm_model::Node) -> Option<f64> {
    match node.kind() {
        pm_model::NodeKind::Number(n) => Some(*n),
        _ => None,
    }
}
fn node_string(node: &pm_model::Node) -> Option<&str> {
    match node.kind() {
        pm_model::NodeKind::String(s) => s.as_str(),
        _ => None,
    }
}
