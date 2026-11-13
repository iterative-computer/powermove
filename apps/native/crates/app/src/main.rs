// Rust allocations use the system allocator. The bundle launches with
// `MallocLargeCache=0` (Info.plist LSEnvironment, `packaging/package.py`), so
// freed large blocks go back to macOS at once; small blocks return per 16 KiB
// page (`pm_memory::pressure_relief`). mimalloc kept whole 64 KiB pages of
// the project load's long-lived nodes, about 140 MB more with Paper open
// (installed app: idle 563 → 416 MB, stopped 700 → 522 MB). Memory reviews:
// the Rust heap is "Malloc Small/Large" (not mimalloc's tag-100
// "IOAccelerator"). A bare binary needs `MallocLargeCache=0` exported to match.

mod after_open;
mod agent_clients;
mod alert;
mod builtins;
#[cfg(test)]
mod builtins_tests;
mod cloud;
mod cloud_media;
#[cfg(test)]
mod cloud_tests;
mod colour;
mod colour_panel;
mod diag;
#[path = "surfaces/dialogs.rs"]
mod dialogs;
mod entry;
mod error_log;
mod export_coordinator;
mod expression_sheet;
mod extension_authoring;
mod extension_vars;
mod extensions;
mod figma_paste;
mod file_coordination;
mod first_run;
mod generated_panel;
mod gizmo;
#[cfg(all(test, target_os = "macos"))]
mod gpui_memory_tests;
mod interpret_sheet;
mod keyframe_sheets;
mod poster;

mod agent_speech;
mod captions_generate;
mod captions_panel;
#[cfg(test)]
mod gizmo_tests;
mod layer_commands;
#[path = "surfaces/library.rs"]
mod library_surface;
mod lifecycle;
mod mcp;
mod mcp_services;
mod media;
mod media_import;
mod media_refresh;
mod media_sequence;
#[cfg(debug_assertions)]
mod memory_review;
mod migration_report;
mod mixer_panel;
mod notes_panel;
mod notifications;
mod panel_bridge;
#[cfg(all(test, target_os = "macos"))]
mod panel_capture_tests;
mod panel_library;
mod panels;
#[cfg(test)]
mod parity_tests;
mod perf_gate;
mod perf_panel;
mod platform;
mod play_range;
mod playback;
mod playhead;
mod pro_media;
mod project_library;
mod providers;
mod publish;
mod raster_overrides;
mod render_frames;
mod rows;
mod session;
mod shader_panel;
mod shape_tool;
mod sheets;
mod shuttle;
mod skimming;
mod smoke;
mod store_window;
#[cfg(test)]
mod store_window_tests;
mod surface_routes;
mod surfaces;
#[cfg(test)]
mod takes_notes_tests;
mod takes_panel;
mod tasks_window;
mod text_host;
mod ticket;
mod timeline_surface;
#[cfg(test)]
mod timeline_tests;
mod transcription;
mod updates;
mod v4_compile;
mod v4_control;
mod v4_native;
mod v4_view;
#[cfg(test)]
mod v4_view_tests;
mod viewer2;
mod viewer_overlays;
mod waveforms;
mod workspaces;
use anyhow::{Context, Result};
use gpui::AppContext;
use pm_render::text::host::RenderFonts;
use pm_text::host::{FontBroker, FontService};
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
    diag::mark_start();
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let headless = args
        .first()
        .is_some_and(|a| a == "--engine-job" || a == "--smoke" || a == "--gate-p1-smoke")
        || args.iter().any(|a| a == "--powermove-mcp");
    if !headless {
        platform::route_sigterm();
    }
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
    let mut agent_run = None;
    let mut profile = std::env::var_os("PM_NATIVE_PROFILE").map(PathBuf::from);
    let mut lane_flags: Vec<&'static str> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--powermove-mcp" {
            // Same app executable and newline-delimited stdio contract.
        } else if args[i] == "--agent-run" && mcp_mode {
            // The provider run this tool server belongs to (orchestration).
            i += 1;
            agent_run = Some(
                args.get(i)
                    .context("--agent-run needs a run id")?
                    .to_string_lossy()
                    .into_owned(),
            );
        } else if args[i] == "--profile" {
            i += 1;
            profile = Some(PathBuf::from(
                args.get(i).context("--profile needs a directory")?,
            ));
        } else if let Some(name) = review_override(&args[i], cfg!(debug_assertions)) {
            lane_flags.push(name);
            i += 1;
            let value = args.get(i).context("development overrides need a value")?;
            // SAFETY: single-threaded here; nothing has read the environment yet.
            unsafe { std::env::set_var(name, value) };
        } else if args[i].to_string_lossy().starts_with('-') {
            anyhow::bail!("Unknown argument: {}", args[i].to_string_lossy());
        } else {
            anyhow::ensure!(launch.is_none(), "Open one launch path at a time");
            launch = Some(PathBuf::from(&args[i]));
        }
        i += 1;
    }
    // The lane launcher's development flag (never the environment): sheets
    // wait while the app is inactive, and with `--open` stand for captures.
    #[cfg(debug_assertions)]
    if let Some(stage) = sheet_hold(&lane_flags) {
        gpui_macos::child_window::hold_sheets_while_inactive(stage);
    }
    #[cfg(not(debug_assertions))]
    let _ = lane_flags;
    let plan = first_run::Plan::resolve(
        profile,
        std::env::var_os("PM_NATIVE_ELECTRON_PROFILE").map(PathBuf::from),
        std::env::var_os("HOME"),
    )
    .context("HOME is required when no native profile override is provided")?;
    let profile = plan.profile.clone();
    // Development builds: a profile holding `cage-report.json` gets the cage
    // account written there each second, for memory reviews of many windows.
    if cfg!(debug_assertions) && profile.join("cage-report.json").exists() {
        cage_report(profile.join("cage-report.json"));
    }
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
            return std::thread::spawn(move || pm_mcp::transport::proxy_stdio(&profile, agent_run))
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
    transcription::install(&profile);
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
    let (bridge, bridge_calls) = panel_bridge::channel();
    let bridge_windows = panel_bridge::Shared::default();
    let job_overrides = raster_overrides::OverrideWindows::default();
    let agent_overrides = job_overrides.clone();
    let jobs_broker = broker.clone();
    let (actor, engine, mcp_host, tools) = std::thread::spawn(move || -> Result<_> {
        let actor = pm_kernel::KernelActor::spawn(pm_kernel::KernelConfig {
            journal_root: Some(worker_profile.join("journals")),
            text: Some(text_host::text_host(&text_fonts)),
            backup_root: Some(worker_profile.join("backups")),
            write_coordinator: Some(file_coordination::system()),
            trash: Some(file_coordination::trash()),
            idle_memory_relief: Some(pm_engine_service::memory::pressure_relief),
            ..Default::default()
        });
        let mcp_host = mcp::Host::new(actor.handle(), worker_profile.clone());
        let gpu = pm_gpu::Gpu::new("powermove-native")?;
        let engine = Arc::new(pm_engine_service::EngineService::start_with_fonts(
            gpu,
            fonts,
            Some(worker_profile.join("shader-cache")),
        )?);
        let tools = mcp::Tools::new(mcp_host.clone(), exports);
        // A project no window has open draws with the windows' own worker and fonts.
        agent_overrides.set_painters(raster_overrides::Painters {
            layers: engine.layer_rasters(),
            fonts: jobs_broker,
        });
        mcp_services::connect_engine(&tools, engine.export_activity(), agent_overrides)?;
        {
            let mut automation = tools.automation.lock().unwrap();
            automation.surfaces = Some(Arc::new(bridge));
            // Only tokens this process issued to its provider runs resolve.
            automation.agent_grants = Some(Arc::new(|token: &str| {
                pm_agents::launch::grants::resolve(token).map(|(run, access)| {
                    let access = if access == pm_agents::Access::Editor {
                        "editor"
                    } else {
                        "project"
                    };
                    (run, access.to_owned())
                })
            }));
        }
        // The server itself starts once the library is attached (first_run).
        Ok((actor, engine, mcp_host, tools))
    })
    .join()
    .map_err(|_| anyhow::anyhow!("Engine initialization panicked"))??;
    diag::line(serde_json::json!({"startup":"engine"}));
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
    // The command-line project opens in the first window only.
    let launch = Cell::new(launch);
    let result = pm_shell::run_gated(
        device,
        Rc::new({
            let engine = engine.clone();
            let broker = broker.clone();
            move |host, window, cx| {
                if !started.replace(true) {
                    diag::line(serde_json::json!({"startup":"window"}));
                    diag::install_probe(cx);
                    error_log::persist(&profile, cx);
                    notifications::install(cx);
                    panel_bridge::serve(bridge_calls.clone(), bridge_windows.clone(), cx);
                    let rx = wake_rx.clone();
                    let broker = broker.clone();
                    let service = service.clone();
                    cx.spawn(async move |_| {
                        while rx.recv().await.is_ok() {
                            pm_text::host::serve(&mut *service.borrow_mut(), broker.take_asks());
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
                entity.update(cx, |v, _| {
                    v.font_broker = Some(font_broker);
                    v.job_overrides = job_overrides.clone();
                });
                render_frames::serve_background(&engine, job_overrides.clone());
                panel_bridge::register(&bridge_windows, &entity, window.window_handle());
                lifecycle::install(&entity, window, cx);
                viewer2::install(&entity, cx);
                entity.update(cx, |c, cx| {
                    c.apply_skimming(cx);
                    c.load_skimming(cx);
                    c.load_playback(cx);
                });
                // Each window owns its project: MCP's slot for this window, and
                // the window wakes only when MCP changes that slot.
                let key = window.window_handle().window_id().as_u64();
                let rx = mcp_host.attach_window(key);
                entity.update(cx, |v, cx| {
                    v.window_key = key;
                    let host = v.mcp.clone();
                    cx.on_release(move |_, _| host.detach_window(key)).detach();
                });
                let weak = entity.downgrade();
                cx.spawn(async move |cx| {
                    while rx.recv().await.is_ok() {
                        if weak.update(cx, |v, cx| v.follow_mcp(cx)).is_err() {
                            break;
                        }
                    }
                })
                .detach();
                playback::install(&entity, cx);
                #[cfg(debug_assertions)]
                memory_review::install(&entity, service.clone(), cx);
                let opening = entry::take_for_new_window(cx)
                    .or_else(|| launch.take().map(entry::Pending::File));
                entity.update(cx, |v, cx| v.initialize(opening, cx));
                entry::register(&entity, window.window_handle(), cx);
                cloud::init(profile.clone(), cx);
                providers::init(profile.clone(), cx);
                agent_clients::init(profile.clone(), cx);
                if let Some(ns) = platform::ns_window(window) {
                    updates::after_launch(profile.clone(), ns, cx);
                }
                perf_gate::install(&entity, cx);
                Rc::new(session::Bridge(entity))
            }
        }),
        gate,
        Rc::new(entry::receive),
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

/// Review captures may force a scheme or accessibility setting for one
/// process (`--appearance`, `--accessibility`) or open surfaces by command
/// (`--open settings,palette`), in development builds only;
/// a release build treats them as unknown arguments.
fn review_override(arg: &std::ffi::OsStr, dev: bool) -> Option<&'static str> {
    if !dev {
        return None;
    }
    match arg.to_str()? {
        "--appearance" => Some("PM_NATIVE_APPEARANCE"),
        "--accessibility" => Some("PM_NATIVE_ACCESSIBILITY"),
        "--open" => Some("PM_NATIVE_REVIEW_OPEN"),
        "--draw-occluded" => Some("PM_NATIVE_DRAW_OCCLUDED"),
        "--save-dir" => Some("PM_NATIVE_REVIEW_SAVE_DIR"),
        // A fixture Store on this Mac and its test signing key (store_install).
        "--dev-store-origin" => Some("PM_DEV_STORE_ORIGIN"),
        "--dev-store-key" => Some("PM_DEV_STORE_KEY"),
        // A scratch home whose agent apps a review registers (agent_clients).
        "--dev-agent-home" => Some("PM_DEV_AGENT_HOME"),
        // A stand-in cloud for files on demand (cloud_media::Fixture).
        "--cloud-fixture" => Some("PM_CLOUD_FIXTURE"),
        // The v1 profile a review migrates from: a clone, never the live one.
        "--electron-profile" => Some("PM_NATIVE_ELECTRON_PROFILE"),
        _ => None,
    }
}

/// A lane launch (the launcher always passes `--draw-occluded`) holds sheets
/// while the app is inactive; with `--open` it also stands them for review
/// captures. Read from this process's own command-line flags only: an
/// inherited environment variable never turns it on.
#[cfg(debug_assertions)]
fn sheet_hold(flags: &[&str]) -> Option<bool> {
    flags
        .contains(&"PM_NATIVE_DRAW_OCCLUDED")
        .then(|| flags.contains(&"PM_NATIVE_REVIEW_OPEN"))
}

/// The isolate cage account (`pm_runtime_v8::Admission::cage`), once a second.
fn cage_report(path: PathBuf) {
    std::thread::spawn(move || {
        let cage = pm_runtime_v8::Admission::cage();
        loop {
            let _ = std::fs::write(
                &path,
                serde_json::json!({
                    "usedMiB": cage.used() >> 20,
                    "peakMiB": cage.peak() >> 20,
                    "budgetMiB": cage.budget() >> 20,
                    "isolates": cage.isolates(),
                })
                .to_string(),
            );
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    });
}

#[cfg(test)]
mod review_override_tests {
    #[test]
    fn release_builds_reject_the_review_overrides() {
        use std::ffi::OsStr;
        for arg in [
            "--appearance",
            "--accessibility",
            "--open",
            "--draw-occluded",
            "--dev-store-origin",
            "--dev-store-key",
            "--dev-agent-home",
            "--cloud-fixture",
            "--electron-profile",
        ] {
            assert_eq!(super::review_override(OsStr::new(arg), false), None);
            assert!(super::review_override(OsStr::new(arg), true).is_some());
        }
        assert_eq!(super::review_override(OsStr::new("--profile"), true), None);
    }
    #[test]
    fn only_the_lane_flag_holds_sheets_and_only_with_open_stands_them() {
        // Flags this process was launched with, never the environment:
        // `--open` alone (or an inherited variable) holds nothing.
        assert_eq!(super::sheet_hold(&[]), None);
        assert_eq!(super::sheet_hold(&["PM_NATIVE_REVIEW_OPEN"]), None);
        assert_eq!(super::sheet_hold(&["PM_NATIVE_DRAW_OCCLUDED"]), Some(false));
        assert_eq!(
            super::sheet_hold(&["PM_NATIVE_REVIEW_OPEN", "PM_NATIVE_DRAW_OCCLUDED"]),
            Some(true)
        );
    }
}
