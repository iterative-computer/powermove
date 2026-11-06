//! Permissioned IPC boundary for the confined Store helper. Invoke on workers;
//! AppKit never waits for this runtime or for the kernel owner.
use crate::{KernelStoreAdapter, StoreAuthority};
use pm_kernel::state::{StateOp, StateTransaction};
use pm_runtime_v8::*;
use pm_store_host::{
    adapter::StoreSession,
    install::StoreKernel,
    permission::{MethodPolicy, Permission},
};
use serde_json::{Value, json};
use std::io::Read;
use std::{
    collections::BTreeSet,
    path::Path,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

/// A native dispatcher receives an admitted method and the host-owned Store
/// identity. It must retain its own resource/focus/gesture checks as well.
pub trait StoreDispatch: Send + Sync + 'static {
    fn call(
        &self,
        kernel: &dyn StoreAuthority,
        session: &StoreSession,
        method: &str,
        args: &Value,
        budget: &HostBudget,
    ) -> Result<Value>;
    /// Cleanup is admission-independent: revocation must still remove owned
    /// registrations/callback leases and seal acknowledged gestures.
    fn revoke(&self, kernel: &dyn StoreAuthority, session: &StoreSession, generation: &Generation);
    fn seal(&self, kernel: &dyn StoreAuthority, session: &StoreSession, generation: &Generation);
}

/// Where a helper's views go: DOM batches for its WebKit page and panel reveals.
pub trait StoreViews: Send + Sync {
    /// False is backpressure: the helper keeps the batch and retries.
    fn batch(&self, batch: Value) -> bool;
    fn panel(&self, id: &str, open: bool);
}
pub struct StoreRuntimeHost {
    kernel: Arc<dyn StoreAuthority>,
    session: StoreSession,
    policy: MethodPolicy,
    dispatch: Arc<dyn StoreDispatch>,
    project: String,
    reports: Mutex<(Vec<Value>, usize)>,
    views: Option<Arc<dyn StoreViews>>,
}
impl StoreRuntimeHost {
    pub fn new(
        kernel: Arc<dyn StoreAuthority>,
        project: String,
        session: StoreSession,
        policy: MethodPolicy,
        dispatch: Arc<dyn StoreDispatch>,
    ) -> Self {
        Self {
            kernel,
            project,
            session,
            policy,
            dispatch,
            reports: Mutex::new((vec![], 0)),
            views: None,
        }
    }
    pub fn with_views(mut self, views: Arc<dyn StoreViews>) -> Self {
        self.views = Some(views);
        self
    }
    fn admit(&self, kernel: &dyn StoreAuthority, method: &str) -> Result<()> {
        let id = self.session.data_store.extension.as_str();
        if !kernel
            .state()
            .extensions
            .get(id)
            .is_some_and(|entry| entry.installed && entry.enabled)
        {
            return Err(Error::Host("Store extension is not enabled".into()));
        }
        self.session
            .authorize(&KernelStoreAdapter::new(kernel), &self.policy, method)
            .map_err(|_| Error::Host("Store permission or revision refused".into()))
    }
    /// A helper reveals and closes its own panels only.
    fn panel_call(&self, args: &[Value]) -> Option<Value> {
        let views = self.views.as_ref()?;
        let open = match args[1].as_str()? {
            "open" => true,
            "close" => false,
            _ => return None,
        };
        let id = args[2][0].as_str()?;
        let owner = self.session.data_store.extension.as_str();
        let id = if id.starts_with(&format!("{owner}.")) {
            id.to_owned()
        } else {
            format!("{owner}.{id}")
        };
        views.panel(&id, open);
        Some(json!({}))
    }
    fn storage(&self, kernel: &dyn StoreAuthority) -> Result<Value> {
        self.admit(kernel, "storage.get")?;
        let source = &kernel.state().extensions[self.session.data_store.extension.as_str()].source;
        source
            .get("storeRuntimeStorageV1")
            .map(|value| serde_json::from_str(&pm_model::json::to_canonical_string(value)))
            .transpose()
            .map(|value| value.unwrap_or_else(|| json!({})))
            .map_err(|_| Error::Invalid("Store runtime storage"))
    }
    fn core_call(
        &self,
        kernel: &dyn StoreAuthority,
        method: &str,
        args: &Value,
    ) -> Result<Option<Value>> {
        if ["runtime-error", "sandbox-report", "log"].contains(&method) {
            let mut reports = self.reports.lock().unwrap();
            let report = json!({"method":method,"args":args});
            let bytes = serde_json::to_vec(&report)
                .map_err(|_| Error::Invalid("Store report"))?
                .len();
            if reports.0.len() >= 256 || reports.1 + bytes > 1024 * 1024 {
                return Err(Error::Quota("Store reports"));
            }
            reports.1 += bytes;
            reports.0.push(report);
            return Ok(Some(json!({})));
        }
        if method == "project-snapshot" {
            let (generation, document) = kernel
                .project(&self.project)
                .ok_or(Error::Invalid("Store project is not open"))?;
            return Ok(Some(json!({"value":{"generation":generation,
                "json":pm_model::json::to_canonical_string(&document)}})));
        }
        if method != "invoke" || args[0] != "storage" {
            return Ok(None);
        }
        let member = args[1]
            .as_str()
            .ok_or(Error::Invalid("Store storage member"))?;
        let params = args[2]
            .as_array()
            .ok_or(Error::Invalid("Store storage arguments"))?;
        let arity = match member {
            "set" => 2,
            "get" | "delete" => 1,
            _ => return Err(Error::Invalid("Store storage method")),
        };
        if params.len() != arity {
            return Err(Error::Invalid("Store storage arguments"));
        }
        // The frozen broker applies its 128-character limit after its schema.
        // JavaScript counts UTF-16 code units, not UTF-8 bytes or scalar values.
        let key = params
            .first()
            .and_then(Value::as_str)
            .filter(|key| !key.is_empty() && key.encode_utf16().count() <= 128)
            .ok_or(Error::Invalid("Store storage key"))?;
        let snapshot = kernel.state();
        let mut entry = snapshot.extensions[self.session.data_store.extension.as_str()].clone();
        let mut source = entry
            .source
            .as_object()
            .cloned()
            .ok_or(Error::Invalid("Store source"))?;
        let mut values = source
            .get("storePrivateStorageV1")
            .and_then(pm_model::Value::as_object)
            .cloned()
            .unwrap_or_default();
        if member == "get" {
            let reply = values
                .get(key)
                .map(|value| {
                    serde_json::from_str::<Value>(&pm_model::json::to_canonical_string(value))
                })
                .transpose()
                .map_err(|_| Error::Invalid("Store storage value"))?;
            return Ok(Some(
                reply
                    .map(|value| json!({"value":value}))
                    .unwrap_or_else(|| json!({})),
            ));
        }
        match member {
            "set" => {
                let value = params.get(1).ok_or(Error::Invalid("Store storage value"))?;
                values.insert(
                    key,
                    pm_model::json::parse(&value.to_string())
                        .map_err(|_| Error::Invalid("Store storage value"))?,
                );
            }
            "delete" => {
                values.remove(key);
            }
            _ => return Err(Error::Invalid("Store storage method")),
        }
        if pm_model::json::to_canonical_string(&pm_model::Value::Object(values.clone())).len()
            > 256 * 1024
        {
            return Err(Error::Quota("Store private storage"));
        }
        source.insert("storePrivateStorageV1", pm_model::Value::Object(values));
        entry.source = pm_model::Value::Object(source);
        let revision = snapshot.revision;
        kernel
            .transact(StateTransaction {
                expected_revision: Some(revision),
                ops: vec![StateOp::Extension(entry)],
            })
            .map_err(Error::Host)?;
        Ok(Some(json!({})))
    }
    /// Private diagnostics only; callers must not publish source strings.
    pub fn private_reports(&self) -> Vec<Value> {
        self.reports.lock().unwrap().0.clone()
    }
}
impl EngineProxy for StoreRuntimeHost {
    fn call(
        &self,
        _: &Generation,
        operation: &str,
        input: &Value,
        budget: HostBudget,
    ) -> Result<Value> {
        budget.check()?;
        if operation == "store.uuid" {
            self.admit(&*self.kernel, "storage.get")?;
            let mut bytes = [0u8; 16];
            std::fs::File::open("/dev/urandom")
                .and_then(|mut file| file.read_exact(&mut bytes))
                .map_err(|_| Error::Host("Store entropy unavailable".into()))?;
            bytes[6] = (bytes[6] & 15) | 64;
            bytes[8] = (bytes[8] & 63) | 128;
            let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
            return Ok(json!(format!(
                "{}-{}-{}-{}-{}",
                &hex[..8],
                &hex[8..12],
                &hex[12..16],
                &hex[16..20],
                &hex[20..]
            )));
        }
        if operation == "store.dom" {
            self.admit(&*self.kernel, "storage.get")?;
            return match &self.views {
                Some(views) => Ok(views.batch(input["batch"].clone()).into()),
                None => Err(Error::Host("Store views are not attached".into())),
            };
        }
        if operation != "store.rpc"
            || serde_json::to_vec(input)
                .map_err(|_| Error::Invalid("Store RPC"))?
                .len()
                > 1024 * 1024
        {
            return Err(Error::Invalid("Store RPC boundary"));
        }
        let method = input["method"]
            .as_str()
            .ok_or(Error::Invalid("Store RPC method"))?;
        let args = input["args"]
            .as_array()
            .ok_or(Error::Invalid("Store RPC arguments"))?;
        let policy_method = if method == "invoke" {
            format!(
                "{}.{}",
                args.first()
                    .and_then(Value::as_str)
                    .ok_or(Error::Invalid("Store namespace"))?,
                args.get(1)
                    .and_then(Value::as_str)
                    .ok_or(Error::Invalid("Store member"))?
            )
        } else {
            method.to_owned()
        };
        let kernel = &*self.kernel;
        self.admit(kernel, &policy_method)?;
        let panel = (method == "invoke" && args[0] == "panels")
            .then(|| self.panel_call(args))
            .flatten();
        let reply = match panel {
            Some(reply) => reply,
            None => match self.core_call(kernel, method, &input["args"])? {
                Some(reply) => reply,
                None => {
                    self.dispatch
                        .call(kernel, &self.session, method, &input["args"], &budget)?
                }
            },
        };
        budget.check()?;
        if serde_json::to_vec(&reply)
            .map_err(|_| Error::Invalid("Store RPC reply"))?
            .len()
            > budget.max_response_bytes
        {
            return Err(Error::Quota("Store RPC reply"));
        }
        Ok(reply)
    }
}
impl KernelHooks for StoreRuntimeHost {
    fn load_storage(&self, _: &str) -> Result<Value> {
        self.storage(&*self.kernel)
    }
    fn commit_activation(
        &self,
        _: Option<&Generation>,
        _: &Generation,
        ops: &[Value],
        storage: &Value,
    ) -> Result<()> {
        if !ops.is_empty() {
            return Err(Error::Invalid("Store raw staged operations are forbidden"));
        }
        if !storage.is_object()
            || serde_json::to_vec(storage)
                .map_err(|_| Error::Invalid("Store storage"))?
                .len()
                > 1024 * 1024
        {
            return Err(Error::Quota("Store storage"));
        }
        let kernel = &*self.kernel;
        self.admit(kernel, "storage.set")?;
        let snapshot = kernel.state();
        let mut entry = snapshot.extensions[self.session.data_store.extension.as_str()].clone();
        let mut source = entry
            .source
            .as_object()
            .cloned()
            .ok_or(Error::Invalid("Store source"))?;
        source.insert(
            "storeRuntimeStorageV1",
            pm_model::json::parse(&storage.to_string())
                .map_err(|_| Error::Invalid("Store storage"))?,
        );
        entry.source = pm_model::Value::Object(source);
        let revision = snapshot.revision;
        kernel
            .transact(StateTransaction {
                expected_revision: Some(revision),
                ops: vec![StateOp::Extension(entry)],
            })
            .map_err(Error::Host)
    }
    fn commit_retained(&self, _: &Generation, _: &[Value]) -> Result<()> {
        Err(Error::Invalid(
            "Store retained raw operations are forbidden",
        ))
    }
    fn revoke_generation(&self, generation: &Generation) {
        self.dispatch
            .revoke(&*self.kernel, &self.session, generation);
    }
    fn seal_open_gestures(&self, generation: &Generation) {
        self.dispatch.seal(&*self.kernel, &self.session, generation);
    }
}
impl HostProxy for StoreRuntimeHost {
    /// Admission re-checks the live grant; the helper itself has no network.
    fn fetch(
        &self,
        _: &Generation,
        request: FetchRequest,
        cancel: Cancellation,
    ) -> Result<Box<dyn HostRequest>> {
        self.admit(&*self.kernel, "fetch")?;
        crate::net::fetch(request, cancel)
    }
    fn console(&self, _: &Generation, _: &str, _: &str) {}
}
impl WatchdogHooks for StoreRuntimeHost {
    fn notice(&self, _: &Generation, _: u64) {}
    fn prompt(&self, _: &Generation, _: u64) -> WatchdogDecision {
        WatchdogDecision::Stop
    }
}

/// The frozen asynchronous facade runs in a real confined helper, never a
/// trusted Compat realm. Only copied JSON and callback ids cross its IPC.
pub struct StoreCompatRunner {
    helper: StoreHelper,
    _pool: StorePool,
}
impl StoreCompatRunner {
    pub fn new(
        executable: &Path,
        host: Arc<StoreRuntimeHost>,
        bundle: &str,
        init: &Value,
    ) -> Result<Self> {
        Self::with_prelude(executable, host, bundle, init, "")
    }
    /// Comparison-only prelude, evaluated in the confined helper before code.
    pub fn with_prelude(
        executable: &Path,
        host: Arc<StoreRuntimeHost>,
        bundle: &str,
        init: &Value,
        prelude: &str,
    ) -> Result<Self> {
        if init["id"] != host.session.data_store.extension.as_str()
            || init["manifest"]["id"] != init["id"]
        {
            return Err(Error::Invalid("Store facade identity"));
        }
        {
            host.admit(&*host.kernel, "storage.get")?;
            let state = KernelStoreAdapter::new(&*host.kernel)
                .load(&host.session.data_store.extension)
                .map_err(|_| Error::Invalid("Store installation"))?
                .ok_or(Error::Invalid("Store installation"))?;
            let manifest = state
                .active
                .ok_or(Error::Invalid("Store release"))?
                .manifest;
            let provided: pm_store_host::permission::Manifest =
                serde_json::from_value(init["manifest"].clone())
                    .map_err(|_| Error::Invalid("Store facade manifest"))?;
            if provided != manifest
                || init["apiVersion"] != manifest.api_version
                || !(1..=3).contains(&manifest.api_version)
            {
                return Err(Error::Invalid("Store facade manifest"));
            }
        }
        let cluster = host.session.data_store.key.clone();
        let pool = StorePool::new(
            executable,
            "extcheck-store",
            Config {
                limits: Limits::default(),
                watchdog: WatchdogPolicy::default(),
                kernel: host.clone(),
                host: host.clone(),
                watchdog_hooks: host.clone(),
                engine: Some(host),
                cache: CodeCache::new(4 * 1024 * 1024),
                admission: Admission::new(256 * 1024 * 1024),
            },
        )?;
        let runner = Self {
            helper: pool.helper(&cluster)?,
            _pool: pool,
        };
        if !prelude.is_empty() {
            runner.eval(prelude)?;
        }
        runner.eval(include_str!("globals.js"))?;
        runner.eval(bundle)?;
        runner.eval(include_str!("store-bootstrap.js"))?;
        runner.json(&format!("__storeCheck.create({init})"))?;
        if runner.helper.status().confinement.get("seatbelt") != Some(&Value::Bool(true))
            && cfg!(target_os = "macos")
        {
            return Err(Error::Invalid("Store confinement was not established"));
        }
        Ok(runner)
    }
    pub fn eval(&self, source: &str) -> Result<Evaluation> {
        let ticket = self.helper.eval(source)?;
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(result) = ticket.try_take() {
                return result;
            }
            if Instant::now() > deadline {
                self.helper.close();
                return Err(Error::Host("Store evaluation timeout".into()));
            }
            thread::sleep(Duration::from_millis(1));
        }
    }
    pub fn json(&self, expression: &str) -> Result<Value> {
        let result = self.eval(&format!(
            "Promise.resolve({expression}).then(value=>JSON.stringify(value))"
        ))?;
        serde_json::from_str(&result.value).map_err(|_| Error::Invalid("Store JSON result"))
    }
    pub fn activate(&self, module: &str) -> Result<Value> {
        self.eval(module)?;
        self.json("__storeCheck.activate(ExtCheckModule)")
    }
    /// Graceful teardown runs on the runtime worker before closing its helper.
    pub fn dispose(&self) -> Result<Value> {
        self.json("__storeCheck.dispose()")
    }
    pub fn status(&self) -> StoreStatus {
        self.helper.status()
    }
}
impl Drop for StoreCompatRunner {
    fn drop(&mut self) {
        self.helper.close();
    }
}

/// Explicit native capability table. Resource implementations still validate
/// schemas, owner namespaces, resource ids and user activation independently.
pub fn store_method_policy() -> pm_store_host::Result<MethodPolicy> {
    let mut policy = MethodPolicy::default();
    for name in [
        "register",
        "dispose-registration",
        "project-snapshot",
        "extensions-list",
        "log",
        "sandbox-report",
        "runtime-error",
        "storage.get",
        "storage.set",
        "storage.delete",
        "commands.run",
        "panels.open",
        "panels.close",
        "panels.refresh",
        "panels.isOpen",
    ] {
        policy.register(name.into(), BTreeSet::new())?;
    }
    for name in [
        "project.apply",
        "project.select",
        "project.setTime",
        "project.play",
        "project.pause",
        "project.undo",
        "project.redo",
        "transport.step",
    ] {
        policy.register(name.into(), [Permission::ProjectWrite].into())?;
    }
    policy.register("fetch".into(), [Permission::Network].into())?;
    policy.register("ui.copy".into(), [Permission::Clipboard].into())?;
    Ok(policy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_kernel::{Kernel, KernelConfig, OpenSource};
    use pm_store_host::{
        artifact::{PublisherIdentity, ReleaseStatement, SignedRelease},
        identity::{DataStoreIdentity, ExtensionId},
        install::{DataStoreStatus, ExtensionState, InstalledRelease},
        permission::{Decision, Manifest, PermissionDecision},
    };
    use std::{
        collections::BTreeMap,
        sync::atomic::{AtomicUsize, Ordering},
    };

    #[derive(Default)]
    struct Dispatch {
        calls: AtomicUsize,
        revoked: AtomicUsize,
    }
    impl StoreDispatch for Dispatch {
        fn call(
            &self,
            _: &dyn StoreAuthority,
            _: &StoreSession,
            _: &str,
            _: &Value,
            _: &HostBudget,
        ) -> Result<Value> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Err(Error::Host(
                "Synthetic platform capability is not attached".into(),
            ))
        }
        fn revoke(&self, _: &dyn StoreAuthority, _: &StoreSession, _: &Generation) {
            self.revoked.fetch_add(1, Ordering::Relaxed);
        }
        fn seal(&self, _: &dyn StoreAuthority, _: &StoreSession, _: &Generation) {}
    }
    fn fixture() -> (
        Arc<Mutex<Kernel>>,
        Arc<StoreRuntimeHost>,
        Arc<Dispatch>,
        Value,
    ) {
        let mut kernel = Kernel::new(KernelConfig::default());
        kernel
            .open(
                "synthetic-project",
                OpenSource::Untitled(
                    pm_model::json::parse(&crate::initial()["document"].to_string()).unwrap(),
                ),
            )
            .unwrap();
        let manifest: Manifest = serde_json::from_value(
            json!({"id":"synthetic-store","name":"Synthetic Store fixture",
            "version":"1.0.0","apiVersion":3,"permissions":["project:write"]}),
        )
        .unwrap();
        let id = ExtensionId::new("synthetic-store").unwrap();
        // An already-admitted native authority fixture. Archive/signature
        // admission is covered in store-host; no third-party code uses this.
        let state = ExtensionState {
            revision: 1,
            data_store: DataStoreIdentity::new("synthetic-profile", id.clone()).unwrap(),
            data_status: DataStoreStatus::Active,
            active: Some(InstalledRelease {
                manifest: manifest.clone(),
                publisher: "synthetic-publisher".into(),
                publisher_identity: PublisherIdentity::default(),
                release_id: "synthetic-release".into(),
                bundle: "0".repeat(64),
                local_overlay: None,
                proof: SignedRelease {
                    statement: ReleaseStatement {
                        format: 1,
                        key_id: "synthetic-key".into(),
                        publisher: "synthetic-publisher".into(),
                        extension: id.clone(),
                        release_id: "synthetic-release".into(),
                        archive_sha256: "0".repeat(64),
                        files: BTreeMap::new(),
                    },
                    signature: vec![],
                },
            }),
            archives: vec![],
            permissions: BTreeMap::from([(
                Permission::ProjectWrite,
                PermissionDecision {
                    publisher: "synthetic-publisher".into(),
                    decision: Decision::Allow,
                },
            )]),
        };
        let kernel = Arc::new(Mutex::new(kernel));
        KernelStoreAdapter::new(&*kernel)
            .compare_exchange(&id, None, &state)
            .unwrap();
        let session = StoreSession::admit(&KernelStoreAdapter::new(&*kernel), &id).unwrap();
        let dispatch = Arc::new(Dispatch::default());
        let host = Arc::new(StoreRuntimeHost::new(
            kernel.clone(),
            "synthetic-project".into(),
            session,
            store_method_policy().unwrap(),
            dispatch.clone(),
        ));
        let init = json!({"id":"synthetic-store","apiVersion":3,"manifest":manifest,"vars":{},
            "theme":{"scheme":"light","tokens":{}},"state":{"time":0,"playing":false,"revision":0,"generation":1,"selection":{"layers":[],"keys":[],"chan":null}},"bundleUrl":"synthetic"});
        (kernel, host, dispatch, init)
    }
    fn budget() -> HostBudget {
        HostBudget {
            cancellation: Cancellation::default(),
            deadline: Instant::now() + Duration::from_secs(5),
            max_response_bytes: 1024 * 1024,
        }
    }
    fn generation() -> Generation {
        Generation {
            realm: "synthetic-store".into(),
            number: 1,
        }
    }
    #[test]
    fn permission_revocation_raw_operations_and_unknown_methods_never_reach_dispatch() {
        let (kernel, host, dispatch, _) = fixture();
        assert!(
            host.call(
                &generation(),
                "store.rpc",
                &json!({"method":"invoke","args":["project","apply",[]]}),
                budget()
            )
            .is_err()
        );
        assert_eq!(dispatch.calls.load(Ordering::Relaxed), 1);
        {
            let mut store = KernelStoreAdapter::new(&*kernel);
            let mut state = store
                .load(&host.session.data_store.extension)
                .unwrap()
                .unwrap();
            state.revision += 1;
            state
                .permissions
                .get_mut(&Permission::ProjectWrite)
                .unwrap()
                .decision = Decision::Deny;
            store
                .compare_exchange(&host.session.data_store.extension, Some(1), &state)
                .unwrap();
        }
        assert!(
            host.call(
                &generation(),
                "store.rpc",
                &json!({"method":"invoke","args":["project","apply",[]]}),
                budget()
            )
            .is_err()
        );
        assert!(
            host.call(&generation(), "kernel.call", &json!({}), budget())
                .is_err()
        );
        assert!(
            host.commit_activation(None, &generation(), &[json!({"kind":"write"})], &json!({}))
                .is_err()
        );
        assert_eq!(dispatch.calls.load(Ordering::Relaxed), 1);
    }
    #[test]
    fn storage_matches_frozen_broker_utf16_key_and_byte_limits() {
        let (_kernel, host, _, _) = fixture();
        let write = |key: String, value: Value| {
            host.call(
                &generation(),
                "store.rpc",
                &json!({"method":"invoke","args":["storage","set",[key,value]]}),
                budget(),
            )
        };
        assert!(write("x".repeat(129), json!(1)).is_err());
        assert!(write("x".repeat(128), json!(1)).is_ok());
        assert!(write("😀".repeat(64), json!(1)).is_ok());
        assert!(write("😀".repeat(65), json!(1)).is_err());
        assert!(write("synthetic-limit".into(), json!("x".repeat(256 * 1024))).is_err());
        assert!(write(String::new(), json!(1)).is_err());
        assert!(
            host.call(
                &generation(),
                "store.rpc",
                &json!({"method":"invoke","args":["storage","get",["synthetic-limit","extra"]]}),
                budget()
            )
            .is_err()
        );
    }
    #[test]
    fn private_reports_have_a_total_byte_limit() {
        let (_kernel, host, _, _) = fixture();
        let input = json!({"method":"log","args":["info", "x".repeat(600_000)]});
        host.call(&generation(), "store.rpc", &input, budget())
            .unwrap();
        assert!(matches!(
            host.call(&generation(), "store.rpc", &input, budget()),
            Err(Error::Quota("Store reports"))
        ));
        assert_eq!(host.private_reports().len(), 1);
    }
    #[test]
    fn real_store_commands_publish_native_provider_stacks_and_invoke_the_confined_callback() {
        let (kernel, original, _, init) = fixture();
        let dispatch = Arc::new(
            crate::KernelStoreDispatch::new(
                pm_kernel::tx::Writer {
                    realm: 71,
                    generation: 1,
                },
                original.session.clone(),
            )
            .unwrap(),
        );
        let host = Arc::new(StoreRuntimeHost::new(
            kernel.clone(),
            "synthetic-project".into(),
            original.session.clone(),
            store_method_policy().unwrap(),
            dispatch.clone(),
        ));
        let executable = std::env::var_os("PM_STORE_HELPER_BIN")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::env::current_exe()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .join("pm-store-helper")
            });
        let runner = StoreCompatRunner::new(
            &executable,
            host.clone(),
            crate::STORE_BUNDLE,
            &init,
        )
        .unwrap();
        let result=runner.activate("var ExtCheckModule={async default(api){api.commands.register({id:'calculate',label:'Synthetic calculate',when:()=>false,run:(n)=>n+7});api.commands.register({id:'throws',label:'Synthetic throw',run:()=>{throw Error('synthetic callback error')}});globalThis.result=await api.commands.run('calculate',35);globalThis.fallback=(await api.commands.run('throws'))===undefined;}}").unwrap();
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(runner.json("result").unwrap(), 42);
        assert_eq!(runner.json("fallback").unwrap(), true);
        assert!(
            host.private_reports()
                .iter()
                .any(|report| report["method"] == "runtime-error")
        );
        assert!(
            kernel
                .lock()
                .unwrap()
                .state()
                .provider("commands", "synthetic-store.calculate")
                .is_some()
        );
        assert!(
            runner
                .eval("__storeCheck.api.commands.run('another-owner.command')")
                .is_err()
        );
        assert!(runner.eval("runtime.engine('store.rpc',{method:'register',args:['commands','bad',{id:'bad',label:'Bad',run:1,priority:0}]})").is_err());
        assert!(
            kernel
                .lock()
                .unwrap()
                .state()
                .provider("commands", "synthetic-store.bad")
                .is_none()
        );
        runner.dispose().unwrap();
        assert!(
            kernel
                .lock()
                .unwrap()
                .state()
                .provider("commands", "synthetic-store.calculate")
                .is_none()
        );
        assert!(dispatch.failures().is_empty());
    }
    #[test]
    fn frozen_store_facade_runs_in_confined_helper_and_private_storage_survives_helper_replacement()
    {
        let (_kernel, host, _dispatch, init) = fixture();
        let bundle = crate::STORE_BUNDLE;
        let executable = std::env::var_os("PM_STORE_HELPER_BIN")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::env::current_exe()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .join("pm-store-helper")
            });
        assert!(
            executable.is_file(),
            "build pm-runtime-v8's pm-store-helper before this suite"
        );
        let fixture = include_str!("../../../../../tools/extcheck/clock-fixture.mjs").replacen(
            "export function installClockFixture",
            "function installClockFixture",
            1,
        );
        let fixture = format!("(()=>{{{fixture}\ninstallClockFixture(globalThis,0x504d5032);}})()");
        let runner =
            StoreCompatRunner::with_prelude(&executable, host.clone(), &bundle, &init, &fixture)
                .unwrap();
        assert_eq!(
            runner.json("[Date.now(),performance.now()]").unwrap(),
            json!([1_600_000_060_000_u64, 60_000])
        );
        assert_eq!(
            runner
                .json("__extcheckClock.advance(16).then(()=>performance.now())")
                .unwrap(),
            60_016
        );
        let pid = runner.status().pid;
        let result = runner
            .activate(
                r#"
            var ExtCheckModule = {
                async default(api) {
                    await api.storage.set('synthetic-value', 37);
                    await api.storage.set('nullable', null);
                    let disposals = 0;
                    api.onDispose(() => api.storage.set('disposed', ++disposals));
                    const project = await api.project.get();
                    globalThis.observed = [typeof PM, project.name,
                        await api.storage.get('synthetic-value'),
                        await api.storage.get('nullable'),
                        await api.storage.get('missing') === undefined];
                    try { api.history.undo(); }
                    catch (error) { globalThis.refused = error.name; }
                }
            };
        "#,
            )
            .unwrap();
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(
            runner.json("[observed,refused]").unwrap(),
            json!([
                ["undefined", "Synthetic extension check", 37, null, true],
                "PermissionError"
            ])
        );
        let uuid = runner.json("crypto.randomUUID()").unwrap();
        assert_eq!(uuid.as_str().unwrap().len(), 36);
        assert!(
            runner
                .eval("runtime.stage({kind:'synthetic-forbidden'})")
                .is_err()
        );
        assert_eq!(runner.dispose().unwrap(), true);
        assert_eq!(runner.dispose().unwrap(), true);
        drop(runner);
        let replacement =
            StoreCompatRunner::new(&executable, host.clone(), &bundle, &init).unwrap();
        assert_ne!(replacement.status().pid, pid);
        assert_eq!(
            replacement
                .json("__storeCheck.api.storage.get('synthetic-value')")
                .unwrap(),
            37
        );
        assert!(!host.private_reports().is_empty());
        assert_eq!(
            replacement
                .json("__storeCheck.api.storage.get('disposed')")
                .unwrap(),
            1
        );
    }
}
