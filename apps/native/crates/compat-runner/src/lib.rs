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
pub mod files;
pub mod host;
mod image;
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

/// The font families this Mac has, as `PM.Fonts.system` lists them (main's
/// `setSystemFamilies` list): CoreText's names without the hidden `.` ones.
#[cfg(target_os = "macos")]
pub(crate) fn font_families() -> Vec<String> {
    let names = unsafe { objc2_core_text::CTFontManagerCopyAvailableFontFamilyNames() };
    let mut out: Vec<String> = (0..names.len())
        .filter_map(|i| {
            let name = unsafe { names.value_at_index(i as isize) }
                as *const objc2_core_foundation::CFString;
            unsafe { name.as_ref() }.map(ToString::to_string)
        })
        .filter(|name| !name.starts_with('.'))
        .collect();
    out.sort();
    out.dedup();
    out
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn font_families() -> Vec<String> {
    vec![]
}
/// One realm holds every trusted extension and a live mirror of the project.
pub(crate) fn limits() -> Limits {
    Limits {
        heap_bytes: 1024 * 1024 * 1024,
        // The shipped bundle and a mirrored project start here; more comes
        // from the cage account as extensions and projects need it.
        initial_heap_bytes: 32 * 1024 * 1024,
        external_bytes: 512 * 1024 * 1024,
        max_source_bytes: 256 * 1024 * 1024,
        max_host_ops: usize::MAX / 2,
        max_op_bytes: 256 * 1024 * 1024,
        max_ops: 4096,
        max_response_bytes: 64 * 1024 * 1024,
        max_engine_response_bytes: 256 * 1024 * 1024,
        queue_fetches: true,
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
            admission: Admission::cage(),
        },
    )
}
pub(crate) fn settled(realm: &Realm, source: &str, seconds: u64) -> Result<Evaluation> {
    let mut ticket = realm.eval(source)?;
    let deadline = Instant::now() + Duration::from_secs(seconds);
    wait_until(&mut ticket, deadline).unwrap_or_else(|| {
        realm.stop(ticket.execution);
        Err(Error::Host("compat runner execution timeout".into()))
    })
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

/// One frame of the open composition, opaque over black as the shipped
/// `renderOpaqueFrame` draws it.
pub struct FrameRequest<'a> {
    pub time: f64,
    pub width: u32,
    pub height: u32,
    pub motion_blur: bool,
    /// True once the realm's task is stopped or past its host deadline.
    pub stop: &'a (dyn Fn() -> bool + Sync),
    /// The project the asking realm holds (an export realm's frozen one, a
    /// window realm's last received): the frame is of it, so the realm can
    /// answer its overrides while it waits (`None`: the kernel's project now).
    pub published: Option<Arc<pm_kernel::Published>>,
}
/// A layer's base raster (`render.raster`, today's `PM.raster` before any
/// override): the layer by id, or the object an override passed.
pub struct BaseRasterRequest {
    pub document: Arc<pm_model::Document>,
    pub composition: String,
    pub layer: String,
    pub content: Option<pm_model::Value>,
    pub seconds: f64,
    pub scale: f64,
    /// Today's preview crop window `[x, y, w, h]` in raster pixels.
    pub crop: Option<[u32; 4]>,
    /// Only this window `[x, y, w, h]` of the whole (uncropped) raster is
    /// wanted: the reply holds just its pixels ([`BaseRasterReply::window`]).
    pub window: Option<[u32; 4]>,
}
/// A base raster's straight RGBA8 canvas and today's geometry.
pub struct BaseRasterReply {
    pub width: u32,
    pub height: u32,
    pub straight_rgba: Vec<u8>,
    pub size: (f64, f64),
    pub anchor: (f64, f64),
    pub selection: Option<[f64; 4]>,
    pub font_offset: Option<(f64, f64)>,
    /// The part of the canvas the quad samples: a crop window's canvas is
    /// placed inside the whole raster (today's `uv`).
    pub uv: [f64; 4],
    /// For a window request: the part of the `width` × `height` canvas
    /// `straight_rgba` holds (`[x, y, w, h]`).
    pub window: Option<[u32; 4]>,
}
/// What an override answered for one layer at one frame.
#[derive(Debug)]
pub enum RasterAnswer {
    /// Passed through: the engine draws the layer itself.
    Native,
    /// The override returned nothing: the layer draws nothing, as on main.
    Hidden,
    /// The realm has not seen the revision asked for yet: ask again.
    Retry,
    /// The realm is past the exact revision asked for (`{revision, exact}`):
    /// it cannot answer for it.
    Moved,
    /// The override threw or returned what cannot be drawn: the frame fails.
    Failed(String),
    /// The realm could not answer this time (its canvas could not be read
    /// back, the canvas went with a restarted realm, the answer could not be
    /// delivered): not the override's doing, so it is asked again.
    Unavailable(String),
    /// The override's canvas: premultiplied RGBA8 rows (a canvas's backing
    /// store), the quad's size, anchor and `uv`. With `window`, the rows are
    /// that part (`[x, y, w, h]`) of the `width` × `height` canvas only.
    Raster {
        width: u32,
        height: u32,
        premultiplied_rgba: Vec<u8>,
        size: [f64; 2],
        anchor: [f64; 2],
        uv: [f64; 4],
        window: Option<[u32; 4]>,
    },
}
/// The app's side of raster overrides.
#[derive(Clone)]
pub struct RasterHooks {
    pub base: Arc<
        dyn Fn(BaseRasterRequest) -> std::result::Result<Option<BaseRasterReply>, String>
            + Send
            + Sync,
    >,
    /// The answers to one batch of needs, in the batch's order.
    pub answers: Arc<dyn Fn(u64, Vec<RasterAnswer>) + Send + Sync>,
    /// Whether a `raster` override is registered, the generation of
    /// wrappers answering (it changes whenever they do), and the identity of
    /// the compiled code they run ([`code_identity`]; 0 while unknown).
    pub producing: Arc<dyn Fn(bool, u64, u64) + Send + Sync>,
}

/// Identity of the compiled extension code in a realm's records (each
/// record's id, whether it runs, and its module graph's content hash): the
/// records a window realm boots with and an export realm pins. The same on
/// every launch while that code is unchanged; never 0.
pub fn code_identity(records: &Value) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    for record in records.as_array().into_iter().flatten() {
        record["id"].as_str().hash(&mut hash);
        (record["enabled"] != false).hash(&mut hash);
        record["graph"]["hash"].as_str().hash(&mut hash);
    }
    hash.finish().max(1)
}
/// The window's side of the app's effect clipboard (v1's inspector service):
/// `copy` the effects selected in Effect Controls (`{copied}`), `paste` them
/// onto the selected layers (`{pasted}`, or `{empty}` / `{reason}`), `clear`.
pub type EffectsHook = Arc<dyn Fn(&str) -> std::result::Result<Value, String> + Send + Sync>;
/// Where a window's realm finds its [`EffectsHook`], set once the window's
/// panels run (they start after its extensions).
pub type EffectsSlot = Arc<std::sync::Mutex<Option<EffectsHook>>>;
/// Renders a `FrameRequest` to PNG bytes.
pub type FrameRenderer =
    Arc<dyn Fn(&FrameRequest) -> std::result::Result<Vec<u8>, String> + Send + Sync>;

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
    pub fn compile(
        &self,
        directory: &Path,
        entry: &str,
        api_version: u64,
    ) -> Result<compile::ModuleGraph> {
        let compiler = SvelteCompiler::new(&self._pool, 256 * 1024 * 1024, 8 * 1024 * 1024)?;
        compile::compile(directory, entry, api_version, &compiler, None).map_err(Error::Host)
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
        let host = regex::Regex::new(
            r#"(?:^|[^.A-Za-z0-9_])require\(\s*["']|from\s*["'](?:node:|bun:)|\bprocess\.|\bBun\.|node_modules/happy-dom"#,
        )
        .unwrap();
        assert!(host.is_match("process.env.NODE_ENV"));
        assert!(!host.is_match("$ZodPreprocess.init(inst, def)"));
        for bundle in [COMPAT_BUNDLE, STORE_BUNDLE, PAGE_BUNDLE] {
            assert_eq!(host.find(bundle).map(|m| m.as_str()), None);
        }
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
    /// `roundRect` (the Typewriter cursor): corners are cut, the middle filled.
    #[test]
    fn round_rects_draw_with_their_radii() {
        let runner = runner();
        let pixels = runner
            .json(
                "(()=>{const c=__compat.realm.createCanvas(20,20).getContext('2d');
            c.fillStyle='#00ff00';c.beginPath();c.roundRect(0,0,20,20,[8]);c.fill();
            c.beginPath();c.roundRect(0,0,4,4,{x:1,y:1});c.roundRect(0,0,4,4);
            const p=c.getImageData(0,0,20,20).data;return [p[3],p[(10*20+10)*4+1]]})()",
            )
            .unwrap();
        assert_eq!(pixels, json!([0, 255]));
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
    fn images_decode_natively_draw_into_canvases_and_reach_the_page_as_bytes() {
        let runner = dom_runner();
        // A logo check as Commons Logos does it: a half-transparent PNG blob,
        // decoded, scaled into a canvas and read back.
        let checked = runner.json("(async()=>{const c=document.createElement('canvas');c.width=4;c.height=2;const x=c.getContext('2d');x.fillStyle='#ff0000';x.fillRect(0,0,2,2);
            const blob=await (await fetch(c.toDataURL())).blob();const url=URL.createObjectURL(blob);
            const image=new Image();let loaded=0;image.onload=()=>loaded++;image.src=url;await image.decode();
            const d=document.createElement('canvas');d.width=image.naturalWidth*2;d.height=image.naturalHeight*2;const y=d.getContext('2d',{willReadFrequently:true});
            y.drawImage(image,0,0,d.width,d.height);const p=y.getImageData(0,0,d.width,d.height).data;
            return [blob.type,blob.size>0,image.naturalWidth,image.naturalHeight,image.complete,loaded,p[3],p[p.length-1],[p[0],p[1],p[2]]]})()").unwrap();
        assert_eq!(
            checked,
            json!(["image/png", true, 4, 2, true, 1, 255, 0, [255, 0, 0]])
        );
        // SVG stays vector: drawn 8x it is rasterized at that size, not stretched.
        let svg = runner.json(r##"(async()=>{const image=new Image();image.src='data:image/svg+xml,'+encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="2" height="1"><rect width="1" height="1" fill="#00f"/></svg>');await image.decode();
            const c=document.createElement('canvas');c.width=16;c.height=8;const x=c.getContext('2d');x.drawImage(image,0,0,16,8);const p=x.getImageData(0,0,16,8).data;
            return [image.naturalWidth,image.naturalHeight,[p[2],p[3]],[p[7*4+2],p[7*4+3]],p[8*4+3]]})()"##).unwrap();
        assert_eq!(svg, json!([2, 1, [255, 255], [255, 255], 0]));
        // Broken bytes fire error and refuse to draw; createImageBitmap decodes a Blob.
        let broken = runner.json("(async()=>{const image=new Image();let error=0;image.onerror=()=>error++;image.src=URL.createObjectURL(new Blob(['nope'],{type:'image/png'}));
            const failed=await image.decode().then(()=>false,e=>e.name);const x=document.createElement('canvas').getContext('2d');
            let draw;try{x.drawImage(image,0,0)}catch(e){draw=e.name}
            const c=document.createElement('canvas');c.width=3;c.height=5;const bitmap=await createImageBitmap(await (await fetch(c.toDataURL())).blob());
            return [failed,error,draw,bitmap.width,bitmap.height]})()").unwrap();
        assert_eq!(
            broken,
            json!(["EncodingError", 1, "InvalidStateError", 3, 5])
        );
        // A connected <img> with a realm URL: the page never gets the blob: URL,
        // it gets the image's bytes as a resource.
        runner.json("(async()=>{const c=document.createElement('canvas');c.width=1;c.height=1;globalThis.shown=new Image();shown.src=URL.createObjectURL(await (await fetch(c.toDataURL())).blob());document.body.append(shown);await shown.decode();return 1})()").unwrap();
        let batches = serde_json::to_string(&runner.take_dom_batches()).unwrap();
        assert!(!batches.contains("blob:"), "{batches}");
        let frames = runner.canvas_frames().unwrap();
        let image = frames
            .iter()
            .find(|f| f.surface.starts_with("image-"))
            .unwrap();
        assert_eq!(
            (&*image.mime, image.width, image.height),
            ("image/png", 1, 1)
        );
        assert!(image.png.starts_with(b"\x89PNG"));
    }
    #[test]
    fn hostile_and_edge_images_are_refused_or_drawn_right() {
        let runner = dom_runner();
        // 2^31 × 2^31: no arithmetic abort; drawn through a capped raster.
        let huge = runner.json(r##"(async()=>{const i=new Image();i.src='data:image/svg+xml,'+encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="2147483648" height="2147483648"/>');await i.decode();const c=document.createElement('canvas').getContext('2d');c.drawImage(i,0,0,1,1);return [i.naturalWidth,i.naturalHeight]})()"##).unwrap();
        assert_eq!(huge, json!([2147483648u64, 2147483648u64]));
        // A raster capped at 8192 px still maps intrinsic coordinates: a crop far
        // to the right of a 10000-wide red SVG is red.
        let crop = runner.json(r##"(async()=>{const i=new Image();i.src='data:image/svg+xml,'+encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="10000" height="100"><rect width="10000" height="100" fill="#f00"/></svg>');await i.decode();const c=document.createElement('canvas');c.width=c.height=10;const x=c.getContext('2d');x.drawImage(i,9000,0,10,10,0,0,10,10);const p=x.getImageData(0,0,10,10).data;return [p[0],p[3],p[396],p[399]]})()"##).unwrap();
        assert_eq!(crop, json!([255, 255, 255, 255]));
        // createImageBitmap: crop and resize, a fresh canvas, ImageData.
        let bitmaps = runner.json("(async()=>{const c=document.createElement('canvas');c.width=4;c.height=2;const a=await createImageBitmap(c,0,0,1,1,{resizeWidth:8,resizeHeight:8});const b=await createImageBitmap(c);const d=await createImageBitmap({width:2,height:1,data:new Uint8ClampedArray(8)});return [a.width,a.height,b.width,b.height,d.width,d.height]})()").unwrap();
        assert_eq!(bitmaps, json!([8, 8, 4, 2, 2, 1]));
    }
    #[test]
    fn file_reader_data_urls_and_atob_behave_as_the_platform() {
        let runner = dom_runner();
        let reader = runner.json(r#"(async()=>{const r=new FileReader();const events=[];for(const t of ['loadstart','progress','load','loadend','abort'])r.addEventListener(t,()=>events.push(t));
            r.readAsText(new Blob(['old']));r.abort();const done=new Promise(ok=>r.onload=()=>ok(r.result));r.readAsText(new Blob(['new']));const first=await done;
            const big=new FileReader();const binary=await new Promise(ok=>{big.onloadend=()=>ok([big.readyState,big.result.length]);big.readAsBinaryString(new Blob([new Uint8Array(300000)]))});
            const latin=await new Promise(ok=>{const t=new FileReader();t.onload=()=>ok(t.result);t.readAsText(new Blob([new Uint8Array([0xe9])]),'iso-8859-1')});
            return [first,events,binary,latin]})()"#).unwrap();
        assert_eq!(
            reader,
            json!([
                "new",
                [
                    "abort",
                    "loadend",
                    "loadstart",
                    "progress",
                    "load",
                    "loadend"
                ],
                [2, 300000],
                "é"
            ])
        );
        let data = runner.json("(async()=>[...new Uint8Array(await (await fetch('data:application/octet-stream,%FF%00%80')).arrayBuffer())])()").unwrap();
        assert_eq!(data, json!([255, 0, 128]));
        assert_eq!(
            runner.json("atob('YQ'.repeat(200000)).length").unwrap(),
            json!(300000)
        );
    }
    type Log<T> = Arc<std::sync::Mutex<Vec<T>>>;
    type RasterRunner = (CompatRunner, Log<Value>, Log<Value>, Log<(bool, u64)>);
    #[test]
    fn code_identity_follows_compiled_code_not_health() {
        let records = |hash: &str, enabled: bool, health: &str| json!([{"id":"a","enabled":enabled,"health":{"state":health},"graph":{"hash":hash}}]);
        let base = code_identity(&records("h1", true, "ok"));
        assert_eq!(base, code_identity(&records("h1", true, "unresponsive")));
        assert_ne!(base, code_identity(&records("h2", true, "ok")));
        assert_ne!(base, code_identity(&records("h1", false, "ok")));
        assert_ne!(code_identity(&json!([])), 0);
    }
    /// A runner whose base rasters are 2×2 red, recording what was asked
    /// (layer, composition, whether a layer object was passed) and
    /// answered (as JSON), and the producing calls.
    fn raster_runner() -> RasterRunner {
        let runner = dom_runner();
        let painted = Arc::new(std::sync::Mutex::new(vec![]));
        let answered = Arc::new(std::sync::Mutex::new(vec![]));
        let producing = Arc::new(std::sync::Mutex::new(vec![]));
        let (p, a, o) = (painted.clone(), answered.clone(), producing.clone());
        runner.host.set_rasters(Some(crate::RasterHooks {
            base: Arc::new(move |request: crate::BaseRasterRequest| {
                p.lock().unwrap().push(json!([
                    request.layer,
                    request.composition,
                    request.content.is_some()
                ]));
                // A window of the whole 2 × 2 raster holds just its pixels.
                let window = request.window.map(|[x, y, w, h]| {
                    let (x, y) = (x.min(2), y.min(2));
                    [x, y, w.min(2 - x), h.min(2 - y)]
                });
                let held = window.map_or(4, |[_, _, w, h]| (w * h) as usize);
                Ok(Some(crate::BaseRasterReply {
                    width: 2,
                    height: 2,
                    window,
                    straight_rgba: [255, 0, 0, 255].repeat(held),
                    size: (2., 2.),
                    anchor: (1., 1.),
                    selection: None,
                    font_offset: Some((0.5, 0.)),
                    // A crop window's canvas inside the whole raster.
                    uv: if request.crop.is_some() {
                        [-0.5, 0., 2., 1.]
                    } else {
                        [0., 0., 1., 1.]
                    },
                }))
            }),
            answers: Arc::new(move |batch, answers| {
                for answer in answers {
                    a.lock().unwrap().push(match answer {
                        crate::RasterAnswer::Native => json!([batch, "native"]),
                        crate::RasterAnswer::Hidden => json!([batch, "hidden"]),
                        crate::RasterAnswer::Retry => json!([batch, "retry"]),
                        crate::RasterAnswer::Moved => json!([batch, "moved"]),
                        crate::RasterAnswer::Failed(error) => json!([batch, "failed", error]),
                        crate::RasterAnswer::Unavailable(error) => {
                            json!([batch, "unavailable", error])
                        }
                        crate::RasterAnswer::Raster {
                            width,
                            height,
                            premultiplied_rgba,
                            size,
                            anchor,
                            uv,
                            window: None,
                        } => json!([
                            batch,
                            width,
                            height,
                            &premultiplied_rgba[..4],
                            &premultiplied_rgba[premultiplied_rgba.len() - 4..],
                            size,
                            anchor,
                            uv
                        ]),
                        crate::RasterAnswer::Raster {
                            width,
                            height,
                            premultiplied_rgba,
                            size,
                            anchor,
                            uv,
                            window: Some(window),
                        } => json!([
                            batch,
                            "window",
                            width,
                            height,
                            window,
                            premultiplied_rgba.len(),
                            size,
                            anchor,
                            uv
                        ]),
                    });
                }
            }),
            producing: Arc::new(move |on, epoch, _| o.lock().unwrap().push((on, epoch))),
        }));
        (runner, painted, answered, producing)
    }
    #[test]
    fn raster_overrides_answer_with_pixels_and_pass_layers_through_unpainted() {
        let (runner, painted, answered, producing) = raster_runner();
        let manifest = json!({"id":"widen","name":"Widen","version":"1.0.0","apiVersion":3,"entry":"index.js"});
        runner
            .activate(
                &manifest,
                "export default function activate(api){const original=api.services.get('raster');
                 const r=api.services.register('raster',(layer,scale,time,up,crop)=>{if(layer.id!=='t1')return original(layer,scale,time,up,crop);
                   const base=original(layer,scale,time,up,crop);const c=document.createElement('canvas');c.width=base.cv.width+2;c.height=base.cv.height;
                   const x=c.getContext('2d');x.drawImage(base.cv,0,0);x.fillStyle='#0000ff';x.fillRect(base.cv.width,0,2,base.cv.height);
                   return {...base,cv:c,w:base.w+2,fontOffset:base.fontOffset};});globalThis.dropRaster=()=>r.dispose();}",
            )
            .unwrap();
        assert_eq!(*producing.lock().unwrap(), vec![(true, 2)]);
        runner
            .eval("__compat.realm.PM.proj.layers.push({id:'t1',type:'text',name:'T1',d:{text:'a'}},{id:'t2',type:'text',name:'T2',d:{text:'b'}})")
            .unwrap();
        runner
            .eval("__compat.answerRasters(7,[{composition:'synthetic-comp',layer:'t1',seconds:0,scale:1},{composition:'synthetic-comp',layer:'t2',seconds:0,scale:1}])")
            .unwrap();
        assert_eq!(
            *answered.lock().unwrap(),
            vec![
                json!([
                    7,
                    4,
                    2,
                    [255, 0, 0, 255],
                    [0, 0, 255, 255],
                    [4., 2.],
                    [1., 1.],
                    [0., 0., 1., 1.]
                ]),
                json!([7, "native"])
            ]
        );
        // The pass-through layer was never painted; t1 painted in its composition.
        assert_eq!(
            *painted.lock().unwrap(),
            vec![json!(["t1", "synthetic-comp", false])]
        );
        runner.eval("dropRaster()").unwrap();
        // An extension that replaces PM.raster itself (variable-font-anchor) counts
        // too; each change is a new generation of wrappers.
        runner
            .eval(
                "const PM=__compat.realm.PM;globalThis.kept=PM.raster;PM.raster=(...a)=>kept(...a)",
            )
            .unwrap();
        runner.eval("__compat.realm.PM.raster=kept").unwrap();
        assert_eq!(
            *producing.lock().unwrap(),
            vec![(true, 2), (false, 3), (true, 4), (false, 5)]
        );
    }
    #[test]
    fn a_base_raster_counts_as_the_layer_only_while_untouched() {
        let (runner, painted, answered, _) = raster_runner();
        let manifest = json!({"id":"cases","name":"Cases","version":"1.0.0","apiVersion":3,"entry":"index.js"});
        // Each layer id is one way an override can use the base.
        runner
            .activate(
                &manifest,
                "export default function activate(api){const base=api.services.get('raster');
                 api.services.register('raster',(L,s,T,up,crop)=>{switch(L.id){
                   case 'same': return base(L,s,T,up,crop);
                   case 'replaced': return base({...L,d:{...L.d,text:'replacement'}},s,T,up,crop);
                   case 'later': return base(L,s,T+1,up,crop);
                   case 'painted': {const r=base(L,s,T,up,crop);const x=r.cv.getContext('2d');x.fillStyle='#00ff00';x.fillRect(0,0,1,1);return r}
                   case 'moved': {const r=base(L,s,T,up,crop);r.anchorX+=0.5;return r}
                   case 'spread': return {...base(L,s,T,up,crop)};
                   case 'gone': return null;
                   case 'throws': throw new Error('broken wrapper');
                   case 'odd': return {cv:{},w:1,h:1,anchorX:0,anchorY:0};
                   default: return base(L,s,T,up,crop);}});}",
            )
            .unwrap();
        runner
            .eval("for(const id of ['same','replaced','later','painted','moved','spread','gone','throws','odd'])__compat.realm.PM.proj.layers.push({id,type:'text',name:id,d:{text:id}});
                   __compat.realm.PM.proj.comps.inner={id:'inner',name:'Inner',w:100,h:100,fps:30,dur:1,layers:[{id:'nested',type:'text',name:'n',d:{text:'n'}}]}")
            .unwrap();
        runner
            .eval("const ask=(layer,crop)=>({composition:'synthetic-comp',layer,seconds:0,scale:1,crop});
                   __compat.answerRasters(1,['same','replaced','later','painted','moved','gone','throws','odd'].map(id=>ask(id)));
                   __compat.answerRasters(2,[ask('spread',{x:1,y:0,width:2,height:2})]);
                   __compat.answerRasters(3,[{composition:'inner',layer:'nested',seconds:0,scale:1}]);")
            .unwrap();
        let answered = answered.lock().unwrap().clone();
        let kinds: Vec<Value> = answered
            .iter()
            .map(|a| match a[1].as_str() {
                Some(kind) => json!(kind),
                None => json!([a[3], a[6], a[7]]),
            })
            .collect();
        let red = json!([255, 0, 0, 255]);
        assert_eq!(
            kinds,
            vec![
                json!("native"),
                json!([red, [1., 1.], [0., 0., 1., 1.]]),
                json!([red, [1., 1.], [0., 0., 1., 1.]]),
                json!([[0, 255, 0, 255], [1., 1.], [0., 0., 1., 1.]]),
                json!([red, [1.5, 1.], [0., 0., 1., 1.]]),
                json!("hidden"),
                json!("failed"),
                json!("failed"),
                // A cropped base keeps its place inside the whole raster.
                json!([red, [1., 1.], [-0.5, 0., 2., 1.]]),
                json!("native"),
            ]
        );
        assert_eq!(answered[6][2], "broken wrapper");
        // The replacement painted as given; the nested layer in its composition.
        let painted = painted.lock().unwrap().clone();
        assert!(painted.contains(&json!(["replaced", "synthetic-comp", true])));
        assert!(!painted.iter().any(|p| p[0] == "same" || p[0] == "nested"));
    }
    /// A batch indexes each composition's layers once; it finds what a scan
    /// of the composition's layers, then the project's, finds: the first
    /// layer with the id, and a layer an override adds mid-batch.
    #[test]
    fn an_answer_batch_finds_each_layer_as_a_scan_would() {
        let (runner, _, answered, _) = raster_runner();
        let manifest =
            json!({"id":"pick","name":"Pick","version":"1.0.0","apiVersion":3,"entry":"index.js"});
        runner
            .activate(
                &manifest,
                "export default function activate(api){const base=api.services.get('raster');
                 api.services.register('raster',(L,s,T,up,crop)=>{
                   if(L.id==='adder')globalThis.__compat.realm.PM.proj.layers.push({id:'late',type:'text',name:'late',d:{text:'first'}});
                   return L.d.text==='first'?null:base(L,s,T,up,crop);});}",
            )
            .unwrap();
        runner
            .eval("const P=__compat.realm.PM.proj;
                   P.layers.push({id:'dup',type:'text',name:'top',d:{text:'top'}},{id:'adder',type:'text',name:'a',d:{text:'a'}},{id:'dup',type:'text',name:'second',d:{text:'first'}});
                   P.comps.inner={id:'inner',name:'Inner',w:100,h:100,fps:30,dur:1,layers:[{id:'dup',type:'text',name:'n',d:{text:'first'}}]}")
            .unwrap();
        runner
            .eval("const ask=(composition,layer)=>({composition,layer,seconds:0,scale:1});
                   __compat.answerRasters(1,[ask('inner','dup'),ask('synthetic-comp','dup'),ask('synthetic-comp','late'),ask('synthetic-comp','adder'),ask('synthetic-comp','late'),ask('inner','late')]);")
            .unwrap();
        let kinds: Vec<Value> = answered
            .lock()
            .unwrap()
            .iter()
            .map(|a| a[1].clone())
            .collect();
        assert_eq!(
            kinds,
            vec![
                json!("hidden"),
                json!("native"),
                // Not in the project yet: passed through, as a scan finds nothing.
                json!("native"),
                json!("native"),
                json!("hidden"),
                json!("hidden"),
            ]
        );
    }
    /// An untouched base raster returned without the frame's crop window is
    /// answered with only that window painted, at the whole raster's size; a
    /// base the override read (geometry or canvas) or forwarded keeps today's
    /// answer.
    #[test]
    fn an_uncropped_base_raster_is_answered_with_just_the_crop_window() {
        let (runner, painted, answered, _) = raster_runner();
        let manifest =
            json!({"id":"drop","name":"Drop","version":"1.0.0","apiVersion":3,"entry":"index.js"});
        runner
            .activate(
                &manifest,
                "export default function activate(api){const base=api.services.get('raster');
                 api.services.register('raster',(L,s,T,up,crop)=>{switch(L.id){
                   case 'dropped': return base(L,s,T);
                   case 'measured': {const r=base(L,s,T);void r.w;return r}
                   case 'read': {const r=base(L,s,T);void r.cv;return r}
                   default: return base(L,s,T,up,crop);}});}",
            )
            .unwrap();
        runner
            .eval("for(const id of ['dropped','measured','read','forwarded'])__compat.realm.PM.proj.layers.push({id,type:'shape',name:id,d:{}})")
            .unwrap();
        runner
            .eval("const ask=(layer,crop)=>({composition:'synthetic-comp',layer,seconds:0,scale:1,crop});
                   __compat.answerRasters(1,[ask('dropped',{x:1,y:0,width:5,height:1}),ask('measured',{x:1,y:0,width:1,height:1}),
                     ask('read',{x:1,y:0,width:1,height:1}),ask('forwarded',{x:1,y:0,width:1,height:1}),ask('dropped')]);")
            .unwrap();
        let answered = answered.lock().unwrap().clone();
        // The window is clamped to the 2 × 2 raster: one texel, 4 bytes.
        assert_eq!(
            answered[0],
            json!([
                1,
                "window",
                2,
                2,
                [1, 0, 1, 1],
                4,
                [2., 2.],
                [1., 1.],
                [0., 0., 1., 1.]
            ])
        );
        assert_eq!(answered[1][1], 2, "a measured base is the whole canvas");
        assert_eq!(answered[2][1], 2, "a read canvas is the whole canvas");
        assert_eq!(answered[3], json!([1, "native"]));
        // Without a crop window the base is the layer itself.
        assert_eq!(answered[4], json!([1, "native"]));
        assert_eq!(
            painted.lock().unwrap().len(),
            3,
            "passed-through bases paint nothing"
        );
    }
    /// A runner whose 2 × 2 base rasters record the revision of the document
    /// they were painted from.
    fn revision_runner() -> (CompatRunner, Log<Value>, Log<Value>) {
        let (runner, _, _, _) = raster_runner();
        let painted = Arc::new(std::sync::Mutex::new(vec![]));
        let answered = Arc::new(std::sync::Mutex::new(vec![]));
        let (p, a) = (painted.clone(), answered.clone());
        runner.host.set_rasters(Some(crate::RasterHooks {
            base: Arc::new(move |request: crate::BaseRasterRequest| {
                p.lock().unwrap().push(json!([
                    request.layer,
                    request.document.revision(),
                    request.window.is_some()
                ]));
                let held = request
                    .window
                    .map_or(4, |[_, _, w, h]| (w.min(2) * h.min(2)) as usize);
                Ok(Some(crate::BaseRasterReply {
                    width: 2,
                    height: 2,
                    window: request
                        .window
                        .map(|[x, y, w, h]| [x.min(2), y.min(2), w.min(2), h.min(2)]),
                    straight_rgba: [255, 0, 0, 255].repeat(held),
                    size: (2., 2.),
                    anchor: (1., 1.),
                    selection: None,
                    font_offset: None,
                    uv: [0., 0., 1., 1.],
                }))
            }),
            answers: Arc::new(move |_, answers| {
                for answer in answers {
                    a.lock().unwrap().push(match answer {
                        crate::RasterAnswer::Raster {
                            window: Some(_), ..
                        } => json!("window"),
                        crate::RasterAnswer::Raster {
                            premultiplied_rgba, ..
                        } => json!(premultiplied_rgba[..4]),
                        crate::RasterAnswer::Native => json!("native"),
                        other => json!(format!("{other:?}")),
                    });
                }
            }),
            producing: Arc::new(|_, _, _| {}),
        }));
        (runner, painted, answered)
    }
    /// A base the frame's need gets only a window of is decided once the
    /// whole batch ran: a later override drawing into its canvas makes it an
    /// override's canvas, read back with that drawing (review of 3065d9378).
    #[test]
    fn a_base_another_override_draws_into_later_in_the_batch_is_read_whole() {
        let (runner, _, answered) = revision_runner();
        let manifest =
            json!({"id":"keep","name":"Keep","version":"1.0.0","apiVersion":3,"entry":"index.js"});
        runner
            .activate(
                &manifest,
                "export default function activate(api){const base=api.services.get('raster');
                 api.services.register('raster',(L,s,T,up,crop)=>{
                   if(L.id==='a'){globalThis.saved=base(L,s,T);return globalThis.saved}
                   const x=globalThis.saved.cv.getContext('2d');x.fillStyle='#00ff00';x.fillRect(0,0,1,1);
                   return base(L,s,T,up,crop);});}",
            )
            .unwrap();
        runner
            .eval("for(const id of ['a','b'])__compat.realm.PM.proj.layers.push({id,type:'shape',name:id,d:{}})")
            .unwrap();
        runner
            .eval("const crop={x:0,y:0,width:1,height:1};
                   __compat.answerRasters(1,[{composition:'synthetic-comp',layer:'a',seconds:0,scale:1,crop},
                     {composition:'synthetic-comp',layer:'b',seconds:0,scale:1,crop}]);")
            .unwrap();
        assert_eq!(
            *answered.lock().unwrap(),
            vec![json!([0, 255, 0, 255]), json!("native")]
        );
    }
    /// A windowed base an extension keeps paints, when it reads it later, the
    /// frame it was returned for, from that revision's document, whatever the
    /// project is now (review of 3065d9378).
    #[test]
    fn a_kept_windowed_base_reads_the_revision_it_was_returned_for() {
        let (runner, painted, answered) = revision_runner();
        let manifest =
            json!({"id":"hold","name":"Hold","version":"1.0.0","apiVersion":3,"entry":"index.js"});
        runner
            .activate(
                &manifest,
                "export default function activate(api){const base=api.services.get('raster');
                 api.services.register('raster',(L,s,T)=>{globalThis.saved=base(L,s,T);return globalThis.saved});}",
            )
            .unwrap();
        runner
            .eval("__compat.realm.PM.proj.layers.push({id:'a',type:'shape',name:'a',d:{}})")
            .unwrap();
        let before = runner
            .json("(__compat.realm.flush(),__compat.realm.revision)")
            .unwrap();
        runner
            .eval("__compat.answerRasters(1,[{composition:'synthetic-comp',layer:'a',seconds:0,scale:1,crop:{x:0,y:0,width:1,height:1}}]);")
            .unwrap();
        assert_eq!(*answered.lock().unwrap(), vec![json!("window")]);
        runner
            .eval("__compat.realm.PM.proj.layers.push({id:'later',type:'shape',name:'later',d:{}})")
            .unwrap();
        let after = runner
            .json("(__compat.realm.flush(),__compat.realm.revision)")
            .unwrap();
        assert_ne!(before, after, "the project moved on");
        assert_eq!(runner.json("globalThis.saved.cv.width").unwrap(), json!(2));
        let painted = painted.lock().unwrap().clone();
        assert_eq!(painted.len(), 2, "{painted:?}");
        assert_eq!(painted[1], json!(["a", before, false]), "{painted:?}");
    }
    /// A base an override keeps returning keeps the document it was first
    /// answered from: answering it again after an edit doesn't move it to the
    /// project as it is now (review of 3bcf93827).
    #[test]
    fn a_base_answered_again_after_an_edit_keeps_its_first_document() {
        let (runner, painted, answered) = revision_runner();
        let manifest =
            json!({"id":"same","name":"Same","version":"1.0.0","apiVersion":3,"entry":"index.js"});
        runner
            .activate(
                &manifest,
                "export default function activate(api){const base=api.services.get('raster');
                 api.services.register('raster',(L,s,T)=>globalThis.saved??=base(L,s,T));}",
            )
            .unwrap();
        runner
            .eval("__compat.realm.PM.proj.layers.push({id:'a',type:'shape',name:'a',d:{}})")
            .unwrap();
        let first = runner
            .json("(__compat.realm.flush(),__compat.realm.revision)")
            .unwrap();
        let batch = "__compat.answerRasters(1,[{composition:'synthetic-comp',layer:'a',seconds:0,scale:1,crop:{x:0,y:0,width:1,height:1}}]);";
        runner.eval(batch).unwrap();
        runner
            .eval("__compat.realm.PM.proj.layers.push({id:'later',type:'shape',name:'later',d:{}})")
            .unwrap();
        let second = runner
            .json("(__compat.realm.flush(),__compat.realm.revision)")
            .unwrap();
        assert_ne!(first, second);
        runner.eval(batch).unwrap();
        assert_eq!(
            *answered.lock().unwrap(),
            vec![json!("window"), json!("window")]
        );
        runner.json("globalThis.saved.cv.width").unwrap();
        let painted = painted.lock().unwrap().clone();
        assert_eq!(
            painted,
            vec![
                json!(["a", first, true]),
                json!(["a", first, true]),
                json!(["a", first, false])
            ]
        );
    }
    /// What a batch's flush emits may draw into a candidate base: it is
    /// checked after the flush, and read whole (review of 3bcf93827).
    #[test]
    fn a_base_drawn_into_while_the_batch_flushes_is_read_whole() {
        let (runner, _, answered) = revision_runner();
        let manifest = json!({"id":"flush","name":"Flush","version":"1.0.0","apiVersion":3,"entry":"index.js"});
        runner
            .activate(
                &manifest,
                "export default function activate(api){const base=api.services.get('raster');
                 api.services.register('raster',(L,s,T)=>globalThis.saved=base(L,s,T));}",
            )
            .unwrap();
        runner
            .eval("__compat.realm.PM.proj.layers.push({id:'a',type:'shape',name:'a',d:{}});
                   const realm=__compat.realm,flush=realm.flush.bind(realm);
                   realm.flush=()=>{flush();const b=globalThis.saved;if(b&&!globalThis.drawn){globalThis.drawn=true;const x=b.cv.getContext('2d');x.fillStyle='#00ff00';x.fillRect(0,0,1,1)}};")
            .unwrap();
        runner
            .eval("__compat.answerRasters(1,[{composition:'synthetic-comp',layer:'a',seconds:0,scale:1,crop:{x:0,y:0,width:1,height:1}}]);")
            .unwrap();
        assert_eq!(*answered.lock().unwrap(), vec![json!([0, 255, 0, 255])]);
    }
    /// Kept documents are capped (past the cap a base is read whole), and a
    /// collected base lets its document go (review of 3bcf93827).
    #[test]
    fn kept_documents_are_capped_and_go_with_their_bases() {
        let (runner, _, answered) = revision_runner();
        let manifest =
            json!({"id":"many","name":"Many","version":"1.0.0","apiVersion":3,"entry":"index.js"});
        runner
            .activate(
                &manifest,
                "export default function activate(api){const base=api.services.get('raster');globalThis.kept=[];
                 api.services.register('raster',(L,s,T)=>{const b=base(L,s,T);kept.push(b);return b});}",
            )
            .unwrap();
        let round = |i: usize| {
            runner
                .eval(&format!("__compat.realm.PM.proj.layers.push({{id:'l{i}',type:'shape',name:'l',d:{{}}}});__compat.realm.flush();
                    __compat.answerRasters({i},[{{composition:'synthetic-comp',layer:'l{i}',seconds:0,scale:1,crop:{{x:0,y:0,width:1,height:1}}}}]);"))
                .unwrap();
            answered.lock().unwrap().last().cloned().unwrap()
        };
        for i in 0..crate::recorder::MAX_PINNED_REVISIONS {
            assert_eq!(round(i), json!("window"), "revision {i}");
        }
        // Every kept base holds its own revision: the next one is read whole.
        assert_eq!(round(100), json!([255, 0, 0, 255]));
        // Once the realm collects them, their documents go.
        runner.eval("globalThis.kept=[]").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            runner
                .eval("(()=>{let n=0;for(let i=0;i<64;i++)n+=new Array(65536).fill(i).length;return n})()")
                .unwrap();
            let next = 200 + answered.lock().unwrap().len();
            if round(next) == json!("window") {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the kept documents never went"
            );
            runner.eval("globalThis.kept=[]").unwrap();
        }
    }
    /// The canvas round trip of a window is the window of the whole canvas's.
    #[test]
    fn a_window_s_canvas_round_trip_is_the_whole_canvas_s_there() {
        let (w, h) = (37u32, 11u32);
        let straight: Vec<u8> = (0..w * h * 4)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
            .collect();
        let whole = crate::recorder::canvas_round_trip(w, h, straight.clone()).unwrap();
        let [x, y, ww, wh] = [5u32, 3, 20, 7];
        let window: Vec<u8> = (y..y + wh)
            .flat_map(|r| {
                straight[((r * w + x) * 4) as usize..((r * w + x + ww) * 4) as usize].to_vec()
            })
            .collect();
        let part = crate::recorder::canvas_round_trip(ww, wh, window).unwrap();
        let expected: Vec<u8> = (y..y + wh)
            .flat_map(|r| {
                whole[((r * w + x) * 4) as usize..((r * w + x + ww) * 4) as usize].to_vec()
            })
            .collect();
        assert_eq!(part, expected);
    }
    /// An override's own throw is deterministic; an answer the realm could
    /// not read back or deliver is not, and is reported as such so the viewer
    /// asks again instead of leaving the layer out until the next edit.
    #[test]
    fn only_an_overrides_throw_is_a_failure_the_rest_is_unavailable() {
        let (runner, _, answered, _) = raster_runner();
        runner
            .eval(
                "runtime.engine('compat.answers',{batch:9,answers:[
                   {error:'broken wrapper',threw:true},
                   {error:'The answers could not be delivered'},
                   {surface:'a-canvas-of-a-restarted-realm',w:1,h:1}]})",
            )
            .unwrap();
        assert_eq!(
            *answered.lock().unwrap(),
            vec![
                json!([9, "failed", "broken wrapper"]),
                json!([9, "unavailable", "The answers could not be delivered"]),
                json!([9, "unavailable", "The override's canvas is gone"]),
            ]
        );
    }
    #[test]
    fn a_capture_asks_its_waiting_realm_for_overrides_inline() {
        let (runner, _, answered, _) = raster_runner();
        let manifest =
            json!({"id":"blue","name":"Blue","version":"1.0.0","apiVersion":3,"entry":"index.js"});
        runner
            .activate(
                &manifest,
                "export default function activate(api){api.services.register('raster',(L)=>{const c=document.createElement('canvas');c.width=1;c.height=1;
                 const x=c.getContext('2d');x.fillStyle='#0000ff';x.fillRect(0,0,1,1);return {cv:c,w:1,h:1,anchorX:0,anchorY:0}});}",
            )
            .unwrap();
        runner
            .eval(
                "__compat.realm.PM.proj.layers.push({id:'t1',type:'text',name:'T1',d:{text:'a'}})",
            )
            .unwrap();
        // The capture's renderer (an export child, in the app) needs t1 drawn
        // by this realm, which is waiting for the capture.
        let host = runner.host.clone();
        let seen = answered.clone();
        runner.host.set_frames(Some(Arc::new(move |frame: &crate::FrameRequest| {
            let ask = json!({"needs":[{"composition":"synthetic-comp","layer":"t1","seconds":0,"scale":1}],"ask":{}});
            host.nested_ask(41, ask).map_err(|_| "the realm is not waiting".to_owned())?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while seen.lock().unwrap().is_empty() {
                if std::time::Instant::now() > deadline || (frame.stop)() {
                    return Err("no answer".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let mut png = vec![];
            let mut encoder = png::Encoder::new(&mut png, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[0, 0, 255, 255]).unwrap();
            drop(writer);
            Ok(png)
        })));
        let frame = runner
            .json("(()=>{const c=PM.renderFrameTo(0,1,1);return [...c.getContext('2d').getImageData(0,0,1,1).data]})()")
            .unwrap();
        assert_eq!(frame, json!([0, 0, 255, 255]));
        let answered = answered.lock().unwrap();
        assert_eq!(answered[0][0], 41);
        assert_eq!(answered[0][3], json!([0, 0, 255, 255]));
        // Done waiting: asks go to the realm's inbox again.
        assert!(runner.host.nested_ask(42, json!({})).is_err());
    }
    #[test]
    fn tool_and_viewer_services_follow_the_shell() {
        let runner = dom_runner();
        let value = runner
            .json("(()=>{const s=__compat.realm.services;const t=s.get('tool'),v=s.get('viewer');let heard=null;__compat.realm.PM.bus.on('tool',x=>heard=x);
                const before=[t.tool,v.zoom,v.fit,v.stage.id];__compat.shellState({tool:'hand'});__compat.shellState({zoom:2,fit:false,pan:[3,4]});
                const shell=[t.tool,heard,v.zoom,v.fit,v.pan];
                // As main: setZoom leaves fit, clamped, at once; setTool reads back at once.
                const zoomed=[v.setZoom(20),v.zoom,v.fit,v.setZoom(NaN)];t.setTool('shape','ellipse');const shape=[t.tool,t.toolShape,heard];t.toolShape='star';
                return [before,...shell,zoomed,shape,t.toolShape,typeof v.layout]})()")
            .unwrap();
        assert_eq!(
            value,
            json!([
                ["select", 1, true, "stage"],
                "hand",
                "hand",
                2,
                false,
                [3, 4],
                [8, 8, false, false],
                ["shape", "ellipse", "shape"],
                "star",
                "function"
            ])
        );
    }
    /// v1's viewer stage: `#stage` over the viewer, `#stage-inner` on the
    /// composition frame, `ov` at device pixels; frames and placement reach
    /// listeners as main's `frame:rendered`, `layout` and `overlay`.
    #[test]
    fn viewer_stage_follows_the_shell_and_hears_presented_frames() {
        let runner = dom_runner();
        let value = runner
            .json(r#"(()=>{const api=__compat.realm.extension('probe',3).api;const v=api.services.get('viewer');const heard=[];
                for(const e of ['layout','overlay','frame:rendered'])api.events.on(e,x=>heard.push(x===undefined?e:[e,x]));
                const stage=v.stage,inner=stage.querySelector(':scope > #stage-inner');
                __compat.shellState({stage:[100,50,800,450],frame:[260,140,480,270]});
                const ov=v.ov;
                const placed=[stage.id,stage.style.left,stage.style.width,inner.style.left,inner.style.top,inner.style.width,ov.width,ov.height,v.shown,api.render.gl.previewViewport,v.stage===stage,stage.isConnected];
                const ctx=ov.getContext('2d');ctx.fillStyle='#f00';ctx.fillRect(0,0,4,4);const drawn=ctx.getImageData(0,0,1,1).data[3];
                __compat.presented({time:1.5,version:7,quality:.5});
                const cleared=ctx.getImageData(0,0,1,1).data[3];
                __compat.shellState({frame:[-200,-100,960,540]});
                const crop=api.render.gl.previewViewport;
                __compat.shellState({stage:null});
                return [placed,drawn,cleared,heard,crop,stage.style.display,v.layerAtPoint(10,10)]})()"#)
            .unwrap();
        assert_eq!(
            value,
            json!([
                ["stage", "100px", "800px", "160px", "90px", "480px", 1600, 900, 1.5, null, true, true],
                255,
                0,
                [
                    "layout",
                    "overlay",
                    ["frame:rendered", {"time": 1.5, "viewport": null, "version": 7, "quality": 0.5}],
                    "overlay",
                    "overlay",
                    "layout",
                    "overlay"
                ],
                {"x": 100, "y": 50, "width": 220, "height": 130},
                "none",
                null
            ])
        );
    }
    /// `layerAtPoint` and `render.gl.pick` answer with main's layer, from a
    /// window point through the composition frame the shell reports.
    #[test]
    fn viewer_layer_at_point_picks_through_the_frame() {
        let runner = dom_runner();
        let value = runner
            .json(r#"(()=>{const api=__compat.realm.extension('probe',3).api;const v=api.services.get('viewer');
                const made=api.edit.apply([{type:'add_layer',id:'box',layerType:'solid',name:'Box',from:0,duration:2,properties:{x:80,y:45,w:40,h:30}}]);
                __compat.shellState({stage:[0,0,800,500],frame:[0,0,640,360]});
                const hit=v.layerAtPoint(160,90),miss=v.layerAtPoint(700,400);
                return [made.ok,hit?.id,hit===api.model.layer('box'),miss,api.render.gl.pick(80,45,0)?.id]})()"#)
            .unwrap();
        assert_eq!(value, json!([true, "box", true, null, "box"]));
    }
    /// Main's `timeline` service over the native timeline's reported view and
    /// rows; its edges come from the project as timeline.ts computes them.
    #[test]
    fn timeline_service_reads_the_native_view_and_rows() {
        let runner = dom_runner();
        let value = runner
            .json(r#"(()=>{const api=__compat.realm.extension('probe',3).api;const t=api.services.get('timeline');
                api.edit.apply([{type:'add_layer',id:'a',layerType:'solid',from:.5,duration:1}]);
                let redraws=0;api.events.on('invalidate',w=>{if(w==='timeline')redraws++});
                __compat.shellState({timeline:{graph:true,pps:50,scrollT:1,scrollY:0,keySelectionActive:true,rect:[10,500,600,200],ruler:24,column:200,rows:[['a',524,550]]}});
                const read=[t.graph,t.pps,t.scrollT,t.keySelectionActive,t.cv,redraws];
                t.pps=1e6;t.scrollY=-5;const written=[t.pps,t.scrollY];
                PM.setTime(.2);const early=[t.prevEdge(),t.nextEdge()];PM.setTime(1);const mid=[t.prevEdge(),t.nextEdge()];
                const hits=[t.layerAtPoint(100,530)?.id,t.layerAtPoint(100,505),t.layerAtPoint(5,530),t.layerAtPoint(100,560)];
                return [read,written,early,mid,hits,typeof t.frameView,typeof t.reveal]})()"#)
            .unwrap();
        assert_eq!(
            value,
            json!([
                [true, 50, 1, true, null, 1],
                [4000, 0],
                [0, 0.5],
                [0.5, 1.5],
                ["a", null, null, null],
                "function",
                "function"
            ])
        );
    }
    /// shaderHooks reconciles `d.uniforms` with the kernel's `@param` parser,
    /// as main's inspector did; the inspector's effect clipboard is the app's
    /// (`effects.clipboard`), with main's booleans, and a paste is readable
    /// as soon as it returns, as main's `edit.apply` was.
    #[test]
    fn shader_hooks_and_inspector_service() {
        let runner = dom_runner();
        let kernel = runner.actor.as_ref().unwrap().handle();
        let asked = Arc::new(std::sync::Mutex::new(vec![]));
        let log = asked.clone();
        let hook: crate::EffectsHook = Arc::new(move |op: &str| {
            let mut asked = log.lock().unwrap();
            asked.push(op.to_owned());
            Ok(match (op, asked.len()) {
                ("copy", 1) => json!({"copied": 0}),
                ("copy", _) => json!({"copied": 2}),
                ("paste", 3) => json!({"empty": true}),
                ("paste", 4) => json!({"reason": "select"}),
                ("paste", _) => {
                    // The app pastes through the kernel; the realm hears it now.
                    let rename = pm_kernel::Call::Transact {
                        label: Some("Paste Effects".into()),
                        body: pm_kernel::WriteBody::Commands(vec![pm_kernel::Command::Layer(
                            pm_kernel::LayerCommand::Set {
                                layer_id: "sh".into(),
                                patch: pm_kernel::commands::layers::LayerPatch {
                                    name: Some("Pasted".into()),
                                    ..Default::default()
                                },
                            },
                        )]),
                        expected_revision: None,
                    };
                    kernel
                        .call_blocking(
                            PROJECT,
                            pm_kernel::Request::new(pm_kernel::Caller::native(), rename),
                        )
                        .map_err(|e| e.to_string())?;
                    json!({"pasted": 2})
                }
                _ => json!({}),
            })
        });
        runner
            .host
            .set_effects(Some(Arc::new(std::sync::Mutex::new(Some(hook)))));
        let value = runner
            .json(r#"(()=>{const api=__compat.realm.extension('probe',3).api;const h=api.services.get('shaderHooks'),i=api.services.get('inspector');
                api.edit.apply([{type:'add_layer',id:'sh',layerType:'shader',from:0,duration:2}]);
                const before=Object.keys(PM.L('sh').d.uniforms).sort();
                api.history.do('Code',()=>{const L=PM.L('sh');L.d.code='uniform float uGain; // @param 2 0 4\nuniform vec3 uTint; // @param #ff0000\nvoid main(){}';h.syncShaderUniforms(L);});
                const L=api.model.layer('sh');const after=Object.keys(L.d.uniforms).sort();
                const clip=[i.copySelectedEffects(),i.copySelectedEffects(),i.pasteCopiedEffects(),i.pasteCopiedEffects(),i.pasteCopiedEffects(),api.model.layer('sh').name,i.clearEffectClipboard()];
                return [before,after,L.d.uniforms.uGain.v,L.d.uniforms.uTint.v,i===api.services.get('inspector'),i.body,i.syncs.length,typeof i.refresh,clip]})()"#)
            .unwrap();
        assert_eq!(
            value,
            json!([
                ["uScale", "uSpeed", "uTint"],
                ["uGain", "uTint"],
                2,
                "#FF6B1A",
                true,
                null,
                0,
                "function",
                [false, true, false, true, true, "Pasted", null]
            ])
        );
        assert_eq!(
            *asked.lock().unwrap(),
            ["copy", "copy", "paste", "paste", "paste", "clear"]
        );
    }
    /// D7's presenter fixture: bind a canvas, resize it, read its pixels and
    /// observe frame events; frames reach it by readback from the engine.
    #[test]
    fn presenter_canvas_is_filled_by_readback_before_frame_events() {
        let runner = dom_runner();
        let asked = Arc::new(std::sync::Mutex::new(vec![]));
        let seen = asked.clone();
        runner
            .host
            .set_frames(Some(Arc::new(move |frame: &crate::FrameRequest| {
                seen.lock().unwrap().push((frame.width, frame.height));
                let (w, h) = (frame.width, frame.height);
                let mut png = vec![];
                let mut encoder = png::Encoder::new(&mut png, w, h);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                let mut writer = encoder.write_header().unwrap();
                // Left half red, right half blue: a crop shows which part it took.
                let row: Vec<u8> = (0..w)
                    .flat_map(|x| {
                        if x < w / 2 {
                            [255, 0, 0, 255]
                        } else {
                            [0, 0, 255, 255]
                        }
                    })
                    .collect();
                writer.write_image_data(&row.repeat(h as usize)).unwrap();
                drop(writer);
                Ok(png)
            })));
        let value = runner
            .json(r#"(()=>{const api=__compat.realm.extension('probe',3).api;const gl=api.render.gl;
                const canvas=document.createElement('canvas');canvas.width=4;canvas.height=2;
                const px=(x)=>[...canvas.getContext('2d').getImageData(x,0,1,1).data];
                const bound=[gl.init({}),gl.init(canvas),PM.GL.canvas===canvas,px(0),px(3)];
                const resized=[gl.resize(4,2),gl.resize(8,2),canvas.width];
                let heard=null;api.events.on('frame:rendered',()=>{heard=px(7)});
                const v=__compat.realm.revision;__compat.presented({time:0,version:v,quality:1});
                const cropped=gl.resize(8,2,{x:160,y:0,width:160,height:180});__compat.presented({time:0,version:v,quality:1});
                return [bound,resized,heard,cropped,px(0),gl.previewViewport.x]})()"#)
            .unwrap();
        assert_eq!(
            value,
            json!([
                [false, true, true, [255, 0, 0, 255], [0, 0, 255, 255]],
                [false, true, 8],
                [0, 0, 255, 255],
                false,
                [0, 0, 255, 255],
                160
            ])
        );
        // init, resize, a presented frame, then a frame with the crop: the
        // whole composition at the crop's scale (8 px across 160 of 320).
        assert_eq!(*asked.lock().unwrap(), [(4, 2), (8, 2), (8, 2), (16, 9)]);
    }
    /// `api.scene3d` edits through the native Studio command (one undo
    /// step), describes and selects as main; the studio-only view controls
    /// say they are missing.
    #[test]
    fn scene3d_edits_through_the_native_studio_command() {
        let runner = dom_runner();
        let value = runner
            .json(r#"(()=>{const api=__compat.realm.extension('probe',3).api;const s=api.scene3d;const heard=[];api.events.on('scene3d:selection',x=>heard.push(x));
                const made=s.edit({operation:'add_object',id:'cube',name:'Cube'});
                api.history.undo();const undone=api.model.layer('cube');api.history.redo();
                const scene=s.describe();s.select(null,['cube']);
                let refused=null;try{s.setView('editor')}catch(e){refused=e.message}
                const facts=[made.ok,made.data.result.layerId,scene.objects.map(o=>o.id),s.selection(),heard,s.isGroup('cube'),s.convert('cube').message,
                  s.createScene().lights.length,s.createObject('o1').source.primitive,s.createLight('l1','point').type,s.getView(),refused];
                return [...facts,undone]})()"#)
            .unwrap();
        assert_eq!(
            value,
            json!([
                true,
                "cube",
                ["cube"],
                {"layerId": "cube", "ids": ["cube"]},
                [{"layerId": "cube", "ids": ["cube"]}],
                false,
                "Already a 3D layer",
                1,
                "box",
                "point",
                "camera",
                "scene3d.setView('editor') needs the 3D studio, which this build does not have",
                null
            ])
        );
    }
    #[test]
    fn ui_modal_keeps_main_semantics_with_native_frame_data() {
        let runner = dom_runner();
        let value = runner
            .json(r#"(async()=>{const PM=__compat.realm.PM;const ran=[];const input=document.createElement('input');
                const m=PM.modal({title:'Curve',body:input,width:360,actions:[{label:'Stay',run:()=>{ran.push('stay');return false}},{label:'Done',pri:true,run:async()=>{ran.push('done')}}]});
                const frame=JSON.parse(m.el.getAttribute('data-pm-modal-frame'));const open=()=>m.el.isConnected;
                m.el.querySelector('[data-pm-modal-action="0"]').click();const afterStay=open();
                m.el.querySelector('[data-pm-modal-action="1"]').click();await new Promise(r=>setTimeout(r,0));
                let closes=0;const n=PM.modal({title:'Fixed',body:'Text',dismissible:false,onClose:()=>closes++});n.el.querySelector('[data-pm-modal-dismiss]').click();const fixed=n.el.isConnected;n.close();n.close();
                return [frame,m.body.contains(input),afterStay,open(),ran,fixed,n.el.isConnected,closes]})()"#)
            .unwrap();
        assert_eq!(
            value,
            json!([{"title":"Curve","width":360,"actions":[{"label":"Stay","pri":false},{"label":"Done","pri":true}],"dismissible":true},
                true, true, false, ["stay", "done"], true, false, 1])
        );
    }
    #[test]
    fn render_frame_to_and_snapshot_read_the_engine_frame() {
        let runner = dom_runner();
        let asked = Arc::new(std::sync::Mutex::new(vec![]));
        let log = asked.clone();
        runner
            .host
            .set_frames(Some(Arc::new(move |frame: &crate::FrameRequest| {
                let (width, height) = (frame.width, frame.height);
                log.lock()
                    .unwrap()
                    .push((frame.time, width, height, frame.motion_blur));
                // An opaque engine frame is RGB.
                let mut png = vec![];
                let mut encoder = png::Encoder::new(&mut png, width, height);
                encoder.set_color(png::ColorType::Rgb);
                encoder.set_depth(png::BitDepth::Eight);
                let mut writer = encoder.write_header().unwrap();
                writer
                    .write_image_data(&[0, 128, 255].repeat((width * height) as usize))
                    .unwrap();
                drop(writer);
                Ok(png)
            })));
        let frame = runner
            .json("(()=>{const c=PM.renderFrameTo(1.5,4,2);const p=c.getContext('2d').getImageData(0,0,4,2).data;return [c.width,c.height,[p[0],p[1],p[2],p[3]]]})()")
            .unwrap();
        assert_eq!(frame, json!([4, 2, [0, 128, 255, 255]]));
        // Options reach the renderer; a target canvas receives the frame and is returned.
        let target = runner
            .json("(()=>{const t=document.createElement('canvas');const r=PM.renderFrameTo(0,3,3,{mblur:false},t);return [r===t,t.width,t.height]})()")
            .unwrap();
        assert_eq!(target, json!([true, 3, 3]));
        let snapshot = runner.json("PM.Export.snapshot(0, 96)").unwrap();
        assert!(
            snapshot
                .as_str()
                .unwrap()
                .starts_with("data:image/jpeg;base64,/9j/")
        );
        let (w, h) = runner.host.kernel_size();
        let asked = asked.lock().unwrap().clone();
        assert_eq!(asked[0], (1.5, 4, 2, true));
        assert_eq!(asked[1], (0., 3, 3, false));
        assert_eq!(asked[2], (0., 96, (96. * h / w).round() as u32, false));
    }
    #[test]
    fn a_stopped_capture_reaches_the_renderer() {
        let runner = dom_runner();
        runner
            .host
            .set_frames(Some(Arc::new(|frame: &crate::FrameRequest| {
                // The renderer polls `stop` and gives up; the realm sees an error.
                let deadline = Instant::now() + Duration::from_secs(5);
                while !(frame.stop)() {
                    if Instant::now() > deadline {
                        return Err("never stopped".into());
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                Err("Capture cancelled".into())
            })));
        let mut ticket = runner.realm.eval("PM.renderFrameTo(0,2,2)").unwrap();
        thread::sleep(Duration::from_millis(50));
        runner.realm.stop(ticket.execution);
        let deadline = Instant::now() + Duration::from_secs(5);
        assert!(
            pm_runtime_v8::wait_until(&mut ticket, deadline).is_some(),
            "the capture outlived Stop"
        );
    }
    #[test]
    fn extension_bytes_are_staged_as_files_and_held_files_read_lazily() {
        let runner = dom_runner();
        let staged = runner
            .json("runtime.bytes('assets.stage', new File(['logo bytes'], '../logo.svg', {type:'image/svg+xml'}), {name:'../logo.svg'}).token")
            .unwrap();
        let token: u64 = staged.as_str().unwrap().parse().unwrap();
        let path = crate::files::path(token).unwrap();
        assert_eq!(path.file_name().unwrap(), "_logo.svg");
        assert_eq!(std::fs::read(&path).unwrap(), b"logo bytes");
        let read = runner
            .json(&format!("(async()=>{{const f=runtime.hostFile({{token:{staged},name:'logo.svg',type:'image/svg+xml',size:10,lastModified:1}});return [f.size,f.name,f instanceof File,runtime.fileToken(f),await f.text()]}})()"))
            .unwrap();
        assert_eq!(read, json!([10, "logo.svg", true, staged, "logo bytes"]));
        crate::files::release_staged(token);
        assert!(!path.exists());
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
