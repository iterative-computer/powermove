//! Additive host seam. The app owns sessions/panels; the shell owns placement.
use crate::{
    layout::Rect,
    native::{Command, Host},
};
use gpui::{AnyElement, App, Window};
use std::{cell::RefCell, rc::Rc};

pub trait Integration {
    fn observe(&self, cx: &mut gpui::Context<crate::view::Editor>);
    fn command(&self, command: &Command, window: &mut Window, cx: &mut App) -> bool;
    fn panel(
        &self,
        id: &str,
        bounds: Rect,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement>;
    /// Content for panel `id`'s 32 pt header, replacing the shell's title
    /// (a tab group keeps its tab strip). `None` = the shell's header.
    fn header(
        &self,
        _id: &str,
        _bounds: Rect,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<AnyElement> {
        None
    }
    /// Home (v1's Projects screen), drawn in place of the docks while the
    /// window shows it; `None` = the docks.
    fn home(&self, _window: &mut Window, _cx: &mut App) -> Option<AnyElement> {
        None
    }
    /// Commands that cannot run now, so menus never offer a silent no-op.
    fn disabled(&self, _cx: &App) -> std::collections::BTreeSet<String> {
        Default::default()
    }
    /// True when a native surface below GPUI draws this panel's body. The shell
    /// leaves the body transparent and routes its pointer events to that surface.
    fn native_panel(&self, _id: &str, _cx: &App) -> bool {
        false
    }
    /// A tab's context menu. Return the entries to show now, or `None` when the
    /// app adds asynchronous items and shows the menu itself.
    fn panel_menu(
        &self,
        _id: &str,
        entries: Vec<crate::native::MenuEntry>,
        _at: (f32, f32),
        _cx: &mut App,
    ) -> Option<Vec<crate::native::MenuEntry>> {
        Some(entries)
    }
    /// The kernel's workspace store as this window sees it, the same `Rc`
    /// until something in it changes. `None`: the window keeps its own copy.
    fn workspaces(&self, _cx: &App) -> Option<std::rc::Rc<crate::workspace::Store>> {
        None
    }
    /// Sends one change to the kernel's workspace store, without waiting.
    fn write_workspace(&self, _write: crate::workspace::Write, _cx: &mut App) {}
    /// A passing note in the status line (the shell's own messages: a
    /// workspace saved or deleted, a definition that can't be used).
    fn note(&self, _text: &str, _cx: &mut App) {}
    /// Panels this window's extensions contribute: id and title.
    fn contributed_panels(&self, _cx: &App) -> Vec<(String, String)> {
        Vec::new()
    }
    /// The workspace's open panels, after any layout change.
    fn open_panels(&self, _open: &[&str], _cx: &mut App) {}
    /// The native panels with a body this frame: open ones outside it are tabs
    /// behind another, drawn by no one.
    fn shown_panels(&self, _shown: &[&str], _cx: &mut App) {}
    fn project_id(&self, cx: &App) -> Option<String>;
    fn status(&self, cx: &App) -> String;
    /// The panel's body paints its own ground (the timeline's track material);
    /// the shell then fills only its header.
    fn paints_own_ground(&self, _id: &str, _cx: &App) -> bool {
        false
    }
    /// Format facts on the status line's trailing side (`1920×1080 · 30 fps`).
    fn status_trailing(&self, _cx: &App) -> String {
        String::new()
    }
    /// A problem the person can press to see (`2 files offline`), in
    /// `danger` before the format facts; empty for none.
    fn status_alert(&self, _cx: &App) -> String {
        String::new()
    }
    fn status_alert_pressed(&self, _window: &mut gpui::Window, _cx: &mut App) {}
    fn drop_paths(&self, paths: Vec<std::path::PathBuf>, cx: &mut App);
}
pub type Factory = Rc<dyn Fn(Rc<RefCell<Host>>, &mut Window, &mut App) -> Rc<dyn Integration>>;
pub(crate) struct AppIntegration(pub Factory);
impl gpui::Global for AppIntegration {}
