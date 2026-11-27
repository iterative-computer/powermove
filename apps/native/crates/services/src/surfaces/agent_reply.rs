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
/// An image, video or sound a reply names (main's agent markdown media).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyMedia {
    pub kind: MediaKind,
    /// The local path, resolved, or the `https` address as written.
    pub path: String,
    /// The alt text, or the file name.
    pub name: String,
    /// A web address: opened only when the person asks.
    pub remote: bool,
}
impl ReplyMedia {
    pub fn kind_name(&self) -> &'static str {
        match self.kind {
            MediaKind::Image => "image",
            MediaKind::Video => "video",
            MediaKind::Audio => "audio",
        }
    }
}
fn file_name(value: &str) -> String {
    let path = value.split(['?', '#']).next().unwrap_or(value);
    path.rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or(value)
        .to_owned()
}
/// main's `mediaRun`: a local media path anywhere it's written; a web
/// address only when written as an image (`![alt](https://…)`, `<img>`).
fn media_at(target: &str, alt: &str, image: bool, home: &Path) -> Option<ReplyMedia> {
    let name = |path: &str| {
        if alt.is_empty() {
            file_name(path)
        } else {
            alt.to_owned()
        }
    };
    if let Some(local) = local_media(target, home) {
        let path = local.to_string_lossy().into_owned();
        return Some(ReplyMedia {
            kind: media_type(&local)?.0,
            name: name(&path),
            path,
            remote: false,
        });
    }
    let target = target.trim();
    if image
        && target.len() > 8
        && target
            .get(..8)
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"))
    {
        let bare = target.split(['?', '#']).next().unwrap_or(target);
        return Some(ReplyMedia {
            kind: media_type(Path::new(bare)).map_or(MediaKind::Image, |(kind, _)| kind),
            name: name(target),
            path: target.to_owned(),
            remote: true,
        });
    }
    None
}
/// The media a reply's prose names, in reading order, once per path:
/// `![alt](path)`, `[text](path)`, `<img src>`, a path in backticks, or a
/// bare absolute or `~/` path. Fenced code is code, never media.
pub fn reply_media(text: &str, home: &Path) -> Vec<ReplyMedia> {
    use regex::Regex;
    use std::sync::LazyLock;
    static LINK: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"^(!?)\[((?:[^\]\[]|\[[^\]\[]*\])*)\]\((?:<([^<>\n]+)>|((?:[^()\s<>]|\([^()\s]*\))+))(?:\s+"[^"]*")?\)"#).unwrap()
    });
    static IMG: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)^<img\s[^<>]*?src=["']([^"']+)["'][^<>]*>"#).unwrap());
    static ALT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)alt=["']([^"']*)["']"#).unwrap());
    static BARE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)^(?:~/|/)[^\s<>()\[\]`'"]*\.(?:png|jpe?g|gif|webp|avif|svg|bmp|mp4|m4v|mov|webm|mp3|m4a|wav|aac|ogg|flac)"#).unwrap()
    });
    fn scan(text: &str, home: &Path, out: &mut Vec<ReplyMedia>) {
        let push = |media: Option<ReplyMedia>, out: &mut Vec<ReplyMedia>| {
            if let Some(media) = media
                && !out.iter().any(|m| m.path == media.path)
            {
                out.push(media);
            }
        };
        let mut i = 0;
        while i < text.len() {
            let rest = &text[i..];
            let prev = text[..i].chars().next_back().unwrap_or(' ');
            if let Some(escaped) = rest.strip_prefix('\\') {
                i += 1 + escaped.chars().next().map_or(0, char::len_utf8);
                continue;
            }
            if rest.starts_with('`') {
                let ticks = rest.len() - rest.trim_start_matches('`').len();
                let fence = &rest[..ticks];
                if let Some(end) = rest[ticks..].find(fence) {
                    let inner = &rest[ticks..ticks + end];
                    push(media_at(inner.trim(), "", false, home), out);
                    i += ticks + end + ticks;
                } else {
                    i += ticks;
                }
                continue;
            }
            if let Some(m) = IMG.captures(rest) {
                let alt = ALT
                    .captures(&m[0])
                    .map_or(String::new(), |a| a[1].to_owned());
                push(media_at(&m[1], &alt, true, home), out);
                i += m[0].len();
                continue;
            }
            if let Some(m) = LINK.captures(rest) {
                let image = !m[1].is_empty();
                let target = m.get(3).or(m.get(4)).map_or("", |t| t.as_str());
                let label = &m[2];
                // A linked image ([![alt](src)](url)) shows the picture too.
                if !image {
                    scan(label, home, out);
                }
                push(
                    media_at(target, if image { label } else { "" }, image, home),
                    out,
                );
                i += m[0].len();
                continue;
            }
            if matches!(prev, ' ' | '\t' | '\n' | '(' | '[' | '{' | '\'' | '"')
                && let Some(m) = BARE.find(rest)
                && !rest[m.end()..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '/')
            {
                push(media_at(m.as_str(), "", false, home), out);
                i += m.end();
                continue;
            }
            i += rest.chars().next().map_or(1, char::len_utf8);
        }
    }
    let mut out = Vec::new();
    let mut prose = String::new();
    let mut fenced = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
        } else if !fenced {
            prose.push_str(line);
            prose.push('\n');
        }
    }
    scan(&prose, home, &mut out);
    out
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
    fn reply_media_finds_what_main_rendered_in_reading_order_once() {
        let home = Path::new("/Users/p");
        let text = "Before ![before](/tmp/before.png) and `~/out/render.mov`.\n\
            See /tmp/clip.mp3, [the take](file:///tmp/take%201.wav) and \
            ![remote](https://example.com/a.webp?x=1) and \
            <img src=\"/tmp/i.gif\" alt=\"Logo\">. Again /tmp/before.png.\n\
            ```\n/tmp/in-code.png\n```\n\
            Not media: [web](https://example.com/b.png), /tmp/notes.txt, \
            ../x.png, a/tmp/y.png, /tmp/z.pngs, \\/tmp/escaped.png";
        let found: Vec<_> = reply_media(text, home)
            .into_iter()
            .map(|m| (m.kind_name(), m.path, m.name, m.remote))
            .collect();
        assert_eq!(
            found,
            [
                ("image", "/tmp/before.png".into(), "before".into(), false),
                (
                    "video",
                    "/Users/p/out/render.mov".into(),
                    "render.mov".into(),
                    false
                ),
                ("audio", "/tmp/clip.mp3".into(), "clip.mp3".into(), false),
                (
                    "audio",
                    "/tmp/take 1.wav".into(),
                    "take 1.wav".into(),
                    false
                ),
                (
                    "image",
                    "https://example.com/a.webp?x=1".into(),
                    "remote".into(),
                    true
                ),
                ("image", "/tmp/i.gif".into(), "Logo".into(), false),
            ]
        );
    }
    #[test]
    fn reply_media_never_panics_on_any_unicode_reply() {
        // Pieces the parser reacts to, mixed with multi-byte characters.
        const PIECES: [&str; 22] = [
            "![",
            "](",
            "[",
            "]",
            "(",
            ")",
            "`",
            "``",
            "<img src=\"",
            "\">",
            "\\",
            "/",
            "~/",
            ".png",
            "https://",
            "你好",
            "é",
            "🎬",
            "\u{301}",
            " ",
            "\n",
            "```\n",
        ];
        let home = Path::new("/Users/p");
        assert!(reply_media("![x](你好你好你好)", home).is_empty());
        assert!(reply_media("![x](https你好你好)", home).is_empty());
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..20_000 {
            let mut text = String::new();
            for _ in 0..next() % 24 {
                let pick = next();
                if pick % 3 == 0 {
                    text.push(char::from_u32((next() % 0x2_0000) as u32).unwrap_or('\u{fffd}'));
                } else {
                    text.push_str(PIECES[(pick % PIECES.len() as u64) as usize]);
                }
            }
            let _ = reply_media(&text, home);
        }
    }
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
