//! Visual replies (`publish_visual_reply`): main's input limits, its resource
//! rules (shared/visual-reply.ts, panels/agent/visual-reply.ts) and the viewer
//! document with its Content Security Policy. Parsing is conservative: markup
//! this scanner can't read with certainty is refused, never passed through.
use serde_json::Value;

pub const MAX_BYTES: usize = 384 * 1024;
pub const MAX_PER_RUN: usize = 4;
pub const CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:; font-src 'none'; connect-src 'none'; media-src 'none'; worker-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'";
const LIMITS: &str = "Visual reply requires a title (1–120 characters), HTML up to 384 KiB and height from 160 to 900.";
const EMBEDS: &str = "Visual replies cannot embed pages, forms, metadata redirects or external resources. Use inline CSS, SVG and scripts.";
const RESOURCES: &str = "Visual reply resources must be embedded raster images (up to 192 KiB each) or fragment links. File paths and remote links are unavailable.";

#[derive(Clone, Debug, PartialEq)]
pub struct Prepared {
    pub title: String,
    pub html: String,
    pub height: u32,
}
/// Main's `prepareVisualReply`: limits, then resources, then the normalized
/// document (`<!doctype html><html><head></head><body>…</body></html>`).
pub fn prepare(value: &Value) -> Result<Prepared, String> {
    let object = value.as_object().ok_or(LIMITS)?;
    if object
        .keys()
        .any(|k| !matches!(k.as_str(), "title" | "html" | "height"))
    {
        return Err(LIMITS.into());
    }
    let title = object.get("title").and_then(Value::as_str).ok_or(LIMITS)?;
    let html = object.get("html").and_then(Value::as_str).ok_or(LIMITS)?;
    let height = match object.get("height") {
        None => 360,
        Some(h) => h
            .as_u64()
            .filter(|h| (160..=900).contains(h))
            .ok_or(LIMITS)? as u32,
    };
    if title.trim().is_empty()
        || title.encode_utf16().count() > 120
        || html.trim().is_empty()
        || html.len() > MAX_BYTES
    {
        return Err(LIMITS.into());
    }
    let body = scan(html)?;
    let html = format!("<!doctype html><html><head></head><body>{body}</body></html>");
    if html.len() > MAX_BYTES {
        return Err("Normalized visual reply exceeds 384 KiB.".into());
    }
    Ok(Prepared {
        title: title.trim().into(),
        html,
        height,
    })
}
/// A saved reply as main reads it back (`readVisualReply`).
pub fn read(value: &Value) -> Option<Prepared> {
    let id = value["id"].as_str()?;
    let valid_id = id.strip_prefix("visual-").is_some_and(|rest| {
        !rest.is_empty()
            && rest.len() <= 120
            && rest
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
    });
    if value["version"] != 1 || !valid_id || value["runId"].as_str()?.len() > 160 {
        return None;
    }
    let mut input = serde_json::json!({"title":value["title"],"html":value["html"]});
    if !value["height"].is_null() {
        input["height"] = value["height"].clone();
    }
    prepare(&input).ok()
}
/// The viewer page: main's policy meta first, its quiet defaults, and a boot
/// script that keeps links and forms inside the page. No bridge to the app.
pub fn document(html: &str, dark: bool) -> String {
    let (background, foreground, muted, accent, scheme) = if dark {
        ("#202022", "#eee", "#999", "#a7a0ff", "dark")
    } else {
        ("#ffffff", "#1d1d1f", "#6e6e73", "#5e5ce6", "light")
    };
    let boot = "(() => { addEventListener('click', e => { if (e.target.closest?.('a[href]:not([href^=\"#\"])')) { e.preventDefault(); e.stopImmediatePropagation(); } }, true); addEventListener('submit', e => e.preventDefault(), true); })();";
    let prefix = format!(
        "<meta http-equiv=\"Content-Security-Policy\" content=\"{CSP}\"><meta name=\"referrer\" content=\"no-referrer\"><style>:root{{--background:{background};--foreground:{foreground};--muted:{muted};--accent:{accent};color-scheme:{scheme}}}html{{background:var(--background);color:var(--foreground);font:13px system-ui}}body{{margin:0;padding:16px;box-sizing:border-box}}*{{box-sizing:border-box}}button,input,select{{font:inherit}}a{{color:var(--accent)}}</style><script>{boot}</script>"
    );
    match html.find("<head>") {
        Some(at) => format!("{}{prefix}{}", &html[..at + 6], &html[at + 6..]),
        None => format!("<!doctype html><html><head>{prefix}</head><body>{html}</body></html>"),
    }
}

const RAW_TEXT: &[&str] = &[
    "script", "style", "textarea", "title", "xmp", "iframe", "noembed", "noframes", "noscript",
];
const BLOCKED: &[&str] = &[
    "base", "iframe", "frame", "frameset", "object", "embed", "link", "form", "template",
];
const RESOURCE_ATTRS: &[&str] = &[
    "src",
    "srcset",
    "href",
    "xlink:href",
    "action",
    "formaction",
    "poster",
    "background",
    "ping",
];
/// Walk the markup's tags; returns it without document-level tags.
fn scan(html: &str) -> Result<String, String> {
    let bytes = html.as_bytes();
    let mut out = String::with_capacity(html.len());
    let mut i = 0;
    let mut copied = 0;
    while let Some(offset) = html[i..].find('<') {
        let start = i + offset;
        let rest = &html[start..];
        if let Some(comment) = rest.strip_prefix("<!--") {
            let end = comment.find("-->").ok_or(EMBEDS)? + start + 7;
            i = end;
            continue;
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            // A doctype (or bogus comment) is dropped, as template parsing does.
            let end = rest.find('>').ok_or(EMBEDS)? + start + 1;
            out.push_str(&html[copied..start]);
            copied = end;
            i = end;
            continue;
        }
        let closing = rest.starts_with("</");
        let name_start = start + if closing { 2 } else { 1 };
        let name_len = html[name_start..]
            .bytes()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b':' | b'_'))
            .count();
        if name_len == 0 || !bytes[name_start].is_ascii_alphabetic() {
            // Not a tag: literal text.
            i = start + 1;
            continue;
        }
        let name = html[name_start..name_start + name_len].to_ascii_lowercase();
        let (attributes, end) = attributes(html, name_start + name_len)?;
        if !closing {
            if BLOCKED.contains(&name.as_str())
                || name == "meta" && attributes.iter().any(|(k, _)| k == "http-equiv")
            {
                return Err(EMBEDS.into());
            }
            for (key, value) in &attributes {
                if RESOURCE_ATTRS.contains(&key.as_str()) && !allowed(&name, key, value)? {
                    return Err(RESOURCES.into());
                }
            }
        }
        if matches!(name.as_str(), "html" | "head" | "body") {
            out.push_str(&html[copied..start]);
            copied = end;
        }
        i = end;
        if !closing && RAW_TEXT.contains(&name.as_str()) {
            // Its text isn't markup; find its end tag.
            let lower = html[i..].to_ascii_lowercase();
            let close = format!("</{name}");
            i += lower.find(&close).ok_or(EMBEDS)?;
        }
    }
    out.push_str(&html[copied..]);
    Ok(out)
}
/// Attributes from just after a tag name to its `>`; values are decoded only
/// for the characters main's checks look at, so encoded forms are refused.
fn attributes(html: &str, mut i: usize) -> Result<(Vec<(String, String)>, usize), String> {
    let bytes = html.as_bytes();
    let mut out = vec![];
    loop {
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'/') {
            i += 1;
        }
        if i >= bytes.len() {
            return Err(EMBEDS.into());
        }
        if bytes[i] == b'>' {
            return Ok((out, i + 1));
        }
        let key_start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && !b"/>=".contains(&bytes[i]) {
            i += 1;
        }
        let key = html[key_start..i].to_ascii_lowercase();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if i < bytes.len() && bytes[i] == b'=' {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            match bytes.get(i) {
                Some(&quote @ (b'"' | b'\'')) => {
                    let end = html[i + 1..].find(quote as char).ok_or(EMBEDS)? + i + 1;
                    value = html[i + 1..end].into();
                    i = end + 1;
                }
                Some(_) => {
                    let start = i;
                    while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                        i += 1;
                    }
                    value = html[start..i].into();
                }
                None => return Err(EMBEDS.into()),
            }
        }
        out.push((key, value));
    }
}
fn allowed(tag: &str, key: &str, value: &str) -> Result<bool, String> {
    let value = value.trim();
    if (key == "href" || key == "xlink:href") && value.starts_with('#') && !value.contains('&') {
        return Ok(true);
    }
    if key != "src" || tag != "img" || value.len() > 256 * 1024 {
        return Ok(false);
    }
    let Some((mime, data)) = value.split_once(',') else {
        return Ok(false);
    };
    let base64 = |s: &str| {
        let body = s.trim_end_matches('=');
        !body.is_empty()
            && body
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'+' || c == b'/')
            && s.len() - body.len() <= 2
    };
    if !matches!(
        mime,
        "data:image/png;base64"
            | "data:image/jpeg;base64"
            | "data:image/webp;base64"
            | "data:image/gif;base64"
    ) || !base64(data)
    {
        return Ok(false);
    }
    let head = decode(&data[..data.len().min(24)]);
    let raster = match mime {
        "data:image/png;base64" => head.starts_with(b"\x89PNG\r\n\x1a\n"),
        "data:image/jpeg;base64" => head.starts_with(b"\xff\xd8\xff"),
        "data:image/gif;base64" => head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a"),
        _ => head.starts_with(b"RIFF") && head.get(8..12) == Some(b"WEBP"),
    };
    if !raster {
        return Err("An embedded image does not match its raster format.".into());
    }
    Ok(true)
}
fn decode(text: &str) -> Vec<u8> {
    let mut out = vec![];
    let (mut acc, mut bits) = (0u32, 0u32);
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => break,
        } as u32;
        acc = acc << 6 | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn html(markup: &str) -> Result<Prepared, String> {
        prepare(&json!({"title":"Chart","html":markup}))
    }
    #[test]
    fn a_self_contained_page_is_normalized_like_mains_template() {
        let page = html("<!DOCTYPE html><html><head><style>b{color:red}</style></head><body><svg><use href=\"#a\"/></svg><script>const s = '<iframe src=x>';</script></body></html>").unwrap();
        assert_eq!(
            page.html,
            "<!doctype html><html><head></head><body><style>b{color:red}</style><svg><use href=\"#a\"/></svg><script>const s = '<iframe src=x>';</script></body></html>"
        );
        assert_eq!(page.height, 360);
        let png = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
        assert!(html(&format!("<img src=\"{png}\">")).is_ok());
    }
    #[test]
    fn ambient_resources_and_embeds_are_refused_with_mains_messages() {
        for markup in [
            "<iframe></iframe>",
            "<FORM></FORM>",
            "<meta http-equiv=refresh content=0>",
            "<link rel=stylesheet>",
            "<base href=/>",
            "<template></template>",
        ] {
            assert_eq!(html(markup).unwrap_err(), EMBEDS, "{markup}");
        }
        for markup in [
            "<img src='file:///etc/hosts'>",
            "<a href=https://example.com>x</a>",
            "<svg><image href='x.png'/></svg>",
            "<video poster=x>",
            "<a href=\"&#35;x\">encoded</a>",
            "<img src=\"data:image/svg+xml;base64,PHN2Zz4=\">",
        ] {
            assert_eq!(html(markup).unwrap_err(), RESOURCES, "{markup}");
        }
        assert_eq!(
            html("<img src=\"data:image/png;base64,R0lGODlhAQABAAAAACw=\">").unwrap_err(),
            "An embedded image does not match its raster format."
        );
        // Markup the scanner can't close is refused, not passed through.
        assert_eq!(html("<div class=\"x>").unwrap_err(), EMBEDS);
    }
    #[test]
    fn limits_follow_main() {
        assert_eq!(
            prepare(&json!({"title":" ","html":"<p>x</p>"})).unwrap_err(),
            LIMITS
        );
        assert_eq!(
            prepare(&json!({"title":"t","html":"x","height":100})).unwrap_err(),
            LIMITS
        );
        assert_eq!(
            prepare(&json!({"title":"t","html":"x","extra":1})).unwrap_err(),
            LIMITS
        );
        let big = "x".repeat(MAX_BYTES + 1);
        assert_eq!(
            prepare(&json!({"title":"t","html":big})).unwrap_err(),
            LIMITS
        );
        assert_eq!(
            prepare(&json!({"title":"t","html":"x","height":900}))
                .unwrap()
                .height,
            900
        );
    }
    #[test]
    fn the_viewer_document_leads_with_the_policy() {
        let page = html("<p>Hi</p>").unwrap();
        let doc = document(&page.html, true);
        assert!(doc.starts_with(&format!(
            "<!doctype html><html><head><meta http-equiv=\"Content-Security-Policy\" content=\"{CSP}\">"
        )));
        assert!(doc.ends_with("<body><p>Hi</p></body></html>"));
        let saved = json!({"version":1,"id":"visual-a1","runId":"run-1","title":"Chart","html":page.html,"height":360});
        assert_eq!(read(&saved), Some(page));
        assert_eq!(
            read(&json!({"version":1,"id":"bad","runId":"r","title":"t","html":"x"})),
            None
        );
    }
}
