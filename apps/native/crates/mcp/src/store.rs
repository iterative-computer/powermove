//! Frozen Store read adapters. Transport is a host capability, shared with the
//! native Store service; MCP never discovers credentials or owns an HTTP client.
use crate::{Error, Result};
use serde::Deserialize;
use serde_json::{Value, json};

pub const JSON_LIMIT: usize = 4 * 1024 * 1024;
pub const FILE_LIMIT: usize = 2 * 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// Origin-relative path, with every caller-supplied segment percent encoded.
    pub path: String,
    pub query: Vec<(String, String)>,
    pub max_bytes: usize,
}
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}
pub trait StoreRead: Send + Sync {
    /// Worker-only. Enforce max_bytes while streaming, same-origin redirects,
    /// TLS and the application's Store origin/auth policy. Never echo secrets
    /// or a server response body in transport errors.
    fn get(&self, request: Request) -> Result<Response>;
}
fn upstream(message: &str) -> Error {
    Error {
        code: "store_error".into(),
        message: message.into(),
        backing: Some("native Store transport".into()),
    }
}
fn required_option<'de, D, T>(deserializer: D) -> std::result::Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::deserialize(deserializer)
}
fn segment(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
fn coordinate(s: &str) -> Result<String> {
    if s.is_empty() || [".", ".."].contains(&s) || s.len() > 1024 {
        return Err(Error::invalid("Invalid Store coordinate"));
    }
    Ok(segment(s))
}
fn get(
    host: &dyn StoreRead,
    path: String,
    query: Vec<(String, String)>,
    max_bytes: usize,
) -> Result<Vec<u8>> {
    let response = host.get(Request {
        path,
        query,
        max_bytes,
    })?;
    if response.body.len() > max_bytes {
        return Err(upstream("Store response exceeds the byte limit"));
    }
    if response.status != 200 {
        return Err(upstream(&format!(
            "Store request failed (HTTP {})",
            response.status
        )));
    }
    Ok(response.body)
}
fn read<T: serde::de::DeserializeOwned>(
    host: &dyn StoreRead,
    path: String,
    query: Vec<(String, String)>,
) -> Result<T> {
    serde_json::from_slice(&get(host, path, query, JSON_LIMIT)?)
        .map_err(|_| upstream("Invalid Store metadata response"))
}
#[derive(Deserialize)]
struct Owner {
    handle: String,
    #[serde(default)]
    verified: bool,
}
#[derive(Deserialize)]
struct Latest {
    id: String,
    version: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Listing {
    owner: Owner,
    slug: String,
    name: String,
    tagline: String,
    category: String,
    install_count: u64,
    permissions: Vec<String>,
    #[serde(deserialize_with = "required_option")]
    latest: Option<Latest>,
}
impl Listing {
    fn view(&self) -> Value {
        json!({"handle":self.owner.handle,"slug":self.slug,"name":self.name,"tagline":self.tagline,
            "category":self.category,"installs":self.install_count,"verified":self.owner.verified,
            "permissions":self.permissions,"latest":self.latest.as_ref().map(|l|json!({"releaseId":l.id,"version":l.version}))})
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Page {
    items: Vec<Listing>,
    #[serde(deserialize_with = "required_option")]
    next_cursor: Option<String>,
}
#[derive(Deserialize)]
struct Manifest {
    permissions: Vec<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Release {
    id: String,
    version: String,
    api_version: u32,
    published_at: String,
    #[serde(deserialize_with = "required_option")]
    yanked_at: Option<String>,
    file_count: u32,
    size_bytes: u64,
    manifest: Manifest,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Detail {
    #[serde(flatten)]
    listing: Listing,
    repo_id: String,
    #[serde(deserialize_with = "required_option")]
    about: Option<String>,
    releases: Vec<Release>,
}
#[derive(Deserialize)]
struct File {
    path: String,
    size: u64,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Tree {
    tree_sha: String,
    files: Vec<File>,
}
#[derive(Deserialize)]
struct SourceRelease {
    handle: String,
    slug: String,
    version: String,
}

pub fn search(host: &dyn StoreRead, args: &Value) -> Result<Value> {
    let query = [
        ("query", "q"),
        ("category", "category"),
        ("sort", "sort"),
        ("cursor", "cursor"),
    ]
    .into_iter()
    .filter_map(|(input, output)| {
        args[input]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(|s| (output.into(), s.into()))
    })
    .collect();
    let page: Page = read(host, "/v1/store/extensions".into(), query)?;
    Ok(
        json!({"items":page.items.iter().map(Listing::view).collect::<Vec<_>>(),"nextCursor":page.next_cursor}),
    )
}
pub fn detail(host: &dyn StoreRead, args: &Value) -> Result<Value> {
    let path = format!(
        "/v1/store/x/{}/{}",
        coordinate(args["handle"].as_str().unwrap())?,
        coordinate(args["slug"].as_str().unwrap())?
    );
    let detail: Detail = read(host, path, vec![])?;
    let mut result = detail.listing.view();
    result["repoId"] = json!(detail.repo_id);
    result["about"] = json!(detail.about);
    result["latestReleaseId"] = json!(detail.listing.latest.as_ref().map(|l| &l.id));
    result["releases"] = json!(detail.releases.into_iter().map(|r|json!({"releaseId":r.id,"version":r.version,"apiVersion":r.api_version,
        "publishedAt":r.published_at,"yanked":r.yanked_at.is_some(),"fileCount":r.file_count,"sizeBytes":r.size_bytes,"permissions":r.manifest.permissions})).collect::<Vec<_>>());
    Ok(result)
}
pub fn source(host: &dyn StoreRead, args: &Value) -> Result<Value> {
    let release = coordinate(args["releaseId"].as_str().unwrap())?;
    let base = format!("/v1/store/releases/{release}");
    let Some(path) = args["path"].as_str().filter(|s| !s.is_empty()) else {
        let tree: Tree = read(host, format!("{base}/tree"), vec![])?;
        return Ok(
            json!({"treeSha":tree.tree_sha,"files":tree.files.into_iter().map(|f|json!({"path":f.path,"size":f.size})).collect::<Vec<_>>()}),
        );
    };
    if path.starts_with('/')
        || path.contains('\\')
        || path.contains('\0')
        || path
            .split('/')
            .any(|p| p.is_empty() || [".", ".."].contains(&p))
    {
        return Err(Error::invalid("Invalid Store source path"));
    }
    let release: SourceRelease = read(host, base, vec![])?;
    let url = format!(
        "/v1/store/x/{}/{}/r/{}/file/{}",
        coordinate(&release.handle)?,
        coordinate(&release.slug)?,
        coordinate(&release.version)?,
        path.split('/').map(segment).collect::<Vec<_>>().join("/")
    );
    let bytes = get(host, url, vec![], FILE_LIMIT)?;
    let text = String::from_utf8_lossy(&bytes);
    Ok(json!({"path":path,"text":text.strip_prefix('\u{feff}').unwrap_or(&text)}))
}
