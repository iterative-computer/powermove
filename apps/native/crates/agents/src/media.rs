//! Agent reply media: inert bytes behind a native file grant. Parsing a path or
//! seeing it in markdown never grants filesystem authority or remote-image IO.
use crate::error;
use serde_json::{Value, json};
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::Path,
};
const TYPES: [(&str, &str, &str); 18] = [
    ("png", "image", "image/png"),
    ("jpg", "image", "image/jpeg"),
    ("jpeg", "image", "image/jpeg"),
    ("gif", "image", "image/gif"),
    ("webp", "image", "image/webp"),
    ("avif", "image", "image/avif"),
    ("svg", "image", "image/svg+xml"),
    ("bmp", "image", "image/bmp"),
    ("mp4", "video", "video/mp4"),
    ("m4v", "video", "video/mp4"),
    ("mov", "video", "video/quicktime"),
    ("webm", "video", "video/webm"),
    ("mp3", "audio", "audio/mpeg"),
    ("m4a", "audio", "audio/mp4"),
    ("wav", "audio", "audio/wav"),
    ("aac", "audio", "audio/aac"),
    ("ogg", "audio", "audio/ogg"),
    ("flac", "audio", "audio/flac"),
];
pub fn extension(value: &str) -> String {
    let path = value.split(['?', '#']).next().unwrap_or_default();
    path.rsplit_once('.')
        .map(|(_, s)| s)
        .filter(|s| !s.is_empty() && s.bytes().all(|v| v.is_ascii_alphanumeric()))
        .unwrap_or_default()
        .to_ascii_lowercase()
}
pub fn kind(value: &str) -> Option<&'static str> {
    let ext = extension(value);
    TYPES.iter().find(|v| v.0 == ext).map(|v| v.1)
}
pub fn mime(value: &str) -> Option<&'static str> {
    let ext = extension(value);
    TYPES.iter().find(|v| v.0 == ext).map(|v| v.2)
}
fn decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = vec![];
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let code = value.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(code, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}
pub fn local_path(value: &str) -> Option<String> {
    let mut value = value.trim().to_owned();
    if value.to_ascii_lowercase().starts_with("file://") {
        value = decode(url::Url::parse(&value).ok()?.path())?;
    }
    if !(value.starts_with('/') || value.starts_with("~/"))
        || value.split('/').any(|v| v == "..")
        || kind(&value).is_none()
    {
        return None;
    }
    Some(value)
}
pub fn file_url(value: &str) -> String {
    let encoded = value
        .as_bytes()
        .iter()
        .map(|v| {
            if v.is_ascii_alphanumeric() || b"-_.!~*'()".contains(v) {
                char::from(*v).to_string()
            } else {
                format!("%{v:02X}")
            }
        })
        .collect::<String>();
    format!("app://powermove/_agent-file?path={encoded}")
}
pub fn parse_file(request: &str, home: &Path) -> Option<std::path::PathBuf> {
    let url = url::Url::parse(request).ok()?;
    let raw = url.query_pairs().find(|(k, _)| k == "path")?.1.into_owned();
    let path = if let Some(suffix) = raw.strip_prefix("~/") {
        home.join(suffix)
    } else {
        raw.into()
    };
    if !path.is_absolute()
        || path.to_string_lossy().contains('\0')
        || path.to_string_lossy().split(['\\', '/']).any(|s| s == "..")
        || mime(&path.to_string_lossy()).is_none()
    {
        return None;
    }
    Some(path)
}
#[derive(Debug, Clone, PartialEq)]
pub struct Range {
    pub status: u16,
    pub start: u64,
    pub length: u64,
    pub headers: Value,
    pub body: bool,
}
pub fn range(size: u64, method: &str, requested: Option<&str>, mime: &str) -> Range {
    let mut headers = json!({"Content-Type":mime,"Cache-Control":"no-store","Content-Security-Policy":"default-src 'none'; sandbox","X-Content-Type-Options":"nosniff","Accept-Ranges":"bytes"});
    let mut start = 0;
    let mut end = size.saturating_sub(1);
    let mut status = 200;
    if let Some(requested) = requested {
        let valid = regex::Regex::new(r"^bytes=(\d+)-(\d*)$").unwrap();
        let parsed = valid.captures(requested.trim()).and_then(|m| {
            let start = m[1].parse::<u64>().ok()?;
            let end = if m[2].is_empty() {
                size.saturating_sub(1)
            } else {
                m[2].parse::<u64>()
                    .unwrap_or(u64::MAX)
                    .min(size.saturating_sub(1))
            };
            (start <= 9_007_199_254_740_991 && start < size && end >= start).then_some((start, end))
        });
        let Some((from, to)) = parsed else {
            headers["Content-Range"] = json!(format!("bytes */{size}"));
            return Range {
                status: 416,
                start: 0,
                length: 0,
                headers,
                body: false,
            };
        };
        start = from;
        end = to.min(start.saturating_add(4 * 1024 * 1024 - 1));
        status = 206;
        headers["Content-Range"] = json!(format!("bytes {start}-{end}/{size}"));
    }
    let length = if size == 0 { 0 } else { end - start + 1 };
    headers["Content-Length"] = json!(length.to_string());
    Range {
        status,
        start,
        length,
        headers,
        body: method != "HEAD" && size > 0,
    }
}
pub struct Grant {
    file: File,
    size: u64,
    mime: &'static str,
}
impl Grant {
    /// The actor must authorize this root/run/document before calling. The open
    /// file pins one regular inode; every ancestor is descriptor-relative and
    /// no user symlink is followed, including intermediate components.
    pub fn open(root: &Path, relative: &Path) -> io::Result<Self> {
        let name = relative.to_string_lossy();
        let mime = mime(&name).ok_or_else(|| error("Unsupported media extension"))?;
        let root = root.canonicalize()?;
        let file = crate::workspace::open_relative(&root, relative, false, false)?;
        let info = file.metadata()?;
        if !info.is_file() {
            return Err(error("Media grant is not a regular file"));
        }
        Ok(Self {
            file,
            size: info.len(),
            mime,
        })
    }
    pub fn response(&self, method: &str, requested: Option<&str>) -> Range {
        range(self.size, method, requested, self.mime)
    }
    /// Bounded chunks support full-file streaming without buffering a movie.
    pub fn read(&mut self, offset: u64, length: usize) -> io::Result<Vec<u8>> {
        if length > 4 * 1024 * 1024
            || offset > self.size
            || self.file.metadata()?.len() != self.size
        {
            return Err(error("Invalid or changed media grant"));
        }
        let count = (self.size - offset).min(length as u64) as usize;
        let mut bytes = vec![0; count];
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.read_exact(&mut bytes)?;
        Ok(bytes)
    }
}

/// The application endpoint keeps this wrapper, never an unscoped raw grant.
/// Every read/capture rechecks connection and document identity.
pub struct BoundGrant {
    lease: crate::session::Lease,
    grant: Grant,
}
impl BoundGrant {
    pub fn open(
        gate: &crate::session::Gate,
        lease: &crate::session::Lease,
        root: &Path,
        relative: &Path,
    ) -> io::Result<Self> {
        gate.check(lease)?;
        let grant = Grant::open(root, relative)?;
        gate.check(lease)?;
        Ok(Self {
            lease: lease.clone(),
            grant,
        })
    }
    pub fn response(
        &self,
        gate: &crate::session::Gate,
        method: &str,
        range: Option<&str>,
    ) -> io::Result<Range> {
        gate.check(&self.lease)?;
        Ok(self.grant.response(method, range))
    }
    pub fn read(
        &mut self,
        gate: &crate::session::Gate,
        offset: u64,
        length: usize,
    ) -> io::Result<Vec<u8>> {
        gate.check(&self.lease)?;
        self.grant.read(offset, length)
    }
}
