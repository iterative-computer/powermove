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
use pm_design::{Elevation, Text, Tokens, motion::Preset};
use pm_kernel::{KernelHandle, session::ShellReply};
use pm_shell::{
    kit::{self, Button, Item, Kit, Recipes, ink},
    native_control::{ControlSpec, Kind, NativeControl},
    symbol::Weight,
};
use serde_json::{Value, json};

/// Home's own thumbnails: a cache under the profile, and the renderer of the
/// project this window has open. The kernel's library carries a thumbnail
/// only for projects v1 saved; everything else gets its poster frame here.
pub struct ThumbnailSource {
    /// `<profile>/caches/thumbnails`: `<project>.png` and its `.stamp`.
    pub dir: PathBuf,
    /// One frame of the open project as PNG, through the engine's frame path.
    pub renderer: Option<pm_compat_runner::FrameRenderer>,
}
use std::{cell::RefCell, collections::HashMap, path::PathBuf, rc::Rc, sync::Arc};

/// A menu opened from one of Home's buttons (ours, never NSMenu).
const ACCOUNT_MENU: [(&str, &str); 3] = [
    ("Your Library", "store"),
    ("Account Settings…", "settings"),
    ("Sign Out", "cloudSignOut"),
];
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Menu {
    Sort,
    Account,
}

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
    /// Hover washes, the gliding layout toggle, cards sliding to new places.
    motion: pm_shell::motion::Motion<u64>,
    /// Decoded thumbnails by project id, with the length of the source they
    /// were read from (a new save replaces it).
    thumbs: HashMap<String, (usize, Option<Arc<Image>>)>,
    /// Home's thumbnail cache and renderer (set by the window that shows it).
    pub thumbnails: Option<ThumbnailSource>,
    /// Projects whose cached poster has been read, and renders started
    /// (project, the modification it was started for).
    posters_read: std::collections::HashSet<String>,
    posters_started: std::collections::HashSet<(String, i64)>,
    /// Development builds: the gallery's real controls.
    #[cfg(debug_assertions)]
    gallery_native: Vec<Entity<NativeControl>>,
    /// The open button menu, and where its button was last painted.
    menu: Option<Menu>,
    /// The open menu's rows: which one the pointer or the keyboard has lit.
    pick: pm_shell::transient::Picker,
    anchors: Rc<RefCell<HashMap<Menu, Bounds<Pixels>>>>,
    /// Columns and card width at the last layout: when they change (a resize)
    /// cards are placed at once; otherwise (a sort, a search) they slide.
    grid: (usize, u32),
    /// The editor window Home is drawn in: its sheets and alerts attach here.
    editor: Option<objc2::rc::Retained<objc2_app_kit::NSWindow>>,
    /// v1's `projectsView`.
    pub layout: Layout,
    search: Option<Entity<pm_shell::text::TextField>>,
    /// How much of the header is the system's glass (the owner is choosing).
    glass: HeaderGlass,
    /// New Project, a real glass button.
    new_project: Option<Entity<NativeControl>>,
    /// `HeaderGlass::All`: the search field and the view switch on glass.
    glass_search: Option<Entity<NativeControl>>,
    glass_view: Option<Entity<NativeControl>>,
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
            thumbs: HashMap::new(),
            thumbnails: None,
            posters_read: Default::default(),
            posters_started: Default::default(),
            #[cfg(debug_assertions)]
            gallery_native: Vec::new(),
            menu: None,
            pick: Default::default(),
            anchors: Rc::default(),
            grid: (0, 0),
            editor,
            layout,
            search: None,
            glass: HeaderGlass::chosen(),
            new_project: None,
            glass_search: None,
            glass_view: None,
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
        // An open menu has the keyboard: arrows move its highlight, Return
        // chooses the lit row, a letter lights the first row it starts.
        if self.menu.is_some() {
            let key = event.keystroke.key.as_str();
            match key {
                "escape" => self.menu = None,
                "enter" | "space" => {
                    if let Some(row) = self.pick.lit() {
                        self.choose(row, cx);
                    }
                }
                "up" | "down" | "home" | "end" => {
                    self.pick.navigate(key);
                }
                _ => {
                    if let Some(typed) = &event.keystroke.key_char {
                        self.pick.type_ahead(typed);
                    }
                }
            }
            cx.notify();
            cx.stop_propagation();
            return;
        }
        match event.keystroke.key.as_str() {
            "f" if event.keystroke.modifiers.platform => {
                if let Some(field) = &self.glass_search {
                    field.read(cx).focus();
                } else if let Some(field) = &self.search {
                    field.update(cx, |f, cx| f.begin_editing_selecting_all(cx));
                }
            }
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
/// Home's header in the variants shown to the owner. New Project is the
/// system's glass button in each; `PM_NATIVE_HOME_GLASS` picks the others
/// (`glyph`: the button shows its plus alone; `all`: the search field and
/// the view switch stand on glass too). The rest of the header is drawn.
#[derive(Clone, Copy, PartialEq)]
enum HeaderGlass {
    Label,
    Glyph,
    All,
}
impl HeaderGlass {
    fn chosen() -> Self {
        match std::env::var("PM_NATIVE_HOME_GLASS").as_deref() {
            Ok("glyph") => Self::Glyph,
            Ok("all") => Self::All,
            _ => Self::Label,
        }
    }
}
const PRESET_COUNT: usize = pm_builtins::RESOLUTIONS.len();
/// Space either side of Home's sheet content: the title, the controls and
/// the cards share this one inset.
const INSET: f32 = 24.;
/// The header row, and the height of what stands in it: the search pill and
/// the icon actions, with their glyph.
const HEADER: f32 = 48.;
const ACTION: f32 = 32.;
const ACTION_SYMBOL: f32 = 18.;
/// The search pill is at most this wide, and leaves this much of the sheet
/// to the title and the actions.
const SEARCH_MAX: f32 = 360.;
const SEARCH_ROOM: f32 = 320.;
/// A cell of the grid/list switch.
const VIEW_CELL: (f32, f32) = (30., 24.);
/// Cards are this wide when four or more fit, and never narrower than the
/// least otherwise.
const CARD: f32 = 240.;
const CARD_LEAST: f32 = 160.;
impl ProjectLibrary {
    fn motion(&mut self) -> &mut pm_shell::motion::Motion<u64> {
        &mut self.motion
    }
    /// A sidebar row: symbol, label, trailing count.
    fn nav_row(
        &self,
        kit: &Kit,
        id: &'static str,
        (symbol, label): (&str, String),
        count: Option<usize>,
        selected: bool,
    ) -> Stateful<Div> {
        let t = &kit.t;
        let (c, m) = (&t.colors, &t.metrics);
        kit.row(id, selected)
            .debug_selector(|| id.into())
            .text_size(px(t.typography.reading))
            .role(Role::Button)
            .aria_label(match count {
                Some(n) => format!("{label}, {n}"),
                None => label.clone(),
            })
            .child(kit.symbol(
                symbol,
                m.symbol_panel,
                Weight::Regular,
                if selected { c.text } else { c.muted },
            ))
            .child(div().flex_1().min_w(px(0.)).truncate().child(label))
            .children(count.map(|n| {
                div()
                    .flex_none()
                    .text_role(t, Text::Caption)
                    .font_features(kit::tabular())
                    // On system glass: a step that reads over any desktop.
                    .text_color(rgb(c.muted))
                    .child(n.to_string())
            }))
    }
    /// Opens or closes a button menu; it enters on the `Enter` spring.
    fn toggle_menu(&mut self, menu: Menu, cx: &mut Context<Self>) {
        self.menu = (self.menu != Some(menu)).then_some(menu);
        // Nothing is lit as it opens.
        self.pick = pm_shell::transient::Picker {
            choices: self.menu.map(|menu| self.rows_of(menu)).unwrap_or_default(),
            open: self.menu.is_some(),
            ..Default::default()
        };
        self.motion
            .values
            .borrow_mut()
            .insert(kit::key("home-menu"), pm_design::motion::Animated::unit(0.));
        cx.notify();
    }
    /// A header action: an icon alone, no fill at rest, a wash under the
    /// pointer, a deeper one while pressed; its name and shortcut in the tip.
    fn header_action(
        kit: &Kit,
        id: &'static str,
        symbol: &str,
        label: &'static str,
        tip: &'static str,
    ) -> Stateful<Div> {
        let t = &kit.t;
        let (text, pressed) = (t.colors.text, t.translucency.pressed);
        let d = div()
            .id(id)
            .debug_selector(move || id.into())
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(ACTION))
            .rounded(px(t.radius.md))
            .role(Role::Button)
            .aria_label(label)
            .tooltip(kit::tooltip(t, tip))
            .child(kit.symbol(symbol, ACTION_SYMBOL, Weight::Regular, t.colors.secondary))
            .active_style(move |s| s.bg(ink(text, pressed)));
        kit.wash(d, kit::key(id), t.radius.md)
    }
    /// The rows of a button menu.
    fn rows_of(&self, menu: Menu) -> Vec<pm_shell::transient::Choice> {
        use pm_shell::transient::Choice;
        match menu {
            Menu::Sort => [Sort::Recent, Sort::Name]
                .into_iter()
                .map(|sort| {
                    Choice::new(sort.label(), sort.key())
                        .checked(sort == self.controller.query.sort)
                })
                .collect(),
            Menu::Account => ACCOUNT_MENU
                .iter()
                .map(|(label, command)| Choice::new(*label, *command))
                .collect(),
        }
    }
    /// Runs a row of the open menu and closes it.
    fn choose(&mut self, row: usize, cx: &mut Context<Self>) {
        match self.menu.take() {
            Some(Menu::Sort) => {
                if let Some(sort) = [Sort::Recent, Sort::Name].get(row) {
                    self.set_sort(*sort, cx);
                }
            }
            Some(Menu::Account) => {
                if let Some((_, command)) = ACCOUNT_MENU.get(row) {
                    (self.sink)(Outbound::Command(command), cx);
                }
            }
            None => {}
        }
        cx.notify();
    }
    /// The open button menu: our popover (rows light only under the pointer),
    /// hanging from its button, closed by a click elsewhere or Escape.
    fn menu_overlay(&self, kit: &Kit, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.menu?;
        let anchor = self.anchors.borrow().get(&menu).copied()?;
        let t = &kit.t;
        let shown = kit
            .animate(kit::key("home-menu"), 1., Some(Preset::Enter))
            .clamp(0., 1.);
        let close = cx.listener(|v, _: &MouseDownEvent, _, cx| {
            v.menu = None;
            cx.notify();
        });
        let rows: Vec<Stateful<Div>> = self
            .pick
            .choices
            .iter()
            .enumerate()
            .map(|(row, choice)| {
                kit.menu_row(
                    format!("home-menu-{}", choice.command),
                    choice.label.clone(),
                    choice.checked,
                    None,
                )
                // The highlight follows the pointer and leaves with it.
                .on_hover(cx.listener(move |v, hovered: &bool, _, cx| {
                    if (*hovered || v.pick.hover == Some(row))
                        && v.pick.point(hovered.then_some(row))
                    {
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(move |v, _, _, cx| v.choose(row, cx)))
            })
            .collect();
        let lit = self.pick.lit().map(|row| row as f32 * t.metrics.row);
        // It grows out of its button: a few points of travel with the fade.
        let travel = (1. - shown) * t.space.sm;
        let (corner, at) = match menu {
            Menu::Sort => (
                Anchor::TopRight,
                point(anchor.right(), anchor.bottom() + px(t.space.xs - travel)),
            ),
            Menu::Account => (
                Anchor::BottomLeft,
                point(anchor.left(), anchor.top() - px(t.space.xs - travel)),
            ),
        };
        Some(
            deferred(
                anchored().anchor(corner).position(at).child(
                    kit.menu()
                        .id("home-menu")
                        .min_w(px(t.metrics.sidebar - 2. * t.space.md))
                        .opacity(shown)
                        .font_family(pm_design::TEXT_FONT)
                        .role(Role::Menu)
                        .on_mouse_down_out(close)
                        .child(kit.menu_light("home-menu", lit))
                        .children(rows),
                ),
            )
            .priority(1)
            .into_any_element(),
        )
    }
    /// A poster was written for `id`: its card reads it from the cache again.
    pub fn poster_written(&mut self, id: &str, cx: &mut Context<Self>) {
        self.posters_read.remove(id);
        self.posters_started
            .retain(|(started, stamp)| started != id || *stamp != i64::MIN);
        cx.notify();
    }
    /// Home shows again: posters written since (a project saved or closed in
    /// any window) are read from the cache once more.
    pub fn show_posters(&mut self, source: ThumbnailSource) {
        self.thumbnails = Some(source);
        self.posters_read.clear();
        self.posters_started.retain(|(_, stamp)| *stamp != i64::MIN);
    }
    /// A project's thumbnail: the one v1 stored with it (a data URL, decoded
    /// once per source), else Home's own poster frame (`crate::poster`). A
    /// poster is read from the cache off the main thread on first sight, and
    /// rendered (also off the main thread) when this window has the project
    /// open and the cache has none or an older one. Until it arrives the
    /// card shows its empty canvas, then the image fades in.
    fn thumbnail(&mut self, row: &Row, cx: &mut Context<Self>) -> Option<Arc<Image>> {
        if let Some(source) = row.summary.as_ref().and_then(|s| s.thumbnail.as_deref()) {
            if let Some((length, image)) = self.thumbs.get(&row.id)
                && *length == source.len()
            {
                return image.clone();
            }
            let image = source
                .strip_prefix("data:image/")
                .and_then(|rest| rest.split_once(";base64,"))
                .and_then(|(kind, data)| {
                    let format = match kind {
                        "jpeg" | "jpg" => ImageFormat::Jpeg,
                        "png" => ImageFormat::Png,
                        "webp" => ImageFormat::Webp,
                        _ => return None,
                    };
                    use base64::Engine as _;
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(data)
                        .ok()?;
                    Some(Arc::new(Image::from_bytes(format, bytes)))
                });
            self.thumbs
                .insert(row.id.clone(), (source.len(), image.clone()));
            return image;
        }
        let cached = self.thumbs.get(&row.id).cloned();
        let image = cached.as_ref().and_then(|(_, image)| image.clone());
        let Some(source) = &self.thumbnails else {
            return image;
        };
        let (id, stamp) = (row.id.clone(), row.modified_ms.unwrap_or(0));
        let (file, stamp_file) = crate::poster::files(&source.dir, &id);
        let fade = kit::key(&format!("poster-{id}"));
        let store = move |v: &mut Self, id: String, stamp: i64, png: Vec<u8>| {
            v.thumbs.insert(
                id,
                (
                    stamp as usize,
                    Some(Arc::new(Image::from_bytes(ImageFormat::Png, png))),
                ),
            );
            v.motion
                .values
                .borrow_mut()
                .insert(fade, pm_design::motion::Animated::unit(0.));
        };
        if !self.posters_read.contains(&id) {
            if self.posters_started.insert((id.clone(), i64::MIN)) {
                cx.spawn(async move |this, cx| {
                    let read = cx
                        .background_executor()
                        .spawn(async move {
                            let (stamp, _) = crate::poster::stamp(&stamp_file)?;
                            Some((stamp, std::fs::read(file).ok()?))
                        })
                        .await;
                    let _ = this.update(cx, |v, cx| {
                        v.posters_read.insert(id.clone());
                        // The one already shown stays as it is.
                        let shown = v.thumbs.get(&id).map(|(at, _)| *at as i64);
                        if let Some((stamp, png)) = read
                            && shown != Some(stamp)
                        {
                            store(v, id, stamp, png);
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
            return image;
        }
        // Taken at or after the project last changed on disk.
        let fresh = cached.is_some_and(|(at, image)| image.is_some() && at as i64 >= stamp);
        if !fresh
            && self.active.as_deref() == Some(id.as_str())
            && let Some(renderer) = source.renderer.clone()
            && self.posters_started.insert((id.clone(), stamp))
        {
            let size = row.dimensions().unwrap_or((1920, 1080));
            let duration = row.summary.as_ref().and_then(|s| s.duration_seconds);
            let dir = source.dir.clone();
            let taken = crate::poster::now_ms().max(stamp);
            cx.spawn(async move |this, cx| {
                let poster = id.clone();
                let png = cx
                    .background_executor()
                    .spawn(async move {
                        crate::poster::write(&renderer, None, &dir, &poster, taken, size, duration)
                    })
                    .await;
                let _ = this.update(cx, |v, cx| {
                    if let Some(png) = png {
                        store(v, id, taken, png);
                    }
                    cx.notify();
                });
            })
            .detach();
        }
        image
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
    /// The frame a project shows in: its thumbnail, or, when the library has
    /// none, an empty canvas in the composition's own shape (wide, square or
    /// tall reads at a glance). A hairline of pure ink keeps a bright or dark
    /// frame apart from the sheet.
    fn frame(
        &self,
        kit: &Kit,
        thumbnail: Option<Arc<Image>>,
        dimensions: Option<(u32, u32)>,
        radius: f32,
        fade: f32,
        (width, height): (f32, f32),
    ) -> Div {
        let t = &kit.t;
        let dark = t.scheme == pm_design::Scheme::Dark;
        let outline = |radius: f32| {
            div()
                .absolute()
                .inset_0()
                .rounded(px(radius))
                .shadow(vec![BoxShadow {
                    color: ink(if dark { 0xffffff } else { 0x000000 }, 0.1).into(),
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: px(0.5),
                    inset: true,
                }])
        };
        let frame = div()
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(px(radius))
            .overflow_hidden()
            .w(px(width))
            .h(px(height));
        match thumbnail {
            // The whole picture in the composition's own shape, on the
            // viewer's ground where its shape leaves room.
            Some(image) => frame
                .bg(rgb(t.colors.viewer))
                .child({
                    // A clip is a rectangle, so a picture that reaches the
                    // frame's corners carries the frame's radius itself; one
                    // that stands clear of them keeps its square corners.
                    let (w, h) = dimensions.unwrap_or((16, 9));
                    let aspect = w.max(1) as f32 / h.max(1) as f32;
                    let clear_x = (width - height * aspect).max(0.) / 2.;
                    let clear_y = (height - width / aspect).max(0.) / 2.;
                    let picture = img(ImageSource::Image(image))
                        .size_full()
                        .object_fit(ObjectFit::Contain)
                        .opacity(fade.clamp(0., 1.));
                    if clear_x < radius && clear_y < radius {
                        picture.rounded(px(radius))
                    } else {
                        picture
                    }
                })
                .child(outline(radius)),
            None => {
                let (w, h) = dimensions.unwrap_or((16, 9));
                let aspect = (w.max(1) as f32 / h.max(1) as f32).clamp(0.25, 4.);
                // The canvas takes 56 % of the frame's height, or of its
                // width when it is wider than the frame itself.
                let canvas = div()
                    .relative()
                    .rounded(px(t.radius.xs))
                    .bg(rgb(t.colors.viewer))
                    .child(outline(t.radius.xs));
                frame
                    .bg(ink(t.colors.text, t.translucency.hover))
                    .child(if aspect > 16. / 9. {
                        canvas.w(relative(0.56)).map(|mut d| {
                            d.style().aspect_ratio = Some(aspect);
                            d
                        })
                    } else {
                        canvas.h(relative(0.56)).map(|mut d| {
                            d.style().aspect_ratio = Some(aspect);
                            d
                        })
                    })
            }
        }
    }
    /// Under the poster: the name, with a small mark when the project is
    /// open (the accent when it is this window's), over one line of facts.
    fn caption(&self, kit: &Kit, row: &Row, facts: &str) -> Div {
        let t = &kit.t;
        let c = &t.colors;
        let active = self.active.as_deref() == Some(row.id.as_str());
        div()
            .flex()
            .flex_col()
            .min_w(px(0.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(t.space.sm))
                    .child(
                        div()
                            .min_w(px(0.))
                            .truncate()
                            .text_role(t, Text::Title)
                            .text_color(rgb(c.text))
                            .child(row.name.clone()),
                    )
                    .children(self.tag(row).map(|_| {
                        div()
                            .flex_none()
                            .size(px(t.space.sm))
                            .rounded_full()
                            .bg(if active {
                                rgb(c.accent)
                            } else {
                                ink(c.text, t.translucency.faint)
                            })
                    })),
            )
            .child(
                div()
                    .truncate()
                    .text_role(t, Text::Caption)
                    .font_features(kit::tabular())
                    .text_color(rgb(c.muted))
                    .child(facts.to_string()),
            )
    }
    /// A project is a framed page with its poster on it, and the name and
    /// facts under it. Every card has one shape; a project with no poster
    /// yet is an empty page that says its size.
    fn card(
        &mut self,
        kit: &Kit,
        row: &Row,
        (width, height): (f32, f32),
        now: i64,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = &kit.t;
        let c = &t.colors;
        let selected = self.controller.selected.as_ref() == Some(&row.id);
        let facts = row.facts(now);
        let thumbnail = self.thumbnail(row, cx);
        let fade = kit.animate(
            kit::key(&format!("poster-{}", row.id)),
            1.,
            Some(Preset::Show),
        );
        let hover = kit::key(&format!("card-{}", row.id));
        let over = kit.hovered(hover);
        // A frame of ink around the page, corners concentric: the page's
        // corner plus the frame. Under the pointer the frame deepens a step;
        // selected, it carries a ring of ink inside its edge.
        let frame = t.space.xs;
        let inner = t.radius.xl;
        let place = (width, (width * 9. / 16.).round());
        let (w, h) = row.dimensions().unwrap_or((16, 9));
        let aspect = w.max(1) as f32 / h.max(1) as f32;
        // A picture in the frame's own shape fills the page; any other
        // stands whole on the viewer's ground.
        let fills = (aspect / (place.0 / place.1) - 1.).abs() < 0.03;
        let tr = t.translucency;
        let page = div()
            .flex()
            .items_center()
            .justify_center()
            .size_full()
            .rounded(px(inner))
            .map(|page| match thumbnail {
                Some(image) => page.bg(rgb(c.viewer)).child(
                    img(ImageSource::Image(image))
                        .size_full()
                        .object_fit(if fills {
                            ObjectFit::Cover
                        } else {
                            ObjectFit::Contain
                        })
                        .when(fills, |picture| picture.rounded(px(inner)))
                        .opacity(fade.clamp(0., 1.)),
                ),
                None => page.bg(rgb(c.panel)).child(
                    div()
                        .text_role(t, Text::Caption)
                        .font_features(kit::tabular())
                        .text_color(ink(c.text, tr.faint))
                        .children(row.dimensions().map(|(w, h)| format!("{w} × {h}"))),
                ),
            });
        let poster = div()
            .flex()
            .flex_none()
            .w(px(place.0))
            .h(px(place.1))
            .p(px(frame))
            .corner(t, inner + frame)
            .bg(rgb(pm_design::color::mix(
                c.panel,
                c.text,
                if selected {
                    tr.pressed
                } else {
                    tr.hover + (tr.selected - tr.hover) * over
                },
            )))
            .when(selected, |poster| {
                poster.shadow(vec![BoxShadow {
                    color: ink(c.text, 0.72).into(),
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: px(frame / 2.),
                    inset: true,
                }])
            })
            .child(page);
        let card = div()
            .id(SharedString::from(format!("project-{}", row.id)))
            .debug_selector(|| format!("project-{}", row.id))
            .flex()
            .flex_col()
            .w(px(width))
            .h(px(height))
            .gap(px(t.space.md))
            .role(Role::ListItem)
            .aria_selected(selected)
            .aria_label(match self.tag(row) {
                Some(tag) => format!("{}, {tag}, {facts}", row.name),
                None => format!("{}, {facts}", row.name),
            })
            .child(poster)
            .child(self.caption(kit, row, &facts).px(px(frame)));
        let card = kit.track_hover(card, hover);
        self.interactive(card, &row.id, cx)
    }
    fn list_row(
        &mut self,
        kit: &Kit,
        row: &Row,
        now: i64,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = &kit.t;
        let selected = self.controller.selected.as_ref() == Some(&row.id);
        let facts = row.facts(now);
        let thumbnail = self.thumbnail(row, cx);
        let height = t.metrics.row + t.space.lg;
        let d = kit
            .row(format!("project-{}", row.id), selected)
            .debug_selector(|| format!("project-{}", row.id))
            .h(px(height))
            .gap(px(t.space.lg))
            .role(Role::ListItem)
            .aria_label(match self.tag(row) {
                Some(tag) => format!("{}, {tag}, {facts}", row.name),
                None => format!("{}, {facts}", row.name),
            })
            .child(self.frame(
                kit,
                thumbnail,
                row.dimensions(),
                t.radius.xs,
                1.,
                (
                    ((height - t.space.md) * 16. / 9.).round(),
                    height - t.space.md,
                ),
            ))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w(px(0.))
                    .items_baseline()
                    .gap(px(t.space.sm))
                    .child(
                        div()
                            .min_w(px(0.))
                            .truncate()
                            .text_role(t, Text::Title)
                            .text_color(rgb(t.colors.text))
                            .child(row.name.clone()),
                    )
                    .children(self.tag(row).map(|tag| {
                        div()
                            .flex_none()
                            .text_role(t, Text::Caption)
                            .text_color(rgb(t.colors.muted))
                            .child(tag)
                    })),
            )
            .child(
                div()
                    .flex_none()
                    .text_role(t, Text::Caption)
                    .font_features(kit::tabular())
                    .text_color(rgb(t.colors.muted))
                    .child(facts),
            );
        self.interactive(d, &row.id, cx)
    }
}
impl Render for ProjectLibrary {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.motion.frame(window);
        let t = self.tokens.clone();
        let (c, m) = (&t.colors, &t.metrics);
        let kit = Kit::new(&t, &self.motion, Self::motion, window, cx);
        if self.search.is_none() && pm_shell::text::available(window) {
            let tokens = t.clone();
            // A bare field in the header's own pill: no bezel, no ring.
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
                field.set_bare(true);
                field
            });
            cx.subscribe(&search, |v: &mut Self, _, event, cx| {
                use pm_shell::text::TextEvent;
                match event {
                    TextEvent::Changed(text) | TextEvent::Committed(text) => {
                        v.set_text(text.clone(), cx)
                    }
                    // Escape in the field clears the search, as on Home.
                    TextEvent::Cancelled => v.clear_search(cx),
                    _ => {}
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
        // Sidebar (v1): Recents and Trash with counts, Store, then the
        // account at the foot. Navigation only, on the window's own material.
        let account = crate::cloud::fact(cx);
        let identity = account
            .get("identity")
            .and_then(serde_json::Value::as_str)
            .filter(|_| account.get("signedIn") == Some(&serde_json::Value::Bool(true)))
            .map(str::to_owned);
        let anchor = |menu: Menu| {
            let anchors = self.anchors.clone();
            canvas(
                move |bounds, _, _| {
                    anchors.borrow_mut().insert(menu, bounds);
                },
                |_, _, _, _| (),
            )
            .absolute()
            .size_full()
        };
        let sidebar = div()
            .id("home-sidebar")
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(t.space.hair))
            .w(px(m.sidebar))
            .h_full()
            .px(px(m.dock_gap))
            .pt(px(t.space.sm))
            .pb(px(m.dock_gap))
            .role(Role::Group)
            .aria_label("Home")
            .child(
                self.nav_row(
                    &kit,
                    "home-recents",
                    ("clock", "Recents".into()),
                    Some(live.min(12)),
                    section != Section::Deleted,
                )
                .on_click(cx.listener(|v, _, _, cx| v.set_section(Section::Recents, cx))),
            )
            .child(
                self.nav_row(
                    &kit,
                    "home-trash",
                    ("trash", "Trash".into()),
                    Some(trash),
                    section == Section::Deleted,
                )
                .on_click(cx.listener(|v, _, _, cx| v.set_section(Section::Deleted, cx))),
            )
            .child(div().h(px(t.space.lg)))
            .child(
                self.nav_row(&kit, "home-store", ("basket", "Store".into()), None, false)
                    .on_click(cx.listener(|v, _, _, cx| (v.sink)(Outbound::Command("store"), cx))),
            )
            .child(div().flex_1())
            .child(match identity {
                Some(name) => self
                    .nav_row(
                        &kit,
                        "home-account",
                        ("person.crop.circle", name),
                        None,
                        false,
                    )
                    .child(anchor(Menu::Account))
                    .on_click(cx.listener(|v, _, _, cx| v.toggle_menu(Menu::Account, cx))),
                None => self
                    .nav_row(
                        &kit,
                        "home-account",
                        ("person.crop.circle", "Sign in".into()),
                        None,
                        false,
                    )
                    .on_click(
                        cx.listener(|v, _, _, cx| (v.sink)(Outbound::Command("cloudSignIn"), cx)),
                    ),
            });
        // The sheet's header, drawn and quiet on one inset. First row: the
        // title, the search pill in the middle of the sheet, two icon
        // actions. Second row: the count, the sort as a text action and the
        // small view switch, the header's one raised element.
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
        let tr = t.translucency;
        if self.new_project.is_none() && pm_shell::text::available(window) {
            let glass = self.glass;
            let mut make = |kind, label: &str, value: Value, options: Vec<Value>, symbol| {
                let tokens = t.clone();
                cx.new(|cx| {
                    NativeControl::new(
                        ControlSpec {
                            kind,
                            label,
                            identifier: "home.header",
                            value: &value,
                            options,
                            disabled: false,
                            default: false,
                            destructive: false,
                            inline: false,
                            quiet: false,
                            tint: None,
                            symbol,
                            glass: true,
                            symbol_only: glass == HeaderGlass::Glyph,
                        },
                        &tokens,
                        window,
                        cx,
                    )
                })
            };
            let new_project = make(Kind::Push, "New Project", Value::Null, vec![], Some("plus"));
            if glass == HeaderGlass::All {
                let search = make(
                    Kind::Search,
                    "Search projects",
                    json!(self.controller.query.text),
                    vec![],
                    Some("⌘F"),
                );
                let view = make(
                    Kind::Segmented,
                    "Project layout",
                    json!(usize::from(self.layout == Layout::List)),
                    vec![
                        json!({"symbol": "square.grid.2x2", "label": "Grid view"}),
                        json!({"symbol": "list.bullet", "label": "List view"}),
                    ],
                    None,
                );
                cx.subscribe(&search, |v: &mut Self, _, event: &Value, cx| {
                    match (event.as_str(), event.get("key").and_then(Value::as_str)) {
                        (Some(text), _) => v.set_text(text.to_owned(), cx),
                        (_, Some("Escape")) => v.clear_search(cx),
                        _ => {}
                    }
                })
                .detach();
                cx.subscribe(&view, |v: &mut Self, _, event: &Value, cx| {
                    v.set_layout(
                        if event.as_u64() == Some(1) {
                            Layout::List
                        } else {
                            Layout::Grid
                        },
                        cx,
                    );
                })
                .detach();
                (self.glass_search, self.glass_view) = (Some(search), Some(view));
            }
            cx.subscribe(&new_project, |v: &mut Self, _, _: &Value, cx| {
                (v.sink)(Outbound::Command("newProject"), cx)
            })
            .detach();
            self.new_project = Some(new_project);
        }
        let sheet = (f32::from(window.viewport_size().width) - m.sidebar - m.dock_gap).max(0.);
        let search_width = SEARCH_MAX.min(sheet - SEARCH_ROOM).max(CARD_LEAST);
        let search_key = kit::key("home-search");
        let search = kit.track_hover(
            div()
                .id("home-search")
                .flex()
                .flex_none()
                .items_center()
                .gap(px(t.space.md))
                .w(px(search_width))
                .h(px(ACTION))
                .px(px(t.space.md + t.space.hair))
                .rounded(px(t.radius.md))
                .bg(ink(
                    c.text,
                    tr.hover + (tr.selected - tr.hover) * kit.hovered(search_key),
                ))
                .child(kit.symbol(
                    "magnifyingglass",
                    m.symbol_toolbar + 1.,
                    Weight::Regular,
                    c.muted,
                ))
                .child(div().flex_1().min_w(px(0.)).children(self.search.clone()))
                .when(self.controller.query.text.is_empty(), |d| {
                    d.child(
                        div()
                            .flex_none()
                            .text_role(&t, Text::Caption)
                            .text_color(rgb(c.faint))
                            .child("⌘F"),
                    )
                })
                .on_click(cx.listener(|v, _, _, cx| {
                    if let Some(field) = &v.search {
                        field.update(cx, |f, cx| f.begin_editing(cx));
                    }
                })),
            search_key,
        );
        let side = || div().flex().flex_1().min_w(px(0.)).items_center();
        let title_row =
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(t.space.xl))
                .h(px(HEADER))
                .child(
                    side().child(
                        div()
                            .id("home-heading")
                            .flex_none()
                            .role(Role::Heading)
                            .aria_label(format!("{}, {count}", section.title()))
                            .text_role(&t, Text::Headline)
                            .child(section.title()),
                    ),
                )
                .child(match &self.glass_search {
                    Some(field) => div()
                        .flex_none()
                        .w(px(search_width))
                        .child(field.clone())
                        .into_any_element(),
                    None => search.into_any_element(),
                })
                .child(
                    side()
                        .justify_end()
                        .gap(px(t.space.sm))
                        .child(
                            Self::header_action(&kit, "home-open", "folder", "Open…", "Open… ⌘O")
                                .on_click(cx.listener(|v, _, _, cx| {
                                    (v.sink)(Outbound::Command("open"), cx)
                                })),
                        )
                        // New Project is the header's one raised action:
                        // the system's glass button. A window that cannot
                        // host it (a test) keeps the quiet icon.
                        .child(match &self.new_project {
                            Some(button) => button.clone().into_any_element(),
                            None => Self::header_action(
                                &kit,
                                "home-new",
                                "plus",
                                "New Project",
                                "New Project ⌥⌘N",
                            )
                            .on_click(cx.listener(|v, _, _, cx| {
                                (v.sink)(Outbound::Command("newProject"), cx)
                            }))
                            .into_any_element(),
                        }),
                );
        let list = self.layout == Layout::List;
        let cell = |id: &'static str, symbol: &str, label: &'static str, on: bool| {
            div()
                .id(id)
                .relative()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .w(px(VIEW_CELL.0))
                .h(px(VIEW_CELL.1))
                .role(Role::RadioButton)
                .aria_label(label)
                .aria_selected(on)
                .tooltip(kit::tooltip(&t, label))
                .child(kit.symbol(
                    symbol,
                    m.symbol_toolbar,
                    Weight::Regular,
                    if on { c.text } else { c.secondary },
                ))
        };
        // The on fill glides between the two cells.
        let at = kit
            .animate(kit::key("home-view"), f32::from(list), Some(Preset::Layout))
            .clamp(-0.1, 1.1);
        let view_switch = div()
            .id("home-layout")
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(t.space.hair))
            .p(px(m.border))
            .rounded(px(t.radius.md))
            .bg(rgb(c.raised))
            .elevated(&t, Elevation::Raised)
            .role(Role::RadioGroup)
            .aria_label("Project layout")
            .child(
                div()
                    .absolute()
                    .top(px(m.border))
                    .left(px(m.border + at * (VIEW_CELL.0 + t.space.hair)))
                    .w(px(VIEW_CELL.0))
                    .h(px(VIEW_CELL.1))
                    .rounded(px(t.radius.nested(t.radius.md, m.border + t.space.hair)))
                    .bg(ink(c.text, tr.selected)),
            )
            .child(
                cell("home-grid", "square.grid.2x2", "Grid view", !list)
                    .on_click(cx.listener(|v, _, _, cx| v.set_layout(Layout::Grid, cx))),
            )
            .child(
                cell("home-list", "list.bullet", "List view", list)
                    .on_click(cx.listener(|v, _, _, cx| v.set_layout(Layout::List, cx))),
            );
        let controls_row = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(t.space.md))
            .h(px(m.control))
            .child(
                div()
                    .flex_none()
                    .text_role(&t, Text::Body)
                    .font_features(kit::tabular())
                    .text_color(rgb(c.muted))
                    .child(count),
            )
            // A real project can take seconds to read: say so here until it
            // shows in the window.
            .children(opening.map(|name| {
                div()
                    .id("home-status")
                    .min_w(px(0.))
                    .truncate()
                    .text_role(&t, Text::Body)
                    .text_color(rgb(c.secondary))
                    .role(Role::Status)
                    .aria_label(name.clone())
                    .child(name)
            }))
            .child(div().flex_1())
            // The sort is a text action: it opens our menu.
            .child(
                kit.wash(
                    div()
                        .id("home-sort")
                        .debug_selector(|| "home-sort".into())
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(t.space.sm))
                        .h(px(m.control))
                        .px(px(t.space.md + t.space.hair))
                        .rounded(px(t.radius.md))
                        .role(Role::Button)
                        .aria_label(format!("Sort projects: {}", sort.label()))
                        .text_role(&t, Text::Body)
                        .text_color(rgb(c.muted))
                        .child(kit.symbol(
                            "arrow.up.arrow.down",
                            m.symbol_toolbar,
                            Weight::Regular,
                            c.muted,
                        ))
                        .child(sort.label())
                        .child(anchor(Menu::Sort))
                        .on_click(cx.listener(|v, _, _, cx| v.toggle_menu(Menu::Sort, cx))),
                    kit::key("home-sort"),
                    t.radius.md,
                ),
            )
            .child(div().flex_none().w(px(t.space.md)))
            .child(match &self.glass_view {
                Some(switch) => switch.clone().into_any_element(),
                None => view_switch.into_any_element(),
            });
        let header = div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(t.space.lg))
            .px(px(INSET))
            .pb(px(t.space.md))
            .child(title_row)
            .child(controls_row);
        // Body: cards or rows; an empty section says so with v1's copy.
        let width = (sheet - 2. * INSET).max(0.);
        // As many cards of the usual width as fit when four or more do;
        // on a narrow sheet, one to three wider ones.
        let column_gap = t.space.xl;
        let fit = |least: f32| ((width + column_gap) / (least + column_gap)).floor() as usize;
        self.columns = if fit(CARD) >= 4 {
            fit(CARD)
        } else {
            fit(CARD_LEAST).clamp(1, 3)
        };
        let card_width =
            ((width - column_gap * (self.columns - 1) as f32) / self.columns as f32).floor();
        let text = t.text(Text::Title).size * t.text(Text::Title).leading
            + t.text(Text::Caption).size * t.text(Text::Caption).leading;
        let card_height =
            ((card_width * 9. / 16.).round() + t.space.md + text.ceil() + t.space.xs).round();
        let centred = || {
            div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(t.space.xl))
                .pb(px(m.toolbar * 2.))
        };
        let body: AnyElement = match self.controller.content() {
            // Loading: the grid's own shape, breathing, so nothing jumps when
            // the projects arrive.
            Content::Loading => div()
                .flex_1()
                .px(px(INSET))
                .pt(px(t.space.sm))
                .flex()
                .gap(px(column_gap))
                .children((0..self.columns.min(4)).map(|_| {
                    div()
                        .w(px(card_width))
                        .h(px((card_width * 9. / 16.).round()))
                        .rounded(px(t.radius.xl))
                        .bg(ink(c.text, t.translucency.hover))
                }))
                .with_animation(
                    "home-loading",
                    Animation::new(std::time::Duration::from_millis(1400))
                        .repeat()
                        .with_easing(pulsating_between(0.45, 1.)),
                    |this, delta| this.opacity(delta),
                )
                .into_any_element(),
            Content::Unavailable { message, action } => centred()
                .child(kit.empty_state("exclamationmark.triangle", message, ""))
                .child(
                    kit.button("home-retry", action, Button::Neutral, None, None)
                        .on_click(cx.listener(|v, _, _, cx| {
                            let _ = v.controller.refresh();
                            v.pump(cx);
                        })),
                )
                .into_any_element(),
            Content::Empty(state) => centred()
                .id("home-empty")
                .role(Role::Group)
                .aria_label(state.title)
                .child(kit.empty_state(state.symbol, state.title, &state.detail))
                .when_some(state.action, |d, action| {
                    d.child(
                        kit.button("home-create", action, Button::Primary, Some("plus"), None)
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
                    .px(px(INSET))
                    // Room for a card to lift under the pointer.
                    .pt(px(t.space.sm))
                    .pb(px(t.space.xxl))
                    .role(Role::List)
                    .aria_label(section.title());
                match self.layout {
                    Layout::Grid => {
                        // Cards are placed by hand so a sort or a search slides
                        // them to their new places; a resize places them at once.
                        let layout = (self.columns, card_width as u32);
                        let slide = (self.grid == layout).then_some(Preset::Layout);
                        self.grid = layout;
                        let row_gap = t.space.xl;
                        let lines = rows.len().div_ceil(self.columns);
                        let mut grid = div()
                            .relative()
                            .w_full()
                            .h(px(lines as f32 * (card_height + row_gap)));
                        for (i, row) in rows.iter().enumerate() {
                            let (column, line) = (i % self.columns, i / self.columns);
                            let x = kit.animate(
                                kit::key(&format!("card-x-{}", row.id)),
                                column as f32 * (card_width + column_gap),
                                slide,
                            );
                            let y = kit.animate(
                                kit::key(&format!("card-y-{}", row.id)),
                                line as f32 * (card_height + row_gap),
                                slide,
                            );
                            grid =
                                grid.child(div().absolute().left(px(x)).top(px(y)).child(
                                    self.card(&kit, row, (card_width, card_height), now, cx),
                                ));
                        }
                        list.child(grid).into_any_element()
                    }
                    Layout::List => {
                        self.grid = (0, 0);
                        list.child(
                            div().flex().flex_col().gap(px(t.space.hair)).children(
                                rows.iter()
                                    .map(|row| self.list_row(&kit, row, now, cx))
                                    .collect::<Vec<_>>(),
                            ),
                        )
                        .into_any_element()
                    }
                }
            }
        };
        #[cfg(debug_assertions)]
        let body = if kit::GALLERY.load(std::sync::atomic::Ordering::Relaxed) {
            if self.gallery_native.is_empty() && pm_shell::text::available(window) {
                type Sample = (Kind, &'static str, Value, Vec<Value>, Option<u32>);
                let specs: [Sample; 9] = [
                    (Kind::Switch, "Switch off", json!(false), vec![], None),
                    (Kind::Switch, "Switch on", json!(true), vec![], None),
                    (Kind::Checkbox, "Checkbox", json!(true), vec![], None),
                    (Kind::Radio, "Radio", json!(true), vec![], None),
                    (Kind::Slider, "Slider", json!(0.35), vec![], None),
                    (
                        Kind::Segmented,
                        "Timeline mode",
                        json!(0),
                        vec![json!("Layers"), json!("Tracks")],
                        None,
                    ),
                    (
                        Kind::Popup,
                        "Appearance",
                        json!("Dark"),
                        vec![json!("System"), json!("Light"), json!("Dark")],
                        None,
                    ),
                    (Kind::Push, "Open…", Value::Null, vec![], None),
                    (
                        Kind::Push,
                        "New Project",
                        Value::Null,
                        vec![],
                        Some(t.colors.accent),
                    ),
                ];
                for (kind, label, value, options, tint) in specs {
                    let tokens = t.clone();
                    self.gallery_native.push(cx.new(|cx| {
                        NativeControl::new(
                            ControlSpec {
                                kind,
                                label,
                                identifier: "home.gallery",
                                value: &value,
                                options,
                                disabled: false,
                                default: false,
                                destructive: false,
                                inline: false,
                                quiet: false,
                                tint,
                                symbol: None,
                                glass: false,
                                symbol_only: false,
                            },
                            &tokens,
                            window,
                            cx,
                        )
                    }));
                }
            }
            self.gallery(&kit, cx)
        } else {
            body
        };
        // One inset sheet on the window ground, as Settings.
        let content = div()
            .flex_1()
            .min_w(px(0.))
            .mr(px(m.dock_gap))
            .mb(px(m.dock_gap))
            .flex()
            .flex_col()
            .corner(&t, m.panel_radius)
            .bg(rgb(c.panel))
            .elevated(&t, Elevation::Panel)
            .overflow_hidden()
            .child(header)
            .child(body);
        let menu = self.menu_overlay(&kit, cx);
        div()
            .id("home")
            .debug_selector(|| "home".into())
            .size_full()
            .flex()
            .text_color(rgb(c.text))
            .text_role(&t, Text::Body)
            .font_family(pm_design::TEXT_FONT)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .child(sidebar)
            .child(content)
            .children(menu)
            .child(kit.settle())
    }
}
/// Development builds: every kit control and strip state in place of the
/// projects (`--open reviewKit`), so a design review sees the whole system on
/// screen in either appearance without a pointer.
#[cfg(debug_assertions)]
impl ProjectLibrary {
    fn gallery(&self, kit: &Kit, cx: &App) -> AnyElement {
        use pm_design::StripState;
        let t = &kit.t;
        let c = &t.colors;
        let section = |title: &str| {
            div()
                .flex()
                .flex_col()
                .flex_none()
                .gap(px(t.space.md))
                .w(px(420.))
                .child(
                    div()
                        .text_role(t, Text::Caption)
                        .text_color(rgb(c.muted))
                        .child(title.to_string()),
                )
        };
        let line = || div().flex().flex_wrap().items_center().gap(px(t.space.md));
        let none = |_: usize, _: &mut Window, _: &mut App| {};
        let buttons = |disabled: bool| {
            let row = line()
                .child(kit.button(
                    format!("g-primary-{disabled}"),
                    "New Project",
                    Button::Primary,
                    Some("plus"),
                    None,
                ))
                .child(kit.button(
                    format!("g-neutral-{disabled}"),
                    "Open…",
                    Button::Neutral,
                    None,
                    None,
                ))
                .child(kit.button(
                    format!("g-popup-{disabled}"),
                    "Dark",
                    Button::Neutral,
                    None,
                    Some("chevron.down"),
                ))
                .child(kit.button(
                    format!("g-quiet-{disabled}"),
                    "Recently edited",
                    Button::Quiet,
                    None,
                    Some("chevron.down"),
                ))
                .child(kit.button(
                    format!("g-danger-{disabled}"),
                    "Delete",
                    Button::Destructive,
                    None,
                    None,
                ));
            if disabled { kit.disabled(row) } else { row }
        };
        let strip = |fill: u32, state: StripState, name: &str| {
            let s = t.strip(fill, state);
            let height = t.metrics.track_row - 2. * t.strip_gap();
            div()
                .relative()
                .flex()
                .flex_none()
                .items_center()
                .w(px(96.))
                .h(px(height))
                .px(px(t.space.md))
                .rounded(px(t.strip_radius(height)))
                .bg(linear_gradient(
                    180.,
                    linear_color_stop(ink(s.top, s.opacity), 0.),
                    linear_color_stop(ink(s.bottom, s.opacity), 1.),
                ))
                .shadow(vec![
                    BoxShadow {
                        color: ink(s.rim_top.0, (s.rim_top.1 - s.rim.1).max(0.)).into(),
                        offset: point(px(0.), px(1.)),
                        blur_radius: px(0.),
                        spread_radius: px(0.),
                        inset: true,
                    },
                    BoxShadow {
                        color: ink(s.rim.0, s.rim.1).into(),
                        offset: point(px(0.), px(0.)),
                        blur_radius: px(0.),
                        spread_radius: px(1.),
                        inset: true,
                    },
                ])
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded(px(t.strip_radius(height)))
                        .bg(linear_gradient(
                            180.,
                            linear_color_stop(ink(s.glow.0, 0.), 0.42),
                            linear_color_stop(ink(s.glow.0, s.glow.1), 1.),
                        )),
                )
                .text_role(t, Text::Caption)
                .text_color(rgb(s.foreground))
                .child(name.to_string())
                .when(state == StripState::Hover, |d| {
                    let grip = t.trim_grip();
                    d.child(
                        div()
                            .absolute()
                            .right(px(grip.inset))
                            .top(relative((1. - grip.height) / 2.))
                            .w(px(grip.width))
                            .h(relative(grip.height))
                            .rounded_full()
                            .bg(rgb(grip.color)),
                    )
                })
        };
        let states = [
            (StripState::Rest, "Rest"),
            (StripState::Hover, "Hover"),
            (StripState::Selected, "Selected"),
            (StripState::Dragging, "Dragging"),
            (StripState::Disabled, "Disabled"),
        ];
        let roles = [
            (Text::Display, "Display 24"),
            (Text::Heading, "Heading 15"),
            (Text::Title, "Title 13"),
            (Text::Reading, "Reading 13"),
            (Text::Label, "Label 12"),
            (Text::Body, "Body 12"),
            (Text::Caption, "Caption 11"),
            (Text::Timecode, "00:01:23:12"),
        ];
        div()
            .id("kit-gallery")
            .flex_1()
            .overflow_y_scroll()
            .px(px(INSET))
            .pb(px(t.space.xxl))
            .flex()
            .flex_wrap()
            .items_start()
            .gap(px(t.space.section))
            .child(
                section("Real AppKit controls (hosted; the system draws them)")
                    .w(px(560.))
                    .child(line().gap(px(t.space.lg)).children(self.gallery_native.iter().cloned()))
                    .children(self.gallery_native.first().map(|switch| {
                        let (w, h) = (switch.read(cx).width(), switch.read(cx).height());
                        div()
                            .text_role(t, Text::Caption)
                            .text_color(rgb(c.faint))
                            .child(format!("NSSwitch at regular size is {w}×{h} pt"))
                    })),
            )
            .child(
                section("Buttons: rest, then disabled")
                    .child(buttons(false))
                    .child(buttons(true)),
            )
            .child(
                section("Icon buttons, segmented control, tabs")
                    .child(
                        line()
                            .child(kit.icon_button("g-icon-a", "magnifyingglass", "Search", false))
                            .child(kit.icon_button("g-icon-b", "plus", "Add", false))
                            .child(kit.icon_button("g-icon-c", "sidebar.leading", "Sidebar", true))
                            .child(kit.segmented(
                                "g-seg-symbols",
                                &[
                                    Item::Symbol("square.grid.2x2", "Grid view"),
                                    Item::Symbol("list.bullet", "List view"),
                                ],
                                0,
                                false,
                                none,
                            ))
                            .child(kit.segmented(
                                "g-seg-titles",
                                &[Item::Title("Layers"), Item::Title("Tracks")],
                                0,
                                false,
                                none,
                            )),
                    )
                    .child(kit.segmented(
                        "g-tabs",
                        &[
                            Item::Title("Project"),
                            Item::Title("Effect Controls"),
                            Item::Title("Notes"),
                        ],
                        0,
                        true,
                        none,
                    )),
            )
            .child(
                section("Search, switch, checkbox, radio, slider, progress")
                    .child(kit.search(None, Some("⌘F")))
                    .child(
                        line()
                            .child(kit.switch("g-switch-off", false))
                            .child(kit.switch("g-switch-on", true))
                            .child(kit.check("g-check-off", false, false))
                            .child(kit.check("g-check-on", true, false))
                            .child(kit.check("g-radio-off", false, true))
                            .child(kit.check("g-radio-on", true, true))
                            .child(kit.disclosure(false))
                            .child(kit.disclosure(true)),
                    )
                    .child(kit.slider("g-slider", 0.35))
                    .child(kit.progress(Some(0.6)))
                    .child(kit.progress(None)),
            )
            .child(
                section("Rows and a menu")
                    .child(kit.row("g-row-a", false).child("Gaussian Blur"))
                    .child(kit.row("g-row-b", true).child("Directional Blur"))
                    .child(kit.row("g-row-c", false).child("Sharpen"))
                    .child(
                        kit.menu()
                            .w(px(220.))
                            .child(kit.menu_light("g-menu", Some(t.metrics.row)))
                            .child(kit.menu_row("g-menu-a", "Recently edited", Some(true), None))
                            .child(kit.menu_row("g-menu-b", "Name", Some(false), None))
                            .child(kit.menu_row(
                                "g-menu-c",
                                "Open in New Window",
                                Some(false),
                                Some("⌘O"),
                            )),
                    ),
            )
            .child(
                section("Card and form rows, banner, empty state")
                    .child(
                        kit.card()
                            .child(kit.form_row(
                                "Reopen last project on launch",
                                Some("Open only the project you were last working on."),
                                kit.switch("g-form-switch", true).into_any_element(),
                            ))
                            .child(kit.form_row(
                                "Appearance",
                                Some("Follow your system setting, or pick a light or dark theme."),
                                kit.button(
                                    "g-form-popup",
                                    "Dark",
                                    Button::Neutral,
                                    None,
                                    Some("chevron.down"),
                                )
                                .into_any_element(),
                            )),
                    )
                    .child(kit.banner("checkmark.circle.fill", c.success, "Exported to Desktop"))
                    .child(kit.empty_state(
                        "magnifyingglass",
                        "No Results",
                        "No project matches this search.",
                    )),
            )
            .child(
                section("Timeline strips by layer colour and state")
                    .w(px(560.))
                    .children(
                        [c.labels.teal, c.labels.violet, c.labels.orange, c.labels.gray].map(
                            |fill| {
                                line()
                                    .gap(px(t.strip_gap()))
                                    .children(states.map(|(state, name)| strip(fill, state, name)))
                            },
                        ),
                    ),
            )
            .child(
                section("Type scale").children(roles.map(|(role, sample)| {
                    div()
                        .text_role(t, role)
                        .text_color(rgb(c.text))
                        .child(sample)
                })),
            )
            .into_any_element()
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
