//! S017 Project Library and S018/S019 sheets over `LibraryController`. The worker
//! owns all kernel and file work; this view renders its presentation and turns
//! host effects into windows, sheets and alerts. Decision: one quiet list of names
//! with facts. The grid's thumbnail slot was never specified, and pro lists read
//! by name, so the native collection scaffold is gone.
use crate::{
    library_surface::{
        appkit::{new_project_sheet, present_alert},
        controller::{Content, HostEffect, Key, LibraryController, LibraryHost, Presentation},
        form::Format,
        kernel::KernelLibraryPort,
        model::*,
    },
    sheets::OpenSheet,
};
use gpui::{prelude::*, *};
use pm_design::Tokens;
use pm_kernel::{KernelHandle, session::ShellReply};
use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc, time::Duration};

/// What the library needs from whichever editor is in front.
#[derive(Debug)]
pub enum Outbound {
    Opened(Box<ShellReply>),
    Focus,
    Message(String),
}
pub type Sink = Rc<dyn Fn(Outbound, &mut App)>;
#[derive(Default)]
struct Effects(Vec<HostEffect>, Option<Presentation>);
impl LibraryHost for Effects {
    fn effect(&mut self, effect: HostEffect) {
        self.0.push(effect);
    }
    fn publish(&mut self, presentation: Presentation, _: WindowMetrics) {
        self.1 = Some(presentation);
    }
}
pub struct ProjectLibrary {
    pub controller: LibraryController<KernelLibraryPort>,
    pub presentation: Option<Presentation>,
    sink: Sink,
    tokens: Tokens,
    focus: FocusHandle,
    sheet: Rc<RefCell<Option<OpenSheet>>>,
    window: Option<WindowHandle<ProjectLibrary>>,
}
impl ProjectLibrary {
    /// The worker and its wake channel, started before the view exists.
    pub fn port(
        kernel: KernelHandle,
        profile: PathBuf,
    ) -> std::io::Result<(KernelLibraryPort, async_channel::Receiver<()>)> {
        let (tx, rx) = async_channel::bounded(1);
        let port = KernelLibraryPort::start(
            kernel,
            profile,
            Arc::new(move || {
                let _ = tx.try_send(());
            }),
        )?;
        Ok((port, rx))
    }
    pub fn new(
        port: KernelLibraryPort,
        rx: async_channel::Receiver<()>,
        tokens: Tokens,
        sink: Sink,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.spawn(async move |this, cx| {
            while rx.recv().await.is_ok() {
                if this.update(cx, |v, cx| v.pump(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        Self {
            controller: LibraryController::new(port),
            presentation: None,
            sink,
            tokens,
            focus: cx.focus_handle(),
            sheet: Rc::default(),
            window: None,
        }
    }
    /// Takes finished work and turns the controller's effects into platform calls.
    pub fn pump(&mut self, cx: &mut Context<Self>) {
        self.controller.poll();
        let mut host = Effects::default();
        self.controller
            .render(&mut host, &self.tokens, Duration::ZERO, 0..usize::MAX);
        self.presentation = host.1;
        for effect in host.0 {
            self.effect(effect, cx);
        }
        cx.notify();
    }
    fn parent(&self, cx: &App) -> Option<objc2::rc::Retained<objc2_app_kit::NSWindow>> {
        let _ = cx;
        let mtm = objc2::MainThreadMarker::new()?;
        objc2_app_kit::NSApplication::sharedApplication(mtm).keyWindow()
    }
    fn effect(&mut self, effect: HostEffect, cx: &mut Context<Self>) {
        match effect {
            HostEffect::ShowLibrary => self.show_window(cx),
            HostEffect::ShowNewProject => self.show_new_project(cx),
            HostEffect::CloseNewProject => {
                if let Some(sheet) = self.sheet.borrow_mut().take() {
                    sheet.close();
                }
            }
            HostEffect::Rename { id } => self.show_rename(id, cx),
            HostEffect::Alert { serial, alert, .. } => {
                let Some(window) = self.parent(cx) else {
                    return;
                };
                let rx = present_alert(&window, &alert);
                cx.spawn(async move |this, cx| {
                    let answer = rx.recv().await.ok().flatten();
                    let _ = this.update(cx, |v, cx| {
                        let _ = v.controller.answer(serial, answer);
                        v.pump(cx);
                    });
                })
                .detach();
            }
            HostEffect::Reveal(path) => crate::platform::reveal(&path),
            HostEffect::OpenEditor { reply, .. } => {
                if let Some(sheet) = self.sheet.borrow_mut().take() {
                    sheet.close();
                }
                (self.sink)(Outbound::Opened(Box::new(reply)), cx);
            }
            HostEffect::FocusProject(_) | HostEffect::SaveAs(_) => (self.sink)(Outbound::Focus, cx),
            HostEffect::Banner(text) => (self.sink)(Outbound::Message(text.into()), cx),
            HostEffect::Announce { text, .. } => {
                if let Some(window) = self.parent(cx) {
                    crate::v4_native::announce(&window, &text);
                }
            }
            HostEffect::IntegrationGap(gap) => eprintln!("Project Library: {gap}"),
            HostEffect::FocusSearch | HostEffect::Subtitle { .. } => {}
        }
    }
    fn show_new_project(&mut self, cx: &mut Context<Self>) {
        let (Some(form), Some(window)) = (self.controller.form.clone(), self.parent(cx)) else {
            return;
        };
        let mtm = objc2::MainThreadMarker::new().unwrap();
        let sheet = new_project_sheet(&form, &self.tokens, mtm);
        let open = OpenSheet::present(sheet, &window, Some("name"));
        let actions = open.actions.clone();
        *self.sheet.borrow_mut() = Some(open);
        cx.spawn(async move |this, cx| {
            while let Ok(action) = actions.recv().await {
                let done = this.update(cx, |v, cx| v.new_project_action(&action, cx));
                if !matches!(done, Ok(false)) {
                    break;
                }
            }
        })
        .detach();
    }
    /// Reads the sheet back into the form. Returns true once the sheet has ended.
    fn new_project_action(&mut self, action: &str, cx: &mut Context<Self>) -> bool {
        let sheet = self.sheet.clone();
        let sheet = sheet.borrow();
        let Some(open) = sheet.as_ref() else {
            return true;
        };
        if let Some(form) = self.controller.form.as_mut() {
            form.name = open.text("name");
            form.fps = open.text("fps");
            form.duration = open.text("duration");
            if let Some(colour) = open.colour("background") {
                form.background = colour;
            }
            let _ = form.edit_size(open.text("width"), open.text("height"));
            if action == "resolution" {
                let _ = form.choose_preset(open.popup("resolution").filter(|i| *i < PRESET_COUNT));
                if let Some(field) = open.sheet.fields.get("width") {
                    field.setStringValue(&objc2_foundation::NSString::from_str(&form.width));
                }
                if let Some(field) = open.sheet.fields.get("height") {
                    field.setStringValue(&objc2_foundation::NSString::from_str(&form.height));
                }
            }
            open.enable("create", form.can_create());
        }
        match action {
            "cancel" => {
                drop(sheet);
                let _ = self.controller.key(Key::Escape, 1);
                if let Some(open) = self.sheet.borrow_mut().take() {
                    open.close();
                }
                self.pump(cx);
                true
            }
            "create" => {
                drop(sheet);
                if self.controller.create().is_err() {
                    (self.sink)(
                        Outbound::Message("The project couldn’t be created.".into()),
                        cx,
                    );
                }
                self.pump(cx);
                false
            }
            _ => false,
        }
    }
    fn show_rename(&mut self, id: String, cx: &mut Context<Self>) {
        use crate::dialogs::{DialogId, Field, NativePiece, Surface, WindowId};
        let (Some(draft), Some(window)) = (self.controller.rename.clone(), self.parent(cx)) else {
            return;
        };
        let surface = Surface::sheet(
            DialogId(format!("rename:{id}")),
            WindowId("library".into()),
            "Rename Project",
            vec![Field::new(
                "name",
                "Name",
                NativePiece::TextField,
                vec![draft.text],
            )],
            "Rename",
        );
        let open = OpenSheet::present(
            crate::sheets::from_surface(&surface, &self.tokens),
            &window,
            Some("name"),
        );
        let actions = open.actions.clone();
        let sheet = Rc::new(open);
        cx.spawn(async move |this, cx| {
            while let Ok(action) = actions.recv().await {
                if action == "apply" || action == "cancel" {
                    let text = sheet.text("name");
                    sheet.close();
                    let _ = this.update(cx, |v, cx| {
                        if action == "apply" {
                            if let Some(rename) = v.controller.rename.as_mut() {
                                rename.text = text;
                            }
                            let _ = v.controller.commit_rename();
                        } else {
                            let _ = v.controller.key(Key::Escape, 1);
                        }
                        v.pump(cx);
                    });
                    break;
                }
            }
        })
        .detach();
    }
    fn show_window(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.window
            && handle.update(cx, |_, w, _| w.activate()).is_ok()
        {
            return;
        }
        let this = cx.entity();
        let m = self.tokens.metrics;
        let focus = self.focus.clone();
        let handle = cx.open_window(
            WindowOptions::new()
                .titlebar(Some(TitlebarOptions {
                    title: Some(TITLE.into()),
                    ..Default::default()
                }))
                .window_bounds(Some(WindowBounds::Windowed(Bounds::new(
                    point(px(m.toolbar), px(m.toolbar)),
                    size(px(m.settings_width), px(m.settings_height)),
                )))),
            move |window, cx| {
                window.focus(&focus, cx);
                this
            },
        );
        self.window = handle.ok();
    }
    fn set_section(&mut self, section: Section, cx: &mut Context<Self>) {
        let mut query = self.controller.query.clone();
        query.section = section;
        let _ = self.controller.set_query(query);
        self.pump(cx);
    }
    fn set_text(&mut self, text: String, cx: &mut Context<Self>) {
        let mut query = self.controller.query.clone();
        query.text = text;
        let _ = self.controller.set_query(query);
        self.pump(cx);
    }
    fn on_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let plain = !event.keystroke.modifiers.platform && !event.keystroke.modifiers.control;
        match key {
            "up" => self.controller.move_selection(-1),
            "down" => self.controller.move_selection(1),
            "enter" => {
                if let Some(id) = self.controller.selected.clone() {
                    let _ = self
                        .controller
                        .double_click(&id, event.keystroke.modifiers.alt);
                }
            }
            "backspace" if plain => {
                let mut text = self.controller.query.text.clone();
                text.pop();
                self.set_text(text, cx);
            }
            "escape" if !self.controller.query.text.is_empty() => {
                self.set_text(String::new(), cx);
            }
            _ if plain => {
                if let Some(text) = event.keystroke.key_char.as_deref() {
                    let next = format!("{}{text}", self.controller.query.text);
                    self.set_text(next, cx);
                }
            }
            _ => {}
        }
        self.pump(cx);
    }
    fn row_menu(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.controller.rows.iter().find(|r| r.id == id) else {
            return;
        };
        let actions = context_menu(row);
        let options: Vec<_> = actions
            .iter()
            .map(|action| match action {
                Some(action) => serde_json::json!({"label": action.label()}),
                None => serde_json::json!({"separator": true}),
            })
            .collect();
        let this = cx.entity();
        crate::v4_native::menu(window, &options, cx, move |index, cx| {
            let chosen = index.and_then(|i| actions.get(i).copied().flatten());
            if let Some(action) = chosen {
                this.update(cx, |v, cx| {
                    let _ = v.controller.menu(&id, action);
                    v.pump(cx);
                });
            }
        });
    }
}
const PRESET_COUNT: usize = crate::library_surface::form::PRESETS.len();
impl Render for ProjectLibrary {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.tokens.clone();
        let (c, m) = (&t.colors, &t.metrics);
        let mut sidebar = div()
            .flex()
            .flex_col()
            .gap(px(m.gap / 2.))
            .w(px(m.settings_sidebar))
            .p(px(m.gap));
        for (section, label) in [
            (Section::Recents, "Recents"),
            (Section::All, "All Projects"),
            (Section::Deleted, "Recently Deleted"),
        ] {
            let selected = self.controller.query.section == section;
            sidebar = sidebar.child(
                div()
                    .id(label)
                    .debug_selector(|| format!("section-{label}"))
                    .p(px(m.gap))
                    .rounded(px(m.control_radius))
                    .when(selected, |d| d.bg(rgb(c.selected_fill)))
                    .child(label)
                    .on_click(cx.listener(move |v, _, _, cx| v.set_section(section, cx))),
            );
        }
        let mut list = div()
            .id("library-list")
            .flex_1()
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .p(px(m.gap));
        if !self.controller.query.text.is_empty() {
            list = list.child(
                div()
                    .debug_selector(|| "library-search".into())
                    .p(px(m.gap))
                    .text_color(rgb(c.secondary))
                    .child(self.controller.query.text.clone()),
            );
        }
        match self.presentation.as_ref().map(|p| &p.content) {
            Some(Content::Empty(empty)) => {
                list = list.child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(px(m.gap))
                        .child(div().text_size(px(t.typography.title)).child(empty.title))
                        .child(
                            div()
                                .text_color(rgb(c.secondary))
                                .child(empty.detail.clone()),
                        ),
                );
            }
            Some(Content::Unavailable { message, .. }) => {
                list = list.child(div().p(px(m.inset)).child(*message));
            }
            _ => {}
        }
        for row in self.presentation.iter().flat_map(|p| &p.rows) {
            let id = row.id.clone();
            let selected = self.controller.selected.as_ref() == Some(&row.id);
            let facts = row.facts("").unwrap_or_default();
            list = list.child(
                div()
                    .id(SharedString::from(format!("project-{}", row.id)))
                    .debug_selector(|| format!("project-{id}", id = id.clone()))
                    .flex()
                    .justify_between()
                    .gap(px(m.inset))
                    .p(px(m.gap))
                    .rounded(px(m.control_radius))
                    .when(selected, |d| d.bg(rgb(c.selected_fill)))
                    .child(row.name.clone())
                    .child(div().text_color(rgb(c.secondary)).child(facts))
                    .on_click({
                        let id = id.clone();
                        cx.listener(move |v, event: &ClickEvent, _, cx| {
                            let _ = v.controller.select(&id);
                            if event.click_count() >= 2 {
                                let _ = v.controller.double_click(&id, false);
                            }
                            v.pump(cx);
                        })
                    })
                    .on_mouse_down(MouseButton::Right, {
                        let id = id.clone();
                        cx.listener(move |v, _, window, cx| {
                            let _ = v.controller.select(&id);
                            v.row_menu(id.clone(), window, cx);
                        })
                    }),
            );
        }
        div()
            .size_full()
            .flex()
            .bg(rgb(c.panel))
            .text_color(rgb(c.text))
            .text_size(px(t.typography.body))
            .font_family(pm_design::TEXT_FONT)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .child(sidebar)
            .child(list)
    }
}
/// S018's `Current` seed: the open composition's format, else the shipped default.
pub fn current_format(published: Option<&pm_kernel::Published>) -> Option<Format> {
    let root = published?.snapshot.root()?;
    let number = |key: &str| root.get(key).and_then(crate::node_number);
    Some(Format {
        width: number("w")? as u32,
        height: number("h")? as u32,
        fps: number("fps")?,
        duration: number("dur")?,
        background: root.get("bg").and_then(crate::node_string)?.to_owned(),
    })
}

#[cfg(test)]
#[path = "project_library_tests.rs"]
mod tests;
