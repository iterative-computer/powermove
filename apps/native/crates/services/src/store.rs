//! Read-only Store transport + install/update metadata, never a publisher.
//! Authentication belongs in the host transport; requests/results here carry no secrets.
use crate::error::{Context, Result};
use crate::service_ensure as ensure;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use url::Url;

const MAX_JSON: usize = 4 * 1024 * 1024;
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Method {
    Get,
    PostVersions,
}
#[derive(Debug, Clone)]
pub struct Request {
    pub method: Method,
    pub url: Url,
    pub body: Option<Value>,
}
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}
pub trait StoreTransport {
    /// Enforce max while streaming, bind redirects to the requested origin, and keep
    /// auth headers out of errors/logs. No retry of mutation routes is possible here.
    fn send(&mut self, request: Request, max_bytes: usize) -> Result<Response>;
}
impl<T: StoreTransport + ?Sized> StoreTransport for Box<T> {
    fn send(&mut self, request: Request, max_bytes: usize) -> Result<Response> {
        (**self).send(request, max_bytes)
    }
}
pub struct StoreClient<T> {
    origin: Url,
    pub transport: T,
}
#[derive(Debug, Clone, Default)]
pub struct Search {
    pub query: Option<String>,
    pub category: Option<String>,
    pub sort: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<u8>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPage {
    pub items: Vec<Value>,
    #[serde(deserialize_with = "required_option")]
    pub next_cursor: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledRelease {
    pub repo_id: String,
    pub release_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Latest {
    pub release_id: String,
    pub version: String,
    pub tree_sha: String,
    pub tar_sha256: String,
    pub api_version: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionMetadata {
    pub repo_id: String,
    pub state: String,
    pub current: Current,
    #[serde(deserialize_with = "required_option")]
    pub latest: Option<Latest>,
    #[serde(deserialize_with = "required_option")]
    pub owner_publisher_id: Option<String>,
    #[serde(deserialize_with = "required_option")]
    pub handle: Option<String>,
    #[serde(deserialize_with = "required_option")]
    pub slug: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Current {
    pub yanked: bool,
}
/// One storefront shelf (`featured`, `picks`, `new` or a category).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Shelf {
    pub id: String,
    pub title: String,
    pub items: Vec<Value>,
}
#[derive(Deserialize)]
struct Versions {
    items: Vec<VersionMetadata>,
}

impl<T: StoreTransport> StoreClient<T> {
    pub fn new(origin: &str, transport: T) -> Result<Self> {
        let origin = Url::parse(origin)?;
        ensure!(
            (origin.scheme() == "https" || dev_loopback(&origin))
                && origin.host_str().is_some()
                && origin.username().is_empty()
                && origin.password().is_none()
                && origin.query().is_none()
                && origin.fragment().is_none()
                && origin.path() == "/",
            "Store requires a credential-free HTTPS origin"
        );
        Ok(Self { origin, transport })
    }
    fn request<R: DeserializeOwned>(
        &mut self,
        path: &str,
        query: Vec<(&str, String)>,
        body: Option<Value>,
    ) -> Result<R> {
        let mut url = self.origin.join(path)?;
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        ensure!(
            url.origin() == self.origin.origin(),
            "cross-origin Store request"
        );
        let method = if body.is_some() {
            Method::PostVersions
        } else {
            Method::Get
        };
        let response = self
            .transport
            .send(Request { method, url, body }, MAX_JSON)?;
        ensure!(response.body.len() <= MAX_JSON, TooLarge; "Store response too large");
        if response.status != 200 {
            use crate::error::{ServiceError, ServiceErrorCode};
            let code = match response.status {
                401 | 403 => ServiceErrorCode::PermissionDenied,
                404 | 410 => ServiceErrorCode::NotFound,
                413 => ServiceErrorCode::TooLarge,
                408 | 502..=504 => ServiceErrorCode::Offline,
                _ => ServiceErrorCode::Unavailable,
            };
            return Err(ServiceError::message(
                code,
                format!("Store metadata request failed (HTTP {})", response.status),
            ));
        }
        // Never echo a server body in an error (it may contain account details).
        serde_json::from_slice(&response.body).map_err(|_| {
            crate::error::ServiceError::message(
                crate::error::ServiceErrorCode::InvalidData,
                "invalid Store metadata response",
            )
        })
    }
    pub fn search(&mut self, search: Search) -> Result<SearchPage> {
        ensure!(
            search.limit.is_none_or(|limit| (1..=50).contains(&limit)),
            "invalid Store page limit"
        );
        let mut query = Vec::new();
        if let Some(q) = search.query {
            ensure!(q.len() <= 4096, TooLarge; "query too long");
            query.push(("q", q));
        }
        if let Some(category) = search.category {
            ensure!(
                [
                    "effects",
                    "transitions",
                    "panels",
                    "layers",
                    "themes",
                    "commands",
                    "tools"
                ]
                .contains(&category.as_str()),
                "invalid category"
            );
            query.push(("category", category));
        }
        if let Some(sort) = search.sort {
            ensure!(
                ["new", "installs", "name"].contains(&sort.as_str()),
                "invalid sort"
            );
            query.push(("sort", sort));
        }
        if let Some(cursor) = search.cursor {
            ensure!(cursor.len() <= 4096, TooLarge; "cursor too long");
            query.push(("cursor", cursor));
        }
        if let Some(limit) = search.limit {
            query.push(("limit", limit.to_string()));
        }
        let mut page: SearchPage = self.request("/v1/store/extensions", query, None)?;
        ensure!(
            page.items.len() <= 50 && page.next_cursor.as_ref().is_none_or(|s| s.len() <= 4096),
            "invalid Store page"
        );
        for item in &mut page.items {
            self.admit_listing(item)?;
        }
        Ok(page)
    }
    /// The storefront (`GET /v1/store/browse`): titled shelves of listings.
    pub fn browse(&mut self) -> Result<Vec<Shelf>> {
        #[derive(Deserialize)]
        struct Browse {
            sections: Vec<Shelf>,
        }
        let mut browse: Browse = self.request("/v1/store/browse", vec![], None)?;
        ensure!(browse.sections.len() <= 32, "invalid Store page");
        for shelf in &mut browse.sections {
            ensure!(
                shelf.items.len() <= 50 && shelf.id.len() <= 64 && shelf.title.len() <= 200,
                "invalid Store page"
            );
            for item in &mut shelf.items {
                self.admit_listing(item)?;
            }
        }
        Ok(browse.sections)
    }
    /// One extension's page (`GET /v1/store/x/<handle>/<slug>`), with every
    /// release for the version list.
    pub fn detail(&mut self, handle: &str, slug_value: &str) -> Result<Value> {
        ensure!(valid_handle(handle), "invalid publisher handle");
        slug(slug_value)?;
        let mut detail: Value =
            self.request(&format!("/v1/store/x/{handle}/{slug_value}"), vec![], None)?;
        self.admit_listing(&mut detail)?;
        let releases = detail
            .get("releases")
            .and_then(Value::as_array)
            .context("missing Store releases")?;
        ensure!(releases.len() <= 1000, "invalid Store releases");
        for release in releases {
            uuid(field(release, "id")?)?;
            version(field(release, "version")?)?;
        }
        Ok(detail)
    }
    /// Required listing fields, and the tagged icon policy: only
    /// content-addressed same-origin PNGs keep their URL.
    fn admit_listing(&self, item: &mut Value) -> Result<()> {
        uuid(field(item, "repoId")?)?;
        slug(field(item, "slug")?)?;
        ensure!(
            item.pointer("/owner/handle")
                .and_then(Value::as_str)
                .is_some_and(valid_handle),
            "invalid publisher handle"
        );
        if let Some(latest) = item.get("latest").filter(|l| !l.is_null()) {
            uuid(field(latest, "id")?)?;
            version(field(latest, "version")?)?;
        }
        let icon = item
            .get("iconUrl")
            .and_then(Value::as_str)
            .and_then(|icon| self.origin.join(icon).ok())
            .filter(|url| {
                url.origin() == self.origin.origin()
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.query().is_none()
                    && url.fragment().is_none()
            })
            .filter(|url| {
                url.path()
                    .strip_prefix("/v1/store/icons/")
                    .and_then(|path| path.strip_suffix(".png"))
                    .is_some_and(|hash| validate_hash(hash).is_ok())
            })
            .map(|url| Value::String(url.into()))
            .unwrap_or(Value::Null);
        item.as_object_mut()
            .context("invalid listing")?
            .insert("iconUrl".into(), icon);
        Ok(())
    }
    pub fn versions(&mut self, installed: &[InstalledRelease]) -> Result<Vec<VersionMetadata>> {
        ensure!(installed.len() <= 200, TooLarge; "version batch exceeds 200");
        let mut ids = std::collections::BTreeSet::new();
        for item in installed {
            uuid(&item.repo_id)?;
            uuid(&item.release_id)?;
            ensure!(ids.insert(&item.repo_id), "duplicate repository");
        }
        if installed.is_empty() {
            return Ok(Vec::new());
        }
        let response: Versions = self.request(
            "/v1/store/versions",
            vec![],
            Some(json!({"items":installed})),
        )?;
        ensure!(
            response.items.len() == installed.len(),
            "incomplete versions response"
        );
        for item in &response.items {
            ensure!(ids.remove(&item.repo_id), "unexpected/duplicate repository");
            ensure!(
                ["ok", "hidden", "removed", "tombstoned"].contains(&item.state.as_str()),
                "invalid repository state"
            );
            if let Some(latest) = &item.latest {
                uuid(&latest.release_id)?;
                version(&latest.version)?;
                sha1(&latest.tree_sha)?;
                validate_hash(&latest.tar_sha256)?;
                ensure!(latest.api_version > 0, "invalid apiVersion");
            }
        }
        Ok(response.items)
    }
}
/// Development builds may read a fixture Store on this Mac over plain HTTP
/// (`http://127.0.0.1:<port>/`); release builds take HTTPS only.
fn dev_loopback(origin: &Url) -> bool {
    cfg!(debug_assertions) && origin.scheme() == "http" && origin.host_str() == Some("127.0.0.1")
}
/// The registry's publisher handle grammar (`^[a-z0-9][a-z0-9-]{1,38}$`).
pub fn valid_handle(value: &str) -> bool {
    (2..=39).contains(&value.len())
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}
fn field<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .context("missing Store field")
}
fn uuid(value: &str) -> Result<()> {
    ensure!(
        value.len() == 36
            && value
                .bytes()
                .enumerate()
                .all(|(i, c)| if [8, 13, 18, 23].contains(&i) {
                    c == b'-'
                } else {
                    c.is_ascii_hexdigit()
                }),
        "invalid release/repository id"
    );
    Ok(())
}
fn sha1(value: &str) -> Result<()> {
    ensure!(
        value.len() == 40
            && value
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)),
        "invalid tree/blob hash"
    );
    Ok(())
}
fn version(value: &str) -> Result<()> {
    let parts: Vec<_> = value.split('.').collect();
    ensure!(
        parts.len() == 3
            && parts.iter().all(|part| !part.is_empty()
                && part.len() <= 6
                && part.bytes().all(|c| c.is_ascii_digit())),
        "invalid release version"
    );
    Ok(())
}
fn slug(value: &str) -> Result<()> {
    ensure!(
        (2..=64).contains(&value.len())
            && value.as_bytes()[0].is_ascii_alphanumeric()
            && value
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-'),
        "invalid extension slug"
    );
    Ok(())
}

// Wire nullable fields are required, unlike optional Rust fields. Missing metadata
// must not become an apparently valid empty/latest-unavailable response.
fn required_option<'de, D, T>(deserializer: D) -> std::result::Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}
fn validate_hash(value: &str) -> Result<()> {
    ensure!(
        value.len() == 64
            && value
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)),
        "invalid SHA-256"
    );
    Ok(())
}
