//! Chromium's exact Canvas2D PNG for converted Blender maps. This wraps the
//! ordinary file resolver during snapshot construction, before snapshot asset
//! hashes and Blender surface keys are computed. The host uses the same
//! `compatFrame` window ask as S1 SVG rasters.

use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use pm_model::{Document, Node};
use pm_protocol::common::{ContentHash, Revision, SessionId, Ticks};
use pm_protocol::envelope::{Request, Response};
use pm_protocol::snapshot::{
    BlenderPngRequest, CompatFormat, CompatFrameRequest, CompatSource, PixelPayload,
};
use pm_render::AssetResolver;
use sha2::{Digest, Sha256};

use super::blender::HostPort;

/// Converted PNGs are process-local, never written as source or expected
/// fixture data. One largest accepted PNG fits; older entries leave first.
const CACHE_BYTES: usize = 256 * 1024 * 1024;

#[derive(Clone)]
enum Cached {
    Ready(Arc<Vec<u8>>),
    Loading(Arc<Loading>),
}

#[derive(Default)]
struct Loading {
    result: Mutex<Option<Result<Arc<Vec<u8>>, String>>>,
    ready: Condvar,
}

impl Loading {
    fn wait(&self) -> Result<Arc<Vec<u8>>, String> {
        let mut result = self.result.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(value) = result.as_ref() {
                return value.clone();
            }
            result = self.ready.wait(result).unwrap_or_else(|e| e.into_inner());
        }
    }

    fn finish(&self, value: Result<Arc<Vec<u8>>, String>) {
        *self.result.lock().unwrap_or_else(|e| e.into_inner()) = Some(value);
        self.ready.notify_all();
    }
}

#[derive(Default)]
struct Cache {
    entries: HashMap<[u8; 32], Cached>,
    order: VecDeque<[u8; 32]>,
    bytes: usize,
}

static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();

/// A synchronous renderer call cannot wait for the window's `compatFrame`
/// reply: that reply is pumped by the same engine IPC turn. It may consume
/// only a completed conversion from an earlier preview or export.
fn cached_ready(content: [u8; 32]) -> Option<Arc<Vec<u8>>> {
    let cache = CACHE.get_or_init(|| Mutex::new(Cache::default()));
    let mut state = cache.lock().unwrap_or_else(|e| e.into_inner());
    let Some(Cached::Ready(bytes)) = state.entries.get(&content).cloned() else {
        return None;
    };
    state.order.retain(|key| *key != content);
    state.order.push_back(content);
    Some(bytes)
}

fn source_hash(path: &Path) -> Result<[u8; 32], String> {
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().into())
}

fn converted(
    content: [u8; 32],
    fetch: impl FnOnce() -> Result<Vec<u8>, String>,
) -> Result<Arc<Vec<u8>>, String> {
    let cache = CACHE.get_or_init(|| Mutex::new(Cache::default()));
    let loading = {
        let mut state = cache.lock().unwrap_or_else(|e| e.into_inner());
        match state.entries.get(&content).cloned() {
            Some(Cached::Ready(bytes)) => {
                state.order.retain(|key| *key != content);
                state.order.push_back(content);
                return Ok(bytes);
            }
            Some(Cached::Loading(wait)) => {
                drop(state);
                return wait.wait();
            }
            None => {
                let wait = Arc::new(Loading::default());
                state.entries.insert(content, Cached::Loading(wait.clone()));
                wait
            }
        }
    };
    let result = fetch().map(Arc::new);
    {
        let mut state = cache.lock().unwrap_or_else(|e| e.into_inner());
        match &result {
            Ok(bytes) if bytes.len() <= CACHE_BYTES => {
                while state.bytes + bytes.len() > CACHE_BYTES {
                    let Some(old) = state.order.pop_front() else {
                        break;
                    };
                    if let Some(Cached::Ready(evicted)) = state.entries.remove(&old) {
                        state.bytes -= evicted.len();
                    }
                }
                state.bytes += bytes.len();
                state.order.push_back(content);
                state.entries.insert(content, Cached::Ready(bytes.clone()));
            }
            _ => {
                state.entries.remove(&content);
            }
        }
    }
    loading.finish(result.clone());
    result
}

/// The snapshot's frozen document identity and its existing media resolver.
/// The process cache survives this wrapper's recreation for another frame.
pub(crate) struct HostBlenderPng<P: HostPort + ?Sized> {
    pub source: Arc<dyn AssetResolver>,
    pub port: Arc<P>,
    pub session: SessionId,
    pub revision: Revision,
    pub document: pm_protocol::common::DocumentId,
    cached_only: bool,
    cold_miss: AtomicBool,
}

impl<P: HostPort + ?Sized> HostBlenderPng<P> {
    pub fn new(
        source: Arc<dyn AssetResolver>,
        port: Arc<P>,
        session: SessionId,
        revision: Revision,
        document: &Document,
    ) -> Self {
        Self {
            source,
            port,
            session,
            revision,
            document: super::svg::document_id(document).unwrap_or_else(|| "untitled".into()),
            cached_only: false,
            cold_miss: AtomicBool::new(false),
        }
    }

    /// Use from a synchronous capture only; never ask or wait for the host.
    pub fn cached_only_if(mut self, enabled: bool) -> Self {
        self.cached_only = enabled;
        self
    }

    pub fn cold_miss(&self) -> bool {
        self.cold_miss.load(Ordering::Relaxed)
    }

    fn fetch(&self, asset: &str, content: [u8; 32]) -> Result<Vec<u8>, String> {
        let request = CompatFrameRequest {
            document: self.document.clone(),
            composition: "blender".into(),
            layer: "blender".into(),
            time: Ticks::ZERO,
            source: CompatSource::BlenderPng,
            scale: 1.0,
            revision: self.revision,
            svg: None,
            blender: Some(BlenderPngRequest {
                asset: asset.to_owned(),
                content: ContentHash(content),
            }),
        };
        let answer = self
            .port
            .call(Request::CompatFrame(request), Some(self.session))
            .map_err(|e| e.message)?;
        let Response::CompatFrame(frame) = answer else {
            return Err("The host did not answer the Blender PNG request.".into());
        };
        let expected: String = content.iter().map(|b| format!("{b:02x}")).collect();
        if frame.key != expected
            || frame.format != CompatFormat::Png
            || frame.origin != [0.0, 0.0]
            || frame.scale != 1.0
        {
            return Err("The window answered for a different Blender image.".into());
        }
        let bytes = match frame.payload {
            PixelPayload::Inline { bytes } => bytes.0,
            PixelPayload::SharedMemory { name, length } => {
                pm_bridge::shm::take(&name, length.get() as usize)
                    .map_err(|e| format!("The Blender PNG shared memory could not be read: {e}"))?
            }
        };
        if bytes.len() < 33
            || bytes.len() > 256 * 1024 * 1024
            || !bytes.starts_with(b"\x89PNG\r\n\x1a\n")
            || bytes.get(12..16) != Some(b"IHDR".as_slice())
            || bytes.get(16..20) != Some(frame.width.to_be_bytes().as_slice())
            || bytes.get(20..24) != Some(frame.height.to_be_bytes().as_slice())
        {
            return Err("The window returned an invalid Blender PNG.".into());
        }
        Ok(bytes)
    }
}

impl<P: HostPort + ?Sized> AssetResolver for HostBlenderPng<P> {
    fn path(&self, asset: &str, record: Option<&Node>) -> Option<PathBuf> {
        self.source.path(asset, record)
    }

    fn blender_png(&self, asset: &str, source: &Path) -> Result<Option<Arc<Vec<u8>>>, String> {
        let content =
            source_hash(source).map_err(|e| format!("Could not read Blender map {asset}: {e}"))?;
        if self.cached_only {
            return match cached_ready(content) {
                Some(bytes) => Ok(Some(bytes)),
                None => {
                    self.cold_miss.store(true, Ordering::Relaxed);
                    Err("The exact Blender PNG has not been cached yet.".into())
                }
            };
        }
        Ok(Some(converted(content, || self.fetch(asset, content))?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_cache_shares_successful_bytes_and_retries_failures() {
        let first = converted([0xf1; 32], || Ok(vec![1, 2, 3])).unwrap();
        let again = converted([0xf1; 32], || {
            panic!("a cached map must not ask the window")
        })
        .unwrap();
        assert!(Arc::ptr_eq(&first, &again));
        assert!(converted([0xf2; 32], || Err("window closed".into())).is_err());
        assert_eq!(*converted([0xf2; 32], || Ok(vec![4])).unwrap(), vec![4]);
    }

    #[test]
    fn synchronous_lookup_only_takes_finished_entries() {
        let content = [0xf3; 32];
        assert!(cached_ready(content).is_none());
        let cache = CACHE.get_or_init(|| Mutex::new(Cache::default()));
        let loading = Arc::new(Loading::default());
        cache
            .lock()
            .unwrap()
            .entries
            .insert(content, Cached::Loading(loading));
        assert!(
            cached_ready(content).is_none(),
            "a synchronous request must not await another host ask"
        );
        cache.lock().unwrap().entries.remove(&content);
        let ready = converted(content, || Ok(vec![9, 8, 7])).unwrap();
        assert!(Arc::ptr_eq(&cached_ready(content).unwrap(), &ready));
    }
}
