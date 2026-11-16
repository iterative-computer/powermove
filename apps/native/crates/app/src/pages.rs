//! The Store opens inside the editor window as a page (owner feedback H6),
//! not as a window of its own: a cell in the toolbar's strip after the
//! projects, and while it shows it fills the window below the toolbar as
//! Home does. A project's tab (or Home) leaves it; its cell's close button
//! closes it. (Settings is a window of its own again, PM-375.)
use crate::session::Controller;
use gpui::{prelude::*, *};

/// The pages a window can show (Settings is a window of its own).
const TITLES: [(&str, &str); 1] = [("store", "Store")];

fn title(id: &str) -> Option<&'static str> {
    TITLES
        .iter()
        .find(|(page, _)| *page == id)
        .map(|(_, title)| *title)
}

/// The pages open in one window's strip, in the order they opened, and the
/// one showing over the project.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Pages {
    pub open: Vec<String>,
    pub shown: Option<String>,
    /// The page that showed before the one showing came forward (`None`:
    /// the project or Home), which shows again when that one closes.
    pub previous: Option<String>,
}
impl Pages {
    /// Opens `id` (or brings it forward); an id that is not a page opens
    /// nothing.
    pub fn open(&mut self, id: &str) {
        if title(id).is_none() {
            return;
        }
        if !self.open.iter().any(|p| p == id) {
            self.open.push(id.to_owned());
        }
        if self.shown.as_deref() != Some(id) {
            self.previous = self.shown.take();
        }
        self.shown = Some(id.to_owned());
    }
    /// Closes `id`; whatever it showed over comes back.
    pub fn close(&mut self, id: &str) {
        self.open.retain(|p| p != id);
        if self.previous.as_deref() == Some(id) {
            self.previous = None;
        }
        if self.shown.as_deref() == Some(id) {
            // What it came forward over shows again.
            self.shown = self
                .previous
                .take()
                .filter(|p| self.open.iter().any(|o| o == p));
        }
    }
    /// The toolbar's gear (and ⌘,): a page showing closes, back to what it
    /// came forward over; one open behind another tab comes forward (a
    /// second press then closes it). Whether it closed.
    pub fn toggle_closes(&self, id: &str) -> bool {
        self.shown.as_deref() == Some(id)
    }
    /// The project (or Home) again; the page stays open. Whether one showed.
    pub fn leave(&mut self) -> bool {
        self.previous = None;
        self.shown.take().is_some()
    }
    /// The page at place `index` in the strip.
    pub fn at(&self, index: usize) -> Option<&str> {
        self.open.get(index).map(String::as_str)
    }
    /// The strip's cells (id, title) and the place of the one showing.
    pub fn strip(&self) -> (Vec<(String, String)>, Option<usize>) {
        let shown = self
            .shown
            .as_ref()
            .and_then(|id| self.open.iter().position(|p| p == id));
        let cells = self
            .open
            .iter()
            .filter_map(|id| Some((id.clone(), title(id)?.to_owned())))
            .collect();
        (cells, shown)
    }
}
/// A strip command (`showPage:<i>`, `closePage:<i>`): whether it shows the
/// page and which place it names.
pub(crate) fn parse_command(name: &str) -> Option<(bool, Option<usize>)> {
    let (verb, index) = name.split_once(':')?;
    let show = match verb {
        "showPage" => true,
        "closePage" => false,
        _ => return None,
    };
    Some((show, index.parse().ok()))
}

impl Controller {
    /// Opens page `id` (or brings it forward) over the project.
    pub(crate) fn open_tab_page(&mut self, id: &str, cx: &mut Context<Self>) {
        self.pages.open(id);
        self.sync_pages();
        cx.notify();
    }
    /// Closes page `id`: what it showed over comes back.
    pub(crate) fn close_tab_page(&mut self, id: &str, cx: &mut Context<Self>) {
        self.pages.close(id);
        if id == "store" {
            self.store = None;
        }
        self.sync_pages();
        cx.notify();
    }
    /// The project (or Home) again; the page stays open in the strip.
    pub(crate) fn leave_page(&mut self, cx: &mut Context<Self>) {
        if self.pages.leave() {
            self.sync_pages();
            cx.notify();
        }
    }
    /// The strip's page commands, by the page's place in the strip.
    pub(crate) fn page_command(&mut self, name: &str, cx: &mut Context<Self>) -> bool {
        let Some((show, index)) = parse_command(name) else {
            return false;
        };
        let Some(id) = index.and_then(|i| self.pages.at(i)).map(str::to_owned) else {
            return true;
        };
        if show {
            self.open_tab_page(&id, cx);
        } else {
            self.close_tab_page(&id, cx);
        }
        true
    }
    pub(crate) fn sync_pages(&self) {
        let (cells, shown) = self.pages.strip();
        self.host.borrow().set_pages(cells, shown);
    }
    /// The page showing, for the shell to draw in place of the docks.
    pub(crate) fn page_element(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        // A page that just went: the playback loop places the timeline's
        // playhead and timecode again (as when Home goes).
        let Some(id) = self.pages.shown.clone() else {
            if std::mem::take(&mut self.page_drawn)
                && let Some(wake) = &self.playback_wake
            {
                let _ = wake.try_send(());
            }
            return None;
        };
        self.page_drawn = true;
        // The shell then reports no panel shown, which takes the timeline's
        // native playhead and timecode away (as under Home).
        self.host.borrow().set_home(self.home_shown());
        match id.as_str() {
            "store" => {
                let store = match self.store.clone() {
                    Some(store) => store,
                    None => {
                        let services = crate::store_window::Services::system(
                            self.kernel.clone(),
                            self.profile.clone(),
                        );
                        let tokens = self.host.borrow().tokens.clone();
                        let store = cx.new(|cx| {
                            let mut store =
                                crate::store_window::StoreWindow::new(services, tokens, cx);
                            store.embedded = true;
                            store.attach_search(window, cx);
                            store
                        });
                        let focus = store.read(cx).focus.clone();
                        window.focus(&focus, cx);
                        self.store = Some(store.clone());
                        store
                    }
                };
                Some(store.into_any_element())
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Pages, parse_command};

    #[test]
    fn the_store_opens_shows_leaves_and_closes_as_a_tab_does() {
        let mut p = Pages::default();
        p.open("store");
        assert_eq!(p.open, ["store"]);
        assert_eq!(p.shown.as_deref(), Some("store"));
        // Opening it again keeps the one cell.
        p.open("store");
        assert_eq!(p.open, ["store"]);
        let (cells, shown) = p.strip();
        assert_eq!(cells, [("store".to_owned(), "Store".to_owned())]);
        assert_eq!(shown, Some(0));
        // A project's tab leaves the page; it stays in the strip.
        assert!(p.leave());
        assert!(!p.leave());
        assert_eq!(p.strip().1, None);
        assert_eq!(p.at(0), Some("store"));
        // The toolbar's item again: forward, then closed back to the project.
        assert!(!p.toggle_closes("store"));
        p.open("store");
        assert!(p.toggle_closes("store"));
        p.close("store");
        assert_eq!(p.shown, None);
        assert!(p.open.is_empty());
    }

    #[test]
    fn settings_is_never_a_page() {
        // Settings is a window of its own in either Open Projects In mode.
        let mut p = Pages::default();
        p.open("settings");
        assert!(p.open.is_empty());
        assert_eq!(p.shown, None);
    }

    #[test]
    fn strip_commands_name_a_place() {
        assert_eq!(parse_command("showPage:1"), Some((true, Some(1))));
        assert_eq!(parse_command("closePage:0"), Some((false, Some(0))));
        assert_eq!(parse_command("closePage:x"), Some((false, None)));
        assert_eq!(parse_command("leaveHome"), None);
        assert_eq!(parse_command("panel:show:x"), None);
    }
}
