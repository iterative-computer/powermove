//! A04/A05 backing at main c7f46a0564, shared/agent-media.ts: local media grants,
//! inert remote media and byte-range plans for agent replies. No HTML execution.
//! Conversation grouping lives in pm-panel-agentpanel (work_groups).
use super::{DocumentStamp, Flight, Ticket};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaKind {
    Image,
    Video,
    Audio,
}
pub fn media_type(path: &Path) -> Option<(MediaKind, &'static str)> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => (MediaKind::Image, "image/png"),
        "jpg" | "jpeg" => (MediaKind::Image, "image/jpeg"),
        "gif" => (MediaKind::Image, "image/gif"),
        "webp" => (MediaKind::Image, "image/webp"),
        "avif" => (MediaKind::Image, "image/avif"),
        "svg" => (MediaKind::Image, "image/svg+xml"),
        "bmp" => (MediaKind::Image, "image/bmp"),
        "mp4" | "m4v" => (MediaKind::Video, "video/mp4"),
        "mov" => (MediaKind::Video, "video/quicktime"),
        "webm" => (MediaKind::Video, "video/webm"),
        "mp3" => (MediaKind::Audio, "audio/mpeg"),
        "m4a" => (MediaKind::Audio, "audio/mp4"),
        "wav" => (MediaKind::Audio, "audio/wav"),
        "aac" => (MediaKind::Audio, "audio/aac"),
        "ogg" => (MediaKind::Audio, "audio/ogg"),
        "flac" => (MediaKind::Audio, "audio/flac"),
        _ => return None,
    })
}
pub fn local_media(raw: &str, home: &Path) -> Option<PathBuf> {
    let value = raw.trim();
    if value.contains('\0') {
        return None;
    }
    let path = if value.to_ascii_lowercase().starts_with("file://") {
        let url = url::Url::parse(value).ok()?;
        if !url.username().is_empty()
            || url.password().is_some()
            || !matches!(url.host_str(), None | Some("" | "localhost"))
        {
            return None;
        }
        url.to_file_path().ok()?
    } else if let Some(relative) = value.strip_prefix("~/") {
        if !home.is_absolute() {
            return None;
        }
        home.join(relative)
    } else {
        PathBuf::from(value)
    };
    if !path.is_absolute()
        || path.components().any(|c| matches!(c, Component::ParentDir))
        || media_type(&path).is_none()
    {
        return None;
    }
    Some(path)
}
pub fn safe_link(raw: &str) -> bool {
    url::Url::parse(raw).is_ok_and(|u| {
        matches!(u.scheme(), "http" | "https")
            && u.host_str().is_some()
            && u.username().is_empty()
            && u.password().is_none()
    })
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub start: u64,
    pub length: u64,
    pub partial: bool,
}
pub fn byte_range(header: Option<&str>, size: u64) -> Option<Range> {
    let Some(header) = header else {
        return Some(Range {
            start: 0,
            length: size,
            partial: false,
        });
    };
    let (a, b) = header.trim().strip_prefix("bytes=")?.split_once('-')?;
    if a.is_empty()
        || !a.bytes().all(|b| b.is_ascii_digit())
        || !b.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let start = a.parse::<u64>().ok()?;
    if start >= size {
        return None;
    }
    let end = if b.is_empty() {
        size - 1
    } else {
        b.parse::<u64>().ok()?.min(size - 1)
    };
    if end < start {
        return None;
    }
    Some(Range {
        start,
        length: (end - start + 1).min(4 * 1024 * 1024),
        partial: true,
    })
}

/// Remote images remain inert until the person asks. Grant IDs also bind the
/// document lifetime; a stale remote completion cannot attach to another turn.
#[derive(Debug, Default)]
pub struct RemoteMedia {
    flight: Flight,
    target: Option<String>,
    stamp: Option<DocumentStamp>,
    pub loaded: bool,
}
impl RemoteMedia {
    pub fn request(
        &mut self,
        url: &str,
        stamp: DocumentStamp,
        user_requested: bool,
    ) -> Option<Ticket> {
        let parsed = url::Url::parse(url).ok()?;
        if !user_requested || parsed.scheme() != "https" || !safe_link(url) {
            return None;
        }
        self.target = Some(url.into());
        self.stamp = Some(stamp);
        self.loaded = false;
        Some(self.flight.start())
    }
    pub fn receive(
        &mut self,
        ticket: Ticket,
        url: &str,
        current: &DocumentStamp,
        loaded: bool,
    ) -> bool {
        if !self.flight.finish(ticket)
            || self.target.as_deref() != Some(url)
            || self.stamp.as_ref() != Some(current)
        {
            return false;
        }
        self.loaded = loaded;
        true
    }
    pub fn close(&mut self) {
        self.flight.cancel();
        self.loaded = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_media_requires_supported_absolute_paths_and_explicit_home() {
        let home = std::env::temp_dir();
        assert_eq!(
            local_media("~/frame.png", &home),
            Some(home.join("frame.png"))
        );
        assert!(local_media("relative.png", &home).is_none());
        assert!(local_media("~/../private.png", &home).is_none());
        assert!(local_media("/private/file.txt", &home).is_none());
        assert!(local_media("file://other.example/a.png", &home).is_none());
        assert_eq!(
            media_type(Path::new("clip.MOV")),
            Some((MediaKind::Video, "video/quicktime"))
        );
    }
    #[test]
    fn media_ranges_match_main_caps_and_refuse_suffix_or_invalid_ranges() {
        assert_eq!(
            byte_range(Some("bytes=1-"), 10),
            Some(Range {
                start: 1,
                length: 9,
                partial: true
            })
        );
        assert_eq!(
            byte_range(Some("bytes=0-9999999"), 10_000_000)
                .unwrap()
                .length,
            4 * 1024 * 1024
        );
        for range in ["bytes=-5", "bytes=10-11", "bytes=5-4", "bytes=0-1,2-3"] {
            assert!(byte_range(Some(range), 10).is_none());
        }
        assert_eq!(
            byte_range(None, 0),
            Some(Range {
                start: 0,
                length: 0,
                partial: false
            })
        );
    }
    #[test]
    fn remote_media_needs_explicit_request_and_current_document() {
        let mut media = RemoteMedia::default();
        let stamp = DocumentStamp {
            project: "p".into(),
            session: 1,
            revision: 1,
        };
        assert!(
            media
                .request("https://example.com/a.png", stamp.clone(), false)
                .is_none()
        );
        assert!(
            media
                .request("http://example.com/a.png", stamp.clone(), true)
                .is_none()
        );
        let t = media
            .request("https://example.com/a.png", stamp.clone(), true)
            .unwrap();
        let mut current = stamp;
        current.session = 2;
        assert!(!media.receive(t, "https://example.com/a.png", &current, true));
        assert!(!media.loaded);
        for url in [
            "javascript:alert(1)",
            "file:///private/a",
            "https://user@example.com",
        ] {
            assert!(!safe_link(url));
        }
    }
}
