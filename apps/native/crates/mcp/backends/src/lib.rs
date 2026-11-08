//! Worker-side bindings to the actual native services. No second kernel writer,
//! profile discovery, HTTP implementation, or JS runtime is created here.

use pm_mcp::{
    Automation, Error, Result,
    media::{MediaRegistry, MediaState},
    store,
};
use pm_promedia::media::{Availability, MediaCatalog};
use pm_services::store::{StoreClient, StoreTransport};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, RwLock},
};
use url::Url;

/// Shares the native Store transport's TLS/origin/auth/streaming policy. The
/// origin is admitted by pm-services, exactly as it is for its StoreClient.
pub struct NativeStoreReads<T> {
    origin: Url,
    client: Mutex<StoreClient<T>>,
}
impl<T: StoreTransport> NativeStoreReads<T> {
    pub fn new(origin: &str, transport: T) -> Result<Self> {
        let client = StoreClient::new(origin, transport).map_err(service_error)?;
        let origin = Url::parse(origin).map_err(|_| Error::invalid("Invalid Store origin"))?;
        Ok(Self {
            origin,
            client: Mutex::new(client),
        })
    }
}
fn service_error(e: pm_services::ServiceError) -> Error {
    // Diagnostics can contain host URLs/credentials. Only expose the service's
    // classified code; neither Display nor the source chain crosses MCP.
    Error {
        code: "store_error".into(),
        message: format!("Native Store request failed ({:?})", e.code),
        backing: Some("pm-services StoreTransport".into()),
    }
}
impl<T: StoreTransport + Send> store::StoreRead for NativeStoreReads<T> {
    fn get(&self, request: store::Request) -> Result<store::Response> {
        if !request.path.starts_with("/v1/store/")
            || request.path.contains(['?', '#'])
            || request.max_bytes > store::JSON_LIMIT
        {
            return Err(Error::invalid("Invalid native Store read request"));
        }
        let mut url = self
            .origin
            .join(&request.path)
            .map_err(|_| Error::invalid("Invalid Store route"))?;
        if url.origin() != self.origin.origin() || !url.path().starts_with("/v1/store/") {
            return Err(Error::invalid("Cross-origin Store route"));
        }
        if !request.query.is_empty() {
            url.query_pairs_mut().extend_pairs(request.query);
        }
        let response = self
            .client
            .lock()
            .map_err(|_| Error::unavailable("native Store worker stopped"))?
            .transport
            .send(
                pm_services::store::Request {
                    method: pm_services::store::Method::Get,
                    url,
                    body: None,
                },
                request.max_bytes,
            )
            .map_err(service_error)?;
        if response.body.len() > request.max_bytes {
            return Err(Error::invalid("Store response exceeds the byte limit"));
        }
        Ok(store::Response {
            status: response.status,
            body: response.body,
        })
    }
}
/// Bind once on the MCP worker. The application continues to own authentication
/// and the actual network transport; no provider login is read or changed.
pub fn bind_store<T: StoreTransport + Send + 'static>(
    host: &mut Automation,
    origin: &str,
    transport: T,
) -> Result<()> {
    host.store_reads = Some(Arc::new(NativeStoreReads::new(origin, transport)?));
    Ok(())
}

/// The native catalog, rather than project metadata, supplies live availability
/// and measured source duration. File monitoring/relink stays with promedia.
#[derive(Default)]
pub struct NativeMediaRegistry {
    catalogs: RwLock<BTreeMap<String, Arc<Mutex<MediaCatalog>>>>,
}
impl NativeMediaRegistry {
    pub fn bind(&self, project_id: String, catalog: Arc<Mutex<MediaCatalog>>) -> Result<()> {
        self.catalogs
            .write()
            .map_err(|_| Error::unavailable("native media registry stopped"))?
            .insert(project_id, catalog);
        Ok(())
    }
    pub fn retire(&self, project_id: &str) -> Result<()> {
        self.catalogs
            .write()
            .map_err(|_| Error::unavailable("native media registry stopped"))?
            .remove(project_id);
        Ok(())
    }
}
impl MediaRegistry for NativeMediaRegistry {
    fn state(&self, project_id: &str, asset_id: &str) -> MediaState {
        let stopped = || MediaState {
            error: Some("Native media registry stopped".into()),
            ..Default::default()
        };
        let Ok(catalogs) = self.catalogs.read() else {
            return stopped();
        };
        let Some(catalog) = catalogs.get(project_id).cloned() else {
            return MediaState::default();
        };
        drop(catalogs);
        let Ok(catalog) = catalog.lock() else {
            return stopped();
        };
        let Some(item) = catalog.get(asset_id) else {
            return MediaState::default();
        };
        let Ok(status) = catalog.status(asset_id) else {
            return stopped();
        };
        let info = &item.authored_info;
        let duration = info.duration_ticks as f64 * info.time_base.numerator as f64
            / info.time_base.denominator as f64;
        let available = !matches!(status.availability, Availability::Offline);
        MediaState {
            available,
            duration: Some(duration),
            has_audio: info.audio_channels > 0,
            audio_duration: (info.audio_channels > 0).then_some(duration),
            ..Default::default()
        }
    }
}
