//! `$exception` payloads for panics, crash reports and hangs, in PostHog's
//! native format: raw frames by instruction address with the binary's load
//! address and Mach-O UUID (`$debug_images`), so PostHog symbolicates them
//! against the uploaded dSYM; names resolved on the Mac are added where the
//! binary carries them. Scrubbed: no paths (binary names and repository-
//! relative source files only), no quoted text from messages.
use serde_json::{Map, Value, json};

/// One frame, innermost first as captured.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Frame {
    pub addr: u64,
    pub function: Option<String>,
    pub file: Option<String>,
    pub line: Option<u32>,
}

/// A loaded binary.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub base: u64,
    pub size: u64,
    /// Mach-O UUID, uppercase with dashes.
    pub uuid: String,
    /// The file name only.
    pub name: String,
}

/// What the exception is.
pub struct Kind<'a> {
    pub exception_type: &'a str,
    pub message: &'a str,
    /// "panic", "crash" or "hang".
    pub mechanism: &'static str,
    pub level: &'static str,
}

/// The `$exception_list` and `$debug_images` properties for `frames`.
pub fn properties(kind: Kind<'_>, frames: &[Frame], images: &[Image]) -> Map<String, Value> {
    let main = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
    let image_of = |addr: u64| {
        images
            .iter()
            .find(|i| addr >= i.base && addr < i.base.saturating_add(i.size.max(1)))
    };
    let mut used: Vec<&Image> = vec![];
    // Wire order: outermost first, the crash site last.
    let wire: Vec<Value> = frames
        .iter()
        .rev()
        .map(|f| {
            let image = image_of(f.addr);
            if let Some(image) = image
                && !used.iter().any(|u| u.base == image.base)
            {
                used.push(image);
            }
            let ours = image.is_some_and(|i| Some(&i.name) == main.as_ref());
            let function = f.function.as_deref().map(normalize);
            let in_app = ours
                && function.as_deref().is_none_or(|name| {
                    !["std::", "core::", "alloc::", "backtrace::", "__rust"]
                        .iter()
                        .any(|p| name.starts_with(p))
                });
            let mut frame = json!({
                "platform": "native",
                "lang": "rust",
                "in_app": in_app,
                "synthetic": false,
                "instruction_addr": format!("{:#x}", f.addr),
                "client_resolved": function.is_some(),
            });
            let obj = frame.as_object_mut().expect("object");
            if let Some(image) = image {
                obj.insert("image_addr".into(), format!("{:#x}", image.base).into());
            }
            if let Some(function) = function {
                obj.insert("function".into(), function.into());
            }
            if let Some(file) = f.file.as_deref().and_then(scrub_source_path) {
                obj.insert("filename".into(), file.into());
                if let Some(line) = f.line {
                    obj.insert("lineno".into(), line.into());
                }
            }
            frame
        })
        .collect();
    let debug_images: Vec<Value> = used
        .iter()
        .filter(|i| !i.uuid.is_empty())
        .map(|i| {
            json!({
                "type": "macho",
                "debug_id": i.uuid,
                "image_addr": format!("{:#x}", i.base),
                "image_size": i.size,
                "code_file": i.name,
                "arch": "arm64",
            })
        })
        .collect();
    let mut props = Map::new();
    props.insert(
        "$exception_list".into(),
        json!([{
            "type": kind.exception_type,
            "value": scrub_message(kind.message),
            "mechanism": {"type": kind.mechanism, "handled": false, "synthetic": false},
            "stacktrace": {"type": "raw", "frames": wire},
        }]),
    );
    props.insert("$exception_level".into(), kind.level.into());
    if !debug_images.is_empty() {
        props.insert("$debug_images".into(), debug_images.into());
    }
    props
}

/// The binaries loaded in this process (for panics and hangs).
pub fn loaded_images() -> Vec<Image> {
    use findshlibs::{IterationControl, SharedLibrary, SharedLibraryId, TargetSharedLibrary};
    let mut images = vec![];
    TargetSharedLibrary::each(|lib| {
        let uuid = match lib.id() {
            Some(SharedLibraryId::Uuid(bytes)) => uuid::Uuid::from_bytes(bytes)
                .hyphenated()
                .to_string()
                .to_uppercase(),
            _ => String::new(),
        };
        let name = lib.name().to_string_lossy();
        images.push(Image {
            base: lib.actual_load_addr().0 as u64,
            size: lib.len() as u64,
            uuid,
            name: name.rsplit('/').next().unwrap_or_default().to_owned(),
        });
        IterationControl::Continue
    });
    images
}

/// Names for raw addresses from the binaries' own symbols, where they have
/// them (a stripped release binary leaves its own frames to PostHog).
pub fn resolve(addrs: &[u64]) -> Vec<Frame> {
    addrs
        .iter()
        .map(|&addr| {
            let mut frame = Frame {
                addr,
                ..Default::default()
            };
            backtrace::resolve(addr as *mut std::ffi::c_void, |symbol| {
                if frame.function.is_none() {
                    frame.function = symbol.name().map(|n| n.to_string());
                    frame.file = symbol.filename().map(|p| p.to_string_lossy().into_owned());
                    frame.line = symbol.lineno();
                }
            });
            frame
        })
        .collect()
}

/// Drops the `::h0123…` hash and `[0123…]` crate disambiguators.
pub fn normalize(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut rest = name;
    while let Some(i) = rest.find('[') {
        let (head, tail) = rest.split_at(i);
        out.push_str(head);
        match tail.find(']') {
            Some(j) if tail[1..j].bytes().all(|b| b.is_ascii_hexdigit()) => rest = &tail[j + 1..],
            _ => {
                out.push('[');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    match out.rfind("::h") {
        Some(i) if out.len() - i == 19 && out[i + 3..].bytes().all(|b| b.is_ascii_hexdigit()) => {
            out.truncate(i);
            out
        }
        _ => out,
    }
}

/// A source path from debug info, kept only relative to a known root (the
/// repository, a crate from the registry, the standard library); any other
/// absolute path (a home folder) is dropped.
pub fn scrub_source_path(path: &str) -> Option<String> {
    if !path.starts_with('/') && !path.starts_with('~') && !path.contains(":\\") {
        return Some(path.to_owned());
    }
    for root in ["/apps/native/", "/engine/"] {
        if let Some(i) = path.find(root) {
            return Some(path[i + 1..].to_owned());
        }
    }
    if let Some(i) = path.find("/registry/src/") {
        let rest = &path[i + "/registry/src/".len()..];
        return rest
            .split_once('/')
            .map(|(_, crate_path)| crate_path.to_owned());
    }
    if let Some(i) = path.find("/rustc/") {
        let rest = &path[i + "/rustc/".len()..];
        return rest
            .split_once('/')
            .map(|(_, std_path)| format!("rustc/{std_path}"));
    }
    if let Some(i) = path.find("/library/") {
        return Some(format!("rustc{}", &path[i..]));
    }
    None
}

/// A message without anything a person typed or named: quoted text, paths,
/// URLs and e-mail addresses are replaced, and it is cut to 300 characters.
pub fn scrub_message(message: &str) -> String {
    let mut unquoted = String::with_capacity(message.len());
    let mut chars = message.chars().peekable();
    while let Some(c) = chars.next() {
        let close = match c {
            '"' => Some('"'),
            '\'' if unquoted.chars().last().is_none_or(|p| !p.is_alphanumeric()) => Some('\''),
            '`' => Some('`'),
            '“' => Some('”'),
            _ => None,
        };
        match close {
            Some(close) if chars.clone().any(|n| n == close) => {
                for n in chars.by_ref() {
                    if n == close {
                        break;
                    }
                }
                unquoted.push(c);
                unquoted.push('…');
                unquoted.push(close);
            }
            _ => unquoted.push(c),
        }
    }
    let words: Vec<String> = unquoted
        .split(' ')
        .map(|word| {
            let bare = word.trim_start_matches(['(', '[', '{', '<', '=']);
            let lower = bare.to_ascii_lowercase();
            if bare.starts_with('/') && bare.len() > 1
                || bare.starts_with("~/")
                || lower.contains("://")
                || lower.starts_with("file:")
            {
                format!("{}<path>", &word[..word.len() - bare.len()])
            } else if bare.contains('@') && bare.contains('.') {
                format!("{}<email>", &word[..word.len() - bare.len()])
            } else {
                word.to_owned()
            }
        })
        .collect();
    let joined = words.join(" ");
    match joined.char_indices().nth(300) {
        Some((i, _)) => format!("{}…", &joined[..i]),
        None => joined,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_lose_paths_quotes_and_addresses() {
        assert_eq!(
            scrub_message(r#"failed to open "/Users/jane/Secret Launch.pmv": No such file"#),
            r#"failed to open "…": No such file"#
        );
        assert_eq!(
            scrub_message("layer 'My Logo' missing at /Users/jane/x.pmv (see ~/notes)"),
            "layer '…' missing at <path> (see <path>"
        );
        assert_eq!(
            scrub_message("called `Option::unwrap()` on a `None` value"),
            "called `…` on a `…` value"
        );
        assert_eq!(
            scrub_message("mail jane@example.com now"),
            "mail <email> now"
        );
        assert_eq!(
            scrub_message("index out of bounds: the len is 3 but the index is 7"),
            "index out of bounds: the len is 3 but the index is 7"
        );
        assert!(scrub_message(&"x".repeat(400)).chars().count() <= 301);
        // An apostrophe inside a word is not a quote.
        assert_eq!(scrub_message("can't draw"), "can't draw");
    }

    #[test]
    fn source_paths_keep_only_their_repository_relative_part() {
        assert_eq!(
            scrub_source_path("/Users/jane/Developer/pm/apps/native/crates/app/src/main.rs")
                .as_deref(),
            Some("apps/native/crates/app/src/main.rs")
        );
        assert_eq!(
            scrub_source_path(
                "/Users/jane/.cargo/registry/src/index.crates.io-1949/gpui-0.1/src/a.rs"
            )
            .as_deref(),
            Some("gpui-0.1/src/a.rs")
        );
        assert_eq!(
            scrub_source_path("/rustc/4b06a43a1/library/core/src/panicking.rs").as_deref(),
            Some("rustc/library/core/src/panicking.rs")
        );
        assert_eq!(scrub_source_path("/Users/jane/elsewhere/x.rs"), None);
        assert_eq!(
            scrub_source_path("src/lib.rs").as_deref(),
            Some("src/lib.rs")
        );
    }

    #[test]
    fn frames_go_out_outermost_first_with_their_binary_and_no_paths() {
        let main = std::env::current_exe().unwrap();
        let main = main.file_name().unwrap().to_string_lossy().into_owned();
        let images = vec![
            Image {
                base: 0x1_0000_0000,
                size: 0x10_0000,
                uuid: "AAAA".into(),
                name: main,
            },
            Image {
                base: 0x2_0000_0000,
                size: 0x1000,
                uuid: "BBBB".into(),
                name: "libsystem_c.dylib".into(),
            },
        ];
        let frames = vec![
            Frame {
                addr: 0x2_0000_0010,
                function: Some("abort".into()),
                ..Default::default()
            },
            Frame {
                addr: 0x1_0000_0100,
                function: Some("pm_app::open::h0123456789abcdef".into()),
                file: Some("/Users/jane/pm/apps/native/crates/app/src/open.rs".into()),
                line: Some(7),
            },
        ];
        let props = properties(
            Kind {
                exception_type: "Panic",
                message: "boom",
                mechanism: "panic",
                level: "fatal",
            },
            &frames,
            &images,
        );
        let list = &props["$exception_list"][0];
        let wire = list["stacktrace"]["frames"].as_array().unwrap();
        assert_eq!(wire[0]["function"], "pm_app::open");
        assert_eq!(wire[0]["filename"], "apps/native/crates/app/src/open.rs");
        assert_eq!(wire[0]["in_app"], true);
        assert_eq!(wire[1]["function"], "abort");
        assert_eq!(wire[1]["in_app"], false);
        assert_eq!(wire[1]["image_addr"], "0x200000000");
        assert_eq!(props["$debug_images"].as_array().unwrap().len(), 2);
        let text = serde_json::to_string(&props).unwrap();
        assert!(!text.contains("/Users/"), "{text}");
    }

    #[test]
    fn this_process_has_its_images_and_resolves_its_own_frames() {
        let images = loaded_images();
        assert!(images.iter().any(|i| !i.uuid.is_empty()));
        assert!(images.iter().all(|i| !i.name.contains('/')));
        let here = this_process_has_its_images_and_resolves_its_own_frames as *const () as u64;
        let frames = resolve(&[here + 4]);
        assert!(frames[0].function.is_some());
    }
}
