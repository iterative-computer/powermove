//! Home (v1.2.0's Projects screen, `legacy/ui/projects.ts`) over
//! `LibraryController`, drawn inside the editor window in place of its docks.
//! The worker owns all kernel and file work; this view renders its
//! presentation and turns host effects into sheets and alerts on the editor
//! window. Sidebar: search, Recents and Trash with counts, Store, the account
//! at the foot. Main: the section with its count, grid/list, sort, Open… and
//! New Project; cards and rows read name, Active/Open tag and v1's facts line.
use crate::{
    library_surface::{
        appkit::new_project_sheet,
        controller::{Content, HostEffect, LibraryController},
        form::Format,
        kernel::KernelLibraryPort,
        model::*,
    },
    sheets::OpenSheet,
};
use gpui::{prelude::*, *};
use pm_design::Tokens;
use pm_kernel::{KernelHandle, session::ShellReply};
use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc};

/// What Home needs from the editor window it is drawn in.
#[derive(Debug)]
pub enum Outbound {
    Opened {
        reply: Box<ShellReply>,
        name: String,
        file: Option<std::path::PathBuf>,
        new_window: bool,
    },
    /// The project is open: show it (this window) or bring its window forward.
    FocusProject(String),
    /// Escape, or the project this window shows was opened again.
    HideHome,
    /// A menu command this window runs (Open…, New Project, Store, sign-in).
    Command(&'static str),
}
pub type Sink = Rc<dyn Fn(Outbound, &mut App)>;
pub struct ProjectLibrary {
    pub controller: LibraryController<KernelLibraryPort>,
    sink: Sink,
    tokens: Tokens,
    focus: FocusHandle,
    sheet: Rc<RefCell<Option<OpenSheet>>>,
    /// Hover washes on sidebar rows, cards and buttons.
    motion: pm_shell::motion::Motion<u64>,
    /// The editor window Home is drawn in: its sheets and alerts attach here.
    editor: Option<objc2::rc::Retained<objc2_app_kit::NSWindow>>,
    /// v1's `projectsView`.
    pub layout: Layout,
    search: Option<Entity<pm_shell::text::TextField>>,
    /// The project this window shows (its card reads Active).
    pub active: Option<String>,
    /// Cards per grid row at the last layout, for arrow keys.
    columns: usize,
    /// A failure Home reports in its header (the editor's status line is
    /// hidden under Home), until the next action.
    notice: Option<String>,
    /// v1's Home settings (`projectsSection`, `projectsSort`, `projectsView`)
    /// are written through the kernel, the only writer.
    kernel: KernelHandle,
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
        editor: Option<objc2::rc::Retained<objc2_app_kit::NSWindow>>,
        kernel: KernelHandle,
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
        let setting = |key: &str| {
            kernel
                .state()
                .settings
                .get(key)
                .and_then(pm_model::Value::as_str)
                .map(str::to_owned)
                .unwrap_or_default()
        };
        let mut controller = LibraryController::new(port);
        controller.query.section = Section::from_key(&setting("projectsSection"));
        controller.query.sort = Sort::from_key(&setting("projectsSort"));
        let layout = Layout::from_key(&setting("projectsView"));
        Self {
            controller,
            sink,
            tokens,
            focus: cx.focus_handle(),
            sheet: Rc::default(),
            motion: Default::default(),
            editor,
            layout,
            search: None,
            active: None,
            columns: 1,
            notice: None,
            kernel,
        }
    }
    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }
    /// Takes finished work and turns the controller's effects into platform calls.
    pub fn pump(&mut self, cx: &mut Context<Self>) {
        let busy = self.controller.busy();
        self.controller.poll();
        // A change made here shows in every other window's Home too, as
        // v1's `projects:open` repainted them.
        if busy && !self.controller.busy() {
            let me = cx.entity_id();
            cx.defer(move |cx| {
                for editor in crate::entry::editors(cx) {
                    let library = editor.read(cx).library.clone();
                    if let Some(library) = library.filter(|l| l.entity_id() != me) {
                        library.update(cx, |v, cx| {
                            let _ = v.controller.refresh();
                            v.pump(cx);
                        });
                    }
                }
            });
        }
        if self.controller.opening().is_some() {
            self.notice = None;
        }
        for effect in self.controller.take_effects() {
            self.effect(effect, cx);
        }
        cx.notify();
    }
    /// Sheets and alerts attach to the editor window Home is drawn in,
    /// whether or not the app is active.
    fn parent(&self) -> Option<objc2::rc::Retained<objc2_app_kit::NSWindow>> {
        if let Some(window) = self.editor.clone() {
            return Some(window);
        }
        let mtm = objc2::MainThreadMarker::new()?;
        objc2_app_kit::NSApplication::sharedApplication(mtm).keyWindow()
    }
    fn effect(&mut self, effect: HostEffect, cx: &mut Context<Self>) {
        match effect {
            HostEffect::ShowNewProject => self.show_new_project(cx),
            HostEffect::CloseNewProject => {
                if let Some(sheet) = self.sheet.borrow_mut().take() {
                    sheet.close();
                }
            }
            HostEffect::Rename { id } => self.show_rename(id, cx),
            HostEffect::Alert { serial, alert, .. } => {
                let Some(window) = self.parent() else {
                    return;
                };
                let rx = alert.sheet(&window);
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
            HostEffect::OpenEditor {
                reply,
                name,
                file,
                new_window,
            } => {
                if let Some(sheet) = self.sheet.borrow_mut().take() {
                    sheet.close();
                }
                (self.sink)(
                    Outbound::Opened {
                        reply: Box::new(reply),
                        name,
                        file,
                        new_window,
                    },
                    cx,
                );
            }
            HostEffect::FocusProject(id) => (self.sink)(Outbound::FocusProject(id), cx),
            HostEffect::Announce(text) => {
                if let Some(window) = self.parent() {
                    crate::v4_native::announce(&window, &text);
                }
            }
            HostEffect::IntegrationGap(gap) => eprintln!("Project Library: {gap}"),
            HostEffect::Diagnostic(detail) => {
                crate::error_log::shared(cx).borrow_mut().push("Open", &detail);
            }
        }
    }
    fn show_new_project(&mut self, cx: &mut Context<Self>) {
        let (Some(form), Some(window)) = (self.controller.form.clone(), self.parent()) else {
            return;
        };
        let mtm = objc2::MainThreadMarker::new().unwrap();
        let sheet = new_project_sheet(&form, &self.tokens, mtm);
        let open = OpenSheet::present((sheet, Default::default()), &window, Some("name"));
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
            if let Some(colour) = open.changed_colour("background") {
                form.background = colour;
            }
            let (width, height) = (open.text("width"), open.text("height"));
            match action {
                // The end of a size edit: a locked aspect moves the other side.
                "width" | "height" => {
                    let is_width = action == "width";
                    let draft = if is_width { width } else { height };
                    let current = if is_width { &form.width } else { &form.height };
                    if draft != *current {
                        let _ = form.edit_dimension(is_width, draft);
                    }
                    open.set_text("width", &form.width);
                    open.set_text("height", &form.height);
                }
                "link" => {
                    form.linked = form.linked.map(|linked| !linked);
                    if let Some(link) = open.sheet.buttons.get("link") {
                        crate::library_surface::appkit::show_link(link, form);
                    }
                }
                "resolution" => {
                    let _ =
                        form.choose_preset(open.popup("resolution").filter(|i| *i < PRESET_COUNT));
                    open.set_text("width", &form.width);
                    open.set_text("height", &form.height);
                }
                // Mid-edit, a locked size waits for the edit to end; an unlocked
                // one is taken as typed so Create tracks it. Create takes what
                // the fields show, even if Return beat the end of an edit.
                _ if (form.linked != Some(true) || action == "create")
                    && (width != form.width || height != form.height) =>
                {
                    let _ = form.edit_size(width, height);
                }
                _ => {}
            }
            open.enable("create", form.can_create());
        }
        match action {
            "cancel" => {
                drop(sheet);
                self.controller.escape();
                if let Some(open) = self.sheet.borrow_mut().take() {
                    open.close();
                }
                self.pump(cx);
                true
            }
            "create" => {
                drop(sheet);
                if self.controller.create().is_err() {
                    self.notice = Some("The project couldn’t be created.".into());
                }
                self.pump(cx);
                false
            }
            _ => false,
        }
    }
    fn show_rename(&mut self, id: String, cx: &mut Context<Self>) {
        use crate::dialogs::{DialogId, Field, NativePiece, Surface, WindowId};
        let (Some(draft), Some(window)) = (self.controller.rename.clone(), self.parent()) else {
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
                            v.controller.escape();
                        }
                        v.pump(cx);
                    });
                    break;
                }
            }
        })
        .detach();
    }
    fn set_section(&mut self, section: Section, cx: &mut Context<Self>) {
        let mut query = self.controller.query.clone();
        query.section = section;
        self.persist("projectsSection", section.key());
        let _ = self.controller.set_query(query);
        self.pump(cx);
    }
    fn set_sort(&mut self, sort: Sort, cx: &mut Context<Self>) {
        let mut query = self.controller.query.clone();
        query.sort = sort;
        self.persist("projectsSort", sort.key());
        let _ = self.controller.set_query(query);
        self.pump(cx);
    }
    fn set_layout(&mut self, layout: Layout, cx: &mut Context<Self>) {
        self.layout = layout;
        self.persist("projectsView", layout.key());
        cx.notify();
    }
    fn set_text(&mut self, text: String, cx: &mut Context<Self>) {
        if text == self.controller.query.text {
            return;
        }
        let mut query = self.controller.query.clone();
        query.text = text;
        let _ = self.controller.set_query(query);
        self.pump(cx);
    }
    /// One of v1's Home settings, through the kernel (off the main thread).
    fn persist(&self, key: &str, value: &str) {
        drop(
            self.kernel
                .state_transaction(pm_kernel::state::StateTransaction {
                    expected_revision: None,
                    ops: vec![pm_kernel::state::StateOp::Setting {
                        key: key.into(),
                        value: Some(pm_model::Value::String(value.into())),
                    }],
                }),
        );
    }
    fn on_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let columns = match self.layout {
            Layout::Grid => self.columns.max(1) as isize,
            Layout::List => 1,
        };
        match event.keystroke.key.as_str() {
            "up" => self.controller.move_selection(-columns),
            "down" => self.controller.move_selection(columns),
            "left" if columns > 1 => self.controller.move_selection(-1),
            "right" if columns > 1 => self.controller.move_selection(1),
            "enter" => {
                if let Some(id) = self.controller.selected.clone() {
                    let _ = self
                        .controller
                        .double_click(&id, event.keystroke.modifiers.alt);
                }
            }
            "escape" if !self.controller.query.text.is_empty() => {
                self.clear_search(cx);
            }
            // v1: Escape leaves Home for the project behind it.
            "escape" => (self.sink)(Outbound::HideHome, cx),
            _ => return,
        }
        cx.stop_propagation();
        self.pump(cx);
    }
    pub(crate) fn clear_search(&mut self, cx: &mut Context<Self>) {
        if let Some(field) = &self.search {
            field.update(cx, |f, cx| f.set_value("", cx));
        }
        self.set_text(String::new(), cx);
    }
    fn row_menu(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.controller.rows.iter().find(|r| r.id == id) else {
            return;
        };
        let actions = context_menu(row);
        let this_window = self.active.as_deref() == Some(id.as_str());
        let options: Vec<_> = actions
            .iter()
            .map(|action| match action {
                // v1: the project this window shows is already open here.
                Some(MenuAction::Open) if this_window => {
                    serde_json::json!({"label": MenuAction::Open.label(), "disabled": true})
                }
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
    fn sort_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.controller.query.sort;
        let sorts = [Sort::Recent, Sort::Name];
        let options: Vec<_> = sorts
            .iter()
            .map(|s| serde_json::json!({"label": s.label(), "checked": *s == current}))
            .collect();
        let this = cx.entity();
        crate::v4_native::menu(window, &options, cx, move |index, cx| {
            if let Some(sort) = index.and_then(|i| sorts.get(i).copied()) {
                this.update(cx, |v, cx| v.set_sort(sort, cx));
            }
        });
    }
    /// v1's account row: Sign in, or your account's menu.
    fn account_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let items: [(&str, &'static str); 3] = [
            ("Your Library", "store"),
            ("Account Settings…", "settings"),
            ("Sign Out", "cloudSignOut"),
        ];
        let options: Vec<_> = items
            .iter()
            .map(|(label, _)| serde_json::json!({ "label": label }))
            .collect();
        let sink = self.sink.clone();
        crate::v4_native::menu(window, &options, cx, move |index, cx| {
            if let Some((_, command)) = index.and_then(|i| items.get(i)) {
                sink(Outbound::Command(command), cx);
            }
        });
    }
}
/// Development builds: an on-screen review runs a row's context-menu choice
/// (the NSMenu callback's body) without pressing a menu item.
#[cfg(debug_assertions)]
impl ProjectLibrary {
    pub fn review_row_menu(&mut self, name: &str, label: &str, cx: &mut Context<Self>) {
        let Some(row) = self.controller.rows.iter().find(|r| r.name == name) else {
            return;
        };
        let id = row.id.clone();
        if let Some(action) = context_menu(row)
            .into_iter()
            .flatten()
            .find(|a| a.label() == label)
        {
            let _ = self.controller.select(&id);
            let _ = self.controller.menu(&id, action);
            self.pump(cx);
        }
    }
}
const PRESET_COUNT: usize = pm_builtins::RESOLUTIONS.len();
/// Hover key for a sidebar section, row or button.
fn key(name: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut h);
    h.finish()
}
impl ProjectLibrary {
    /// A hover wash (spring) over any control; `selected` is the neutral on state.
    fn wash(
        &self,
        d: Stateful<Div>,
        name: &str,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = &self.tokens;
        let k = key(name);
        let amount = self.motion.hovers.amount(k);
        let alpha = (t.translucency.hover * amount * 255.).round() as u32;
        let radius = t.metrics.control_radius;
        d.relative()
            .when(selected, |d| d.bg(rgb(t.colors.fill_strong)))
            .when(!selected && alpha > 0, |d| {
                d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded(px(radius))
                        .bg(rgba(t.colors.text << 8 | alpha)),
                )
            })
            .on_hover(cx.listener(move |v, hovered: &bool, _, cx| {
                let tokens = v.tokens.clone();
                v.motion.hovers.hover(k, *hovered, &tokens);
                cx.notify();
            }))
    }
    /// A sidebar row: (symbol, label), trailing count.
    fn nav_row(
        &self,
        id: &'static str,
        (symbol, label): (&str, String),
        count: Option<usize>,
        selected: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = &self.tokens;
        let (c, m) = (&t.colors, &t.metrics);
        let row = div()
            .id(id)
            .debug_selector(|| id.into())
            .flex()
            .flex_none()
            .items_center()
            .gap(px(m.gap))
            .h(px(m.row))
            .px(px(m.gap))
            .rounded(px(m.control_radius))
            .text_color(rgb(if selected { c.text } else { c.secondary }))
            .role(Role::Button)
            .aria_selected(selected)
            .aria_label(match count {
                Some(n) => format!("{label}, {n}"),
                None => label.clone(),
            })
            .child(pm_shell::symbol::symbol(
                symbol,
                m.symbol_inline,
                pm_shell::symbol::Weight::Regular,
                if selected { c.text } else { c.muted },
                window,
            ))
            .child(div().flex_1().min_w(px(0.)).truncate().child(label))
            .children(count.map(|n| {
                div()
                    .flex_none()
                    .text_size(px(t.typography.label))
                    .text_color(rgb(c.muted))
                    .font_features(FontFeatures(Arc::new(vec![("tnum".into(), 1)])))
                    .child(n.to_string())
            }));
        self.wash(row, id, selected, cx)
    }
    /// A quiet push button (`field` ground); `primary` is the window's one
    /// accent action.
    fn button(
        &self,
        id: &'static str,
        label: &'static str,
        symbol: Option<&str>,
        primary: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = &self.tokens;
        let (c, m) = (&t.colors, &t.metrics);
        let ink = if primary { c.selected_text } else { c.text };
        // A pop-up's chevron trails its title; an action's symbol leads.
        let trailing = symbol.is_some_and(|s| s.starts_with("chevron"));
        let glyph = symbol.map(|s| {
            pm_shell::symbol::symbol(
                s,
                m.symbol_inline,
                pm_shell::symbol::Weight::Medium,
                if trailing { c.muted } else { ink },
                window,
            )
        });
        let (lead, trail) = if trailing {
            (None, glyph)
        } else {
            (glyph, None)
        };
        let d = div()
            .id(id)
            .debug_selector(|| id.into())
            .flex()
            .flex_none()
            .items_center()
            .gap(px(m.border * 4.))
            .h(px(m.control))
            .px(px(m.inset))
            .rounded(px(m.control_radius))
            .bg(rgb(if primary { c.accent } else { c.field }))
            .text_color(rgb(ink))
            .font_weight(FontWeight(t.typography.medium_weight))
            .role(Role::Button)
            .aria_label(label)
            .children(lead)
            .child(label)
            .children(trail);
        self.wash(d, id, false, cx)
    }
    /// One icon segment of the grid/list toggle.
    fn segment(
        &self,
        layout: Layout,
        symbol: &str,
        label: &'static str,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = &self.tokens;
        let (c, m) = (&t.colors, &t.metrics);
        let on = self.layout == layout;
        let d = div()
            .id(label)
            .flex()
            .items_center()
            .justify_center()
            .size(px(m.control))
            .rounded(px(m.control_radius))
            .role(Role::Button)
            .aria_label(label)
            .aria_selected(on)
            .child(pm_shell::symbol::symbol(
                symbol,
                m.symbol_inline,
                pm_shell::symbol::Weight::Regular,
                if on { c.text } else { c.muted },
                window,
            ))
            .on_click(cx.listener(move |v, _, _, cx| v.set_layout(layout, cx)));
        self.wash(d, label, on, cx)
    }
    /// Click selects; double-click (or Return) opens, ⌥ in a new window;
    /// right-click selects and shows the row's menu.
    fn interactive(&self, d: Stateful<Div>, id: &str, cx: &mut Context<Self>) -> Stateful<Div> {
        d.on_click({
            let id = id.to_owned();
            cx.listener(move |v, event: &ClickEvent, window, cx| {
                window.focus(&v.focus, cx);
                let _ = v.controller.select(&id);
                if event.click_count() >= 2 {
                    let _ = v.controller.double_click(&id, event.modifiers().alt);
                }
                v.pump(cx);
            })
        })
        .on_mouse_down(MouseButton::Right, {
            let id = id.to_owned();
            cx.listener(move |v, _, window, cx| {
                let _ = v.controller.select(&id);
                v.row_menu(id.clone(), window, cx);
            })
        })
    }
    /// v1's tag beside the name: Active (this window) or Open (elsewhere).
    fn tag(&self, row: &Row) -> Option<&'static str> {
        if row.trashed {
            None
        } else if self.active.as_deref() == Some(row.id.as_str()) {
            Some("Active")
        } else if row.open {
            Some("Open")
        } else {
            None
        }
    }
    fn card(&self, row: &Row, width: f32, now: i64, cx: &mut Context<Self>) -> Stateful<Div> {
        let t = &self.tokens;
        let (c, m, ty) = (&t.colors, &t.metrics, &t.typography);
        let selected = self.controller.selected.as_ref() == Some(&row.id);
        let facts = row.facts(now);
        let tag = self.tag(row);
        let k = key(&format!("card-{}", row.id));
        let wash = self.motion.hovers.amount(k);
        let alpha = (t.translucency.hover * wash * 255.).round() as u32;
        // The project's poster frame; without one the frame shows the
        // canvas size instead of a blank slab.
        let poster = (!row.trashed)
            .then(|| crate::thumbnails::poster(&row.id))
            .flatten();
        let dims = poster
            .is_none()
            .then(|| row.dimensions().map(|(w, h)| format!("{w}×{h}")))
            .flatten();
        let thumb = div()
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .w(px(width))
            .h(px(width * 9. / 16.))
            .rounded(px(m.control_radius))
            .bg(rgb(c.field))
            .border(px(if selected { m.border * 2. } else { m.border }))
            .border_color(rgb(if selected { c.secondary } else { c.hairline }))
            .text_size(px(ty.label))
            .text_color(rgb(c.muted))
            .overflow_hidden()
            .children(poster.map(|path| {
                img(path)
                    .absolute()
                    .inset_0()
                    .size_full()
                    .object_fit(ObjectFit::Cover)
                    .rounded(px(m.control_radius))
            }))
            .children(dims)
            .when(alpha > 0, |d| {
                d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded(px(m.control_radius))
                        .bg(rgba(c.text << 8 | alpha)),
                )
            });
        let card = div()
            .id(SharedString::from(format!("project-{}", row.id)))
            .debug_selector(|| format!("project-{}", row.id))
            .flex()
            .flex_col()
            .flex_none()
            .w(px(width))
            .gap(px(m.gap))
            .role(Role::ListItem)
            .aria_selected(selected)
            .aria_label(match tag {
                Some(tag) => format!("{}, {tag}, {facts}", row.name),
                None => format!("{}, {facts}", row.name),
            })
            .child(thumb)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .px(px(m.border * 2.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(m.gap))
                            .child(
                                div()
                                    .min_w(px(0.))
                                    .truncate()
                                    .font_weight(FontWeight(ty.medium_weight))
                                    .child(row.name.clone()),
                            )
                            .children(tag.map(|tag| {
                                div()
                                    .flex_none()
                                    .text_size(px(ty.label))
                                    .text_color(rgb(c.muted))
                                    .child(tag)
                            })),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(ty.label))
                            .text_color(rgb(c.muted))
                            .child(facts),
                    ),
            )
            .on_hover(cx.listener(move |v, hovered: &bool, _, cx| {
                let tokens = v.tokens.clone();
                v.motion.hovers.hover(k, *hovered, &tokens);
                cx.notify();
            }));
        self.interactive(card, &row.id, cx)
    }
    fn list_row(&self, row: &Row, now: i64, cx: &mut Context<Self>) -> Stateful<Div> {
        let t = &self.tokens;
        let (c, m, ty) = (&t.colors, &t.metrics, &t.typography);
        let selected = self.controller.selected.as_ref() == Some(&row.id);
        let facts = row.facts(now);
        let tag = self.tag(row);
        let d = div()
            .id(SharedString::from(format!("project-{}", row.id)))
            .debug_selector(|| format!("project-{}", row.id))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(m.inset))
            .h(px(m.row))
            .px(px(m.gap))
            .rounded(px(m.control_radius))
            .role(Role::ListItem)
            .aria_selected(selected)
            .aria_label(match tag {
                Some(tag) => format!("{}, {tag}, {facts}", row.name),
                None => format!("{}, {facts}", row.name),
            })
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w(px(0.))
                    .items_center()
                    .gap(px(m.gap))
                    .child(div().min_w(px(0.)).truncate().child(row.name.clone()))
                    .children(tag.map(|tag| {
                        div()
                            .flex_none()
                            .text_size(px(ty.label))
                            .text_color(rgb(c.muted))
                            .child(tag)
                    })),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(ty.label))
                    .text_color(rgb(c.muted))
                    .font_features(FontFeatures(Arc::new(vec![("tnum".into(), 1)])))
                    .child(facts),
            );
        let d = self.wash(d, &format!("row-{}", row.id), selected, cx);
        self.interactive(d, &row.id, cx)
    }
}
impl Render for ProjectLibrary {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.motion.frame(window);
        let t = self.tokens.clone();
        let (c, m, ty) = (&t.colors, &t.metrics, &t.typography);
        if self.search.is_none() && pm_shell::text::available(window) {
            let tokens = t.clone();
            let search = cx.new(|cx| {
                let mut field = pm_shell::text::TextField::new(
                    pm_shell::text::TextKind::Primary,
                    "Search projects",
                    &self.controller.query.text,
                    tokens,
                    window,
                    cx,
                );
                field.set_placeholder("Search projects");
                field
            });
            cx.subscribe(&search, |v: &mut Self, _, event, cx| {
                use pm_shell::text::TextEvent;
                if let TextEvent::Changed(text)
                | TextEvent::Committed(text)
                | TextEvent::Submitted(text) = event
                {
                    v.set_text(text.clone(), cx);
                }
            })
            .detach();
            self.search = Some(search);
        }
        let section = self.controller.query.section;
        let live = self
            .controller
            .snapshot
            .rows
            .iter()
            .filter(|r| !r.trashed)
            .count();
        let trash = self.controller.snapshot.rows.len() - live;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        let (ground, ground_alpha) = t.ground();
        // Sidebar (v1): search, Recents and Trash with counts, Store, then
        // the account at the foot.
        let account = crate::cloud::fact(cx);
        let identity = account
            .get("identity")
            .and_then(serde_json::Value::as_str)
            .filter(|_| account.get("signedIn") == Some(&serde_json::Value::Bool(true)))
            .map(str::to_owned);
        let sidebar = div()
            .id("home-sidebar")
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(m.border))
            .w(px(m.settings_sidebar))
            .h_full()
            .p(px(m.gap))
            .bg(pm_shell::view::tint(ground, ground_alpha))
            .role(Role::Group)
            .aria_label("Home")
            .children(self.search.clone().map(|f| div().mb(px(m.gap)).child(f)))
            .child(
                self.nav_row(
                    "home-recents",
                    ("clock", "Recents".into()),
                    Some(live.min(12)),
                    section != Section::Deleted,
                    window,
                    cx,
                )
                .on_click(cx.listener(|v, _, _, cx| v.set_section(Section::Recents, cx))),
            )
            .child(
                self.nav_row(
                    "home-trash",
                    ("trash", "Trash".into()),
                    Some(trash),
                    section == Section::Deleted,
                    window,
                    cx,
                )
                .on_click(cx.listener(|v, _, _, cx| v.set_section(Section::Deleted, cx))),
            )
            .child(div().h(px(m.gap)))
            .child(
                self.nav_row(
                    "home-store",
                    ("basket", "Store".into()),
                    None,
                    false,
                    window,
                    cx,
                )
                .on_click(cx.listener(|v, _, _, cx| (v.sink)(Outbound::Command("store"), cx))),
            )
            .child(div().flex_1())
            .child(match identity {
                Some(name) => self
                    .nav_row(
                        "home-account",
                        ("person.crop.circle", name),
                        None,
                        false,
                        window,
                        cx,
                    )
                    .on_click(cx.listener(|v, _, window, cx| v.account_menu(window, cx))),
                None => self
                    .nav_row(
                        "home-account",
                        ("person.crop.circle", "Sign in".into()),
                        None,
                        false,
                        window,
                        cx,
                    )
                    .on_click(
                        cx.listener(|v, _, _, cx| (v.sink)(Outbound::Command("cloudSignIn"), cx)),
                    ),
            });
        // Header (v1): section and count, grid/list, sort, Open…, New Project.
        let shown = self.controller.rows.len();
        let count = format!(
            "{shown} {}",
            if shown == 1 { "project" } else { "projects" }
        );
        let sort = self.controller.query.sort;
        let opening = self
            .controller
            .opening()
            .map(|name| format!("Opening “{name}”…"))
            .or_else(|| self.notice.clone());
        let header = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(m.gap))
            .h(px(m.toolbar))
            .px(px(m.inset + m.gap))
            .child(
                div()
                    .id("home-heading")
                    .flex()
                    .flex_1()
                    .min_w(px(0.))
                    .items_baseline()
                    .gap(px(m.gap))
                    .role(Role::Heading)
                    .aria_label(format!("{}, {count}", section.title()))
                    .child(
                        div()
                            .text_size(px(ty.heading))
                            .font_weight(FontWeight(ty.semibold_weight))
                            .child(section.title()),
                    )
                    .child(
                        div()
                            .text_size(px(ty.label))
                            .text_color(rgb(c.muted))
                            .child(count),
                    )
                    // A real project can take seconds to read: say so here
                    // until it shows in the window.
                    .children(opening.map(|name| {
                        div()
                            .id("home-status")
                            .min_w(px(0.))
                            .truncate()
                            .text_size(px(ty.label))
                            .text_color(rgb(c.secondary))
                            .role(Role::Status)
                            .aria_label(name.clone())
                            .child(name)
                    })),
            )
            .child(
                div()
                    .id("home-layout")
                    .flex()
                    .flex_none()
                    .gap(px(m.border * 2.))
                    .role(Role::Group)
                    .aria_label("Project layout")
                    .child(self.segment(Layout::Grid, "square.grid.2x2", "Grid view", window, cx))
                    .child(self.segment(Layout::List, "list.bullet", "List view", window, cx)),
            )
            .child(
                self.button(
                    "home-sort",
                    sort.label(),
                    Some("chevron.down"),
                    false,
                    window,
                    cx,
                )
                .aria_label(format!("Sort projects: {}", sort.label()))
                .on_click(cx.listener(|v, _, window, cx| v.sort_menu(window, cx))),
            )
            .child(
                self.button("home-open", "Open…", None, false, window, cx)
                    .on_click(cx.listener(|v, _, _, cx| (v.sink)(Outbound::Command("open"), cx))),
            )
            .child(
                self.button("home-new", "New Project", Some("plus"), true, window, cx)
                    .on_click(
                        cx.listener(|v, _, _, cx| (v.sink)(Outbound::Command("newProject"), cx)),
                    ),
            );
        // Body: cards or rows; an empty section says so with v1's copy.
        let width = f32::from(window.viewport_size().width)
            - m.settings_sidebar
            - m.gap * 2.
            - (m.inset + m.gap) * 2.;
        let card = m.thumbnail_min * 2.;
        self.columns = (((width + m.inset) / (card + m.inset)).floor() as usize).max(1);
        let body: AnyElement = match self.controller.content() {
            Content::Loading => div().flex_1().into_any_element(),
            Content::Unavailable { message, action } => div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(m.gap))
                .text_color(rgb(c.secondary))
                .child(message)
                .child(
                    self.button("home-retry", action, None, false, window, cx)
                        .on_click(cx.listener(|v, _, _, cx| {
                            let _ = v.controller.refresh();
                            v.pump(cx);
                        })),
                )
                .into_any_element(),
            Content::Empty(state) => div()
                .id("home-empty")
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .pt(px(m.toolbar * 1.5))
                .gap(px(m.gap))
                .role(Role::Group)
                .aria_label(state.title)
                .child(pm_shell::symbol::symbol(
                    state.symbol,
                    m.symbol_toolbar,
                    pm_shell::symbol::Weight::Regular,
                    c.muted,
                    window,
                ))
                .child(
                    div()
                        .font_weight(FontWeight(ty.semibold_weight))
                        .child(state.title),
                )
                .child(
                    div()
                        .max_w(px(m.sheet_width))
                        .text_center()
                        .text_size(px(ty.reading))
                        .text_color(rgb(c.muted))
                        .child(state.detail.clone()),
                )
                .when_some(state.action, |d, action| {
                    d.child(
                        self.button("home-create", action, Some("plus"), false, window, cx)
                            .on_click(cx.listener(|v, _, _, cx| {
                                (v.sink)(Outbound::Command("newProject"), cx)
                            })),
                    )
                })
                .into_any_element(),
            Content::Projects => {
                let rows = self.controller.rows.clone();
                let list = div()
                    .id("home-projects")
                    .flex_1()
                    .overflow_y_scroll()
                    .px(px(m.inset + m.gap))
                    .pb(px(m.inset))
                    .role(Role::List)
                    .aria_label(section.title());
                match self.layout {
                    Layout::Grid => list
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .gap(px(m.inset))
                                .children(rows.iter().map(|row| self.card(row, card, now, cx))),
                        )
                        .into_any_element(),
                    Layout::List => list
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(m.border))
                                .children(rows.iter().map(|row| self.list_row(row, now, cx))),
                        )
                        .into_any_element(),
                }
            }
        };
        let (edge, edge_alpha) = t.edge();
        let content = div()
            .flex_1()
            .min_w(px(0.))
            .m(px(m.gap))
            .ml(px(0.))
            .flex()
            .flex_col()
            .rounded(px(m.panel_radius))
            .bg(rgb(c.panel))
            .border(px(m.border))
            .border_color(pm_shell::view::tint(edge, edge_alpha))
            .overflow_hidden()
            .child(header)
            .child(body);
        div()
            .id("home")
            .debug_selector(|| "home".into())
            .size_full()
            .flex()
            .text_color(rgb(c.text))
            .text_size(px(ty.body))
            .font_family(pm_design::TEXT_FONT)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .child(sidebar)
            .child(content)
    }
}
/// The defaults of `createNewProjectForm`, for a window with no project.
pub fn default_format() -> Format {
    Format {
        width: 1920,
        height: 1080,
        fps: 30.,
        duration: 10.,
        background: "#09090A".into(),
    }
}
/// S018's `Current` seed: the open composition's format. A key the composition
/// doesn't author comes from the project, and a missing background from the
/// shipped default.
pub fn current_format(published: Option<&pm_kernel::Published>) -> Option<Format> {
    let snapshot = &published?.snapshot;
    let root = snapshot.root()?;
    let open = snapshot.open_comp();
    let comp = pm_kernel::model::comp_node(root, open.as_deref()).unwrap_or(root);
    let get = |key: &str| comp.get(key).or_else(|| root.get(key));
    let number = |key: &str| get(key).and_then(crate::node_number);
    Some(Format {
        width: number("w")? as u32,
        height: number("h")? as u32,
        fps: number("fps")?,
        duration: number("dur")?,
        background: get("bg")
            .and_then(crate::node_string)
            .map_or_else(|| default_format().background, str::to_owned),
    })
}

#[cfg(test)]
#[path = "project_library_tests.rs"]
mod tests;
