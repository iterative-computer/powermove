//! Powermove's native host for Windows.
//!
//! The editor UI is the existing Svelte renderer (`apps/desktop/src/renderer`).
//! It reaches native capability only through the IPC contract in
//! `apps/desktop/src/shared/ipc.ts`; in this host that contract runs over one
//! loopback WebSocket using the framing of `shared/wire.ts`, the same protocol
//! `host/web-bridge.ts` already speaks. Each module here ports one group of
//! Electron main-process channels to Rust.

pub mod agents;
pub mod app;
pub mod dialog;
pub mod extensions;
pub mod fonts;
pub mod host;
pub mod ids;
pub mod media;
pub mod projects;
pub mod router;
pub mod server;
pub mod store;
pub mod wire;

use std::sync::Arc;

use anyhow::Result;

pub use host::{Config, Host};

/// A running host. Load `url` in a WebView to open an editor window.
pub struct Running {
    pub host: Arc<Host>,
    /// Loopback URL carrying the one-time token.
    pub url: String,
    store: Arc<store::Store>,
    projects: projects::ProjectOpener,
    agents: Arc<agents::Agents>,
}

impl Running {
    /// Open a project file passed on the command line or by Explorer.
    pub async fn open_project(&self, path: std::path::PathBuf) {
        self.projects.open(path).await;
    }

    /// Stop agent CLIs and write pending store changes; call before the process exits.
    pub async fn shutdown(&self) {
        self.agents.shutdown().await;
        self.store.flush().await;
    }
}

/// Start the host on a tokio runtime and return once it is listening.
pub async fn start(config: Config) -> Result<Running> {
    tokio::fs::create_dir_all(&config.user_data).await?;
    tokio::fs::create_dir_all(&config.temp_dir).await?;
    let token = hex::encode(rand::random::<[u8; 24]>());
    let store = store::Store::load(config.user_data.join("store")).await?;

    let mut router = router::Router::default();
    app::register(&mut router);
    store::register(&mut router, store.clone());
    agents::changes::recover_all(&config.user_data).await;
    extensions::register(&mut router, &config, store.clone()).await?;
    fonts::register(&mut router);
    let projects = projects::register(&mut router, &config).await?;
    media::register(&mut router, &config).await?;
    let agents = agents::register(&mut router, &config).await?;

    let host = Host::new(config, token.clone());
    host.install_router(router);
    let (address, serve) = server::bind(host.clone()).await?;
    tokio::spawn(serve);
    let url = format!("http://{address}/?token={token}");
    tracing::info!("Powermove host listening on http://{address}");
    Ok(Running { host, url, store, projects, agents })
}
