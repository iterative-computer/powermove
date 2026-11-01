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
    fn project_id(&self, cx: &App) -> Option<String>;
    fn status(&self, cx: &App) -> String;
    fn drop_paths(&self, paths: Vec<std::path::PathBuf>, cx: &mut App);
}
pub type Factory = Rc<dyn Fn(Rc<RefCell<Host>>, &mut Window, &mut App) -> Rc<dyn Integration>>;
pub(crate) struct AppIntegration(pub Factory);
impl gpui::Global for AppIntegration {}
