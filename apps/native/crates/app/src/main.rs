mod builtins;
#[cfg(test)]
mod builtins_tests;
mod colour;
mod colour_panel;
mod diag;
#[path = "surfaces/dialogs.rs"]
pub mod dialogs;
mod error_log;
mod export_coordinator;
mod extension_authoring;
mod extensions;
mod first_run;
mod gizmo;
mod keyframe_sheets;

#[path = "surfaces/editing.rs"]
pub mod editing;
#[cfg(test)]
mod gizmo_tests;
mod layer_commands;
#[path = "surfaces/library.rs"]
pub mod library_surface;
mod lifecycle;
mod mcp;
mod mcp_services;
mod media;
mod media_import;
mod media_refresh;
mod migration_report;
mod panels;
mod perf_gate;
mod platform;
mod playback;
mod pro_media;
mod project_library;
mod rows;
mod session;
mod shape_tool;
mod sheets;
mod smoke;
mod surface_routes;
mod surfaces;
mod tasks_window;
mod text_host;
mod timeline_surface;
#[cfg(test)]
mod timeline_tests;
mod v4_compile;
mod v4_control;
mod v4_native;
mod v4_view;
#[cfg(test)]
mod v4_view_tests;
mod viewer2;
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

/// MCP's host export folder is ~/Powermove for the default native profile.
/// An explicit test or lane profile keeps its exports to itself.
fn export_folder(profile: &std::path::Path, home: Option<std::ffi::OsString>) -> PathBuf {
    let home = home.filter(|home| !home.is_empty()).map(PathBuf::from);
    match home {
        Some(home)
            if pm_first_launch::default_paths(&home.join("Library/Application Support"))
                .native_profile
                == profile =>
        {
            home.join("Powermove")
        }
        _ => profile.join("exports"),
    }
}

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
    let mcp_mode = args.iter().any(|a| a == "--powermove-mcp");
    let mut launch = None;
    let mut profile = std::env::var_os("PM_NATIVE_PROFILE").map(PathBuf::from);
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--powermove-mcp" {
            // Same app executable and newline-delimited stdio contract.
        } else if args[i] == "--profile" {
            i += 1;
            profile = Some(PathBuf::from(
                args.get(i).context("--profile needs a directory")?,
            ));
        } else if args[i].to_string_lossy().starts_with('-') {
            anyhow::bail!("Unknown argument: {}", args[i].to_string_lossy());
        } else {
            anyhow::ensure!(launch.is_none(), "Open one launch path at a time");
            launch = Some(PathBuf::from(&args[i]));
        }
        i += 1;
    }
    let plan = first_run::Plan::resolve(
        profile,
        std::env::var_os("PM_NATIVE_ELECTRON_PROFILE").map(PathBuf::from),
        std::env::var_os("HOME"),
    )
    .context("HOME is required when no native profile override is provided")?;
    let profile = plan.profile.clone();
    anyhow::ensure!(
        profile.is_absolute(),
        "Use an absolute isolated native profile"
    );
    let exports = export_folder(&profile, std::env::var_os("HOME"));
    if mcp_mode {
        if profile.join("mcp/connection.json").exists() {
            anyhow::ensure!(
                launch.is_none(),
                "Open projects in the running app before attaching MCP"
            );
            return std::thread::spawn(move || pm_mcp::transport::proxy_stdio(&profile))
                .join()
                .map_err(|_| anyhow::anyhow!("MCP proxy panicked"))?
                .map_err(Into::into);
        }
        // A library made here would stop the first launch copy from ever running.
        anyhow::ensure!(
            plan.launch != pm_first_launch::Launch::Migrate,
            "Open Powermove once to finish copying your library"
        );
        return mcp_services::headless(profile, exports, launch);
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
    let text_fonts = fonts.clone();
    let worker_profile = profile.clone();
    let (actor, engine, mcp_host, tools) = std::thread::spawn(move || -> Result<_> {
        let actor = pm_kernel::KernelActor::spawn(pm_kernel::KernelConfig {
            journal_root: Some(worker_profile.join("journals")),
            text: Some(text_host::text_host(&text_fonts)),
            backup_root: Some(worker_profile.join("backups")),
            ..Default::default()
        });
        let (mcp_host, _changes) = mcp::Host::new(actor.handle(), worker_profile.clone());
        let gpu = pm_gpu::Gpu::new("powermove-native")?;
        let engine = Arc::new(pm_engine_service::EngineService::start_with_fonts(
            gpu,
            fonts,
            Some(worker_profile.join("shader-cache")),
        )?);
        let tools = mcp::Tools::new(mcp_host.clone(), exports);
        mcp_services::connect_engine(&tools, engine.export_activity())?;
        // The server itself starts once the library is attached (first_run).
        Ok((actor, engine, mcp_host, tools))
    })
    .join()
    .map_err(|_| anyhow::anyhow!("Engine initialization panicked"))??;
    // Filled once the library is attached; the MCP server waits for that too.
    let server = Arc::new(std::sync::Mutex::new(None));
    let handle = actor.handle();
    let device = engine.metal_device();
    let service = Rc::new(RefCell::new(
        FontService::new().map_err(anyhow::Error::msg)?,
    ));
    let started = Rc::new(Cell::new(false));
    let electron = plan.electron();
    let gate = first_run::gate(plan, handle.clone(), tools, server.clone());
    let result = pm_shell::run_gated(
        device,
        Rc::new({
            let engine = engine.clone();
            let broker = broker.clone();
            move |host, window, cx| {
                if !started.replace(true) {
                    diag::install_probe(cx);
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
                        electron.clone(),
                        mcp_host.clone(),
                        host,
                        cx,
                    )
                });
                let font_broker = broker.clone();
                entity.update(cx, |v, _| v.font_broker = Some(font_broker));
                lifecycle::install(&entity, window, cx);
                viewer2::install(&entity, cx);
                let weak = entity.downgrade();
                let rx = mcp_host.subscribe();
                cx.spawn(async move |cx| {
                    while rx.recv().await.is_ok() {
                        if weak.update(cx, |v, cx| v.follow_mcp(cx)).is_err() {
                            break;
                        }
                    }
                })
                .detach();
                playback::install(&entity, cx);
                entity.update(cx, |v, cx| v.initialize(launch.clone(), cx));
                perf_gate::install(&entity, cx);
                Rc::new(session::Bridge(entity))
            }
        }),
        gate,
    );
    broker.close();
    std::thread::spawn(move || {
        drop(server.lock().map(|mut s| s.take()));
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
