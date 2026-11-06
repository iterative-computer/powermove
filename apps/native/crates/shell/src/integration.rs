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
    /// True when a native surface below GPUI draws this panel's body. The shell
    /// leaves the body transparent and routes its pointer events to that surface.
    fn native_panel(&self, _id: &str, _cx: &App) -> bool {
        false
    }
    /// The workspace's open panels, after any layout change.
    fn open_panels(&self, _open: &[&str], _cx: &mut App) {}
    fn project_id(&self, cx: &App) -> Option<String>;
    fn status(&self, cx: &App) -> String;
    fn drop_paths(&self, paths: Vec<std::path::PathBuf>, cx: &mut App);
}
pub type Factory = Rc<dyn Fn(Rc<RefCell<Host>>, &mut Window, &mut App) -> Rc<dyn Integration>>;
pub(crate) struct AppIntegration(pub Factory);
impl gpui::Global for AppIntegration {}
