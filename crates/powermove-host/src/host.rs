//! Shared host state: configuration, connected windows, and the router.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use parking_lot::Mutex;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::router::{Client, Router};

/// Where the host finds its files. Built by the Windows shell or the dev binary.
#[derive(Debug, Clone)]
pub struct Config {
    /// Profile directory: store, extensions, project associations, backups.
    pub user_data: PathBuf,
    /// The Vite-built renderer (`apps/desktop/out/renderer`).
    pub renderer_dir: PathBuf,
    /// Built-in extension sources (`apps/desktop/src/extensions`), one folder per manifest.
    pub builtin_extensions: PathBuf,
    /// Staged app resources: `api-pack/` (agent docs and types), `compiler/`
    /// (mod compiler assets). Packaged builds use the install's resource folder;
    /// dev builds use `target/powermove-resources` (see `scripts/windows/stage-*.ps1`).
    pub resources_dir: PathBuf,
    /// The ffmpeg executable used for proxies and exports.
    pub ffmpeg: PathBuf,
    /// Scratch space for conversions and uploads.
    pub temp_dir: PathBuf,
    pub version: String,
    /// Loopback port. Keep it stable across launches: the WebView's storage is
    /// keyed by origin. A busy port falls back to a free one; 0 always picks one.
    pub port: u16,
}

pub struct Host {
    pub config: Config,
    /// Shared secret the window presents once (`?token=`) to get its cookie.
    pub token: String,
    router: OnceLock<Router>,
    clients: Mutex<Vec<Arc<Client>>>,
    next_client: AtomicU64,
    /// Videos each window exported through a dialog; only these may get a
    /// caption sidecar written beside them (`export:sidecar`).
    exports: Mutex<HashMap<u64, Vec<String>>>,
}

const RECENT_EXPORTS: usize = 16;

impl Host {
    pub fn new(config: Config, token: String) -> Arc<Self> {
        Arc::new(Self {
            config,
            token,
            router: OnceLock::new(),
            clients: Mutex::default(),
            next_client: AtomicU64::new(1),
            exports: Mutex::default(),
        })
    }

    pub(crate) fn install_router(&self, router: Router) {
        assert!(self.router.set(router).is_ok(), "router installed twice");
    }

    pub fn router(&self) -> &Router {
        self.router.get().expect("router is installed before the server starts")
    }

    pub(crate) fn connect(self: &Arc<Self>, outbox: mpsc::UnboundedSender<Vec<u8>>, project: Option<String>) -> Arc<Client> {
        let id = self.next_client.fetch_add(1, Ordering::Relaxed);
        let client = Arc::new(Client::new(id, outbox, project));
        self.clients.lock().push(client.clone());
        let host = Arc::downgrade(self);
        client.on_close(move || {
            if let Some(host) = host.upgrade() {
                host.clients.lock().retain(|other| other.id != id);
                host.exports.lock().remove(&id);
            }
        });
        client
    }

    /// Live windows, oldest first.
    pub fn clients(&self) -> Vec<Arc<Client>> {
        self.clients.lock().iter().filter(|client| !client.is_closed()).cloned().collect()
    }

    pub fn remember_export(&self, owner: u64, path: &str) {
        let mut exports = self.exports.lock();
        let list = exports.entry(owner).or_default();
        list.retain(|item| item != path);
        list.push(path.to_owned());
        let excess = list.len().saturating_sub(RECENT_EXPORTS);
        list.drain(..excess);
    }

    pub fn exported(&self, owner: u64, path: &str) -> bool {
        self.exports.lock().get(&owner).is_some_and(|list| list.iter().any(|item| item == path))
    }

    /// Send one JSON event to every window except `except`.
    pub fn broadcast(&self, channel: &str, arg: Value, except: Option<u64>) {
        for client in self.clients() {
            if Some(client.id) != except {
                client.emit(channel, arg.clone());
            }
        }
    }
}
