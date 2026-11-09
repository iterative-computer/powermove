//! Window ▸ Error Log (S053). Rows are what the app told the person, newest
//! first, on the recovery lane's table recipe. Nothing here leaves this Mac
//! unless the person saves a diagnostics bundle, which holds no free text.
//!
//! Rows persist through pm-services `diagnostics` on one worker thread that
//! owns the profile's `diagnostics` store; the main thread only sends to it.
//! The same worker records an unexpected quit as a crash at the next launch.
use gpui::{prelude::*, *};
use pm_services::{
    diagnostics::{CrashRecord, Diagnostics, Level, LogRedactor},
    storage::ServiceStorage,
};
use pm_shell::tokens;
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{OnceLock, mpsc},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const LOG_TITLE: &str = "Error Log";
const LOG_EMPTY_TITLE: &str = "No Errors";
const LOG_EMPTY_LINE: &str = "Problems Powermove notices appear here.";
const UNEXPECTED_QUIT: &str = "Powermove quit unexpectedly last time.";
/// Rows brought back from earlier launches.
const KEPT_ROWS: usize = 500;
const RUNNING: &str = "app.session.running";

#[derive(Clone, Debug)]
pub struct Entry {
    pub at: SystemTime,
    pub source: String,
    pub message: String,
}
#[derive(Default)]
pub struct Log {
    pub entries: Vec<Entry>,
    /// Waiting for the status bar of the next project to open.
    notice: Option<String>,
}
impl Log {
    /// A row, plus one status bar line: for failures nothing else would show.
    pub fn tell(&mut self, source: &str, message: &str) {
        self.push(source, message);
        self.notice = Some(message.into());
    }
    pub fn take_notice(&mut self) -> Option<String> {
        self.notice.take()
    }
    /// Repeats of the newest row collapse: a stuck failure is one row.
    pub fn push(&mut self, source: &str, message: &str) {
        if self
            .entries
            .last()
            .is_some_and(|e| e.source == source && e.message == message)
        {
            return;
        }
        self.entries.push(Entry {
            at: SystemTime::now(),
            source: source.into(),
            message: message.into(),
        });
        send(Persist::Log {
            code: code(source),
            text: message.into(),
        });
    }
    /// Rows from earlier launches go before this launch's.
    fn restore(&mut self, earlier: Vec<Entry>) {
        let now = std::mem::take(&mut self.entries);
        self.entries = earlier;
        self.entries.extend(now);
    }
    fn clear(&mut self) {
        self.entries.clear();
        send(Persist::Clear);
    }
}
/// The stored code for a source name (`Extension`, `Open`): letters and digits.
fn code(source: &str) -> String {
    source
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .take(128)
        .collect()
}

enum Persist {
    Log { code: String, text: String },
    Clear,
    Bundle(PathBuf, async_channel::Sender<Result<(), String>>),
    Ended(async_channel::Sender<()>),
}
static WORKER: OnceLock<mpsc::Sender<Persist>> = OnceLock::new();
fn send(request: Persist) {
    if let Some(worker) = WORKER.get() {
        let _ = worker.send(request);
    }
}
/// The person's home folder reads as `~` in stored rows.
struct Home(Option<String>);
impl LogRedactor for Home {
    fn redact(&mut self, text: &str) -> String {
        match &self.0 {
            Some(home) if !home.is_empty() => text.replace(home.as_str(), "~"),
            _ => text.into(),
        }
    }
}
/// Starts the store worker once per process and brings back earlier rows.
pub fn persist(profile: &Path, cx: &mut App) {
    if WORKER.get().is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel();
    let _ = WORKER.set(tx);
    let (rows_tx, rows_rx) = async_channel::bounded(1);
    let profile = profile.to_path_buf();
    let spawned = std::thread::Builder::new()
        .name("diagnostics".into())
        .spawn(move || worker(&profile, rx, rows_tx));
    if let Err(error) = spawned {
        eprintln!("Error Log can't be kept: {error}");
        return;
    }
    let log = shared(cx);
    cx.spawn(async move |cx| {
        if let Ok(rows) = rows_rx.recv().await {
            log.borrow_mut().restore(rows);
            cx.update(|cx| cx.refresh_windows());
        }
    })
    .detach();
}
fn worker(profile: &Path, rx: mpsc::Receiver<Persist>, rows: async_channel::Sender<Vec<Entry>>) {
    let mut storage = match ServiceStorage::open(profile.join("diagnostics")) {
        Ok(storage) => storage,
        Err(error) => {
            eprintln!("Error Log can't be kept: {error}");
            let _ = rows.try_send(vec![]);
            // Still answer quit and bundle requests.
            for request in rx {
                match request {
                    Persist::Bundle(_, reply) => {
                        let _ = reply.try_send(Err(error.to_string()));
                    }
                    Persist::Ended(reply) => {
                        let _ = reply.try_send(());
                    }
                    _ => {}
                }
            }
            return;
        }
    };
    let mut redactor = Home(std::env::var("HOME").ok());
    launch(&mut storage, profile, &mut redactor);
    let earlier = Diagnostics::recent(&storage, KEPT_ROWS)
        .unwrap_or_default()
        .into_iter()
        .map(|row| Entry {
            at: UNIX_EPOCH + Duration::from_millis(row.at_ms),
            source: row.code.replace('-', " "),
            message: row.text,
        })
        .collect();
    let _ = rows.try_send(earlier);
    for request in rx {
        match request {
            Persist::Log { code, text } => {
                let _ = Diagnostics::log(&mut storage, &mut redactor, Level::Error, &code, &text);
            }
            Persist::Clear => {
                let _ = Diagnostics::clear(&mut storage);
            }
            Persist::Bundle(path, reply) => {
                let result = Diagnostics::crashes(&storage)
                    .and_then(|ids| Diagnostics::bundle(&storage, &ids))
                    .map_err(|e| e.to_string())
                    .and_then(|bytes| std::fs::write(&path, bytes).map_err(|e| e.to_string()));
                let _ = reply.try_send(result);
            }
            Persist::Ended(reply) => {
                let _ = storage.archive_setting(RUNNING);
                let _ = reply.try_send(());
            }
        }
    }
}
/// A launch while the last one's marker is still set follows an unexpected
/// quit: record it as a crash, say so once in the log, and set the marker.
fn launch(storage: &mut ServiceStorage, profile: &Path, redactor: &mut Home) {
    if let Ok(Some(previous)) = storage.setting(RUNNING) {
        let crash = CrashRecord {
            id: format!("shell-{}", previous.as_u64().unwrap_or(0)),
            process: "shell".into(),
            reason: "unknown".into(),
            recovered: unsaved_work(profile),
            recovered_document: None,
        };
        if Diagnostics::crash(storage, crash).is_ok() {
            let _ = Diagnostics::log(
                storage,
                redactor,
                Level::Error,
                "Powermove",
                UNEXPECTED_QUIT,
            );
        }
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    let _ = storage.set(RUNNING, serde_json::json!(now));
}
/// Journals holding unsaved sessions: the next open offers that work back.
fn unsaved_work(profile: &Path) -> bool {
    std::fs::read_dir(profile.join("journals")).is_ok_and(|dirs| {
        dirs.flatten()
            .any(|d| !pm_kernel::journal::sessions(&d.path()).is_empty())
    })
}
/// A clean quit clears the running marker. A worker that died drops the
/// reply, so this never waits on it.
pub async fn session_ended() {
    let (tx, rx) = async_channel::bounded(1);
    send(Persist::Ended(tx));
    let _ = rx.recv().await;
}
/// System-relative time, in the column's short form.
fn age(at: SystemTime) -> String {
    let minutes = at.elapsed().map_or(0, |e| e.as_secs() / 60);
    match minutes {
        0 => "Now".into(),
        1..=59 => format!("{minutes} min ago"),
        60..1440 => format!("{} hr ago", minutes / 60),
        1440..2880 => "Yesterday".into(),
        _ => format!("{} days ago", minutes / 1440),
    }
}
pub type Shared = Rc<RefCell<Log>>;
#[derive(Default)]
struct AppLog(Shared);
impl Global for AppLog {}
/// One log per app: failures before any editor window opens are in it too.
pub fn shared(cx: &mut App) -> Shared {
    cx.default_global::<AppLog>().0.clone()
}
struct ErrorLogWindow(Shared);
impl ErrorLogWindow {
    /// A diagnostics bundle: levels, codes and crash kinds, never row text.
    fn save_diagnostics(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ns) = crate::platform::ns_window(window) else {
            return;
        };
        let chosen = crate::platform::paths_named(&ns, true, "Powermove Diagnostics.json");
        cx.spawn(async move |_, _| {
            let Ok(Some(path)) = chosen.recv().await else {
                return;
            };
            let (tx, rx) = async_channel::bounded(1);
            send(Persist::Bundle(path, tx));
            if let Ok(Err(_)) | Err(_) = rx.recv().await {
                crate::alert::note(
                    "The diagnostics couldn’t be saved.",
                    "Try again, or choose another location.",
                );
            }
        })
        .detach();
    }
}
impl Render for ErrorLogWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = tokens();
        let (c, m) = (&t.colors, &t.metrics);
        let empty = self.0.borrow().entries.is_empty();
        let button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .role(Role::Button)
                .aria_label(label)
                .tab_index(0)
                .px(px(m.inset))
                .h(px(m.row))
                .flex()
                .items_center()
                .rounded(px(m.control_radius))
                .bg(rgb(c.panel_secondary))
                .child(label)
        };
        let footer = div()
            .flex()
            .flex_none()
            .justify_end()
            .gap(px(m.gap))
            .p(px(m.inset))
            .child(
                button("save-diagnostics", "Save Diagnostics…")
                    .on_click(cx.listener(|v, _, window, cx| v.save_diagnostics(window, cx))),
            )
            .when(!empty, |f| {
                f.child(
                    button("clear-log", "Clear Log").on_click(cx.listener(|v, _, _, cx| {
                        v.0.borrow_mut().clear();
                        cx.notify();
                    })),
                )
            });
        let log = self.0.borrow();
        let mut body = div()
            .id("error-log")
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .p(px(m.inset))
            .bg(rgb(c.panel))
            .text_color(rgb(c.text))
            .text_size(px(t.typography.body))
            .font_family(pm_design::TEXT_FONT);
        if log.entries.is_empty() {
            body = body
                .items_center()
                .justify_center()
                .gap(px(m.gap))
                // The empty-state anatomy every panel uses: heading, then a
                // reading line in `secondary`.
                .child(
                    div()
                        .text_size(px(t.typography.heading))
                        .font_weight(FontWeight(t.typography.semibold_weight))
                        .child(LOG_EMPTY_TITLE),
                )
                .child(
                    div()
                        .text_size(px(t.typography.reading))
                        .text_color(rgb(c.secondary))
                        .child(LOG_EMPTY_LINE),
                );
        }
        for entry in log.entries.iter().rev() {
            body = body.child(
                div()
                    .flex()
                    .gap(px(m.inset))
                    .p(px(m.gap / 2.))
                    .child(
                        div()
                            .flex_none()
                            .w(px(m.settings_sidebar))
                            .text_color(rgb(c.secondary))
                            .child(format!("{} · {}", age(entry.at), entry.source)),
                    )
                    .child(entry.message.clone()),
            );
        }
        // A seamless title band over the window's own material, as Settings
        // and Export (`metrics.titlebar`).
        div()
            .size_full()
            .flex()
            .flex_col()
            .pt(px(m.titlebar))
            .bg(rgb(c.panel))
            .text_color(rgb(c.text))
            .text_size(px(t.typography.body))
            .font_family(pm_design::TEXT_FONT)
            .child(body)
            .child(footer)
    }
}
pub fn open(log: Shared, cx: &mut App) {
    if let Some(window) = cx
        .windows()
        .into_iter()
        .find(|w| w.downcast::<ErrorLogWindow>().is_some())
    {
        let _ = window.update(cx, |_, w, _| w.activate());
        return;
    }
    let t = tokens();
    let _ = cx.open_window(
        WindowOptions::new()
            .titlebar(Some(TitlebarOptions {
                title: Some(LOG_TITLE.into()),
                appears_transparent: true,
                ..Default::default()
            }))
            .window_bounds(Some(WindowBounds::Windowed(Bounds::new(
                point(px(t.metrics.toolbar), px(t.metrics.toolbar)),
                size(px(t.metrics.settings_width), px(t.metrics.settings_height)),
            )))),
        |_, cx| cx.new(|_| ErrorLogWindow(log)),
    );
}
#[cfg(test)]
mod tests {
    use super::{Home, Log, LogRedactor, age, code};
    use std::time::{Duration, SystemTime};
    #[test]
    fn stored_rows_use_codes_and_hide_the_home_folder() {
        assert_eq!(code("Media Offline"), "Media-Offline");
        assert_eq!(code("Open"), "Open");
        let mut home = Home(Some("/Users/someone".into()));
        assert_eq!(
            home.redact("“/Users/someone/Film.pmv” couldn’t be opened."),
            "“~/Film.pmv” couldn’t be opened."
        );
        assert_eq!(Home(None).redact("/Users/x"), "/Users/x");
    }
    #[test]
    fn rows_from_earlier_days_say_so() {
        let ago = |minutes: u64| SystemTime::now() - Duration::from_secs(minutes * 60);
        assert_eq!(age(ago(0)), "Now");
        assert_eq!(age(ago(5)), "5 min ago");
        assert_eq!(age(ago(180)), "3 hr ago");
        assert_eq!(age(ago(1500)), "Yesterday");
        assert_eq!(age(ago(3 * 1440 + 5)), "3 days ago");
    }
    #[test]
    fn repeats_collapse_and_order_is_kept() {
        let mut log = Log::default();
        log.push("Editor", "One");
        log.push("Editor", "One");
        log.push("Editor", "Two");
        log.push("Editor", "One");
        assert_eq!(log.entries.len(), 3);
    }
}
