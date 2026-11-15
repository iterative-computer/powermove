//! Settings is a window of its own again, one for the app (PM-375). The
//! editor that opened it hosts it: its Settings surface is the page, under
//! AppKit's sidebar and toolbar (`pm_shell::settings_window`). Another
//! editor asking for Settings takes the window over, so the Project and
//! Memory sections are always the project it was asked from.
use crate::session::Controller;
use gpui::*;
use pm_shell::settings_window::{Event, Section, SettingsChrome};
use serde_json::{Value, json};
use std::{cell::RefCell, rc::Rc};

thread_local! {
    /// The editor hosting the Settings window.
    static HOST: RefCell<Option<WeakEntity<Controller>>> = const { RefCell::new(None) };
}

/// The window's appearance for its chrome: dark or light, and the page's
/// ground.
pub(crate) fn scheme(t: &pm_design::Tokens) -> (bool, u32) {
    (t.scheme == pm_design::Scheme::Dark, t.colors.panel_secondary)
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
    pub fn can(&self) -> (bool, bool) {
        (self.at > 0, self.at + 1 < self.pages.len())
    }
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
    let (dark, ground) = scheme(&owner.read(cx).host.borrow().tokens);
    chrome.set_scheme(dark, ground);
    let weak = owner.downgrade();
    cx.spawn(async move |cx| {
        while let Ok(event) = rx.recv().await {
            if weak.update(cx, |v, cx| v.settings_event(event, cx)).is_err() {
                break;
            }
        }
    })
    .detach();
    let runtime = owner.update(cx, |v, _| {
        v.builtins.settings_chrome = Some(chrome.clone());
        v.builtins.settings_history = History::default();
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

impl Controller {
    /// Settings is the key window.
    pub(crate) fn settings_in_front(&self, cx: &App) -> bool {
        self.builtins
            .windows
            .get("settings")
            .is_some_and(|h| cx.active_window() == Some(*h))
    }
    pub(crate) fn close_settings(&mut self, cx: &mut Context<Self>) {
        self.builtins.settings_chrome = None;
        self.builtins.settings_me = None;
        if let Some(handle) = self.builtins.windows.remove("settings") {
            let _ = handle.update(cx, |_, w, _| w.remove_window());
        }
    }
    /// Settings › Project's header: this project's poster and path, and the
    /// projects open in every editor for the picker (listed only by the
    /// editor hosting Settings).
    pub(crate) fn settings_header(&self, cx: &App) -> Value {
        let Some(session) = &self.session else {
            return Value::Null;
        };
        let id = &session.project;
        let (file, _) = crate::poster::files(&self.profile.join("caches").join("thumbnails"), id);
        let poster = file.exists().then(|| {
            let key = format!("settings/poster/{id}");
            crate::media_thumbs::register(&key, file);
            key
        });
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
        json!({ "path": path, "poster": poster, "projects": projects })
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
                    project: s["project"].as_bool().unwrap_or(false),
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
        let (back, forward) = self.builtins.settings_history.can();
        chrome.set_history(back, forward);
    }
    pub(crate) fn settings_event(&mut self, event: Event, cx: &mut Context<Self>) {
        let intent = match event {
            Event::Select(id) => json!({ "select": id }),
            Event::Search(query) => json!({ "search": query }),
            Event::Back => match self.builtins.settings_history.back() {
                Some(id) => json!({ "select": id }),
                None => return,
            },
            Event::Forward => match self.builtins.settings_history.forward() {
                Some(id) => json!({ "select": id }),
                None => return,
            },
        };
        let Some(runtime) = self.builtins.runtime.clone() else {
            return;
        };
        cx.background_executor()
            .spawn(async move {
                if let Ok(ticket) = runtime.panel_act("settings", &intent) {
                    let _ = ticket.await;
                }
            })
            .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::History;

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
