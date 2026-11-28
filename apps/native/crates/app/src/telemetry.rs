//! Telemetry's place in the app (`pm-telemetry`): it starts before the first
//! window, beats the main thread's heartbeat for the hang watchdog, follows
//! whether the app is active, and reports the launch once the first window
//! has drawn. Release/dist builds compile it in (`--features telemetry`);
//! elsewhere every call is empty.
use std::path::Path;
use std::time::Duration;

/// The mode telemetry runs in this session.
pub fn init(
    profile: &Path,
    info_plist: Option<&str>,
    development: bool,
    device: &metal::Device,
) -> pm_telemetry::Mode {
    let marked = info_plist.is_some_and(|plist| {
        plist.contains("<key>PMRequireProfile</key>") || plist.contains("<key>PMLaneProfile</key>")
    });
    let plist_string = |key: &str| {
        let plist = info_plist?;
        let after = &plist[plist.find(&format!("<key>{key}</key>"))?..];
        let start = after.find("<string>")? + "<string>".len();
        let end = after[start..].find("</string>")?;
        Some(after[start..start + end].trim().to_owned())
    };
    let displays = objc2::MainThreadMarker::new()
        .map(|mtm| {
            let screens = objc2_app_kit::NSScreen::screens(mtm);
            let scale = screens
                .iter()
                .map(|s| s.backingScaleFactor() as f32)
                .fold(1f32, f32::max);
            (screens.len() as u32, scale)
        })
        .unwrap_or((1, 1.));
    let device = device.clone();
    pm_telemetry::init(pm_telemetry::Config {
        profile: profile.to_owned(),
        app_version: plist_string("CFBundleShortVersionString")
            .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned()),
        build: env!("PM_BUILD_COMMIT").to_owned(),
        bundle_id: plist_string("CFBundleIdentifier"),
        headless: false,
        development: development || marked,
        debug_build: cfg!(debug_assertions),
        displays,
        metal_bytes: Some(std::sync::Arc::new(move || device.current_allocated_size())),
    })
}

/// With the first window: the heartbeat, the app's activity, the launch
/// report after the first frame, and the forced-failure checks.
pub fn install(window: &mut gpui::Window, cx: &mut gpui::App) {
    if !pm_telemetry::running() {
        return;
    }
    pm_telemetry::main_thread::install();
    gpui_macos::feel::observe(pm_telemetry::frame, pm_telemetry::note_input);
    // Four beats a second: the watchdog calls a main thread hung after 2 s.
    let timer = cx.background_executor().clone();
    cx.spawn(async move |_| {
        loop {
            pm_telemetry::main_thread::beat();
            timer.timer(Duration::from_millis(250)).await;
        }
    })
    .detach();
    follow_activity();
    let first_window_ms = pm_telemetry::since_process_start_ms();
    window.on_next_frame(move |window, _| {
        // The frame after the first one: the first has been presented.
        window.on_next_frame(move |_, _| {
            pm_telemetry::launched(first_window_ms, pm_telemetry::since_process_start_ms());
        });
        window.refresh();
    });
    if let Some(failure) = pm_telemetry::dev_failure() {
        let timer = cx.background_executor().clone();
        cx.spawn(async move |_| {
            timer.timer(Duration::from_secs(5)).await;
            force(&failure);
        })
        .detach();
    }
}

/// `PM_TELEMETRY_DEV_FAIL` (only with `PM_TELEMETRY_DEV=1`): the failures
/// the end-to-end check forces on the main thread.
fn force(failure: &str) {
    match failure {
        "panic" => panic!("PM_TELEMETRY_DEV_FAIL forced panic"),
        "abort" => std::process::abort(),
        // SAFETY: none; this is the forced segmentation fault the crash
        // report check needs.
        "segv" => unsafe { std::ptr::null_mut::<u64>().write_volatile(1) },
        // The watchdog counts hangs only while the app is active; a lane app
        // stays in the background, so the check marks it active itself.
        "hang" => {
            pm_telemetry::set_active(true);
            std::thread::sleep(Duration::from_secs(3));
        }
        _ => {}
    }
}

fn follow_activity() {
    use objc2_app_kit::{
        NSApplicationDidBecomeActiveNotification, NSApplicationDidResignActiveNotification,
    };
    for (name, active) in [
        (unsafe { NSApplicationDidBecomeActiveNotification }, true),
        (unsafe { NSApplicationDidResignActiveNotification }, false),
    ] {
        let block = block2::RcBlock::new(
            move |_: std::ptr::NonNull<objc2_foundation::NSNotification>| {
                pm_telemetry::set_active(active);
            },
        );
        // SAFETY: a notification name constant; the observer lives for the
        // process.
        let token = unsafe {
            objc2_foundation::NSNotificationCenter::defaultCenter()
                .addObserverForName_object_queue_usingBlock(Some(name), None, None, &block)
        };
        std::mem::forget(token);
    }
    // A `hang`'s surface: the Settings window is key, or another window is.
    let block = block2::RcBlock::new(
        |note: std::ptr::NonNull<objc2_foundation::NSNotification>| {
            // SAFETY: AppKit hands a live notification whose object is the window.
            let window = unsafe { note.as_ref() }
                .object()
                .and_then(|o| o.downcast::<objc2_app_kit::NSWindow>().ok());
            pm_telemetry::note_settings_key(
                window.is_some_and(|w| pm_shell::settings_window::is_settings(&w)),
            );
        },
    );
    // SAFETY: a notification name constant; the observer lives for the process.
    let token = unsafe {
        objc2_foundation::NSNotificationCenter::defaultCenter()
            .addObserverForName_object_queue_usingBlock(
                Some(objc2_app_kit::NSWindowDidBecomeKeyNotification),
                None,
                None,
                &block,
            )
    };
    std::mem::forget(token);
    if let Some(mtm) = objc2::MainThreadMarker::new() {
        pm_telemetry::set_active(objc2_app_kit::NSApplication::sharedApplication(mtm).isActive());
    }
}

/// An open on its way: reported once its first publication arrives (the
/// project on screen), its facts counted off the main thread.
pub struct Opening {
    pub started: std::time::Instant,
    pub source: pm_telemetry::OpenSource,
    /// The kernel opened it; the next publication shows it.
    pub settled: bool,
}

static NEXT_OPEN_NEW: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A new project was made (it opens next, as `new`).
pub fn project_created() {
    if pm_telemetry::enabled() {
        NEXT_OPEN_NEW.store(true, std::sync::atomic::Ordering::Relaxed);
        pm_telemetry::capture(pm_telemetry::Event::ProjectCreated(
            pm_telemetry::ProjectCreated {},
        ));
    }
}

/// An open from Home or the library: a project made just before is `new`.
pub fn home_source() -> pm_telemetry::OpenSource {
    if NEXT_OPEN_NEW.swap(false, std::sync::atomic::Ordering::Relaxed) {
        pm_telemetry::OpenSource::New
    } else {
        pm_telemetry::OpenSource::Recent
    }
}

/// An open by its kernel source (a file, the library, a new project).
pub fn note_source(slot: &mut Option<Opening>, source: &pm_kernel::session::SessionSource) {
    use pm_kernel::session::SessionSource;
    let source = match source {
        SessionSource::File(_) => pm_telemetry::OpenSource::File,
        SessionSource::New(_) => pm_telemetry::OpenSource::New,
        SessionSource::Library { .. } => home_source(),
    };
    match slot {
        Some(opening) if !opening.settled => opening.source = source,
        _ => *slot = opening(source),
    }
}

/// A tool chosen (only the shipped tools are named).
pub fn tool_selected(command: &str) {
    if !pm_telemetry::enabled() {
        return;
    }
    let tool_id = match command {
        "toolSelect" => "select",
        "toolHand" => "hand",
        "toolZoom" => "zoom",
        "toolRotate" => "rotate",
        "toolAnchor" => "anchor",
        "toolShape" => "shape",
        "toolPen" => "pen",
        "toolText" => "text",
        _ => return,
    };
    pm_telemetry::capture(pm_telemetry::Event::ToolSelected(
        pm_telemetry::ToolSelected { tool_id },
    ));
}

pub fn opening(source: pm_telemetry::OpenSource) -> Option<Opening> {
    pm_telemetry::enabled().then(|| Opening {
        started: std::time::Instant::now(),
        source,
        settled: false,
    })
}

pub fn project_opened(opening: Opening, published: std::sync::Arc<pm_kernel::Published>) {
    let open_ms = opening.started.elapsed().as_millis() as u64;
    let _ = std::thread::Builder::new()
        .name("pm-telemetry-open".into())
        .spawn(move || {
            let Some(facts) = Facts::of_published(&published) else {
                return;
            };
            pm_telemetry::capture(pm_telemetry::Event::ProjectOpened(
                pm_telemetry::ProjectOpened {
                    source: opening.source,
                    open_ms,
                    layer_count: pm_telemetry::count_bucket(facts.layers),
                    comp_count: pm_telemetry::count_bucket(facts.comps),
                    duration: pm_telemetry::duration_bucket(facts.duration),
                    fps: facts.fps as f32,
                    resolution: pm_telemetry::resolution_bucket(
                        facts.width as u32,
                        facts.height as u32,
                    ),
                    has_3d: facts.has_3d,
                    extension_effect_count: pm_telemetry::count_bucket(facts.extension_effects),
                },
            ));
        });
}

/// What `project_opened` counts in a document (no names, no content).
#[derive(Debug, Default, PartialEq)]
struct Facts {
    layers: usize,
    comps: usize,
    duration: f64,
    fps: f64,
    width: f64,
    height: f64,
    has_3d: bool,
    extension_effects: usize,
}

impl Facts {
    /// The composition it shows carries the frame and rate.
    fn of_published(published: &pm_kernel::Published) -> Option<Facts> {
        let root = published.snapshot.root()?;
        let comp = published.snapshot.render_comp();
        let main = pm_kernel::model::comp_node(root, Some(&comp)).unwrap_or(root);
        Some(Facts::of(root, main))
    }
    fn of(root: &pm_model::Node, main: &pm_model::Node) -> Facts {
        use pm_model::NodeKind;
        let number = |node: Option<&pm_model::Node>| match node.map(|n| n.kind()) {
            Some(NodeKind::Number(n)) => *n,
            _ => 0.,
        };
        let list = |node: Option<&pm_model::Node>| match node.map(|n| n.kind()) {
            Some(NodeKind::Array(items)) => items.clone(),
            _ => Default::default(),
        };
        let truthy = |node: Option<&pm_model::Node>| match node.map(|n| n.kind()) {
            Some(NodeKind::Bool(b)) => *b,
            Some(NodeKind::Number(n)) => *n != 0.,
            _ => false,
        };
        let mut facts = Facts {
            duration: number(main.get("dur")),
            fps: number(main.get("fps")),
            width: number(main.get("w")),
            height: number(main.get("h")),
            ..Default::default()
        };
        let mut count = |layers: &[pm_model::Node]| {
            for layer in layers {
                facts.layers += 1;
                facts.has_3d |= truthy(layer.get("threeD"));
                for effect in list(layer.get("fx")).iter() {
                    let builtin = match effect.get("type").map(|t| t.kind()) {
                        Some(NodeKind::String(t)) => t
                            .as_str()
                            .is_some_and(|t| pm_fx_defs::builtins().effect(t).is_some()),
                        _ => true,
                    };
                    facts.extension_effects += usize::from(!builtin);
                }
            }
        };
        count(&list(root.get("layers")));
        let mut comps = 0;
        if let Some(NodeKind::Object(map)) = root.get("comps").map(|c| c.kind()) {
            for (_, comp) in map.iter() {
                comps += 1;
                count(&list(comp.get("layers")));
            }
        }
        facts.comps = comps;
        facts
    }
}

/// A person runs a command in an editor window: what it shows, and its
/// project, for a `hang`. The project is counted only if a hang comes, on the
/// telemetry thread, from the kernel's publication at that moment.
pub fn note_editor(
    surface: pm_telemetry::Surface,
    kernel: &pm_kernel::KernelHandle,
    project: Option<&str>,
) {
    if !pm_telemetry::enabled() {
        return;
    }
    let facts = project.map(|project| {
        let (kernel, project) = (kernel.clone(), project.to_owned());
        std::sync::Arc::new(move || -> Option<(usize, bool)> {
            let published = kernel.published(&project)?;
            let facts = Facts::of_published(&published)?;
            Some((facts.layers, facts.has_3d))
        }) as pm_telemetry::ProjectFacts
    });
    pm_telemetry::note_editor(surface, facts);
}

/// A save finished: how long it took, and the file's size (read off the
/// main thread) when it went to a file.
pub fn project_saved(ms: u64, file: Option<std::path::PathBuf>) {
    if !pm_telemetry::enabled() {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("pm-telemetry-save".into())
        .spawn(move || {
            let size = match file.and_then(|f| std::fs::metadata(f).ok()) {
                Some(meta) => pm_telemetry::size_bucket(meta.len()),
                None => "library",
            };
            pm_telemetry::capture(pm_telemetry::Event::ProjectSaved(
                pm_telemetry::ProjectSaved { ms, size },
            ));
        });
}

/// Panels a window shows now: each newly shown one a person asked for is
/// `panel_opened` (a restore or an extension's panel arriving is not).
pub fn panels_shown(
    seen: &mut Option<std::collections::BTreeSet<String>>,
    open: &[&str],
    store_owner: impl Fn(&str) -> Option<String>,
) {
    if !pm_telemetry::enabled() {
        return;
    }
    let now: std::collections::BTreeSet<String> = open.iter().map(|s| (*s).to_owned()).collect();
    if let Some(before) = seen.as_ref() {
        for id in now.difference(before) {
            if !pm_shell::telemetry::take_requested(id) {
                continue;
            }
            let panel_id = if pm_shell::layout::PANELS.contains(&id.as_str()) {
                pm_telemetry::Origin::builtin(id)
            } else {
                match store_owner(id) {
                    Some(extension) => pm_telemetry::Origin::extension(&extension),
                    None => pm_telemetry::Origin::LOCAL,
                }
            };
            pm_telemetry::capture(pm_telemetry::Event::PanelOpened(
                pm_telemetry::PanelOpened { panel_id },
            ));
        }
    }
    *seen = Some(now);
}

/// An export under way: what `export_finished` adds its result to.
pub struct ExportRun {
    started: std::time::Instant,
    export: pm_telemetry::Export,
    seconds: f64,
}

pub fn export_started(
    options: &pm_promedia::export_panel::Options,
    frames: Option<u64>,
) -> Option<ExportRun> {
    use pm_promedia::export_panel::Format;
    if !pm_telemetry::enabled() {
        return None;
    }
    let name = |value: &dyn std::fmt::Debug| format!("{value:?}").to_ascii_lowercase();
    let codec = match options.format {
        Format::Prores => name(&options.profile),
        Format::Mp4 => name(&options.codec),
        _ => "none".into(),
    };
    let seconds = frames.unwrap_or(0) as f64 / options.fps.max(1.);
    let export = pm_telemetry::Export {
        format: name(&options.format),
        codec,
        resolution: pm_telemetry::resolution_bucket(options.width, options.height),
        duration: pm_telemetry::duration_bucket(seconds),
        fps: options.fps as f32,
        render_ms: None,
        realtime_factor: None,
        result: None,
        error_kind: None,
    };
    pm_telemetry::capture(pm_telemetry::Event::ExportStarted(export.clone()));
    Some(ExportRun {
        started: std::time::Instant::now(),
        export,
        seconds,
    })
}

pub fn export_finished(
    run: ExportRun,
    outcome: pm_telemetry::Outcome,
    error_kind: Option<&'static str>,
) {
    let render = run.started.elapsed().as_secs_f64();
    let mut export = run.export;
    export.render_ms = Some((render * 1000.) as u64);
    export.realtime_factor = (outcome == pm_telemetry::Outcome::Ok && render > 0.)
        .then(|| (run.seconds / render) as f32);
    export.result = Some(outcome);
    export.error_kind = error_kind;
    pm_telemetry::capture(pm_telemetry::Event::ExportFinished(export));
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_documents_facts_are_counts_only() {
        let value = serde_json::json!({
            "w": 1920, "h": 1080, "fps": 30, "dur": 12,
            "layers": [{"name": "Secret", "threeD": true, "fx": [{"type": "someone.glow"}]}, {}],
            "comps": {"c1": {"name": "Inner", "layers": [{}]}},
        });
        let root = pm_model::Node::parse_bytes(value.to_string().as_bytes(), 0).unwrap();
        let facts = super::Facts::of(&root, &root);
        assert_eq!(
            facts,
            super::Facts {
                layers: 3,
                comps: 1,
                duration: 12.,
                fps: 30.,
                width: 1920.,
                height: 1080.,
                has_3d: true,
                extension_effects: 1,
            }
        );
    }
}
