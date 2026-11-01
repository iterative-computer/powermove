//! Executes the actual compat bundle in the sandbox-enabled V8 runtime.
//! The executable owns no UI; missing kernel/panel adapters fail explicitly.
use pm_runtime_v8::*;
use serde_json::{Value, json};
use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
mod recorder;
pub use recorder::NativeHost;

pub struct CompatRunner {
    realm: Realm,
    _pool: RuntimePool,
    host: Arc<NativeHost>,
}

impl CompatRunner {
    pub fn new(bundle: &str, initial: &Value) -> Result<Self> {
        let host = Arc::new(NativeHost::default());
        let pool = RuntimePool::new(
            "extcheck",
            Config {
                limits: Limits::default(),
                watchdog: WatchdogPolicy::default(),
                kernel: host.clone(),
                host: host.clone(),
                watchdog_hooks: host.clone(),
                engine: Some(host.clone()),
                cache: CodeCache::new(4 * 1024 * 1024),
                admission: Admission::new(256 * 1024 * 1024),
            },
        )?;
        let realm = pool.realm(RealmKind::Compat)?;
        let runner = Self {
            realm,
            _pool: pool,
            host,
        };
        runner.eval(bundle)?;
        runner.eval("if(typeof PowermoveCompat?.CompatRealm!=='function')throw Error('Missing compat bundle')")?;
        runner.eval(include_str!("bootstrap.js"))?;
        runner.eval(&format!("__extcheck.create({initial})"))?;
        Ok(runner)
    }
    pub fn eval(&self, source: &str) -> Result<Evaluation> {
        let ticket = self.realm.eval(source)?;
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(result) = ticket.try_take() {
                return result;
            }
            if Instant::now() > deadline {
                self.realm.stop(ticket.execution);
                return Err(Error::Host("compat runner execution timeout".into()));
            }
            thread::sleep(Duration::from_millis(1));
        }
    }
    /// Sources are browser IIFEs. No source rewriting or user-id renaming occurs here.
    pub fn activate(&self, manifest: &Value, module: &str) -> Result<Value> {
        if !matches!(
            manifest
                .get("apiVersion")
                .and_then(Value::as_u64)
                .unwrap_or(1),
            1..=3
        ) {
            return Err(Error::Invalid("apiVersion must be 1, 2 or 3"));
        }
        // The reference deactivates before importing a replacement module.
        self.json("__extcheck.dispose()")?;
        self.eval(module)?;
        self.json(&format!("__extcheck.activate({manifest}, ExtCheckModule)"))
    }
    pub fn json(&self, expression: &str) -> Result<Value> {
        let result = self.eval(&format!(
            "Promise.resolve({expression}).then(value=>JSON.stringify(value))"
        ))?;
        serde_json::from_str(&result.value).map_err(|_| Error::Invalid("non-JSON runner result"))
    }
    pub fn diagnostics(&self) -> Value {
        self.host.diagnostics()
    }
    /// Ordered mutations for the rendering host. Admission alone is never a paint oracle.
    pub fn take_dom_batches(&self) -> Vec<Value> {
        self.host.take_dom_batches()
    }
    /// The embedder delivers applier feedback on this owner, outside AppKit/main.
    pub fn receive_dom(&self, message: &Value) -> Result<Value> {
        self.json(&format!(
            "(()=>{{__extcheck.dom.receive({message});__extcheck.dom.flush();return true}})()"
        ))
    }
}
impl Drop for CompatRunner {
    fn drop(&mut self) {
        self.realm.close();
    }
}

pub fn initial() -> Value {
    json!({"revision":0,"document":{"id":"synthetic","name":"Synthetic extension check","created":0,
        "w":320,"h":180,"fps":30,"dur":2,"layers":[],"comps":{}},
        "selection":{"layers":[],"keys":[],"chan":null},"workspace":{},
        "facts":{"historyBusy":false,"canUndo":false,"canRedo":false,"editLive":false}})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;
    fn bundle() -> &'static str {
        static BUNDLE: OnceLock<String> = OnceLock::new();
        BUNDLE.get_or_init(|| {
            let entry = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../js/compat/src/browser.ts");
            let build = std::process::Command::new("bun")
                .args([
                    "build",
                    entry.to_str().unwrap(),
                    "--target=browser",
                    "--format=iife",
                ])
                .output()
                .unwrap();
            assert!(
                build.status.success(),
                "actual production compat bundle must build"
            );
            String::from_utf8(build.stdout).unwrap()
        })
    }
    fn runner() -> CompatRunner {
        CompatRunner::new(bundle(), &initial()).unwrap()
    }
    fn manifest(version: u8) -> Value {
        json!({"id":"synthetic","name":"Synthetic","version":"1.0.0","apiVersion":version})
    }
    #[test]
    fn actual_bundle_executes_without_node_or_chromium() {
        let runner = runner();
        assert_eq!(runner.json("[typeof process,typeof require,typeof document,typeof PowermoveCompat.CompatRealm]").unwrap(),
            json!(["undefined","undefined","undefined","function"]));
    }
    #[test]
    fn every_legacy_version_activates_and_disposes_real_contributions() {
        let runner = runner();
        for version in 1..=3 {
            let result=runner.activate(&manifest(version), "var ExtCheckModule={default(api){api.commands.register({id:'synthetic-command',label:'Synthetic',run(){return 42}});api.onDispose(()=>globalThis.cleaned=(globalThis.cleaned||0)+1)}}").unwrap();
            assert_eq!(result["ok"], true);
            assert_eq!(
                runner
                    .json("__extcheck.realm.kernel.commands.get('synthetic-command').run()")
                    .unwrap(),
                42
            );
            runner.json("__extcheck.dispose()").unwrap();
            assert_eq!(
                runner
                    .json("__extcheck.realm.kernel.commands.list().length")
                    .unwrap(),
                0
            );
        }
        assert_eq!(runner.json("cleaned").unwrap(), 3);
    }
    #[test]
    fn same_realm_reload_preserves_provider_identity_and_storage() {
        let runner = runner();
        let source = "var ExtCheckModule={default(api){api.storage.set('n',(api.storage.get('n')||0)+1);api.services.register('synthetic-service',{value:17})}}";
        runner.activate(&manifest(3), source).unwrap();
        runner
            .eval("globalThis.held=__extcheck.realm.services.get('synthetic-service')")
            .unwrap();
        runner.activate(&manifest(3), source).unwrap();
        assert_eq!(runner.json("[held.value,held===__extcheck.realm.services.get('synthetic-service'),storage['ext.synthetic'].n]").unwrap(),json!([17,false,2]));
    }
    #[test]
    fn thrown_activation_cleans_registrations_and_reports_failure() {
        let runner = runner();
        let result=runner.activate(&manifest(3),"var ExtCheckModule={default(api){api.commands.register({id:'leak',label:'Synthetic',run(){}});throw Error('synthetic failure')}}").unwrap();
        assert_eq!(result["ok"], false);
        assert_eq!(
            runner
                .json("__extcheck.realm.kernel.commands.list().length")
                .unwrap(),
            0
        );
    }
    #[test]
    fn missing_kernel_and_panel_backend_never_report_success() {
        let runner = runner();
        let result = runner
            .activate(
                &manifest(3),
                "var ExtCheckModule={default(api){api.history.begin('Synthetic')}}",
            )
            .unwrap();
        assert_eq!(result["ok"], false);
        assert_eq!(runner.diagnostics()["missing"][0], "kernel.call");
    }
    #[test]
    fn recorder_pixels_are_native_and_source_versions_survive_disposal() {
        let runner = runner();
        let pixels=runner.json("(()=>{const a=__extcheck.realm.createCanvas(2,2),b=__extcheck.realm.createCanvas(2,2); const c=a.getContext('2d');c.fillStyle='#ff0000';c.fillRect(0,0,2,2); b.getContext('2d').drawImage(a,0,0);a.width=4;a.dispose();return b.getContext('2d').getImageData(0,0,1,1).data})()").unwrap();
        assert_eq!(pixels, json!([255, 0, 0, 255]));
    }
    #[test]
    fn invalid_bundle_is_refused() {
        assert!(CompatRunner::new("throw Error('synthetic')", &initial()).is_err());
    }
    #[test]
    fn remote_dom_batches_and_event_closures_execute_inside_real_v8() {
        let entry = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/dom-bundle.ts");
        let built = std::process::Command::new("bun")
            .args([
                "build",
                entry.to_str().unwrap(),
                "--target=browser",
                "--format=iife",
            ])
            .output()
            .unwrap();
        assert!(built.status.success());
        let runner =
            CompatRunner::new(std::str::from_utf8(&built.stdout).unwrap(), &initial()).unwrap();
        let id=runner.json("(()=>{const d=__extcheck.dom.document;const input=d.createElement('input');const out=d.createElement('output');d.body.append(input,out);input.addEventListener('input',()=>out.textContent=input.value);__extcheck.dom.flush();globalThis.syntheticOutput=out;return __extcheck.dom.id(input)})()").unwrap();
        let batches = runner.take_dom_batches();
        assert!(!batches.is_empty());
        let seq = batches.last().unwrap()["seq"].as_u64().unwrap();
        let session = json!({"panel":"extcheck","generation":1});
        runner
            .receive_dom(&json!({"kind":"applied","session":session,"seq":seq}))
            .unwrap();
        runner.receive_dom(&json!({"kind":"event","session":session,"seq":1,"applied":seq,"target":id,"event":"input",
            "bubbles":true,"cancelable":false,"default_prevented":false,"data":{"state":{"value":"Synthetic typed"}}})).unwrap();
        assert_eq!(
            runner.json("syntheticOutput.textContent").unwrap(),
            "Synthetic typed"
        );
    }
}
