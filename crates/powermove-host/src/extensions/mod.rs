//! User extensions ("mods"): discovery, compilation, IPC and the source
//! watcher (port of `apps/desktop/src/main/extensions/**`).
//!
//! Built-ins ship inside the renderer; the host lists their manifests. User
//! mods live in `<userData>/extensions/<id>` and compile without Node into
//! `<userData>/extensions-build/<id>/bundle.js`, which `server.rs` serves at
//! `/ext/<id>/bundle.js` (see `compiler.rs` for how and which assets).
//!
//! # API for other host modules
//!
//! [`for_profile`] returns the [`Extensions`] handle registered for a
//! profile once [`register`] has run (it runs before `agents::register`):
//!
//! - [`Extensions::refresh`] rescans and rebuilds ids (or everything) and
//!   tells windows with `ext:changed`. Call it after writing into
//!   [`Extensions::user_dir`]; the watcher would get there too, 300 ms later.
//! - [`Extensions::validate`] compiles an extension folder anywhere on disk
//!   (an agent's staging copy, say) into a throwaway directory and returns
//!   the bundle hash or the compiler's user-facing error. Nothing live changes.
//! - [`Extensions::fork_builtin`], [`Extensions::update_fork`] and
//!   [`rebase::stage_fork_rebase`] back the fork tools.
//! - [`Extensions::subscribe`] streams every `ext:changed` payload.

pub mod compiler;
pub mod fork;
pub mod manifest;
pub mod rebase;
pub mod registry;
mod svelte;
mod watcher;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};

use anyhow::{Context as _, Result, anyhow, bail};
use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{OnceCell, broadcast};

use crate::host::{Config, Host};
use crate::router::{Call, Router};
use crate::store::Store;
use crate::wire::Payload;
use compiler::{Compiled, Compiler};
use manifest::{Record, is_extension_id, limits, parse_manifest};
use registry::{CreateRequest, Registry, Reported};

pub use rebase::ForkUpdate;

const STORE_KEY: &str = "extensions";

/// Where an [`Extensions`] instance keeps its files.
pub struct Paths {
    /// `<userData>/extensions`: user mod sources, watched.
    pub user_dir: PathBuf,
    /// `<userData>/extensions-build`: compiled bundles.
    pub build_dir: PathBuf,
    /// Built-in extension sources, one folder per manifest.
    pub builtin_dir: PathBuf,
    /// `<resources>/compiler`: esbuild.exe, svelte-compiler.js, runtime-exports.json.
    pub compiler_dir: PathBuf,
    /// Scratch space for fork updates and validation builds, on the same
    /// volume as `user_dir` so a finished fork update can be renamed into place.
    pub scratch_dir: PathBuf,
}

/// The extension registry of one profile. Cheap to clone.
#[derive(Clone)]
pub struct Extensions(Arc<Inner>);

struct Inner {
    registry: Arc<Registry>,
    scratch_dir: PathBuf,
    ready: OnceCell<()>,
    changed: broadcast::Sender<Value>,
    /// The host, learned from the first call (the watcher has no call to reply to).
    host: Mutex<Weak<Host>>,
    watcher: Mutex<Option<watcher::Watcher>>,
}

static PROFILES: Mutex<Vec<(PathBuf, Weak<Inner>)>> = Mutex::new(Vec::new());

/// The [`Extensions`] registered for `user_data`, if [`register`] ran for it.
pub fn for_profile(user_data: &Path) -> Option<Extensions> {
    PROFILES.lock().iter().find(|(path, _)| path == user_data).and_then(|(_, inner)| inner.upgrade()).map(Extensions)
}

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(work).await.context("extension task stopped")?
}

impl Extensions {
    /// A registry over `paths`. `enabled` is the stored `extensions` value;
    /// `persist` receives it whenever it changes. Nothing is scanned yet.
    pub fn new(paths: Paths, enabled: Option<Value>, persist: registry::Persist) -> Self {
        let registry = Registry::new(paths.user_dir, paths.build_dir, paths.builtin_dir, Compiler::new(paths.compiler_dir), enabled, persist);
        Self(Arc::new(Inner {
            registry: Arc::new(registry),
            scratch_dir: paths.scratch_dir,
            ready: OnceCell::new(),
            changed: broadcast::channel(64).0,
            host: Mutex::new(Weak::new()),
            watcher: Mutex::new(None),
        }))
    }

    pub fn user_dir(&self) -> &Path {
        &self.0.registry.user_dir
    }

    pub fn build_dir(&self) -> &Path {
        &self.0.registry.build_dir
    }

    /// The shipped built-in sources forks are made from.
    pub fn builtin_dir(&self) -> &Path {
        &self.0.registry.builtin_dir
    }

    /// Wait for the first full scan (started by [`register`]).
    async fn ready(&self) -> Result<()> {
        let registry = self.0.registry.clone();
        self.0
            .ready
            .get_or_try_init(|| async move { blocking(move || registry.refresh(None)).await })
            .await
            .map(|_| ())
    }

    /// Current records, after the first scan.
    pub async fn list(&self) -> Result<Vec<Record>> {
        self.ready().await?;
        Ok(self.0.registry.list())
    }

    /// Rescan and rebuild `ids` (everything when `None`), then emit
    /// `ext:changed` with `reason` for the ids (all known ids when `None`).
    pub async fn refresh(&self, ids: Option<Vec<String>>, reason: &str) -> Result<()> {
        self.ready().await?;
        let registry = self.0.registry.clone();
        let requested = ids.clone();
        blocking(move || registry.refresh(requested.as_deref())).await?;
        let ids = ids.unwrap_or_else(|| self.0.registry.list().into_iter().map(|record| record.id).collect());
        self.emit_changed(&ids, reason);
        Ok(())
    }

    /// Compile the extension in `dir` (whose folder name is its id) into a
    /// throwaway directory. `Ok(hash)` or the compiler's user-facing error.
    pub async fn validate(&self, dir: PathBuf) -> Result<String, String> {
        let registry = self.0.registry.clone();
        let scratch = self.0.scratch_dir.join("Extension Checks").join(uuid::Uuid::new_v4().simple().to_string());
        tokio::task::spawn_blocking(move || {
            let raw = std::fs::read_to_string(dir.join("manifest.json")).map_err(|error| format!("manifest.json: {error}"))?;
            let raw: Value = serde_json::from_str(&raw).map_err(|error| format!("manifest.json: {error}"))?;
            let manifest = parse_manifest(&raw)?;
            let result = registry.compiler.compile(&dir, manifest.entry(), &scratch).map(|Compiled { hash, .. }| hash);
            let _ = std::fs::remove_dir_all(&scratch);
            result
        })
        .await
        .map_err(|error| error.to_string())?
    }

    /// Fork built-in `id` into the user folder; returns the fork's id.
    pub async fn fork_builtin(&self, id: String) -> Result<String> {
        let (builtin_dir, user_dir) = (self.0.registry.builtin_dir.clone(), self.0.registry.user_dir.clone());
        let source = id.clone();
        let forked = blocking(move || fork::fork_builtin(&std::path::absolute(builtin_dir)?, &source, &std::path::absolute(user_dir)?, None)).await?;
        self.refresh(Some(vec![id, forked.fork_id.clone()]), "create").await?;
        Ok(forked.fork_id)
    }

    /// Merge a stale fork onto the shipped built-in, compile it, and replace
    /// the live folder. Conflicts or a failed compile leave it untouched.
    /// Unlike the Mac app this records no undo history.
    pub async fn update_fork(&self, fork_id: String) -> Result<ForkUpdate> {
        let registry = self.0.registry.clone();
        let scratch = self.0.scratch_dir.join("Fork Updates");
        let id = fork_id.clone();
        let outcome = blocking(move || {
            let user_dir = std::path::absolute(&registry.user_dir)?;
            let builtin_dir = std::path::absolute(&registry.builtin_dir)?;
            let validation = scratch.join(format!("build-{}", uuid::Uuid::new_v4().simple()));
            let compile = |working: &Path| {
                let entry = std::fs::read_to_string(working.join("manifest.json"))
                    .ok()
                    .and_then(|text| serde_json::from_str::<Value>(&text).ok())
                    .and_then(|raw| parse_manifest(&raw).ok())
                    .map(|manifest| manifest.entry().to_owned())
                    .ok_or_else(|| "invalid manifest".to_owned())?;
                let result = registry.compiler.compile(working, &entry, &validation).map(|_| ());
                let _ = std::fs::remove_dir_all(&validation);
                result
            };
            rebase::update_fork(&id, &user_dir, &builtin_dir, &scratch, registry::directory_signature, compile)
        })
        .await?;
        if matches!(outcome, ForkUpdate::Updated { .. }) {
            self.refresh(Some(vec![fork_id]), "reload").await?;
        }
        Ok(outcome)
    }

    /// Every `ext:changed` payload from now on.
    pub fn subscribe(&self) -> broadcast::Receiver<Value> {
        self.0.changed.subscribe()
    }

    /// Send `ext:changed` to every window and subscriber.
    pub fn emit_changed(&self, ids: &[String], reason: &str) {
        let event = json!({ "ids": ids, "reason": reason });
        let _ = self.0.changed.send(event.clone());
        if let Some(host) = self.0.host.lock().upgrade() {
            host.broadcast("ext:changed", event, None);
        }
    }

    /// Rebuild ids changed on disk and emit `ext:changed` (reason `watch`).
    pub fn start_watcher(&self) -> Result<()> {
        let weak = Arc::downgrade(&self.0);
        let watcher = watcher::start(self.user_dir(), self.build_dir(), watcher::DEBOUNCE, move |ids| {
            let weak = weak.clone();
            async move {
                let Some(inner) = weak.upgrade() else { return };
                if let Err(error) = Extensions(inner).refresh(Some(ids), "watch").await {
                    tracing::warn!("extension rebuild after a change failed: {error:#}");
                }
            }
        })?;
        *self.0.watcher.lock() = Some(watcher);
        Ok(())
    }

    fn remember_host(&self, call: &Call) {
        let mut host = self.0.host.lock();
        if host.strong_count() == 0 {
            *host = Arc::downgrade(&call.host);
        }
    }
}

#[derive(Deserialize)]
struct IdRequest {
    id: String,
}

fn id_request(call: &Call, channel: &str) -> Result<String> {
    call.parse::<IdRequest>().ok().map(|request| request.id).filter(|id| is_extension_id(id)).ok_or_else(|| anyhow!("{channel}: expected {{ id }}"))
}

fn records(records: Vec<Record>) -> Result<Value> {
    Ok(serde_json::to_value(records)?)
}

/// Run `work` against the registry on a blocking thread once the first scan is done.
async fn with_registry<T: Send + 'static>(extensions: &Extensions, call: &Call, work: impl FnOnce(&Registry) -> Result<T> + Send + 'static) -> Result<T> {
    extensions.remember_host(call);
    extensions.ready().await?;
    let registry = extensions.0.registry.clone();
    blocking(move || work(&registry)).await
}

pub async fn register(router: &mut Router, config: &Config, store: Arc<Store>) -> Result<()> {
    let paths = Paths {
        user_dir: config.user_data.join("extensions"),
        build_dir: config.user_data.join("extensions-build"),
        builtin_dir: config.builtin_extensions.clone(),
        compiler_dir: config.resources_dir.join("compiler"),
        scratch_dir: config.user_data.clone(),
    };
    tokio::fs::create_dir_all(&paths.user_dir).await.context("create extensions folder")?;
    let persist_store = store.clone();
    let persist: registry::Persist = Box::new(move |value| {
        if let Err(error) = persist_store.set(STORE_KEY, &value) {
            tracing::warn!("could not save extension state: {error:#}");
        }
    });
    let extensions = Extensions::new(paths, store.get_value(STORE_KEY), persist);
    {
        let mut profiles = PROFILES.lock();
        profiles.retain(|(path, inner)| inner.strong_count() > 0 && path != &config.user_data);
        profiles.push((config.user_data.clone(), Arc::downgrade(&extensions.0)));
    }
    if let Err(error) = extensions.start_watcher() {
        tracing::warn!("extension watcher unavailable: {error:#}");
    }
    let startup = extensions.clone();
    tokio::spawn(async move {
        if let Err(error) = startup.ready().await {
            tracing::warn!("extension scan failed: {error:#}");
        }
    });

    router.json_with(&extensions, "ext:list", |extensions, call| async move {
        extensions.remember_host(&call);
        records(extensions.list().await?)
    });
    router.json_with(&extensions, "ext:set-enabled", |extensions, call| async move {
        #[derive(Deserialize)]
        struct Request {
            id: String,
            enabled: bool,
        }
        let request = call.parse::<Request>().ok().filter(|request| is_extension_id(&request.id)).context("ext:set-enabled: expected { id, enabled }")?;
        let (id, enabled) = (request.id.clone(), request.enabled);
        let list = with_registry(&extensions, &call, move |registry| registry.set_enabled(&id, enabled)).await?;
        extensions.emit_changed(&[request.id], if enabled { "enable" } else { "disable" });
        records(list)
    });
    router.json_with(&extensions, "ext:remove", |extensions, call| async move {
        let id = id_request(&call, "ext:remove")?;
        let target = id.clone();
        let list = with_registry(&extensions, &call, move |registry| registry.remove(&target)).await?;
        extensions.emit_changed(&[id], "remove");
        records(list)
    });
    router.json_with(&extensions, "ext:reload", |extensions, call| async move {
        let id = id_request(&call, "ext:reload")?;
        let target = id.clone();
        let list = with_registry(&extensions, &call, move |registry| registry.reload(&target)).await?;
        extensions.emit_changed(&[id], "reload");
        records(list)
    });
    router.json_with(&extensions, "ext:create", |extensions, call| async move {
        let payload = call.arg(0);
        let valid = payload["manifest"].is_object()
            && payload["manifest"]["id"].as_str().is_some_and(is_extension_id)
            && payload["files"].as_object().is_some_and(|files| files.len() <= limits::SOURCE_FILES && files.values().all(Value::is_string));
        if !valid {
            bail!("ext:create: expected {{ manifest, files }}");
        }
        let request: CreateRequest = call.parse()?;
        let (id, list) = with_registry(&extensions, &call, move |registry| {
            let id = registry.create(&request)?;
            Ok((id, registry.list()))
        })
        .await?;
        extensions.emit_changed(&[id], "create");
        records(list)
    });
    router.json_with(&extensions, "ext:fork", |extensions, call| async move {
        let id = id_request(&call, "ext:fork")?;
        extensions.remember_host(&call);
        let fork_id = extensions.fork_builtin(id).await?;
        Ok(json!({ "id": fork_id }))
    });
    router.json_with(&extensions, "ext:reveal", |extensions, call| async move {
        let id = id_request(&call, "ext:reveal")?;
        let path = with_registry(&extensions, &call, move |registry| registry.reveal_path(&id)).await?;
        crate::app::reveal(&path)?;
        Ok(Value::Null)
    });
    router.json_with(&extensions, "ext:read-source", |extensions, call| async move {
        let id = id_request(&call, "ext:read-source")?;
        let files = with_registry(&extensions, &call, move |registry| registry.read_source(&id)).await?;
        Ok(serde_json::to_value(files)?)
    });
    router.on_with(&extensions, "ext:report-health", |extensions, call| async move {
        // One-way telemetry: a stale window may report on a removed extension.
        let payload = call.arg(0);
        let report = payload["id"].as_str().filter(|id| is_extension_id(id)).zip(Reported::parse(&payload["health"]));
        if let Some((id, health)) = report {
            let id = id.to_owned();
            let target = id.clone();
            if let Ok(true) = with_registry(&extensions, &call, move |registry| registry.report_health(&target, health)).await {
                // 'health' refreshes lists without making the loader reload.
                extensions.emit_changed(&[id], "health");
            }
        }
        Ok(Payload::default())
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use compiler::tests::{staged_compiler, write};
    use manifest::Health;

    fn open(root: &Path, compiler_dir: PathBuf) -> (Extensions, Arc<Mutex<Option<Value>>>) {
        let saved = Arc::new(Mutex::new(None));
        let sink = saved.clone();
        let paths = Paths {
            user_dir: root.join("extensions"),
            build_dir: root.join("extensions-build"),
            builtin_dir: root.join("builtin"),
            compiler_dir,
            scratch_dir: root.to_path_buf(),
        };
        (Extensions::new(paths, Some(json!({ "my-mod": false, "BAD": true })), Box::new(move |value| *sink.lock() = Some(value))), saved)
    }

    async fn next_event(events: &mut broadcast::Receiver<Value>) -> Value {
        tokio::time::timeout(Duration::from_secs(20), events.recv()).await.expect("ext:changed in time").unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rebuilds_and_announces_mods_written_to_disk() {
        let Some(compiler_dir) = staged_compiler() else { return };
        let root = tempfile::tempdir().unwrap();
        let (extensions, _) = open(root.path(), compiler_dir);
        std::fs::create_dir_all(extensions.user_dir()).unwrap();
        extensions.start_watcher().unwrap();
        assert!(extensions.list().await.unwrap().is_empty());
        let mut events = extensions.subscribe();

        // What an agent does: write a manifest and sources straight into the folder.
        let dir = extensions.user_dir().join("agent-mod");
        write(&dir, &[
            ("manifest.json", r#"{ "id": "agent-mod", "name": "Agent mod", "version": "1.0.0", "apiVersion": 3 }"#),
            ("index.ts", "export default function activate() { return 1; }"),
        ]);
        let record = loop {
            let event = next_event(&mut events).await;
            assert_eq!(event, json!({ "ids": ["agent-mod"], "reason": "watch" }));
            let record = extensions.list().await.unwrap().into_iter().find(|record| record.id == "agent-mod");
            if let Some(record) = record.filter(|record| record.bundle_hash.is_some()) {
                break record;
            }
        };
        assert_eq!(record.health, Health::Ok);
        let hash = record.bundle_hash.clone().unwrap();
        assert_eq!(record.bundle_url.as_deref(), Some(format!("app://powermove/ext/agent-mod/bundle.js?v={hash}").as_str()));
        assert!(extensions.build_dir().join("agent-mod").join("bundle.js").is_file());

        // An edit rebuilds; a broken edit reports the compiler's error.
        std::fs::write(dir.join("index.ts"), "export default function activate() { return 2; }").unwrap();
        let rebuilt = loop {
            next_event(&mut events).await;
            let record = extensions.list().await.unwrap().into_iter().find(|record| record.id == "agent-mod").unwrap();
            if record.bundle_hash.as_ref().is_some_and(|next| *next != hash) {
                break record;
            }
        };
        assert_eq!(rebuilt.health, Health::Ok);
        std::fs::write(dir.join("index.ts"), "import x from 'left-pad'; export default x;").unwrap();
        loop {
            next_event(&mut events).await;
            let record = extensions.list().await.unwrap().into_iter().find(|record| record.id == "agent-mod").unwrap();
            if let Health::BuildError { error } = record.health {
                assert_eq!(error, "Unsupported bare import: left-pad");
                assert_eq!(record.bundle_url, None);
                break;
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn keeps_enabled_state_and_health_like_the_registry() {
        let root = tempfile::tempdir().unwrap();
        // No compiler: user mods list with a build error instead of a bundle.
        let (extensions, saved) = open(root.path(), root.path().join("no-compiler"));
        write(&root.path().join("builtin").join("tools"), &[("manifest.json", r#"{ "id": "tools", "name": "Tools", "version": "1.0.0", "apiVersion": 3 }"#)]);
        write(&extensions.user_dir().join("my-mod"), &[("manifest.json", r#"{ "id": "my-mod", "name": "Mine", "version": "1.0.0", "apiVersion": 3 }"#), ("index.ts", "")]);
        write(&extensions.user_dir().join("wrong-id"), &[("manifest.json", r#"{ "id": "other", "name": "X", "version": "1.0.0", "apiVersion": 3 }"#)]);

        let list = extensions.list().await.unwrap();
        let ids: Vec<(&str, Value)> = list.iter().map(|record| (record.id.as_str(), serde_json::to_value(&record.health).unwrap())).collect();
        assert_eq!(ids, vec![
            ("tools", json!({ "state": "ok" })),
            ("my-mod", json!({ "state": "disabled" })),
            ("wrong-id", json!({ "state": "manifest-error", "error": "manifest id \"other\" must match directory name \"wrong-id\"" })),
        ]);
        assert_eq!(list[0].trust, "builtin");
        assert_eq!(list[1].trust, "local");

        let registry = extensions.0.registry.clone();
        let list = blocking(move || registry.set_enabled("my-mod", true)).await.unwrap();
        let health = serde_json::to_value(&list[1].health).unwrap();
        assert_eq!(health["state"], "build-error");
        assert!(health["error"].as_str().unwrap().starts_with("The mod compiler is missing"), "{health}");
        assert_eq!(*saved.lock(), Some(json!({ "my-mod": true })));

        let registry = extensions.0.registry.clone();
        assert!(registry.report_health("my-mod", Reported::RuntimeError("boom".into())).unwrap());
        let record = registry.record("my-mod").unwrap();
        assert_eq!((record.enabled, record.health), (false, Health::RuntimeError { error: "boom".into() }));
        assert_eq!(*saved.lock(), Some(json!({ "my-mod": false })));

        let forked = extensions.fork_builtin("tools".into()).await.unwrap();
        assert_eq!(forked, "tools-fork");
        let registry = extensions.0.registry.clone();
        let list = blocking(move || registry.remove("tools-fork")).await.unwrap();
        assert!(list.iter().all(|record| record.id != "tools-fork"));
        assert!(!extensions.user_dir().join("tools-fork").exists());
    }
}
