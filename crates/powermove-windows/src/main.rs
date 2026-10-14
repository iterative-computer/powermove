#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result, ensure};
use tauri::webview::{NewWindowFeatures, NewWindowResponse, PageLoadEvent};
use tauri::{AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tauri_plugin_opener::OpenerExt;
use tracing_subscriber::prelude::*;

static NEXT_WINDOW: AtomicU64 = AtomicU64::new(1);

fn config(app: &AppHandle, user_data: PathBuf) -> Result<powermove_host::Config> {
    let (renderer_dir, builtin_extensions, ffmpeg, resources_dir) = if cfg!(debug_assertions) {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let desktop = repo.join("apps/desktop");
        (
            desktop.join("out/renderer"),
            desktop.join("src/extensions"),
            desktop.join("node_modules/ffmpeg-static/ffmpeg.exe"),
            repo.join("target/powermove-resources"),
        )
    } else {
        let resources = app
            .path()
            .resource_dir()
            .context("resolve bundled resources")?;
        (
            resources.join("renderer"),
            resources.join("builtin-extensions"),
            resources.join("encoder/ffmpeg.exe"),
            resources,
        )
    };
    ensure!(
        renderer_dir.join("index.html").is_file(),
        "Renderer missing; run bun run --cwd apps/desktop build"
    );
    ensure!(
        builtin_extensions.is_dir(),
        "Built-in extension manifests missing"
    );
    ensure!(
        ffmpeg.is_file(),
        "ffmpeg.exe missing at {}",
        ffmpeg.display()
    );
    let port = std::fs::read_to_string(user_data.join(PORT_FILE))
        .ok()
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(DEFAULT_PORT);
    Ok(powermove_host::Config {
        port,
        user_data,
        renderer_dir,
        builtin_extensions,
        resources_dir,
        ffmpeg,
        temp_dir: std::env::temp_dir().join("Powermove"),
        version: env!("CARGO_PKG_VERSION").into(),
    })
}

/// The editor's origin includes the loopback port, and WebView2 keeps media
/// and recovery data per origin, so the port is remembered across launches.
const PORT_FILE: &str = "host-port";
const DEFAULT_PORT: u16 = 47391;

fn remember_port(user_data: &Path, url: &tauri::Url) {
    if let Some(port) = url.port() {
        if let Err(error) = std::fs::write(user_data.join(PORT_FILE), port.to_string()) {
            tracing::warn!(%error, "Could not remember the editor port");
        }
    }
}

fn logging(user_data: &Path) -> Result<tracing_appender::non_blocking::WorkerGuard> {
    let logs = user_data.join("logs");
    std::fs::create_dir_all(&logs)?;
    let file = tracing_appender::rolling::never(logs, "powermove.log");
    let (writer, guard) = tracing_appender::non_blocking(file);
    let file_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(writer);
    let stderr_layer = cfg!(debug_assertions)
        .then(|| tracing_subscriber::fmt::layer().with_writer(std::io::stderr));
    tracing_subscriber::registry()
        .with(tracing_subscriber::filter::LevelFilter::INFO)
        .with(file_layer)
        .with(stderr_layer)
        .try_init()?;
    Ok(guard)
}

// Never log the one-time authentication query or project identifiers.
fn public_url(url: &tauri::Url) -> String {
    format!("{}{}", url.origin().ascii_serialization(), url.path())
}

fn is_editor_url(url: &tauri::Url, origin: &tauri::Url) -> bool {
    // A blob URL can inherit an HTTP origin, but must not become an editor window.
    matches!(url.scheme(), "http" | "https") && url.origin() == origin.origin()
}

fn open_external(app: &AppHandle, url: &tauri::Url) {
    if let Err(error) = app.opener().open_url(url.as_str(), None::<&str>) {
        tracing::error!(%error, "Could not open default browser");
    }
}

fn editor_window(
    app: &AppHandle,
    label: &str,
    url: tauri::Url,
    origin: tauri::Url,
    features: Option<NewWindowFeatures>,
) -> tauri::Result<WebviewWindow> {
    let popup_app = app.clone();
    let popup_origin = origin.clone();
    let navigation_app = app.clone();
    let mut builder = WebviewWindowBuilder::new(app, label, WebviewUrl::External(url))
        .title("Powermove")
        .inner_size(1440.0, 900.0)
        .min_inner_size(960.0, 600.0)
        .background_color(tauri::window::Color(0, 0, 0, 255))
        .theme(Some(tauri::Theme::Dark))
        .decorations(true)
        // Let file drops reach the page as HTML5 drop events (agent composer, media import).
        .disable_drag_drop_handler()
        .on_new_window(move |url, features| {
            if is_editor_url(&url, &popup_origin) {
                let label = format!("editor-{}", NEXT_WINDOW.fetch_add(1, Ordering::Relaxed));
                // Preserve WebView2's environment and opener linkage, including window.close().
                match editor_window(&popup_app, &label, "about:blank".parse().unwrap(), popup_origin.clone(), Some(features)) {
                    Ok(window) => NewWindowResponse::Create { window },
                    Err(error) => {
                        tracing::error!(%error, "Could not create editor window");
                        NewWindowResponse::Deny
                    }
                }
            } else {
                if matches!(url.scheme(), "http" | "https") {
                    open_external(&popup_app, &url);
                }
                NewWindowResponse::Deny
            }
        })
        .on_navigation(move |url| {
            if is_editor_url(url, &origin) {
                true
            } else {
                if matches!(url.scheme(), "http" | "https") {
                    open_external(&navigation_app, url);
                }
                false
            }
        })
        .on_page_load(|window, payload| {
            if payload.event() == PageLoadEvent::Finished {
                tracing::info!(window = window.label(), url = %public_url(payload.url()), "Editor page loaded");
            }
        });
    // Debug builds can expose the WebView to end-to-end tooling over CDP.
    #[cfg(debug_assertions)]
    if let Ok(port) = std::env::var("POWERMOVE_CDP_PORT") {
        builder = builder.additional_browser_args(&format!(
            "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --remote-debugging-port={port}"
        ));
    }
    if let Some(features) = features {
        builder = builder.window_features(features);
    }
    builder.build()
}

/// `.pmv` files among the launch arguments, resolved against the launch folder.
fn project_arguments(args: impl IntoIterator<Item = String>, cwd: &Path) -> Vec<PathBuf> {
    args.into_iter()
        .map(PathBuf::from)
        .filter(|path| path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("pmv")))
        .map(|path| if path.is_absolute() { path } else { cwd.join(path) })
        .collect()
}

/// Hand project files to the host; it opens them once a window listens.
fn open_projects(app: &AppHandle, paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let Some(running) = app.try_state::<powermove_host::Running>() else { return };
        for path in paths {
            tracing::info!("Opening project passed at launch");
            running.open_project(path).await;
        }
    });
}

fn main() -> Result<()> {
    let user_data = dirs::data_dir()
        .context("APPDATA is unavailable")?
        .join("Powermove");
    let _log_guard = logging(&user_data)?;
    // A release build has no console, so a panic would otherwise vanish without a trace.
    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("panic: {info}\n{}", std::backtrace::Backtrace::force_capture());
        default_panic(info);
    }));
    let launch_projects = project_arguments(std::env::args().skip(1), &std::env::current_dir().unwrap_or_default());
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, cwd| {
            open_projects(app, project_arguments(args.into_iter().skip(1), Path::new(&cwd)));
            if let Some(window) = app
                .get_webview_window("main")
                .or_else(|| app.webview_windows().into_values().next())
            {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
            tracing::info!("Existing editor focused after second launch");
        }))
        .plugin(
            tauri_plugin_opener::Builder::new()
                .open_js_links_on_click(false)
                .build(),
        )
        .setup(move |app| {
            let running = tauri::async_runtime::block_on(powermove_host::start(config(
                app.handle(),
                user_data.clone(),
            )?))?;
            let url: tauri::Url = running.url.parse()?;
            remember_port(&user_data, &url);
            tracing::info!(url = %public_url(&url), "Opening main editor");
            app.manage(running);
            editor_window(app.handle(), "main", url.clone(), url, None)?;
            open_projects(app.handle(), launch_projects);
            Ok(())
        })
        .build(tauri::generate_context!())?;
    // Tauri exits when the last native window closes. Flush while the runtime is alive.
    app.run(|app, event| {
        if let RunEvent::Exit = event {
            if let Some(running) = app.try_state::<powermove_host::Running>() {
                tauri::async_runtime::block_on(running.shutdown());
                tracing::info!("Host store flushed; Powermove exiting");
            }
        }
    });
    Ok(())
}
