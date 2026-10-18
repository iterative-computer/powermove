//! Translation of shader-layer code through `pm_glsl` (plan section 11's
//! pipeline), shared by every renderer in the process and by the static
//! capability check: a shader layer's program lives in the document, so
//! [`crate::unsupported`] needs its verdict without a renderer.
//!
//! One [`pm_glsl::Cache`] (memory, and disk once
//! [`crate::Renderer::set_program_cache_dir`] names a folder) keyed by the
//! wrapped source; a program translates once per process and, with a
//! folder, once per machine and driver.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use pm_glsl::{Cache, Failure, Key, Prepared};

/// The cache context: the back end and API, as the effects lane names it.
pub fn context() -> String {
    format!("{}; wgpu 30", std::env::consts::OS)
}

fn slot() -> &'static RwLock<(Option<PathBuf>, Arc<Cache>)> {
    static SLOT: OnceLock<RwLock<(Option<PathBuf>, Arc<Cache>)>> = OnceLock::new();
    SLOT.get_or_init(|| RwLock::new((None, Arc::new(Cache::new(context(), None)))))
}

/// The process's shader-layer translation cache.
pub fn cache() -> Arc<Cache> {
    slot().read().expect("cache lock").1.clone()
}

/// Keeps translated shader layers in `dir` across processes (`None`: memory
/// only). Unchanged folders keep the current cache and its memory entries.
pub fn set_cache_dir(dir: Option<PathBuf>) {
    let mut slot = slot().write().expect("cache lock");
    if slot.0 != dir {
        *slot = (dir.clone(), Arc::new(Cache::new(context(), dir)));
    }
}

/// A cache entry: the prepared program or why it failed.
pub type Translated = Arc<Result<Prepared, Failure>>;

/// Where a program stands.
#[derive(Debug, Clone)]
pub enum Verdict {
    /// Translated: drawn natively (the cache's entry, always `Ok`).
    Ready(Translated),
    /// WebGL refuses the program too (`pm_glsl::FailureStage::webgl_refuses`):
    /// today the compositor's `program()` returns null and the layer draws
    /// nothing; so does the engine, with the diagnostics reported.
    Refused(Failure),
    /// WebGL draws the program and the native pipeline cannot: the
    /// decision 19 fallback runner renders it.
    NeedsFallback(Failure),
}

fn failed() -> &'static Mutex<HashMap<Key, Failure>> {
    static F: OnceLock<Mutex<HashMap<Key, Failure>>> = OnceLock::new();
    F.get_or_init(Default::default)
}

/// Records a program the device refused after translation (Metal pipeline
/// creation): later plans route it to the fallback runner. Kept for the
/// process, like the cache's failures (a driver update must be able to
/// retry it in the next one).
pub fn mark_failed(key: Key, failure: Failure) {
    failed().lock().expect("failed lock").insert(key, failure);
}

/// The verdict of a wrapped fragment program.
pub fn verdict(source: &str, line_offset: u32) -> (Key, Verdict) {
    let cache = cache();
    let key = cache.key(source, line_offset);
    if let Some(f) = failed().lock().expect("failed lock").get(&key) {
        return (key, Verdict::NeedsFallback(f.clone()));
    }
    let entry = cache.prepare(source, line_offset);
    let verdict = match entry.as_ref() {
        Ok(_) => Verdict::Ready(entry.clone()),
        Err(f) if f.stage.webgl_refuses() => Verdict::Refused(f.clone()),
        Err(f) => Verdict::NeedsFallback(f.clone()),
    };
    (key, verdict)
}
