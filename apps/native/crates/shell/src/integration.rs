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
    /// Commands that cannot run now, so menus never offer a silent no-op.
    fn disabled(&self, _cx: &App) -> std::collections::BTreeSet<String> {
        Default::default()
    }
    fn project_id(&self, cx: &App) -> Option<String>;
    fn status(&self, cx: &App) -> String;
    fn drop_paths(&self, paths: Vec<std::path::PathBuf>, cx: &mut App);
}
pub type Factory = Rc<dyn Fn(Rc<RefCell<Host>>, &mut Window, &mut App) -> Rc<dyn Integration>>;
pub(crate) struct AppIntegration(pub Factory);
impl gpui::Global for AppIntegration {}
