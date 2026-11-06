//! Store extensions in the app: each runs in its own confined helper, never in
//! the trusted realm. Installs the shipped app made carry no signed envelope, so
//! they enter through a recorded adoption of the bytes already on this Mac.
use crate::{
    KernelStoreAdapter, KernelStoreDispatch, StoreAuthority, StoreCompatRunner, StoreRuntimeHost,
    compile::ModuleGraph, discovery::Discovered, store_method_policy, store_runtime::StoreViews,
};
use pm_compat_host::{AdmittedDomBatch, RemoteDomPanel};
use pm_kernel::tx::Writer;
use pm_remote_dom::{TrySink, protocol::Session};
use pm_runtime_v8::{Error, Result};
use pm_store_host::{
    adapter::StoreSession,
    artifact::{PublisherIdentity, ReleaseStatement, SignedRelease},
    identity::{DataStoreIdentity, ExtensionId},
    install::{DataStoreStatus, ExtensionState, InstalledRelease, StoreKernel},
    permission::{Decision, Manifest, Permission, PermissionDecision},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

pub const STORE_MARKER: &str = ".powermove-store.json";
const LEGACY_PUBLISHER: &str = "powermove-store-v1";
/// The one panel of a Store extension's dedicated WebKit document.
pub const VIEW_PANEL: u64 = 1;
pub const VIEW_GENERATION: u32 = 1;

/// Next to the app in a build, or named by the environment in a test.
fn helper() -> PathBuf {
    std::env::var_os("PM_STORE_HELPER_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let exe = std::env::current_exe().unwrap_or_default();
            let mut directory = exe.parent().unwrap_or(Path::new(".")).to_path_buf();
            if directory.ends_with("deps") {
                directory.pop();
            }
            directory.join("pm-store-helper")
        })
}

pub fn marker(directory: &Path) -> Option<Value> {
    let path = directory.join(STORE_MARKER);
    let meta = std::fs::symlink_metadata(&path).ok()?;
    if !meta.is_file() {
        return None;
    }
    Some(
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or(json!({})),
    )
}

/// The shipped app installed this release before native signing existed. Record
/// exactly that: a legacy provenance whose declared permissions it already held.
fn adopt(
    kernel: &dyn StoreAuthority,
    namespace: &str,
    found: &Discovered,
    graph: &ModuleGraph,
    marker: &Value,
) -> Result<()> {
    let id = ExtensionId::new(found.id.clone()).map_err(|_| Error::Invalid("Store id"))?;
    let mut store = KernelStoreAdapter::new(kernel);
    let current = store.load(&id).map_err(|e| Error::Host(format!("{e:?}")))?;
    let raw = found
        .manifest
        .as_ref()
        .map_err(|_| Error::Invalid("Store manifest"))?;
    let manifest: Manifest =
        serde_json::from_value(raw.clone()).map_err(|_| Error::Invalid("Store manifest"))?;
    // Only a legacy adoption follows the disk: the shipped app may have updated
    // these bytes. A signed native install keeps exactly what it verified.
    if let Some(active) = current.as_ref().and_then(|state| state.active.as_ref())
        && (active.publisher != LEGACY_PUBLISHER
            || (active.bundle == graph.hash && active.manifest == manifest))
    {
        return Ok(());
    }
    let release_id = marker["releaseId"]
        .as_str()
        .filter(|id| !id.is_empty())
        .unwrap_or(&graph.hash)
        .to_owned();
    let permissions: BTreeMap<_, _> = manifest
        .permissions
        .iter()
        .filter(|p| **p != Permission::FullAccess)
        .map(|permission| {
            (
                *permission,
                PermissionDecision {
                    publisher: LEGACY_PUBLISHER.into(),
                    decision: Decision::Allow,
                },
            )
        })
        .collect();
    let release = InstalledRelease {
        publisher: LEGACY_PUBLISHER.into(),
        publisher_identity: PublisherIdentity::default(),
        release_id: release_id.clone(),
        bundle: graph.hash.clone(),
        local_overlay: None,
        proof: SignedRelease {
            statement: ReleaseStatement {
                format: 0,
                key_id: String::new(),
                publisher: LEGACY_PUBLISHER.into(),
                extension: id.clone(),
                release_id,
                archive_sha256: graph.hash.clone(),
                files: BTreeMap::new(),
            },
            signature: vec![],
        },
        manifest,
    };
    // Decisions already made stand; only what is new takes the legacy grant.
    let mut permissions = permissions;
    if let Some(state) = &current {
        permissions.extend(state.permissions.clone());
    }
    let expected = current.as_ref().map(|state| state.revision);
    let previous_archives = current
        .as_ref()
        .map(|state| state.archives.clone())
        .unwrap_or_default();
    let state = ExtensionState {
        revision: expected.unwrap_or(0) + 1,
        data_store: match current {
            Some(state) => state.data_store,
            None => DataStoreIdentity::new(namespace, id.clone())
                .map_err(|_| Error::Invalid("Store data identity"))?,
        },
        data_status: DataStoreStatus::Active,
        active: Some(release),
        archives: previous_archives,
        permissions,
    };
    store
        .compare_exchange(&id, expected, &state)
        .map_err(|e| Error::Host(format!("{e:?}")))
}

pub struct StoreExtension {
    pub id: String,
    pub session: StoreSession,
    runner: StoreCompatRunner,
    router: RemoteDomPanel,
}
impl StoreExtension {
    pub fn start(
        kernel: Arc<dyn StoreAuthority>,
        project: &str,
        namespace: &str,
        found: &Discovered,
        graph: &ModuleGraph,
        owner: Writer,
        views: Arc<dyn StoreViews>,
    ) -> Result<Self> {
        let marker = found.store.as_ref().unwrap_or(&Value::Null);
        adopt(&*kernel, namespace, found, graph, marker)?;
        let id = ExtensionId::new(found.id.clone()).map_err(|_| Error::Invalid("Store id"))?;
        let session = StoreSession::admit(&KernelStoreAdapter::new(&*kernel), &id)
            .map_err(|e| Error::Host(format!("{e:?}")))?;
        let dispatch = Arc::new(KernelStoreDispatch::new(owner, session.clone())?);
        let host = Arc::new(
            StoreRuntimeHost::new(
                kernel,
                project.to_owned(),
                session.clone(),
                store_method_policy().map_err(|e| Error::Host(format!("{e:?}")))?,
                dispatch,
            )
            .with_views(views),
        );
        let router = RemoteDomPanel::new(
            VIEW_PANEL,
            Session {
                panel: "store".into(),
                generation: VIEW_GENERATION,
            },
        )
        .map_err(|e| Error::Host(format!("{e:?}")))?;
        let manifest = found
            .manifest
            .as_ref()
            .map_err(|_| Error::Invalid("Store manifest"))?;
        let init = json!({"id":found.id,"apiVersion":manifest["apiVersion"],"manifest":manifest,
            "vars":{},"theme":{"scheme":"light","tokens":{}},"bundleUrl":found.id,
            "state":{"time":0,"playing":false,"revision":0,"generation":1,
                "selection":{"layers":[],"keys":[],"chan":null}}});
        let runner = StoreCompatRunner::new(&helper(), host, crate::STORE_BUNDLE, &init)?;
        let reply = runner.json(&format!(
            "__storeCheck.activateGraph({})",
            json!(graph.to_json().to_string())
        ))?;
        if reply["ok"] != true {
            let error = reply["error"]["message"]
                .as_str()
                .unwrap_or("Activation failed");
            return Err(Error::JavaScript(error.to_owned()));
        }
        Ok(Self {
            id: found.id.clone(),
            session,
            runner,
            router,
        })
    }
    /// Engine snapshots of what this extension registered; `origin: store`.
    pub fn definitions(&self, registered: &Value) -> Vec<Value> {
        self.runner
            .json(&format!("StoreCompat.definitions({registered})"))
            .ok()
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default()
    }
    /// The project moved: reads refresh and listeners run inside the helper.
    pub fn tick(&self, revision: u64) {
        let _ = self.runner.eval(&format!(
            "void __storeCheck.control.tick({},[[\"project:changed\",{{kind:\"values\"}}]])",
            json!({"revision":revision,"generation":revision})
        ));
    }
    /// Panels are registered under the extension's namespace; its code knows the
    /// id it wrote.
    pub fn open_panel(&self, panel: &str) {
        let short = panel
            .strip_prefix(&format!("{}.", self.id))
            .unwrap_or(panel);
        let _ = self.runner.eval(&format!(
            "void __storeCheck.openPanel({},{})",
            json!(panel),
            json!(short)
        ));
    }
    pub fn close_panel(&self, panel: &str) {
        let _ = self
            .runner
            .eval(&format!("void __storeCheck.closePanel({})", json!(panel)));
    }
    /// `None` is a tab behind another: it stays open and neither paints nor hits.
    pub fn place_panel(&self, panel: &str, rect: Option<[f32; 4]>) {
        let _ = self.runner.eval(&format!(
            "void __storeCheck.placePanel({},{})",
            json!(panel),
            json!(rect)
        ));
    }
    pub fn enqueue(
        &mut self,
        batch: Value,
        sink: &mut impl TrySink<AdmittedDomBatch>,
    ) -> pm_compat_host::Result<()> {
        self.router.enqueue(batch, sink)
    }
    /// Page feedback into this helper's closures, through the same router checks.
    pub fn page(&mut self, message: Value) {
        struct Delivered(Vec<Value>);
        impl TrySink<pm_remote_dom::protocol::PageMessage> for Delivered {
            fn try_send(
                &mut self,
                message: pm_remote_dom::protocol::PageMessage,
            ) -> std::result::Result<(), pm_remote_dom::Error> {
                self.0.push(
                    serde_json::to_value(message)
                        .map_err(|_| pm_remote_dom::Error::Invalid("page message"))?,
                );
                Ok(())
            }
        }
        let mut delivered = Delivered(vec![]);
        if self.router.receive(message, &mut delivered).is_err() {
            return;
        }
        for message in delivered.0 {
            let _ = self.runner.eval(&format!(
                "__storeCheck.dom.receive({message});__storeCheck.dom.flush()"
            ));
        }
    }
    /// The page queue has room again for what the helper's document held back.
    pub fn flush(&self) {
        let _ = self.runner.eval("void __storeCheck.dom.flush()");
    }
    pub fn run(&self, handle: u64, args: &Value) {
        let _ = self.runner.eval(&format!(
            "void __storeCheck.rpc.invokeHandle({handle},...{args})"
        ));
    }
}
impl Drop for StoreExtension {
    fn drop(&mut self) {
        let _ = self.runner.dispose();
    }
}
