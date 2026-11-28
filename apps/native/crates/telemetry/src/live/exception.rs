//! `$exception` payloads for panics and crash reports, in PostHog's native
//! format. What leaves (Sol review, P0):
//! - the exception's type from a fixed list and a fixed value; never a panic
//!   message, a crash subtype, a termination reason or other report text;
//! - frames in our own executable only: instruction address, the image's load
//!   address and, when our binary names it, the function. Every other frame
//!   is an anonymous placeholder (no address, no name, no binary);
//! - one debug image, ours, by its validated Mach-O UUID, labelled
//!   "powermove". No paths, file names, thread names or other binaries.
use serde_json::{Map, Value, json};

/// One frame, innermost first as captured.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Frame {
    pub addr: u64,
    /// Only kept for frames in our own executable.
    pub function: Option<String>,
}

/// Our executable as loaded.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub base: u64,
    pub size: u64,
    /// Mach-O UUID, uppercase with dashes (validated).
    pub uuid: String,
}

impl Image {
    pub fn contains(&self, addr: u64) -> bool {
        addr >= self.base && addr - self.base < self.size
    }
}

/// What the exception is: `exception_type` from [`crash_type`] or "Panic";
/// `value` a fixed text the caller makes.
pub struct Kind<'a> {
    pub exception_type: &'a str,
    pub value: String,
    pub mechanism: &'static str,
}

const OTHER: &str = "[system or other binary]";
const MAX_HANG_FRAMES: usize = 16;

fn is_in_app_symbol(name: &str) -> bool {
    ![
        "std::",
        "core::",
        "alloc::",
        "backtrace::",
        "__rust",
        "pm_telemetry::",
    ]
    .iter()
    .any(|p| name.starts_with(p))
}

/// Summarizes a sampled main-thread stack (innermost first) for a `hang` event
/// under the same privacy rules as `$exception`: only frames in our own
/// executable carry a sanitized symbol (or `powermove+0x<offset>` in stripped
/// builds); every other binary collapses to `[system or other binary]`.
///
/// Returns `(location, stack, debug_id)`.
pub fn hang_trace(
    frames: &[Frame],
    main: Option<&Image>,
) -> (Option<String>, Option<Vec<String>>, Option<String>) {
    if frames.is_empty() {
        return (None, None, None);
    }
    let mut stack: Vec<String> = Vec::new();
    let mut in_app_location: Option<String> = None;
    let mut any_app_location: Option<String> = None;
    let mut saw_ours = false;

    for f in frames {
        match main.filter(|m| m.contains(f.addr)) {
            Some(image) => {
                saw_ours = true;
                let function = f.function.as_deref().and_then(symbol);
                let in_app = function.as_deref().is_none_or(is_in_app_symbol);
                let label = function
                    .unwrap_or_else(|| format!("powermove+{:#x}", f.addr - image.base));
                if any_app_location.is_none() {
                    any_app_location = Some(label.clone());
                }
                if in_app && in_app_location.is_none() {
                    in_app_location = Some(label.clone());
                }
                if stack.len() < MAX_HANG_FRAMES {
                    stack.push(label);
                }
            }
            None => {
                if stack.len() < MAX_HANG_FRAMES
                    && stack.last().is_none_or(|last| last != OTHER)
                {
                    stack.push(OTHER.to_owned());
                }
            }
        }
    }

    let location = in_app_location
        .or(any_app_location)
        .or_else(|| (!stack.is_empty()).then(|| OTHER.to_owned()));
    let debug_id = saw_ours
        .then(|| main.filter(|m| valid_uuid(&m.uuid)).map(|m| m.uuid.clone()))
        .flatten();
    (location, (!stack.is_empty()).then_some(stack), debug_id)
}

/// The `$exception_list` and `$debug_images` properties.
pub fn properties(kind: Kind<'_>, frames: &[Frame], main: Option<&Image>) -> Map<String, Value> {
    let mut wire: Vec<Value> = vec![];
    // Wire order: outermost first, the crash site last; runs of other
    // binaries' frames fold into one placeholder.
    for f in frames.iter().rev() {
        match main.filter(|m| m.contains(f.addr)) {
            Some(image) => {
                let function = f.function.as_deref().and_then(symbol);
                let in_app = function.as_deref().is_none_or(|name| {
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
                    "image_addr": format!("{:#x}", image.base),
                    "client_resolved": function.is_some(),
                });
                if let Some(function) = function {
                    frame["function"] = function.into();
                }
                wire.push(frame);
            }
            None => {
                if wire.last().is_some_and(|w| w["function"] == OTHER) {
                    continue;
                }
                wire.push(json!({
                    "platform": "native",
                    "lang": "rust",
                    "in_app": false,
                    "synthetic": true,
                    "client_resolved": true,
                    "function": OTHER,
                }));
            }
        }
    }
    let mut props = Map::new();
    props.insert(
        "$exception_list".into(),
        json!([{
            "type": kind.exception_type,
            "value": kind.value,
            "mechanism": {"type": kind.mechanism, "handled": false, "synthetic": false},
            "stacktrace": {"type": "raw", "frames": wire},
        }]),
    );
    props.insert("$exception_level".into(), "fatal".into());
    if let Some(image) = main.filter(|m| valid_uuid(&m.uuid)) {
        props.insert(
            "$debug_images".into(),
            json!([{
                "type": "macho",
                "debug_id": image.uuid,
                "image_addr": format!("{:#x}", image.base),
                "image_size": image.size,
                "code_file": "powermove",
                "arch": "arm64",
            }]),
        );
    }
    props
}

/// A Mach-O UUID as `XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX`.
pub fn valid_uuid(uuid: &str) -> bool {
    uuid.len() == 36
        && uuid.char_indices().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        })
}

/// A symbol of our binary as sent: hashes and crate disambiguators gone, at
/// most 200 characters of the characters Rust and C symbols use.
fn symbol(name: &str) -> Option<String> {
    let name = normalize(name);
    let ok = !name.is_empty()
        && name.len() <= 200
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || " _:<>,&*()[]{}'#-.!$".contains(c));
    ok.then_some(name)
}

/// Our executable in this process.
pub fn main_image() -> Option<Image> {
    use findshlibs::{IterationControl, SharedLibrary, SharedLibraryId, TargetSharedLibrary};
    let exe = std::env::current_exe().ok()?;
    let mut found = None;
    TargetSharedLibrary::each(|lib| {
        if std::path::Path::new(lib.name()) == exe.as_path()
            && let Some(SharedLibraryId::Uuid(bytes)) = lib.id()
        {
            found = Some(Image {
                base: lib.actual_load_addr().0 as u64,
                size: lib.len() as u64,
                uuid: uuid::Uuid::from_bytes(bytes)
                    .hyphenated()
                    .to_string()
                    .to_uppercase(),
            });
            return IterationControl::Break;
        }
        IterationControl::Continue
    });
    found
}

/// Names for addresses in our own executable, where it carries symbols;
/// other addresses stay unnamed.
pub fn resolve(addrs: &[u64], main: Option<&Image>) -> Vec<Frame> {
    addrs
        .iter()
        .map(|&addr| {
            let mut frame = Frame {
                addr,
                function: None,
            };
            if main.is_some_and(|m| m.contains(addr)) {
                backtrace::resolve(addr as *mut std::ffi::c_void, |s| {
                    if frame.function.is_none() {
                        frame.function = s.name().map(|n| n.to_string());
                    }
                });
            }
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

/// A panic's location kept as a repository path (`crates/…`, `engine/…`,
/// a registry crate's own path): plain characters only, never absolute,
/// never a home folder. Anything else is dropped.
pub fn panic_file(path: &str) -> Option<String> {
    let rel = if let Some(i) = path.find("/registry/src/") {
        path[i + "/registry/src/".len()..].split_once('/')?.1
    } else if let Some(i) = path.find("/rustc/") {
        path[i + "/rustc/".len()..].split_once('/')?.1
    } else {
        let mut p = path;
        while let Some(rest) = p.strip_prefix("../") {
            p = rest;
        }
        for root in ["/apps/native/", "/engine/"] {
            if let Some(i) = p.find(root) {
                p = &p[i + 1..];
            }
        }
        p
    };
    let ok = !rel.is_empty()
        && rel.len() <= 160
        && !rel.starts_with('/')
        && !rel.contains("..")
        && rel.ends_with(".rs")
        && rel
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_./-".contains(c));
    ok.then(|| rel.to_owned())
}

/// A crash's type as sent: a known Mach exception and signal, else "Crash".
pub fn crash_type(exception: Option<&str>, signal: Option<&str>) -> String {
    const EXCEPTIONS: &[&str] = &[
        "EXC_BAD_ACCESS",
        "EXC_BAD_INSTRUCTION",
        "EXC_ARITHMETIC",
        "EXC_EMULATION",
        "EXC_SOFTWARE",
        "EXC_BREAKPOINT",
        "EXC_CRASH",
        "EXC_RESOURCE",
        "EXC_GUARD",
        "EXC_CORPSE_NOTIFY",
    ];
    const SIGNALS: &[&str] = &[
        "SIGSEGV", "SIGBUS", "SIGILL", "SIGABRT", "SIGTRAP", "SIGFPE", "SIGKILL", "SIGSYS",
        "SIGTERM",
    ];
    let exception = exception.and_then(|e| EXCEPTIONS.iter().find(|x| **x == e));
    let signal = signal.and_then(|s| SIGNALS.iter().find(|x| **x == s));
    match (exception, signal) {
        (Some(e), Some(s)) => format!("{e} ({s})"),
        (Some(e), None) => (*e).to_owned(),
        (None, Some(s)) => (*s).to_owned(),
        _ => "Crash".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn main() -> Image {
        Image {
            base: 0x1_0000_0000,
            size: 0x10_0000,
            uuid: "6030AB4F-66DD-3529-86E7-67196E0787B0".into(),
        }
    }

    /// Sol P0 (w99 review): only our binary's offsets and symbols leave.
    #[test]
    fn frames_carry_only_our_binary() {
        let frames = vec![
            Frame {
                addr: 0x2_0000_0010,
                function: Some("secret_plugin_jane_render".into()),
            },
            Frame {
                addr: 0x2_0000_0020,
                function: Some("other".into()),
            },
            Frame {
                addr: 0x1_0000_0100,
                function: Some("pm_app::open::h0123456789abcdef".into()),
            },
            Frame {
                addr: 0x1_0000_0200,
                function: Some("/Users/jane/file.rs".into()),
            },
        ];
        let props = properties(
            Kind {
                exception_type: "Panic",
                value: "panic".into(),
                mechanism: "panic",
            },
            &frames,
            Some(&main()),
        );
        let text = serde_json::to_string(&props).unwrap();
        for leak in ["secret_plugin", "jane", "/Users", "0x200000010", "dylib"] {
            assert!(!text.contains(leak), "{leak} in {text}");
        }
        let wire = props["$exception_list"][0]["stacktrace"]["frames"]
            .as_array()
            .unwrap();
        assert_eq!(wire.len(), 3);
        assert_eq!(wire[1]["function"], "pm_app::open");
        assert_eq!(wire[1]["image_addr"], "0x100000000");
        // A name that isn't a symbol is not sent.
        assert!(wire[0].get("function").is_none());
        assert_eq!(wire[2]["function"], OTHER);
        assert!(wire[2].get("instruction_addr").is_none());
        let images = props["$debug_images"].as_array().unwrap();
        assert_eq!(images.len(), 1);
        assert_eq!(images[0]["code_file"], "powermove");
    }

    #[test]
    fn panic_locations_keep_only_repository_paths() {
        assert_eq!(
            panic_file("crates/app/src/telemetry.rs").as_deref(),
            Some("crates/app/src/telemetry.rs")
        );
        assert_eq!(
            panic_file("../../engine/crates/pm-render/src/a.rs").as_deref(),
            Some("engine/crates/pm-render/src/a.rs")
        );
        assert_eq!(
            panic_file("/Users/jane/.cargo/registry/src/index.crates.io-1/gpui-0.1/src/a.rs")
                .as_deref(),
            Some("gpui-0.1/src/a.rs")
        );
        for bad in [
            "/Users/jane/x.rs",
            "My Project/a.rs",
            "a.txt",
            "",
            "/tmp/a.rs",
        ] {
            assert_eq!(panic_file(bad), None, "{bad}");
        }
    }

    #[test]
    fn crash_types_come_from_a_fixed_list() {
        assert_eq!(
            crash_type(Some("EXC_BAD_ACCESS"), Some("SIGSEGV")),
            "EXC_BAD_ACCESS (SIGSEGV)"
        );
        assert_eq!(crash_type(Some("jane's thing"), Some("SIGABRT")), "SIGABRT");
        assert_eq!(crash_type(Some("x"), None), "Crash");
    }

    #[test]
    fn hang_trace_picks_innermost_app_frame_and_redacts_external_binaries() {
        let img = main();
        // Innermost frames: external syscall, std lock, then our app caller.
        let frames = vec![
            Frame {
                addr: 0x2_0000_0010,
                function: Some("secret_plugin_jane_wait".into()),
            },
            Frame {
                addr: 0x2_0000_0020,
                function: Some("libsystem_pthread".into()),
            },
            Frame {
                addr: 0x1_0000_0080,
                function: Some("std::sync::mutex::Mutex::lock::h0123456789abcdef".into()),
            },
            Frame {
                addr: 0x1_0000_0100,
                function: Some("pm_app::extensions::mount_surfaces::h0123456789abcdef".into()),
            },
            Frame {
                addr: 0x1_0000_0200,
                function: None,
            },
        ];
        let (location, stack, debug_id) = hang_trace(&frames, Some(&img));
        assert_eq!(location.as_deref(), Some("pm_app::extensions::mount_surfaces"));
        assert_eq!(
            stack.as_deref(),
            Some(
                &[
                    OTHER.to_owned(),
                    "std::sync::mutex::Mutex::lock".to_owned(),
                    "pm_app::extensions::mount_surfaces".to_owned(),
                    "powermove+0x200".to_owned(),
                ][..]
            )
        );
        assert_eq!(debug_id.as_deref(), Some(img.uuid.as_str()));

        // Stripped release binary: falls back to slide-independent offset + debug_id.
        let stripped = vec![
            Frame {
                addr: 0x2_0000_0010,
                function: None,
            },
            Frame {
                addr: 0x1_0000_1a40,
                function: None,
            },
        ];
        let (loc, st, did) = hang_trace(&stripped, Some(&img));
        assert_eq!(loc.as_deref(), Some("powermove+0x1a40"));
        assert_eq!(
            st.as_deref(),
            Some(&[OTHER.to_owned(), "powermove+0x1a40".to_owned()][..])
        );
        assert_eq!(did.as_deref(), Some(img.uuid.as_str()));

        // External-only frames carry no debug_id and no external symbols.
        let external = vec![Frame {
            addr: 0x2_0000_0010,
            function: Some("secret_jane".into()),
        }];
        let (loc, st, did) = hang_trace(&external, Some(&img));
        assert_eq!(loc.as_deref(), Some(OTHER));
        assert_eq!(st.as_deref(), Some(&[OTHER.to_owned()][..]));
        assert_eq!(did, None);

        // Empty sample produces None.
        assert_eq!(hang_trace(&[], Some(&img)), (None, None, None));
    }

    #[test]
    fn our_image_is_found_and_resolves_its_own_frames() {
        let image = main_image().unwrap();
        assert!(valid_uuid(&image.uuid));
        let here = our_image_is_found_and_resolves_its_own_frames as *const () as u64;
        let frames = resolve(&[here + 4, 0x10], Some(&image));
        assert!(frames[0].function.is_some());
        assert!(frames[1].function.is_none());
    }
}
