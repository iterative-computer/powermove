//! The v1–3 extension runtime: discovery, native compilation, the V8 realm and
//! its production kernel channel. `CompatRunner` wraps the same realm around a
//! disposable project for tests and the comparison harness.
use pm_kernel::{KernelActor, KernelConfig, OpenSource};
use pm_runtime_v8::*;
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
pub mod adapter;
pub mod compile;
pub mod discovery;
pub mod host;
pub mod manifest;
pub mod net;
mod recorder;
mod store_dispatch;
pub mod store_host;
pub mod store_install;
mod store_kernel;
mod store_runtime;
pub use store_dispatch::KernelStoreDispatch;
pub mod transport;
pub use recorder::{CanvasText, NativeHost, NoEvents, RealmEvents};
pub use store_kernel::{KernelStoreAdapter, StoreArchive, StoreAuthority};
pub use store_runtime::{
    StoreCompatRunner, StoreDispatch, StoreRuntimeHost, StoreViews, store_method_policy,
};

pub const COMPAT_BUNDLE: &str = include_str!(concat!(env!("OUT_DIR"), "/compat.js"));
pub const STORE_BUNDLE: &str = include_str!(concat!(env!("OUT_DIR"), "/store.js"));
pub const PAGE_BUNDLE: &str = include_str!(concat!(env!("OUT_DIR"), "/page.js"));

/// The page script with the shipped app's theme variables for this appearance.
/// v1–3 panels style themselves with them (`--tx`, `--bg-field`, `--line`, …);
/// in Electron they came from the app's own stylesheet. The editions are
/// generated from pm-design's tokens.
pub fn page_bundle(dark: bool, increase_contrast: bool, reduce_transparency: bool) -> String {
    macro_rules! edition {
        ($name:literal) => {
            include_str!(concat!("../../../js/themes/editions/v1/", $name))
        };
    }
    let css = match (dark, increase_contrast, reduce_transparency) {
        (false, false, false) => edition!("light.css"),
        (false, false, true) => edition!("light-solid.css"),
        (false, true, false) => edition!("light-contrast.css"),
        (false, true, true) => edition!("light-contrast-solid.css"),
        (true, false, false) => edition!("dark.css"),
        (true, false, true) => edition!("dark-solid.css"),
        (true, true, false) => edition!("dark-contrast.css"),
        (true, true, true) => edition!("dark-contrast-solid.css"),
    };
    let scheme = if dark { "dark" } else { "light" };
    format!(
        "{PAGE_BUNDLE}\n(()=>{{const s=document.createElement('style');s.textContent={};document.head.prepend(s);document.documentElement.dataset.theme={};}})();",
        serde_json::to_string(css).expect("CSS is a string"),
        serde_json::to_string(scheme).expect("scheme is a string"),
    )
}

/// One realm holds every trusted extension and a live mirror of the project.
pub(crate) fn limits() -> Limits {
    Limits {
        heap_bytes: 1024 * 1024 * 1024,
        external_bytes: 512 * 1024 * 1024,
        max_source_bytes: 256 * 1024 * 1024,
        max_host_ops: usize::MAX / 2,
        max_op_bytes: 256 * 1024 * 1024,
        max_ops: 4096,
        max_response_bytes: 64 * 1024 * 1024,
        max_engine_response_bytes: 256 * 1024 * 1024,
        ..Limits::default()
    }
}
pub(crate) fn pool(scope: &str, host: Arc<NativeHost>) -> Result<RuntimePool> {
    RuntimePool::new(
        scope,
        Config {
            limits: limits(),
            watchdog: WatchdogPolicy::default(),
            kernel: host.clone(),
            host: host.clone(),
            watchdog_hooks: host.clone(),
            engine: Some(host),
            cache: CodeCache::new(32 * 1024 * 1024),
            admission: Admission::new(2 * 1024 * 1024 * 1024),
        },
    )
}
pub(crate) fn settled(realm: &Realm, source: &str, seconds: u64) -> Result<Evaluation> {
    let ticket = realm.eval(source)?;
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        if let Some(result) = ticket.try_take() {
            return result;
        }
        if Instant::now() > deadline {
            realm.stop(ticket.execution);
            return Err(Error::Host("compat runner execution timeout".into()));
        }
        thread::sleep(Duration::from_millis(1));
    }
}
pub(crate) fn boot_realm(realm: &Realm, host: &NativeHost, prelude: &str) -> Result<()> {
    if !prelude.is_empty() {
        settled(realm, prelude, 15)?;
    }
    settled(realm, include_str!("globals.js"), 15)?;
    settled(realm, COMPAT_BUNDLE, 60)?;
    settled(realm, include_str!("canvas-dom.js"), 15)?;
    settled(realm, include_str!("bootstrap.js"), 15)?;
    // A real project can be hundreds of megabytes: the realm parses it as JSON
    // through the engine channel, never as a source literal.
    host.stage()?;
    settled(realm, "__compat.create()", 60)?;
    Ok(())
}

pub const PROJECT: &str = "compat";
const COMPAT_REALM: u32 = 2;

pub struct CompatRunner {
    realm: Realm,
    _pool: RuntimePool,
    host: Arc<NativeHost>,
    actor: Option<KernelActor>,
}

impl CompatRunner {
    pub fn new(initial: &Value) -> Result<Self> {
        Self::with_prelude(initial, "")
    }
    /// Disposable comparison fixtures run before the bundle or any extension.
    /// Production embedders keep using `new`, without altered time or entropy.
    pub fn with_prelude(initial: &Value, prelude: &str) -> Result<Self> {
        let actor = KernelActor::spawn(KernelConfig::default());
        let kernel = actor.handle();
        let document = pm_model::json::parse(&initial["document"].to_string())
            .map_err(|_| Error::Invalid("initial document"))?;
        kernel
            .open(PROJECT, OpenSource::Untitled(document))
            .wait()
            .ok_or(Error::Closed)?
            .map_err(|e| Error::Host(e.to_string()))?;
        let host = Arc::new(NativeHost::new(
            adapter::KernelAdapter::new(
                kernel,
                PROJECT,
                PROJECT,
                pm_kernel::Writer {
                    realm: COMPAT_REALM,
                    generation: 1,
                },
            ),
            transport::Transport::default(),
            Box::new(NoEvents),
        ));
        let pool = pool("compat", host.clone())?;
        let realm = pool.realm(RealmKind::Compat)?;
        let runner = Self {
            realm,
            _pool: pool,
            host,
            actor: Some(actor),
        };
        boot_realm(&runner.realm, &runner.host, prelude)?;
        Ok(runner)
    }
    pub fn eval(&self, source: &str) -> Result<Evaluation> {
        settled(&self.realm, source, 15)
    }
    /// Sources are ES modules; the realm links them with no rewriting.
    pub fn activate(&self, manifest: &Value, module: &str) -> Result<Value> {
        let entry = manifest["entry"].as_str().unwrap_or("index.js");
        let sources = json!({
            entry: module,
            compile::BUNDLE: format!(
                "export {{ default }} from {e}; export * from {e};",
                e = json!(format!("./{entry}"))
            ),
        });
        let edges = json!({compile::BUNDLE: {format!("./{entry}"): entry}, entry: {}});
        self.install(manifest, &json!({"sources":sources,"edges":edges}))
    }
    /// A compiled extension directory, through the production compiler.
    pub fn compile(&self, directory: &Path, entry: &str) -> Result<compile::ModuleGraph> {
        let compiler = SvelteCompiler::new(&self._pool, 256 * 1024 * 1024, 8 * 1024 * 1024)?;
        compile::compile(directory, entry, 3, &compiler, None).map_err(Error::Host)
    }
    pub fn install(&self, manifest: &Value, graph: &Value) -> Result<Value> {
        let record = json!({"id":manifest["id"],"scope":"user","manifest":manifest,"enabled":true,
            "trust":"local","health":{"state":"ok"},"graph":graph});
        self.json(&format!("__compat.install({record})"))
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
    /// Called by the worker after flushing DOM and before queueing presentation.
    /// Immutable native resources are separate from the DOM JSON stream.
    pub fn canvas_frames(&self) -> Result<Vec<pm_compat_host::CanvasFrame>> {
        self.eval("__compat.flushCanvases?.()")?;
        self.host.canvas_frames()
    }
    /// The embedder delivers applier feedback on this owner, outside AppKit/main.
    pub fn receive_dom(&self, message: &Value) -> Result<Value> {
        self.json(&format!(
            "(()=>{{__compat.dom.receive({message});__compat.dom.flush();return true}})()"
        ))
    }
}
impl Drop for CompatRunner {
    fn drop(&mut self) {
        self.realm.close();
        while !self.realm.is_disposed() {
            thread::sleep(Duration::from_millis(1));
        }
        if let Some(actor) = self.actor.take() {
            actor.shutdown();
        }
    }
}

pub fn initial() -> Value {
    serde_json::from_str(include_str!("initial.json")).expect("frozen synthetic initial project")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn runner() -> CompatRunner {
        CompatRunner::new(&initial()).unwrap()
    }
    #[test]
    fn native_transport_seeks_snap_without_document_edits_and_preserve_raw_time() {
        let runner = runner();
        assert_eq!(
            runner
                .json("(()=>{PM.setTime(1.25);return [PM.time,PM.playing,PM.hist.canUndo()]})()")
                .unwrap(),
            json!([1.2666666666666666, false, false])
        );
        assert_eq!(
            runner
                .json("(()=>{PM.setTime(.123,{raw:true});PM.setTime(NaN);return PM.time})()")
                .unwrap(),
            json!(0.123)
        );
        assert_eq!(
            runner
                .json("(()=>{PM.step(-100);return PM.time})()")
                .unwrap(),
            json!(0)
        );
    }
    fn manifest(version: u8) -> Value {
        json!({"id":"synthetic","name":"Synthetic","version":"1.0.0","apiVersion":version})
    }
    #[test]
    fn actual_bundle_executes_without_node_or_chromium() {
        let runner = runner();
        assert_eq!(
            runner
                .json("[typeof process,typeof require,typeof PowermoveCompat.CompatRealm]")
                .unwrap(),
            json!(["undefined", "undefined", "function"])
        );
    }
    #[test]
    fn every_legacy_version_activates_and_disposes_real_contributions() {
        let runner = runner();
        for version in 1..=3 {
            let result=runner.activate(&manifest(version), "export default function(api){api.commands.register({id:'synthetic-command',label:'Synthetic',run(){return 42}});api.onDispose(()=>globalThis.cleaned=(globalThis.cleaned||0)+1)}").unwrap();
            assert_eq!(result["ok"], true);
            assert_eq!(
                runner
                    .json("__compat.realm.kernel.commands.get('synthetic-command').run()")
                    .unwrap(),
                42
            );
            runner
                .json("__compat.loader.dispose().then(()=>true)")
                .unwrap();
            assert_eq!(
                runner
                    .json("__compat.realm.kernel.commands.list().length")
                    .unwrap(),
                0
            );
        }
        assert_eq!(runner.json("cleaned").unwrap(), 3);
    }
    #[test]
    fn same_realm_reload_preserves_provider_identity_and_storage() {
        let runner = runner();
        let source = "export default function(api){api.storage.set('n',(api.storage.get('n')||0)+1);api.services.register('synthetic-service',{value:17})}";
        runner.activate(&manifest(3), source).unwrap();
        runner
            .eval("globalThis.held=__compat.realm.services.get('synthetic-service')")
            .unwrap();
        runner.activate(&manifest(3), source).unwrap();
        assert_eq!(runner.json("[held.value,held===__compat.realm.services.get('synthetic-service'),storage['ext.synthetic'].n]").unwrap(),json!([17,false,2]));
    }
    #[test]
    fn thrown_activation_cleans_registrations_and_reports_failure() {
        let runner = runner();
        let result=runner.activate(&manifest(3),"export default function(api){api.commands.register({id:'leak',label:'Synthetic',run(){}});throw Error('synthetic failure')}").unwrap();
        assert_eq!(result["ok"], false);
        assert_eq!(
            runner
                .json("__compat.realm.kernel.commands.list().length")
                .unwrap(),
            0
        );
    }
    #[test]
    fn attached_kernel_executes_history_in_the_native_authority() {
        let runner = runner();
        let result = runner
            .activate(
                &manifest(3),
                "export default function(api){api.history.begin('Synthetic')}",
            )
            .unwrap();
        assert_eq!(result["ok"], true);
        assert_eq!(runner.json("PM.hist.busy()").unwrap(), true);
        assert!(
            runner.diagnostics()["missing"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn native_kernel_history_round_trip_and_external_closures() {
        let runner = runner();
        let value=runner.json("(()=>{const before=PM.proj.name;const result=__compat.realm.history.do('Synthetic',()=>{PM.proj.name='Edited';return 7});const edited=PM.proj.name;const undo=PM.hist.undo();const restored=PM.proj.name;const redo=PM.hist.redo();return [result,edited,undo,restored===before,redo,PM.proj.name]})()").unwrap();
        assert_eq!(value, json!([7, "Edited", true, true, true, "Edited"]));
        assert_eq!(runner.json("(()=>{let count=0;__compat.realm.history.external('Synthetic closure',()=>count--,()=>count++);return [PM.hist.undo(),count,PM.hist.redo(),count]})()").unwrap(),json!([true,-1,true,0]));
    }
    #[test]
    fn native_kernel_publication_exposes_the_legacy_open_composition() {
        let runner = runner();
        assert_eq!(runner.json("PM.proj").unwrap(), initial()["document"]);
        let result=runner.json("(()=>{__compat.realm.history.do('Synthetic marker',()=>PM.proj.markers.push({id:'synthetic-marker',t:0,name:'Synthetic'}));const count=PM.proj.markers.length;PM.hist.undo();return [count,PM.proj.markers.length,PM.proj.w,PM.proj.compId]})()").unwrap();
        assert_eq!(result, json!([1, 0, 320, "synthetic-comp"]));
    }
    #[test]
    fn missing_kernel_method_is_acknowledged_without_poisoning_the_channel() {
        let runner = runner();
        assert_eq!(runner.json("(()=>{try{__compat.realm.channel.call('synthetic.unavailable')}catch{}return [__compat.realm.channel.pending,PM.hist.canUndo()]})()").unwrap(),json!([false,false]));
    }
    #[test]
    fn recorder_pixels_are_native_and_source_versions_survive_disposal() {
        let runner = runner();
        let pixels=runner.json("(()=>{const a=__compat.realm.createCanvas(2,2),b=__compat.realm.createCanvas(2,2); const c=a.getContext('2d');c.fillStyle='#ff0000';c.fillRect(0,0,2,2); b.getContext('2d').drawImage(a,0,0);a.width=4;a.dispose();return b.getContext('2d').getImageData(0,0,1,1).data})()").unwrap();
        assert_eq!(pixels, json!([255, 0, 0, 255]));
    }
    #[test]
    fn a_failing_prelude_refuses_the_realm() {
        assert!(CompatRunner::with_prelude(&initial(), "throw Error('synthetic')").is_err());
    }
    #[test]
    fn remote_dom_batches_and_event_closures_execute_inside_real_v8() {
        let runner = runner();
        let id=runner.json("(()=>{const d=__compat.dom.document;const input=d.createElement('input');const out=d.createElement('output');d.body.append(input,out);input.addEventListener('input',()=>out.textContent=input.value);__compat.dom.flush();globalThis.syntheticOutput=out;return __compat.dom.id(input)})()").unwrap();
        let batches = runner.take_dom_batches();
        assert!(!batches.is_empty());
        let seq = batches.last().unwrap()["seq"].as_u64().unwrap();
        let session = json!({"panel":"compat","generation":1});
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
    fn dom_runner() -> CompatRunner {
        runner()
    }
    fn decode(bytes: &[u8]) -> (png::OutputInfo, Vec<u8>) {
        let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        let mut reader = decoder.read_info().unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut pixels).unwrap();
        pixels.truncate(info.buffer_size());
        (info, pixels)
    }
    #[test]
    fn dom_canvas_uses_native_pixels_stable_context_and_immutable_png_resources() {
        let runner = dom_runner();
        assert_eq!(runner.json("(()=>{globalThis.canvas=document.createElement('canvas');canvas.width=2;canvas.height=2;document.body.append(canvas);globalThis.ctx=canvas.getContext('2d');ctx.fillStyle='#ff0000';ctx.fillRect(0,0,2,2);return [ctx===canvas.getContext('2d'),ctx.canvas===canvas,canvas.getContext('webgl')]})()").unwrap(),json!([true,true,null]));
        let frames = runner.canvas_frames().unwrap();
        assert_eq!(frames.len(), 1);
        let first = &frames[0];
        assert!(first.dom_sequence > 0);
        let (info, pixels) = decode(&first.png);
        assert_eq!((info.width, info.height), (2, 2));
        assert_eq!(pixels, [255, 0, 0, 255].repeat(4));
        runner
            .eval("ctx.fillStyle='#0000ff';ctx.fillRect(0,0,2,2)")
            .unwrap();
        let next = runner.canvas_frames().unwrap();
        assert!(next[0].version > first.version);
        assert_eq!(decode(&next[0].png).1, [0, 0, 255, 255].repeat(4));
        assert_eq!(decode(&first.png).1, pixels);
        runner.eval("canvas.width=2").unwrap();
        assert_eq!(
            decode(&runner.canvas_frames().unwrap()[0].png).1,
            vec![0; 16]
        );
        assert_eq!(runner.json("ctx===canvas.getContext('2d')").unwrap(), true);
        runner.eval("canvas.remove()").unwrap();
        assert!(runner.canvas_frames().unwrap().is_empty());
    }
    #[test]
    fn native_png_encoding_matches_pixel_readback_and_zero_size_contract() {
        use base64::Engine;
        let runner = runner();
        let value=runner.json("(()=>{const c=__compat.realm.createCanvas(3,2),x=c.getContext('2d');x.fillStyle='rgba(255,0,0,0.5)';x.fillRect(0,0,3,2);return [c.toDataURL(),x.getImageData(0,0,3,2).data]})()").unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(
                value[0]
                    .as_str()
                    .unwrap()
                    .strip_prefix("data:image/png;base64,")
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(json!(decode(&bytes).1), value[1]);
        assert_eq!(
            runner
                .json("__compat.realm.createCanvas(0,2).toDataURL()")
                .unwrap(),
            "data:,"
        );
    }
    #[test]
    fn native_path_transform_stroke_and_hit_test_bindings_reach_the_recorder() {
        let runner = runner();
        assert_eq!(runner.json("(()=>{const x=__compat.realm.createCanvas(16,16).getContext('2d');x.setTransform(1,0,0,1,2,3);x.transform(2,0,0,2,0,0);const m=x.getTransform();x.lineCap='round';x.lineJoin='bevel';x.miterLimit=4;x.setLineDash([2,3,4]);x.lineDashOffset=1;x.beginPath();x.moveTo(0,0);x.quadraticCurveTo(1,2,3,4);x.bezierCurveTo(1,2,3,4,5,6);x.arc(2,2,1,0,Math.PI);x.arcTo(2,2,3,3,1);x.ellipse(3,3,2,1,0,0,Math.PI);x.stroke();x.resetTransform();x.beginPath();x.rect(0,0,4,4);return [m.a,m.e,m.f,x.lineCap,x.lineJoin,x.miterLimit,x.lineDashOffset,x.getLineDash(),x.isPointInPath(2,2),x.isPointInPath(7,7)]})()").unwrap(),json!([2,2,3,"round","bevel",4,1,[2,3,4,2,3,4],true,false]));
        assert!(
            runner.diagnostics()["missing"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn canvas_boolean_direction_and_invalid_winding_preserve_authored_geometry() {
        let runner = runner();
        let value=runner.json("(()=>{const draw=ccw=>{const x=__compat.realm.createCanvas(8,8).getContext('2d');x.arc(4,4,3,0,Math.PI/2,ccw);x.lineTo(4,4);x.fill();return JSON.stringify(x.getImageData(0,0,8,8).data)};const x=__compat.realm.createCanvas(1,1).getContext('2d');x.rect(0,0,1,1);let refused=false;try{x.fill('invalid-rule')}catch{refused=true}return [draw(1)===draw(true),draw(1)!==draw(false),refused,x.getImageData(0,0,1,1).data]})()").unwrap();
        assert_eq!(value, json!([true, true, true, [0, 0, 0, 0]]));
    }
}
