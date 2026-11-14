//! Bringing the original app's extensions into an existing library (N004–N006):
//! the Welcome frame for `surfaces::extensions`, the workers, and the window
//! Settings › Storage opens. Startup drives the same model from `first_run`.
//!
//! Only `pm_first_launch::extension_transfer` writes, and it writes only the
//! library's extension folders and their receipts: never projects, settings or
//! sign-ins. Its `run` hashes, copies, then checks each extension in a
//! throwaway runtime (`extension_authoring::check`), never in an open project.
use pm_first_launch::{
    Config,
    extension_transfer::{self as transfer, Progress, Scan, TransferResult},
    surfaces::extensions::{Model, PREVIEW, Place, Tone},
};
use pm_shell::welcome::{Button, Frame, Names, Progress as Bar, Row, Severity, Stat, StepRow};
use std::{cell::RefCell, collections::BTreeMap, path::PathBuf, rc::Rc};

/// The Welcome frame for the step, as the gate and Settings show it: the
/// outcome and its counts first, the exceptions in full, the rest by name.
pub fn frame(model: &Model) -> Frame {
    let view = model.view();
    Frame {
        surface: view.surface.into(),
        top: true,
        eyebrow: Some("Extensions From the Original App".into()),
        title: view.title,
        line: view.line,
        stats: model
            .stats()
            .into_iter()
            .map(|(n, label)| Stat {
                value: n.to_string(),
                label: label.into(),
            })
            .collect(),
        steps: model
            .rows()
            .into_iter()
            .map(|row| StepRow {
                title: row.title,
                value: row.value,
                progress: row.active.map(|fraction| match fraction {
                    Some(fraction) => Bar::Determinate(fraction),
                    None => Bar::Indeterminate,
                }),
            })
            .collect(),
        rows_title: model.items_title().map(str::to_owned),
        rows: model
            .items()
            .into_iter()
            .map(|item| {
                let (symbol, severity) = match item.tone {
                    Tone::Warning => ("exclamationmark.triangle.fill", Severity::Warning),
                    Tone::Note => ("info.circle", Severity::Note),
                };
                Row {
                    symbol,
                    severity,
                    text: item.name,
                    detail: Some(item.reason).filter(|r| !r.is_empty()),
                }
            })
            .collect(),
        names: model.listed().map(|(title, names)| Names {
            title: title.into(),
            names,
            preview: PREVIEW,
        }),
        note: model.note().map(str::to_owned),
        details: model.details().map(str::to_owned),
        buttons: view
            .buttons
            .iter()
            .map(|b| Button {
                id: b.id.into(),
                label: b.label.into(),
                enabled: b.enabled,
                leading: false,
            })
            .collect(),
        default: view.default.map(str::to_owned),
        ..Frame::default()
    }
}

/// Whether this launch may offer: an existing library and an original app.
pub fn eligible(config: Option<&Config>) -> Option<Config> {
    config
        .filter(|c| c.electron_profile.is_dir() && c.native_profile.is_dir())
        .cloned()
}
/// What the scan found, with each extension's display name.
#[derive(Clone, Debug, Default)]
pub struct Found {
    pub scan: Scan,
    pub names: BTreeMap<String, String>,
}
impl Found {
    pub fn model(self, place: Place) -> Model {
        Model::offer(self.scan, place).named(self.names)
    }
}
/// The manifest's display name for `id`, from the original app or here.
fn display_name(config: &Config, id: &str) -> Option<String> {
    [&config.electron_profile, &config.native_profile]
        .into_iter()
        .find_map(|root| {
            let path = root.join("extensions").join(id).join("manifest.json");
            let manifest: serde_json::Value =
                serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
            manifest["name"]
                .as_str()
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_owned)
        })
}
/// Read-only: hashes extension folders and reads their manifests' names;
/// compiles nothing.
pub fn scan(config: &Config) -> Result<Found, String> {
    let scan = transfer::scan(config).map_err(|e| format!("{e:#}"))?;
    let names = scan
        .entries
        .iter()
        .filter_map(|e| Some((e.id.clone(), display_name(config, &e.id)?)))
        .collect();
    Ok(Found { scan, names })
}
/// Not Now: the startup offer stays quiet until the person asks from Settings.
pub fn decline(profile: &std::path::Path) {
    if let Err(error) = transfer::set_declined(profile, true) {
        eprintln!("Couldn’t remember Not Now for extensions: {error:#}");
    }
}
/// Copies new and changed extensions, checking each one loads. Runs on its own
/// thread; `progress` is called there. Running it clears Not Now, so a later
/// extension is offered again at startup.
pub fn run(config: &Config, progress: impl FnMut(Progress)) -> Result<TransferResult, String> {
    let _ = transfer::set_declined(&config.native_profile, false);
    // A dependency that arrives later in this same run is read from the
    // original folder (compiled, never written).
    let roots = [
        config.native_profile.join("extensions"),
        config.native_profile.join("extensions-shared"),
        config.electron_profile.join("extensions"),
    ];
    transfer::run(config, progress, |dir, manifest| {
        crate::extension_authoring::check_with_dependencies(dir, manifest, &roots)
            .map(|()| "It loads.".to_owned())
    })
    .map_err(|e| format!("{e:#}"))
}

#[derive(Debug)]
pub enum Update {
    Scanned(Result<Found, String>),
    Progress(Progress),
    Finished(Result<TransferResult, String>),
}
pub fn spawn_scan(config: Config, tx: async_channel::Sender<Update>) {
    std::thread::spawn(move || {
        let _ = tx.send_blocking(Update::Scanned(scan(&config)));
    });
}
pub fn spawn_run(config: Config, tx: async_channel::Sender<Update>) {
    std::thread::spawn(move || {
        let progress = tx.clone();
        let result = run(&config, move |p| {
            let _ = progress.send_blocking(Update::Progress(p));
        });
        let _ = tx.send_blocking(Update::Finished(result));
    });
}

/// Settings › Storage › Bring In Extensions…: the same step in its own window,
/// which closes (never quits) and leaves the editor as it was. `done` runs
/// after a transfer finished, to rescan extensions and Storage.
pub fn open_from_settings(
    original: PathBuf,
    profile: PathBuf,
    done: impl Fn(&mut gpui::App) + 'static,
    cx: &mut gpui::App,
) {
    thread_local! {
        static OPEN: RefCell<Option<gpui::WindowHandle<pm_shell::welcome_view::WelcomeView>>> =
            const { RefCell::new(None) };
        static OPEN_ACTION: RefCell<Option<pm_shell::welcome_view::OnAction>> =
            const { RefCell::new(None) };
    }
    // One at a time: a second press brings the open one forward.
    if let Some(handle) = OPEN.with(|o| *o.borrow())
        && handle.update(cx, |_, window, _| window.activate()).is_ok()
    {
        return;
    }
    let config = Config {
        electron_profile: original,
        native_profile: profile,
    };
    let (tx, rx) = async_channel::unbounded();
    spawn_scan(config.clone(), tx.clone());
    let model: Rc<RefCell<Option<Model>>> = Rc::default();
    let done = Rc::new(done);
    cx.spawn(async move |cx| {
        while let Ok(update) = rx.recv().await {
            let finished = matches!(update, Update::Finished(_));
            let opened = cx.update(|cx| {
                let first = model.borrow().is_none();
                {
                    let mut slot = model.borrow_mut();
                    match update {
                        Update::Scanned(Ok(found)) => *slot = Some(found.model(Place::Settings)),
                        Update::Scanned(Err(error)) => {
                            let mut m = Model::offer(Scan::default(), Place::Settings);
                            m.finished(Err(error));
                            *slot = Some(m);
                        }
                        Update::Progress(p) => {
                            if let Some(m) = slot.as_mut() {
                                m.progress(p)
                            }
                        }
                        Update::Finished(result) => {
                            if let Some(m) = slot.as_mut() {
                                m.finished(result)
                            }
                        }
                    }
                }
                let Some(current) = model.borrow().as_ref().map(frame) else {
                    return true;
                };
                if first {
                    let on_action: pm_shell::welcome_view::OnAction = Rc::new({
                        let (model, tx, config) = (model.clone(), tx.clone(), config.clone());
                        move |id, cx| match id {
                            "bring_in" | "try_again" => {
                                let frame = {
                                    let mut slot = model.borrow_mut();
                                    let Some(m) = slot.as_mut() else { return };
                                    if m.gate
                                        == pm_first_launch::surfaces::extensions::Gate::Running
                                    {
                                        return;
                                    }
                                    m.start();
                                    frame(m)
                                };
                                spawn_run(config.clone(), tx.clone());
                                if let Some(handle) = OPEN.with(|o| *o.borrow()) {
                                    let _ = handle.update(cx, |v, _, cx| v.set_frame(frame, cx));
                                }
                            }
                            "cancel" | "done" => {
                                if let Some(handle) = OPEN.with(|o| o.borrow_mut().take()) {
                                    let _ = handle.update(cx, |_, w, _| w.remove_window());
                                }
                            }
                            _ => {}
                        }
                    });
                    OPEN_ACTION.with(|a| *a.borrow_mut() = Some(on_action.clone()));
                    match pm_shell::welcome_view::open_welcome_window(
                        pm_shell::tokens(),
                        current,
                        on_action,
                        "Bring In Extensions",
                        false,
                        cx,
                    ) {
                        Ok(handle) => {
                            OPEN.with(|o| *o.borrow_mut() = Some(handle));
                            if let Some(act) = OPEN_ACTION.with(|a| a.borrow().clone()) {
                                review(act, cx);
                            }
                        }
                        Err(error) => {
                            eprintln!("Could not open Bring In Extensions: {error:#}");
                            return false;
                        }
                    }
                } else if let Some(handle) = OPEN.with(|o| *o.borrow()) {
                    let _ = handle.update(cx, |v, _, cx| v.set_frame(current, cx));
                }
                if finished {
                    done(cx);
                }
                true
            });
            if !opened {
                break;
            }
        }
    })
    .detach();
}

/// Development builds only: `--review-welcome wait:4,bring_in`
/// (`PM_NATIVE_REVIEW_WELCOME`) presses this step's buttons by id, with
/// `wait:<seconds>` between, so review captures need no input on the screen.
pub fn review(act: Rc<dyn Fn(&str, &mut gpui::App)>, cx: &mut gpui::App) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    let Some(steps) = std::env::var("PM_NATIVE_REVIEW_WELCOME")
        .ok()
        .filter(|_| cfg!(debug_assertions))
    else {
        return;
    };
    ONCE.call_once(|| {
        cx.spawn(async move |cx| {
            for step in steps.split(',').map(str::trim) {
                if let Some(seconds) = step.strip_prefix("wait:") {
                    let seconds = seconds.parse::<f64>().unwrap_or(1.).clamp(0., 120.);
                    cx.background_executor()
                        .timer(std::time::Duration::from_secs_f64(seconds))
                        .await;
                } else if matches!(
                    step,
                    "bring_in" | "not_now" | "open_native" | "try_again" | "cancel" | "done"
                ) {
                    let _ = cx.update(|cx| act(step, cx));
                }
            }
        })
        .detach();
    });
}
