//! A04/A05 backing at main c7f46a0564, work-groups.ts/shared/agent-media.ts.
//! Non-visual grouping, timer, inert media and range plans. No HTML execution.
//! Rich Markdown rendering remains the existing panel's responsibility; this
//! module preserves every opaque message/step rather than rewriting its prose.
use super::{DocumentStamp, Flight, Ticket};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct ConversationItem {
    pub index: usize,
    pub message: Value,
    pub key: String,
}
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    Message(ConversationItem),
    Work {
        key: String,
        duration_ms: Option<f64>,
        items: Vec<ConversationItem>,
    },
}
fn item(index: usize, message: &Value, part: &str) -> ConversationItem {
    ConversationItem {
        index,
        message: message.clone(),
        key: format!("{}-{index}{part}", message["role"].as_str().unwrap_or("")),
    }
}
fn prompt(message: &Value) -> bool {
    message["role"] == "user" && message["steering"] != true
}
fn split_trace(
    index: usize,
    message: &Value,
) -> (Option<ConversationItem>, Option<ConversationItem>) {
    let steps = message["steps"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let cut = steps
        .iter()
        .rposition(|s| s["kind"] != "text")
        .map_or(0, |i| i + 1);
    let slice = |steps: &[Value], part| {
        let mut next = message.clone();
        next["steps"] = Value::Array(steps.to_vec());
        item(index, &next, part)
    };
    (
        (!steps[..cut].is_empty()).then(|| slice(&steps[..cut], ":work")),
        (!steps[cut..].is_empty()).then(|| slice(&steps[cut..], ":reply")),
    )
}
fn fold(
    conversation: &[Value],
    body: &[usize],
) -> Option<(Vec<ConversationItem>, Vec<ConversationItem>, Option<f64>)> {
    let last = *body.last()?;
    let message = &conversation[last];
    let mut work = vec![];
    let mut reply = vec![];
    let mut end = body.len();
    if message["role"] == "assistant" {
        reply.push(item(last, message, ""));
        end -= 1;
        if let Some(before) = body.get(end.checked_sub(1).unwrap_or(usize::MAX)).copied()
            && conversation[before]["role"] == "trace"
            && conversation[before]["steering"] != true
        {
            let (steps, text) = split_trace(before, &conversation[before]);
            if let Some(text) = text {
                end -= 1;
                if let Some(steps) = steps {
                    work.push(steps);
                }
                reply.insert(0, text);
            }
        }
    } else if message["role"] == "trace" {
        let (steps, text) = split_trace(last, message);
        let text = text?;
        end -= 1;
        if let Some(steps) = steps {
            work.push(steps);
        }
        reply.push(text);
    } else {
        return None;
    }
    let mut earlier = body[..end]
        .iter()
        .map(|i| item(*i, &conversation[*i], ""))
        .collect::<Vec<_>>();
    earlier.extend(work);
    work = earlier;
    if work.is_empty() {
        return None;
    }
    let duration = body
        .iter()
        .map(|i| &conversation[*i])
        .filter(|m| m["role"] == "trace")
        .filter_map(|m| m["durationMs"].as_f64())
        .filter(|n| n.is_finite())
        .max_by(f64::total_cmp);
    Some((work, reply, duration))
}
pub fn group_conversation(conversation: &[Value], live_from: Option<usize>) -> Vec<Entry> {
    let mut entries = vec![];
    let mut index = 0;
    while index < conversation.len() && !prompt(&conversation[index]) {
        entries.push(Entry::Message(item(index, &conversation[index], "")));
        index += 1;
    }
    while index < conversation.len() {
        let start = index;
        let end = (start + 1..conversation.len())
            .find(|i| prompt(&conversation[*i]))
            .unwrap_or(conversation.len());
        index = end;
        entries.push(Entry::Message(item(start, &conversation[start], "")));
        let body = (start + 1..end).collect::<Vec<_>>();
        let live = live_from.is_some_and(|i| i >= start && i < end);
        if let Some((work, reply, duration_ms)) =
            (!live).then(|| fold(conversation, &body)).flatten()
        {
            entries.push(Entry::Work {
                key: format!("work-{start}"),
                duration_ms,
                items: work,
            });
            entries.extend(reply.into_iter().map(Entry::Message));
        } else {
            entries.extend(
                body.iter()
                    .map(|i| Entry::Message(item(*i, &conversation[*i], ""))),
            );
        }
    }
    entries
}
#[derive(Debug, Default)]
pub struct WorkingTimer {
    run: Option<String>,
    started: u64,
    elapsed: u64,
}
impl WorkingTimer {
    pub fn observe(&mut self, run: Option<&str>, now: u64) {
        match run {
            Some(id) if self.run.as_deref() != Some(id) => {
                self.run = Some(id.into());
                self.started = now;
                self.elapsed = 0;
            }
            None if self.run.is_some() => {
                self.elapsed = now.saturating_sub(self.started);
                self.run = None;
            }
            _ => {}
        }
    }
    pub fn elapsed(&self, now: u64) -> u64 {
        if self.run.is_some() {
            now.saturating_sub(self.started)
        } else {
            self.elapsed
        }
    }
}

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
    use serde_json::json;
    #[test]
    fn completed_work_splits_trailing_reply_and_retains_opaque_steps() {
        let messages = vec![
            json!({"role":"user","text":"prompt"}),
            json!({"role":"trace","durationMs":1500,"future":1,"steps":[{"kind":"tool","future":2},{"kind":"text","text":"answer"}]}),
            json!({"role":"assistant","text":"notes"}),
        ];
        let grouped = group_conversation(&messages, None);
        assert_eq!(grouped.len(), 4);
        let Entry::Work {
            items, duration_ms, ..
        } = &grouped[1]
        else {
            panic!("expected folded work");
        };
        assert_eq!(*duration_ms, Some(1500.));
        assert_eq!(items[0].message["future"], 1);
        assert_eq!(items[0].message["steps"][0]["future"], 2);
        let Entry::Message(reply) = &grouped[2] else {
            panic!("expected reply");
        };
        assert_eq!(reply.key, "trace-1:reply");
        assert_eq!(reply.message["steps"][0]["text"], "answer");
    }
    #[test]
    fn live_turn_and_silent_failed_turn_keep_work_open() {
        let messages = vec![
            json!({"role":"user"}),
            json!({"role":"trace","steps":[{"kind":"tool"}]}),
        ];
        assert_eq!(group_conversation(&messages, None).len(), 2);
        let mut messages = messages;
        messages.push(json!({"role":"assistant","text":"done"}));
        assert!(matches!(
            &group_conversation(&messages, Some(1))[1],
            Entry::Message(_)
        ));
    }
    #[test]
    fn steering_stays_in_original_turn_and_duration_uses_max_trace() {
        let messages = vec![
            json!({"role":"user"}),
            json!({"role":"trace","durationMs":20,"steps":[{"kind":"tool"}]}),
            json!({"role":"user","steering":true}),
            json!({"role":"trace","durationMs":30,"steps":[{"kind":"tool"}]}),
            json!({"role":"assistant"}),
        ];
        let grouped = group_conversation(&messages, None);
        let Entry::Work {
            items, duration_ms, ..
        } = &grouped[1]
        else {
            panic!("work");
        };
        assert_eq!(items.len(), 3);
        assert_eq!(*duration_ms, Some(30.));
        let mut timer = WorkingTimer::default();
        timer.observe(Some("run"), 100);
        timer.observe(Some("run"), 200);
        assert_eq!(timer.elapsed(300), 200);
        timer.observe(None, 400);
        assert_eq!(timer.elapsed(500), 300);
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
