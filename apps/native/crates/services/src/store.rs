//! Read-only Store transport + install/update metadata, never a publisher.
//! Authentication belongs in the host transport; requests/results here carry no secrets.
use crate::{media::validate_hash, storage::component};
use anyhow::{Context, Result, ensure};
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseMetadata {
    pub id: String,
    pub repo_id: String,
    pub version: String,
    pub tree_sha: String,
    pub tar_sha256: String,
    pub api_version: u32,
    pub file_count: u32,
    pub size_bytes: u64,
    #[serde(deserialize_with = "required_option")]
    pub yanked_at: Option<String>,
    pub manifest: Value,
    /// Retain all unconsumed metadata for future consumers.
    #[serde(flatten)]
    pub extra: std::collections::BTreeMap<String, Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeFile {
    pub path: String,
    pub sha: String,
    pub size: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseTree {
    pub tree_sha: String,
    pub files: Vec<TreeFile>,
}
/// This is a plan, never a receipt that code has been installed or trusted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallPlan {
    pub origin: String,
    pub release: ReleaseMetadata,
    pub tree: ReleaseTree,
}
#[derive(Deserialize)]
struct Versions {
    items: Vec<VersionMetadata>,
}

impl<T: StoreTransport> StoreClient<T> {
    pub fn new(origin: &str, transport: T) -> Result<Self> {
        let origin = Url::parse(origin)?;
        ensure!(
            origin.scheme() == "https"
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
        ensure!(response.body.len() <= MAX_JSON, "Store response too large");
        ensure!(
            response.status == 200,
            "Store metadata request failed (HTTP {})",
            response.status
        );
        // Never echo a server body in an error (it may contain account details).
        serde_json::from_slice(&response.body)
            .map_err(|_| anyhow::anyhow!("invalid Store metadata response"))
    }
    pub fn search(&mut self, search: Search) -> Result<SearchPage> {
        ensure!(
            search.limit.is_none_or(|limit| (1..=50).contains(&limit)),
            "invalid Store page limit"
        );
        let mut query = Vec::new();
        if let Some(q) = search.query {
            ensure!(q.len() <= 4096, "query too long");
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
            ensure!(cursor.len() <= 4096, "cursor too long");
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
            uuid(field(item, "repoId")?)?;
            slug(field(item, "slug")?)?;
            // Match tagged icon policy: only content-addressed same-origin PNGs.
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
        }
        Ok(page)
    }
    pub fn versions(&mut self, installed: &[InstalledRelease]) -> Result<Vec<VersionMetadata>> {
        ensure!(installed.len() <= 200, "version batch exceeds 200");
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
    /// Resolve latest metadata afresh, so stale UI IDs and yanked/hidden releases
    /// cannot silently authorize an install. Download/verification/activation is a host hook.
    pub fn install_plan(&mut self, requested: InstalledRelease) -> Result<InstallPlan> {
        let versions = self.versions(std::slice::from_ref(&requested))?;
        let item = &versions[0];
        ensure!(item.state == "ok", "repository is unavailable");
        let latest = item.latest.as_ref().context("no installable release")?;
        let release: ReleaseMetadata = self.request(
            &format!("/v1/store/releases/{}", latest.release_id),
            vec![],
            None,
        )?;
        ensure!(
            release.id == latest.release_id
                && release.repo_id == requested.repo_id
                && release.yanked_at.is_none(),
            "release identity/state mismatch"
        );
        version(&release.version)?;
        sha1(&release.tree_sha)?;
        validate_hash(&release.tar_sha256)?;
        ensure!(
            release.version == latest.version
                && release.tree_sha == latest.tree_sha
                && release.tar_sha256 == latest.tar_sha256
                && release.api_version == latest.api_version,
            "release changed; refresh metadata"
        );
        ensure!(
            (1..=4).contains(&release.api_version)
                && release.file_count <= 400
                && release.size_bytes <= 8 * 1024 * 1024,
            "unsupported release limits/API"
        );
        slug(field(&release.manifest, "id")?)?;
        ensure!(
            field(&release.manifest, "version")? == release.version
                && release.manifest.get("apiVersion").and_then(Value::as_u64)
                    == Some(release.api_version as u64),
            "manifest metadata mismatch"
        );
        let tree: ReleaseTree = self.request(
            &format!("/v1/store/releases/{}/tree", release.id),
            vec![],
            None,
        )?;
        ensure!(
            tree.tree_sha == release.tree_sha && tree.files.len() == release.file_count as usize,
            "release tree mismatch"
        );
        let mut paths = std::collections::BTreeSet::new();
        let mut total = 0u64;
        for file in &tree.files {
            safe_release_path(&file.path)?;
            sha1(&file.sha)?;
            ensure!(
                paths.insert(file.path.to_lowercase()) && file.size <= 2 * 1024 * 1024,
                "duplicate/oversized release file"
            );
            total = total.checked_add(file.size).context("tree size overflow")?;
        }
        ensure!(total == release.size_bytes, "release size mismatch");
        Ok(InstallPlan {
            origin: self.origin.to_string(),
            release,
            tree,
        })
    }
    /// Metadata only. Local modifications/forks must go through the extension host's merge path.
    pub fn update_plan(
        &mut self,
        installed: InstalledRelease,
        local_id: &str,
    ) -> Result<InstallPlan> {
        component(local_id)?;
        let plan = self.install_plan(installed.clone())?;
        ensure!(plan.release.id != installed.release_id, "no newer release");
        ensure!(
            field(&plan.release.manifest, "id")? == local_id,
            "update changes extension id"
        );
        Ok(plan)
    }
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
pub fn safe_release_path(path: &str) -> Result<()> {
    ensure!(
        !path.is_empty()
            && path.len() <= 200
            && !path.contains(['\\', ':', '\0'])
            && !path.chars().any(char::is_control),
        "invalid release path"
    );
    for part in path.split('/') {
        ensure!(
            !part.is_empty() && part != "." && part != ".." && !part.ends_with(['.', ' ']),
            "unsafe release path"
        );
    }
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
