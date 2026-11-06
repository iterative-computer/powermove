mod builtins;
#[cfg(test)]
mod builtins_tests;
mod diag;
#[path = "surfaces/dialogs.rs"]
pub mod dialogs;
mod export_coordinator;

#[path = "surfaces/editing.rs"]
pub mod editing;
mod layer_commands;
#[path = "surfaces/library.rs"]
pub mod library_surface;
mod lifecycle;
mod mcp;
mod mcp_services;
mod media;
mod media_import;
mod media_refresh;
mod new_project;
mod panels;
mod perf_gate;
mod platform;
mod playback;
mod rows;
mod session;
mod smoke;
mod surfaces;
mod text_host;
mod v4_control;
mod v4_native;
mod v4_view;
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

fn native_profile(profile: Option<PathBuf>, home: Option<std::ffi::OsString>) -> Option<PathBuf> {
    // N4: the native library is durable and isolated from Electron and test lanes.
    profile.or_else(|| {
        home.filter(|home| !home.is_empty())
            .map(|home| PathBuf::from(home).join("Library/Application Support/Powermove Native"))
    })
}

/// MCP's host export folder is ~/Powermove. An explicit test or lane profile
/// keeps its exports to itself.
fn export_folder(profile: &std::path::Path, home: Option<std::ffi::OsString>) -> PathBuf {
    match home.map(PathBuf::from) {
        Some(home)
            if native_profile(None, Some(home.clone().into())).as_deref() == Some(profile) =>
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
    let profile = native_profile(profile, std::env::var_os("HOME"))
        .context("HOME is required when no native profile override is provided")?;
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
    let (actor, engine, mcp_host, server) = std::thread::spawn(move || -> Result<_> {
        let actor = pm_kernel::KernelActor::spawn(pm_kernel::KernelConfig {
            journal_root: Some(worker_profile.join("journals")),
            text: Some(text_host::text_host(&text_fonts)),
            backup_root: Some(worker_profile.join("backups")),
            ..Default::default()
        });
        mcp::attach(&actor.handle(), &worker_profile)?;
        let (mcp_host, _changes) = mcp::Host::new(actor.handle(), worker_profile.clone());
        let gpu = pm_gpu::Gpu::new("powermove-native")?;
        let engine = Arc::new(pm_engine_service::EngineService::start_with_fonts(
            gpu, fonts,
        )?);
        let tools = mcp::Tools::new(mcp_host.clone(), exports);
        mcp_services::connect_engine(&tools, engine.export_activity())?;
        let server = pm_mcp::transport::LoopbackServer::start_with(&worker_profile, move || {
            tools.connect()
        })?;
        Ok((actor, engine, mcp_host, server))
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
                        mcp_host.clone(),
                        host,
                        cx,
                    )
                });
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
    );
    broker.close();
    std::thread::spawn(move || {
        drop(server);
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

#[cfg(test)]
mod profile_tests {
    use super::native_profile;
    use std::path::PathBuf;

    #[test]
    fn default_is_durable_and_separate_from_electron() {
        let home = std::env::temp_dir().join("synthetic-home");
        let profile = native_profile(None, Some(home.clone().into_os_string())).unwrap();
        assert_eq!(
            profile,
            home.join("Library/Application Support/Powermove Native")
        );
        assert_ne!(profile, home.join("Library/Application Support/Powermove"));
        assert_ne!(
            profile,
            std::env::temp_dir().join("powermove-native-integration")
        );
    }

    #[test]
    fn profile_override_takes_precedence() {
        let profile = std::env::temp_dir().join("synthetic-profile-override");
        assert_eq!(
            native_profile(Some(profile.clone()), Some("synthetic-home".into())),
            Some(profile)
        );
    }

    #[test]
    fn override_does_not_require_home() {
        let profile = std::env::temp_dir().join("synthetic-profile-without-home");
        assert_eq!(native_profile(Some(profile.clone()), None), Some(profile));
    }

    #[test]
    fn missing_home_never_falls_back_to_shared_temp() {
        assert_eq!(native_profile(None, None), None);
        assert_eq!(native_profile(None, Some("".into())), None);
        // Keep the override's existing absolute-path validation at the caller.
        assert_eq!(
            native_profile(Some(PathBuf::from("relative")), None),
            Some(PathBuf::from("relative"))
        );
    }
}
