//! Settings is a window of its own again, one for the app (PM-375). The
//! editor that opened it supplies the settings model for the AppKit form. Another
//! editor asking for Settings takes the window over, so Project settings and
//! notes reflect the project it was asked from.
pub(crate) mod form;

use crate::session::Controller;
use gpui::*;
use pm_shell::settings_window::{Event, Section, SettingsChrome};
use serde_json::{Value, json};
use std::{cell::RefCell, rc::Rc};

thread_local! {
    /// The editor hosting the Settings window.
    static HOST: RefCell<Option<WeakEntity<Controller>>> = const { RefCell::new(None) };
}

/// Where the toolbar's back and forward go: the sections visited, in order.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct History {
    pages: Vec<String>,
    at: usize,
    /// The section back or forward asked for, which arriving does not record.
    going: Option<String>,
}
impl History {
    /// The page shows `id`.
    pub fn visit(&mut self, id: &str) {
        if self.going.as_deref() == Some(id) {
            self.going = None;
            return;
        }
        self.going = None;
        if self.pages.get(self.at).map(String::as_str) == Some(id) {
            return;
        }
        if !self.pages.is_empty() {
            self.pages.truncate(self.at + 1);
        }
        self.pages.push(id.to_owned());
        self.at = self.pages.len() - 1;
    }
    pub fn back(&mut self) -> Option<String> {
        self.at = self.at.checked_sub(1)?;
        self.going = self.pages.get(self.at).cloned();
        self.going.clone()
    }
    pub fn forward(&mut self) -> Option<String> {
        if self.at + 1 >= self.pages.len() {
            return None;
        }
        self.at += 1;
        self.going = self.pages.get(self.at).cloned();
        self.going.clone()
    }
    /// The section showing.
    pub fn current(&self) -> Option<String> {
        self.pages.get(self.at).cloned()
    }
    #[cfg(test)]
    pub fn can(&self) -> (bool, bool) {
        (self.at > 0, self.at + 1 < self.pages.len())
    }
}

/// Runs `closed` once the window `id` has closed, however it closed, after
/// the update that closed it.
pub(crate) fn watch_close(
    id: WindowId,
    closed: impl Fn(&mut App) + 'static,
    cx: &App,
) -> Subscription {
    let closed = Rc::new(closed);
    cx.on_window_closed(move |cx, gone| {
        if gone == id {
            let closed = closed.clone();
            cx.defer(move |cx| closed(cx));
        }
    })
}

/// `events` in order, each run of searches as its last.
fn latest_searches(events: Vec<Event>) -> Vec<Event> {
    let mut out: Vec<Event> = vec![];
    for event in events {
        if let (Event::Search(_), Some(Event::Search(_))) = (&event, out.last()) {
            out.pop();
        }
        out.push(event);
    }
    out
}

/// Frames the Settings window whose GPUI view is `view` (outside any update
/// of that window) and sends what is done in its chrome to `owner`.
pub(crate) fn install(
    view: objc2::rc::Retained<objc2_app_kit::NSView>,
    owner: Entity<Controller>,
    cx: &mut App,
) -> Option<Rc<SettingsChrome>> {
    let (tx, rx) = async_channel::unbounded();
    let chrome = Rc::new(SettingsChrome::install(view, tx)?);
    chrome.update_sidebar_appearance();
    // A check of the window as built, for any bundle (release included).
    if let Some(path) = std::env::var_os("PM_NATIVE_SETTINGS_PROBE") {
        let probe = chrome.clone();
        cx.spawn(async move |cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(2))
                .await;
            let _ = cx.update(|_| std::fs::write(&path, probe.report().to_string()));
        })
        .detach();
    }
    let weak = owner.downgrade();
    // One act at a time, in the order done: a search the panel answered
    // late must never overtake a newer one (typing dropped letters, and a
    // cleared field left the old results). A burst of typing sends only the
    // latest text.
    cx.spawn(async move |cx| {
        while let Ok(first) = rx.recv().await {
            let mut events = vec![first];
            while let Ok(more) = rx.try_recv() {
                events.push(more);
            }
            for event in latest_searches(events) {
                let Ok(act) = weak.update(cx, |v, _| v.settings_intent(event)) else {
                    return;
                };
                if let Some((runtime, intent)) = act {
                    cx.background_executor()
                        .spawn(async move {
                            if let Ok(ticket) = runtime.panel_act("settings", &intent) {
                                let _ = ticket.await;
                            }
                        })
                        .await;
                }
            }
        }
    })
    .detach();
    let runtime = owner.update(cx, |v, cx| {
        v.builtins.settings_chrome = Some(chrome.clone());
        v.builtins.settings_history = History::default();
        v.builtins.settings_form = Some(form::Form::new(chrome.page()));
        v.mount_settings_form(cx);
        v.builtins.runtime.clone()
    });
    // The panel showed before its window was here: it says again what the
    // sidebar lists.
    if let Some(runtime) = runtime {
        cx.background_executor()
            .spawn(async move {
                if let Ok(ticket) = runtime.panel_act("settings", &json!({ "tell": true })) {
                    let _ = ticket.await;
                }
            })
            .detach();
    }
    Some(chrome)
}

/// Settings › Project's poster: the first of `candidates` that exists, keyed
/// by its modification time. Reads the file system: call it off the main
/// thread when it can be (`FactsIo`).
pub(crate) struct PosterIo {
    pub(crate) candidates: [std::path::PathBuf; 2],
    pub(crate) id: String,
}
impl PosterIo {
    pub(crate) fn resolve(self) -> Option<String> {
        let id = self.id;
        self.candidates
            .into_iter()
            .find(|f| f.exists())
            .map(|file| {
                // Its key names the file it shows: a picture written since is a
                // new key, never the cached one.
                let stamp = std::fs::metadata(&file)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_millis());
                let key = format!("settings/poster/{id}/{stamp}");
                crate::media_thumbs::register(&key, file);
                key
            })
    }
}
/// The parts of the built-in panels' facts that read the file system
/// (Settings › Storage, Settings › Project's poster). `Controller::facts`
/// leaves them null; `resolve` fills them in, on a worker for the
/// publication path, so a click or a drag step never waits on the disk.
pub(crate) struct FactsIo {
    pub(crate) storage: crate::storage::StorageIo,
    pub(crate) poster: Option<PosterIo>,
}
impl FactsIo {
    pub(crate) fn resolve(self, facts: &mut Value) {
        facts["storageSettings"] = self.storage.resolve();
        if let Some(poster) = self.poster
            && facts["settingsHeader"].is_object()
        {
            facts["settingsHeader"]["poster"] = json!(poster.resolve());
        }
    }
}
#[cfg(test)]
mod facts_io_tests {
    use super::{FactsIo, PosterIo};
    use crate::storage::StorageIo;
    use serde_json::{Value, json};
    /// The facts a worker completes are the ones `Controller::facts` made on
    /// the main thread: Storage's rows, and the header's poster keyed by the
    /// first picture that exists (registered for the panel to load).
    #[test]
    fn resolving_off_the_main_thread_fills_what_facts_left_null() {
        let dir = tempfile::tempdir().unwrap();
        let (home, last) = (dir.path().join("home.png"), dir.path().join("last.png"));
        std::fs::write(&last, b"png").unwrap();
        let stamp = std::fs::metadata(&last)
            .unwrap()
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let storage = || StorageIo {
            generated: Some(42),
            electron: None,
            cache_limit: 0,
        };
        let mut facts = json!({"storageSettings": null,
            "settingsHeader": {"path": null, "poster": null, "projects": []}});
        FactsIo {
            storage: storage(),
            poster: Some(PosterIo {
                candidates: [home.clone(), last.clone()],
                id: "p1".into(),
            }),
        }
        .resolve(&mut facts);
        // Rows are `storage::rows` of the last measurement (a global other
        // tests replace while this runs): their shape, not their text.
        let rows = facts["storageSettings"]["rows"].as_array().unwrap();
        assert!(!rows.is_empty());
        let key = format!("settings/poster/p1/{stamp}");
        assert_eq!(facts["settingsHeader"]["poster"], json!(key));
        assert_eq!(crate::media_thumbs::resolve(&key), Some(last.clone()));
        // The Home poster wins once it exists.
        std::fs::write(&home, b"png").unwrap();
        let picked = PosterIo {
            candidates: [home.clone(), last],
            id: "p1".into(),
        }
        .resolve()
        .unwrap();
        assert_eq!(crate::media_thumbs::resolve(&picked), Some(home));
        // No project: the header stays null and nothing is looked up.
        let mut facts = json!({"storageSettings": null, "settingsHeader": null});
        FactsIo {
            storage: storage(),
            poster: None,
        }
        .resolve(&mut facts);
        assert_eq!(facts["settingsHeader"], Value::Null);
        assert!(facts["storageSettings"]["rows"].is_array());
    }
}
impl Controller {
    /// Settings is the key window.
    pub(crate) fn settings_in_front(&self, cx: &App) -> bool {
        self.builtins
            .windows
            .get("settings")
            .is_some_and(|h| cx.active_window() == Some(*h))
    }
    pub(crate) fn close_settings(&mut self, cx: &mut Context<Self>) {
        let handle = self.builtins.windows.get("settings").copied();
        self.settings_closed(cx);
        if let Some(handle) = handle {
            let _ = handle.update(cx, |_, w, _| w.remove_window());
        }
    }
    /// The Settings window is gone (closed here, or by its close button or
    /// ⌘W): this editor no longer hosts it, and a restart of its panels does
    /// not show it again (PM-397).
    pub(crate) fn settings_closed(&mut self, _cx: &mut Context<Self>) {
        self.builtins.windows.remove("settings");
        self.cache_closed_settings_form();
        self.builtins.settings_chrome = None;
        self.builtins.settings_me = None;
        self.builtins.settings_watch = None;
        self.builtins.settings_history = History::default();
    }
    /// Settings › Project's header: this project's poster and path, and the
    /// projects open in every editor for the picker (listed only by the
    /// editor hosting Settings). Its poster is left null for `PosterIo`:
    /// built-in facts are made on every publication, and the look-up runs
    /// on the worker that hands them to the panels (`FactsIo::resolve`).
    pub(crate) fn settings_header_deferred(&self, cx: &App) -> (Value, Option<PosterIo>) {
        let Some(session) = &self.session else {
            return (Value::Null, None);
        };
        let id = &session.project;
        // Its Home poster, else the picture its viewer last showed (kept
        // for every project opened here), else none.
        let (file, _) = crate::poster::files(&self.profile.join("caches").join("thumbnails"), id);
        let (last, _) = crate::poster::last_frame_files(&self.profile, id);
        let poster = PosterIo {
            candidates: [file, last],
            id: id.clone(),
        };
        let this = json!({ "id": id, "name": self.project_name() });
        let projects: Vec<Value> = match self.builtins.settings_me {
            Some(me) => crate::entry::editor_windows(cx)
                .into_iter()
                .filter_map(|(editor, _)| {
                    if editor.entity_id() == me {
                        return Some(this.clone());
                    }
                    let other = editor.read(cx);
                    let session = other.session.as_ref()?;
                    Some(json!({ "id": session.project, "name": other.project_name() }))
                })
                .collect(),
            None => vec![this],
        };
        let path = self.path.as_ref().map(|path| {
            let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
            match home.as_deref().and_then(|h| path.strip_prefix(h).ok()) {
                Some(rest) => format!("~/{}", rest.display()),
                None => path.display().to_string(),
            }
        });
        let header = json!({ "path": path, "poster": Value::Null, "projects": projects });
        (header, Some(poster))
    }
    /// The header's picker: Settings shows project `id`, so the editor that
    /// has it open takes the window over.
    pub(crate) fn settings_for_project(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.session.as_ref().is_some_and(|s| s.project == id) {
            return;
        }
        let Some((editor, window)) = crate::entry::editor_windows(cx)
            .into_iter()
            .filter(|(e, _)| e.entity_id() != cx.entity_id())
            .find(|(e, _)| e.read(cx).session.as_ref().is_some_and(|s| s.project == id))
        else {
            return;
        };
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, cx| {
                editor.update(cx, |v, cx| {
                    v.builtins.settings_page = Some("project".into());
                    v.show_builtin("settings", window, cx);
                })
            });
        });
    }
    /// This editor is to host Settings: one another editor hosts closes.
    pub(crate) fn take_settings_from_others(&mut self, cx: &mut Context<Self>) {
        self.builtins.settings_me = Some(cx.entity_id());
        let me = cx.entity().downgrade();
        let other = HOST.with(|h| h.borrow_mut().replace(me.clone()));
        if let Some(other) = other
            && other != me
        {
            let _ = other.update(cx, |v, cx| v.close_settings(cx));
        }
    }
    /// The sidebar, search field and back/forward as the page now is
    /// (`settings.sidebar` from the panel).
    pub(crate) fn settings_sidebar(&mut self, input: &Value, _cx: &mut Context<Self>) {
        let Some(chrome) = self.builtins.settings_chrome.clone() else {
            return;
        };
        let sections: Vec<Section> = input["sections"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| {
                Some(Section {
                    id: s["id"].as_str()?.to_owned(),
                    title: s["title"].as_str()?.to_owned(),
                    account_name: (s["id"] == "account")
                        .then(|| input["accountName"].as_str().map(str::to_owned))
                        .flatten(),
                    account_email: (s["id"] == "account")
                        .then(|| input["accountEmail"].as_str().map(str::to_owned))
                        .flatten(),
                    dim: s["dim"].as_bool().unwrap_or(false),
                })
            })
            .collect();
        let active = input["active"].as_str().unwrap_or_default();
        chrome.show(&sections, active);
        chrome.set_search(input["query"].as_str().unwrap_or_default());
        if !active.is_empty() {
            self.builtins.settings_history.visit(active);
            pm_shell::settings_window::remember_section(active);
        }
    }
    /// What the panel is asked to do for `event` (and the runtime to ask).
    pub(crate) fn settings_intent(
        &mut self,
        event: Event,
    ) -> Option<(std::sync::Arc<pm_builtins::Builtins>, Value)> {
        let intent = match event {
            Event::Select(id) => json!({ "select": id }),
            Event::Escape => json!({ "escape": true }),
            Event::Search(query) => json!({ "search": query }),
            Event::Back => match self.builtins.settings_history.back() {
                Some(id) => json!({ "select": id }),
                None => return None,
            },
            Event::Forward => match self.builtins.settings_history.forward() {
                Some(id) => json!({ "select": id }),
                None => return None,
            },
        };
        Some((self.builtins.runtime.clone()?, intent))
    }
}

#[cfg(test)]
mod tests {
    use super::History;

    /// Typing sends only its latest text, and a search typed after a section
    /// was picked still follows it (the acts stay in order).
    #[test]
    fn a_burst_of_typing_asks_once_with_the_latest_text() {
        use pm_shell::settings_window::Event::*;
        let s = |t: &str| Search(t.into());
        assert_eq!(
            super::latest_searches(vec![s("p"), s("pr"), s("pro"), Select("media".into()), s("pro"), s("")]),
            vec![s("pro"), Select("media".into()), s("")]
        );
        assert_eq!(super::latest_searches(vec![Back, Forward]), vec![Back, Forward]);
    }
    use gpui::{AppContext as _, Empty, TestAppContext, WindowOptions};
    use std::{cell::Cell, rc::Rc};

    /// PM-397: closing the Settings window by its close button or ⌘W (not
    /// through the editor) is heard, once; another window closing is not.
    #[gpui::test]
    fn a_settings_window_closed_any_way_is_heard_once(cx: &mut TestAppContext) {
        let open = |cx: &mut TestAppContext| {
            cx.update(|cx| cx.open_window(WindowOptions::default(), |_, cx| cx.new(|_| Empty)).unwrap())
        };
        let (settings, other) = (open(cx), open(cx));
        let heard = Rc::new(Cell::new(0));
        let _watch = cx.update(|cx| {
            let heard = heard.clone();
            super::watch_close(settings.window_id(), move |_| heard.set(heard.get() + 1), cx)
        });
        other.update(cx, |_, w, _| w.remove_window()).unwrap();
        cx.run_until_parked();
        assert_eq!(heard.get(), 0);
        settings.update(cx, |_, w, _| w.remove_window()).unwrap();
        cx.run_until_parked();
        assert_eq!(heard.get(), 1);
    }

    #[test]
    fn back_and_forward_walk_the_sections_visited() {
        let mut h = History::default();
        h.visit("general");
        assert_eq!(h.can(), (false, false));
        h.visit("media");
        h.visit("storage");
        assert_eq!(h.back().as_deref(), Some("media"));
        // Arriving where back went records nothing.
        h.visit("media");
        assert_eq!(h.can(), (true, true));
        assert_eq!(h.forward().as_deref(), Some("storage"));
        h.visit("storage");
        assert_eq!(h.back().as_deref(), Some("media"));
        h.visit("media");
        // A new visit drops what was ahead.
        h.visit("accounts");
        assert_eq!(h.can(), (true, false));
        assert_eq!(h.back().as_deref(), Some("media"));
        h.visit("media");
        assert_eq!(h.back().as_deref(), Some("general"));
        h.visit("general");
        assert_eq!(h.back(), None);
    }
}

/// Settings comes forward on `page` (the billing return link brings
/// Settings › Account): in the editor hosting it, else the first one.
pub(crate) fn show_page(page: &'static str, cx: &mut App) {
    let editors = crate::entry::editor_windows(cx);
    let host = HOST.with(|h| h.borrow().clone()).and_then(|w| w.upgrade());
    let Some((editor, window)) = editors
        .iter()
        .find(|(e, _)| host.as_ref().is_some_and(|h| h == e))
        .or(editors.first())
        .cloned()
    else {
        return;
    };
    cx.defer(move |cx| {
        let _ = window.update(cx, |_, window, cx| {
            editor.update(cx, |v, cx| {
                v.builtins.settings_page = Some(page.into());
                v.show_builtin("settings", window, cx);
            })
        });
    });
}
