//! S101 Panel Library: Window ▸ Panel Library. One window, bound to the
//! editor it was opened from: a source list (Panels: All, Built In, Made by
//! You, From the Store; Workspaces: All, Built In, Saved by You), a search
//! field, and the matching panels or workspaces as quiet cards. A panel card
//! adds the panel to that editor's workspace, or shows it when it is already
//! there; a workspace card activates it, and a saved one can be deleted (S012's
//! alert). Everything it lists comes from the editor's own panel list and the
//! kernel's workspace store; it keeps no state of its own.
use crate::session::Controller;
use gpui::{prelude::*, *};
use pm_design::Text;
use pm_shell::kit::{self, Kit, Recipes};
use pm_shell::symbol::Weight;
use pm_shell::workspace;

const TITLE: &str = "Panel Library";

fn panel_action(open: bool, elapsed: Option<std::time::Duration>) -> &'static str {
    if open && elapsed.is_some_and(|t| t < std::time::Duration::from_millis(1500)) {
        "Added ✓"
    } else if open {
        "Show"
    } else {
        "Add"
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    BuiltIn,
    /// A workspace's generated panel, or a panel from an extension made here.
    Yours,
    Store,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Panels(Option<Origin>),
    /// `Some(true)`: built-ins; `Some(false)`: saved by the person.
    Workspaces(Option<bool>),
}
impl Page {
    const ALL: [Page; 7] = [
        Page::Panels(None),
        Page::Panels(Some(Origin::BuiltIn)),
        Page::Panels(Some(Origin::Yours)),
        Page::Panels(Some(Origin::Store)),
        Page::Workspaces(None),
        Page::Workspaces(Some(true)),
        Page::Workspaces(Some(false)),
    ];
    fn title(self) -> &'static str {
        match self {
            Page::Panels(None) => "All Panels",
            Page::Panels(Some(Origin::BuiltIn)) | Page::Workspaces(Some(true)) => "Built In",
            Page::Panels(Some(Origin::Yours)) => "Made by You",
            Page::Panels(Some(Origin::Store)) => "From the Store",
            Page::Workspaces(None) => "All Workspaces",
            Page::Workspaces(Some(false)) => "Saved by You",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PanelCard {
    pub id: String,
    pub title: String,
    pub origin: Origin,
    /// In the editor's workspace now.
    pub open: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceCard {
    pub id: String,
    pub name: String,
    pub builtin: bool,
    pub panels: usize,
    pub active: bool,
}

/// What the library lists for editor `v`; `shown` is its window's live
/// layout (what's open, the workspace showing), else the stored record's.
fn cards(
    v: &Controller,
    shown: Option<&pm_shell::layout::Layout>,
) -> (Vec<PanelCard>, Vec<WorkspaceCard>) {
    let store = v.workspace_store();
    let record = store
        .project
        .clone()
        .filter(workspace::usable)
        .or_else(|| {
            store
                .current
                .as_deref()
                .and_then(|id| store.get(id))
                .cloned()
        })
        .unwrap_or_else(|| store.active(None));
    let generated = workspace::generated(&record);
    let mut contributed = v.extensions.contributed.clone();
    for (id, title) in &generated {
        if !contributed.iter().any(|(c, _)| c == id) {
            contributed.push((id.clone(), title.clone()));
        }
    }
    let from_store = |id: &str| v.extensions.stores.contains_key(id);
    let stored = workspace::layout(&record);
    let shown = shown.unwrap_or(&stored);
    let panels = panel_cards(
        &pm_shell::layout::Panels::new(contributed),
        &shown.open_panels(),
        from_store,
    );
    let active = Some(shown.workspace.clone());
    let workspaces = store
        .offered()
        .into_iter()
        .filter_map(|r| {
            let id = workspace::record_id(r)?.to_owned();
            Some(WorkspaceCard {
                active: active.as_deref() == Some(id.as_str()),
                name: workspace::record_name(r).to_owned(),
                builtin: workspace::is_builtin(r),
                panels: workspace::layout(r).open_panels().len(),
                id,
            })
        })
        .collect();
    (panels, workspaces)
}
fn panel_cards(
    panels: &pm_shell::layout::Panels,
    open: &[&str],
    from_store: impl Fn(&str) -> bool,
) -> Vec<PanelCard> {
    panels
        .ids()
        .into_iter()
        .map(|id| PanelCard {
            id: id.to_owned(),
            title: panels.title(id).to_owned(),
            origin: if pm_shell::layout::PANELS.contains(&id) {
                Origin::BuiltIn
            } else if from_store(id) {
                Origin::Store
            } else {
                Origin::Yours
            },
            open: open.contains(&id),
        })
        .collect()
}
fn matches(text: &str, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    query.is_empty() || text.to_lowercase().contains(&query)
}
pub fn shown_panels(cards: &[PanelCard], origin: Option<Origin>, query: &str) -> Vec<PanelCard> {
    cards
        .iter()
        .filter(|c| origin.is_none_or(|o| c.origin == o) && matches(&c.title, query))
        .cloned()
        .collect()
}
pub fn shown_workspaces(
    cards: &[WorkspaceCard],
    builtin: Option<bool>,
    query: &str,
) -> Vec<WorkspaceCard> {
    cards
        .iter()
        .filter(|c| builtin.is_none_or(|b| c.builtin == b) && matches(&c.name, query))
        .cloned()
        .collect()
}

/// Pure shelf geometry, shared with Electron's shelves.ts.
/// The grid's columns for a content `width`: as many cards as fit at
/// `least` wide, `gap` apart; each card's width fills the row.
fn grid(width: f32, least: f32, gap: f32) -> (usize, f32) {
    if !width.is_finite() || width <= 0. {
        return (1, least);
    }
    let columns = (((width + gap) / (least + gap)).floor() as usize).max(1);
    let card = (width - gap * (columns - 1) as f32) / columns as f32;
    (columns, card.max(1.))
}
/// A cover's shape inside its card: a fixed 16:10 frame, the panel's
/// picture fitted inside it, never stretched.
const COVER_ASPECT: f32 = 10. / 16.;
pub(super) fn panel_size(id: &str) -> (f32, f32) {
    match id {
        "timeline" => (960., 360.),
        "inspector" | "layer-effects" => (320., 600.),
        "viewer" => (800., 480.),
        _ => (360., 420.),
    }
}

pub struct PanelLibrary {
    editor: Option<WeakEntity<Controller>>,
    /// The editor's shell view: its live layout.
    shell: Option<WeakEntity<pm_shell::view::Editor>>,
    page: Page,
    query: String,
    /// The sidebar's system search field (no AppKit window: none).
    search: Option<Entity<pm_shell::native_control::NativeControl>>,
    tokens: pm_design::Tokens,
    motion: pm_shell::motion::Motion<u64>,
    focus: std::collections::HashMap<String, FocusHandle>,
    focused: Option<String>,
    scroll: ScrollHandle,
    /// Each card's system push button, by card and title (a button's title
    /// is fixed when it is made: Add becoming Show makes a new one).
    buttons: std::collections::HashMap<String, Entity<pm_shell::native_control::NativeControl>>,
    previews: std::collections::HashMap<String, std::sync::Arc<Image>>,
    /// Each cover's bytes as last shown: a capture that brings the same
    /// picture keeps the shown image (no re-upload, no flicker).
    preview_sums: std::collections::HashMap<String, u64>,
    preview_stamp: Vec<(String, u64)>,
    preview_task: Option<Task<()>>,
    preview_cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    added: std::collections::HashMap<String, std::time::Instant>,
    errors: std::collections::HashMap<String, String>,
    announcement: String,
    #[cfg(test)]
    fixture: Option<(Vec<PanelCard>, Vec<WorkspaceCard>)>,
}
impl PanelLibrary {
    fn refresh_previews(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.as_ref().and_then(WeakEntity::upgrade) else {
            return;
        };
        let e = editor.read(cx);
        // Covers are drawn again only when what they show changes: the
        // panels listed, which are open, the project. Not on every panel
        // publication (a playing viewer republishes each frame, and every
        // capture swapped every cover: the flicker and blanks).
        let (panels, _) = cards(e, None);
        let mut stamp: Vec<_> = panels
            .iter()
            .map(|card| (card.id.clone(), u64::from(card.open)))
            .collect();
        if let Some(p) = &e.published {
            stamp.push((format!("document-{}", p.snapshot.doc_id), 0));
        }
        if self.preview_task.is_some() && stamp == self.preview_stamp {
            return;
        }
        self.preview_stamp = stamp;
        let kernel = e.kernel.clone();
        let project = e.session.as_ref().map(|s| s.project.clone());
        // The viewer's cover is the project's own poster frame (Home's, in
        // the profile's thumbnail cache): its picture is the engine's.
        let poster = project.as_ref().map(|id| {
            crate::poster::files(&e.profile.join("caches/thumbnails"), id).0
        });
        let dir = crate::media::cache_root(&e.profile).join("panel-covers-v1");
        let tokens = self.tokens.clone();
        self.preview_cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.preview_cancel = Default::default();
        let cancel = self.preview_cancel.clone();
        let publications = e
            .builtins
            .views
            .iter()
            .map(|(id, view)| (id.clone(), view.read(cx).publication.clone()))
            .collect();
        let web: Vec<_> = panels
            .iter()
            .filter_map(|card| {
                let page = e.extensions.page_of(&card.id)?;
                let host = e.host.borrow();
                let bodies = host.native_panels.borrow();
                let (_, body) = bodies.iter().find(|(id, _)| id == &card.id)?;
                let ticket = page
                    .borrow()
                    .capture_rect_png(pm_compat_host::Rect::new(
                        body.x as f64,
                        body.y as f64,
                        body.w as f64,
                        body.h as f64,
                    ))
                    .ok()?;
                Some((card.id.clone(), ticket))
            })
            .collect();
        let visible_bodies: Vec<_> = self
            .shell
            .as_ref()
            .and_then(WeakEntity::upgrade)
            .map(|shell| {
                e.builtins
                    .views
                    .keys()
                    // The viewer's picture is the engine's own layer, not in
                    // the window's image: its crop would be a blank slab.
                    .filter(|id| id.as_str() != "viewer")
                    .filter_map(|id| {
                        let mut body = shell.read(cx).panel_rect(id)?;
                        body.y += self.tokens.metrics.header;
                        body.h = (body.h - self.tokens.metrics.header).max(0.);
                        Some((id.clone(), body))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let source_window = e
            .builtins
            .views
            .iter()
            .find(|(id, _)| id.as_str() != "agent")
            .map(|(_, view)| view.read(cx).library_window());
        let drawn = source_window.and_then(|handle| {
            handle
                .update(cx, |_, window, _| {
                    let frame = window.render_to_image().ok()?;
                    let (w, h) = (frame.width(), frame.height());
                    Some((frame.into_raw(), w, h, window.scale_factor()))
                })
                .ok()
                .flatten()
        });
        let panels = if drawn.is_some() {
            panels
                .into_iter()
                .filter(|card| !visible_bodies.iter().any(|(id, _)| id == &card.id))
                .collect()
        } else {
            panels
        };
        self.preview_task = Some(cx.spawn(async move |this, cx| {
            let images = cx
                .background_executor()
                .spawn(async move {
                    let mut images = crate::panel_library_preview::capture(
                        kernel,
                        project,
                        panels,
                        publications,
                        tokens,
                        dir,
                        cancel.clone(),
                    );
                    if let Some((frame, w, h, scale)) = drawn {
                        for (id, rect) in visible_bodies {
                            if let Some(png) = crate::panel_library_preview::crop_drawn(
                                &id, &frame, w, h, scale, rect,
                            ) {
                                images.insert(id, png);
                            }
                        }
                    }
                    for (id, ticket) in web {
                        let deadline =
                            std::time::Instant::now() + std::time::Duration::from_secs(2);
                        while !cancel.load(std::sync::atomic::Ordering::Relaxed)
                            && std::time::Instant::now() < deadline
                        {
                            if let Some(result) = ticket.try_take() {
                                if let Ok(png) = result {
                                    if let Some(png) =
                                        crate::panel_library_preview::fit_web(&id, png)
                                    {
                                        images.insert(id, png);
                                    }
                                }
                                break;
                            }
                            std::thread::sleep(std::time::Duration::from_millis(10));
                        }
                    }
                    if let Some(png) = poster.and_then(|path| std::fs::read(path).ok()) {
                        images.insert("viewer".into(), png);
                    }
                    // A cover with nothing on it (an empty panel's plain
                    // ground) is no picture: the card keeps its symbol.
                    images.retain(|_, png| !crate::panel_library_preview::blank(png));
                    images
                })
                .await;
            let _ = this.update(cx, |v, cx| {
                // Merged, never replaced: a cover this pass couldn't draw
                // keeps the one shown, and the same picture keeps its image.
                let mut changed = false;
                for (id, png) in images {
                    let sum = {
                        use std::hash::{Hash, Hasher};
                        let mut h = std::collections::hash_map::DefaultHasher::new();
                        png.hash(&mut h);
                        h.finish()
                    };
                    if v.preview_sums.get(&id) == Some(&sum) {
                        continue;
                    }
                    v.preview_sums.insert(id.clone(), sum);
                    v.previews.insert(
                        id,
                        std::sync::Arc::new(Image::from_bytes(ImageFormat::Png, png)),
                    );
                    changed = true;
                }
                if changed {
                    cx.notify();
                }
            });
        }));
    }
    fn motion(&mut self) -> &mut pm_shell::motion::Motion<u64> {
        &mut self.motion
    }
    fn set_page(&mut self, page: Page, cx: &mut Context<Self>) {
        self.motion.values.borrow_mut().insert(
            kit::key("library-page-fade"),
            pm_design::motion::Animated::unit(0.),
        );
        self.scroll.set_offset(point(px(0.), px(0.)));
        self.page = page;
        self.query.clear();
        if let Some(search) = &self.search {
            search.read(cx).clear_search();
        }
        cx.notify();
    }
    /// Whether there is a layout to save: the editor holds a project. After
    /// its project closes (the window shows Home) the library stays open,
    /// and Save Current as New has nothing to save.
    fn saves_current(&self, cx: &App) -> bool {
        #[cfg(test)]
        if self.fixture.is_some() {
            return true;
        }
        self.editor
            .as_ref()
            .and_then(WeakEntity::upgrade)
            .is_some_and(|e| e.read(cx).session.is_some())
    }
    fn send(&mut self, command: String, cx: &mut Context<Self>) {
        if let Some(id) = command.strip_prefix("panel:activate:") {
            if let Some(shell) = self.shell.as_ref().and_then(WeakEntity::upgrade) {
                let was_open = shell.read(cx).layout.find(id).is_some();
                let result = shell.update(cx, |v, cx| v.show_library_panel(id, cx));
                match result {
                    Ok(dock) => {
                        self.errors.remove(id);
                        if !was_open {
                            self.added.insert(id.into(), std::time::Instant::now());
                            self.announcement = format!(
                                "{} added to the {} dock",
                                shell.read(cx).panels(cx).title(id),
                                dock
                            );
                            cx.spawn(async move |this, cx| {
                                cx.background_executor()
                                    .timer(std::time::Duration::from_millis(1500))
                                    .await;
                                let _ = this.update(cx, |_, cx| cx.notify());
                            })
                            .detach();
                        }
                    }
                    Err(error) => {
                        self.errors.insert(id.into(), error);
                    }
                }
                cx.notify();
                return;
            }
        }
        if let Some(editor) = self.editor.as_ref().and_then(WeakEntity::upgrade) {
            if let Some(id) = command.strip_prefix("library-delete:") {
                editor.update(cx, |v, cx| {
                    v.confirm_library_panel_delete(id.to_owned(), cx)
                });
                return;
            }
            editor.read(cx).host.borrow().send_command(&command, vec![]);
        }
    }
    /// A system push button for `command` titled `title` (Add, Show,
    /// Activate…): made once per title and kept; pressing it runs the
    /// command. Only a real AppKit window holds one (tests have none).
    fn push(
        &mut self,
        command: &str,
        title: &str,
        disabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<pm_shell::native_control::NativeControl>> {
        use pm_shell::native_control::{ControlSpec, Kind, NativeControl};
        crate::platform::ns_window(window)?;
        let key = format!("{command}|{title}");
        if !self.buttons.contains_key(&key) {
            let prefix = format!("{command}|");
            self.buttons.retain(|k, _| !k.starts_with(&prefix));
            let tokens = self.tokens.clone();
            let control = cx.new(|cx| {
                NativeControl::new(
                    ControlSpec {
                        kind: Kind::Push,
                        label: title,
                        identifier: command,
                        value: &serde_json::Value::Null,
                        options: vec![],
                        disabled,
                        default: false,
                        destructive: false,
                        inline: false,
                        quiet: false,
                        pull: false,
                        tint: None,
                        symbol: None,
                        glass: false,
                        well: false,
                    },
                    &tokens,
                    window,
                    cx,
                )
            });
            let run = command.to_owned();
            cx.subscribe(&control, move |v: &mut Self, _, _: &serde_json::Value, cx| {
                v.send(run.clone(), cx)
            })
            .detach();
            self.buttons.insert(key.clone(), control);
        }
        let control = self.buttons[&key].clone();
        control.update(cx, |c, cx| c.update(&serde_json::Value::Null, &[], disabled, cx));
        Some(control)
    }
    /// The action under a card: the system button, or (a test window) the
    /// kit's.
    fn action(
        &mut self,
        kit: &Kit,
        command: String,
        title: &str,
        disabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match self.push(&command, title, disabled, window, cx) {
            Some(control) => div().flex_none().child(control).into_any_element(),
            None => kit
                .button(
                    SharedString::from(format!("library-action-{command}")),
                    title.to_owned(),
                    kit::Button::Neutral,
                    None,
                    None,
                )
                .on_click(cx.listener(move |v, _, _, cx| v.send(command.clone(), cx)))
                .into_any_element(),
        }
    }
    /// One card, Settings' card anatomy: the well, its corner, one inset
    /// (`space.sm`) on every side; the cover in a fixed 16:10 frame whose
    /// corner is the card's less that inset; under it the name, its source
    /// and the action.
    #[allow(clippy::too_many_arguments)]
    fn card(
        &self,
        kit: &Kit,
        id: &str,
        width: f32,
        cover: AnyElement,
        title: String,
        source: String,
        action: AnyElement,
        delete: Option<AnyElement>,
        dimmed: bool,
    ) -> Stateful<Div> {
        let t = &kit.t;
        let c = &t.colors;
        let st = pm_shell::scale().settings;
        let inset = t.space.sm;
        let corner = st.group_radius;
        let inner = t.radius.nested(corner, inset);
        let cover_w = width - 2. * inset;
        div()
            .id(SharedString::from(format!("library-card-{id}")))
            .flex_none()
            .w(px(width))
            .p(px(inset))
            .flex()
            .flex_col()
            .gap(px(inset))
            .rounded(px(corner))
            .bg(kit::ink(c.text, pm_design::Ink::new().well))
            .border_1()
            .border_color(kit::ink(c.text, pm_design::Ink::new().edge * 0.5))
            .opacity(if dimmed { 0.62 } else { 1. })
            .child(
                div()
                    .relative()
                    .w(px(cover_w))
                    .h(px((cover_w * COVER_ASPECT).round()))
                    .rounded(px(inner))
                    .overflow_hidden()
                    .bg(kit::ink(c.text, pm_design::Ink::new().well))
                    .child(cover)
                    .children(delete.map(|d| {
                        div()
                            .absolute()
                            .top(px(t.space.xs))
                            .right(px(t.space.xs))
                            .child(d)
                    })),
            )
            .child(
                div()
                    .px(px(t.space.xs))
                    .pb(px(t.space.xs))
                    .flex()
                    .items_center()
                    .gap(px(t.space.sm))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .truncate()
                                    .text_role(t, Text::Label)
                                    .text_color(rgb(c.text))
                                    .child(title),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_role(t, Text::Caption)
                                    .text_color(rgb(c.secondary))
                                    .child(source),
                            ),
                    )
                    .child(action),
            )
    }
    /// A panel's cover: its real picture, fitted in the frame (never
    /// stretched), fading in once; until it is drawn, the panel's symbol,
    /// quietly.
    fn cover(&self, kit: &Kit, id: &str, symbol: &str) -> AnyElement {
        let t = &kit.t;
        match self.previews.get(id) {
            Some(image) => {
                let fade = if t.accessibility.reduce_motion {
                    1.
                } else {
                    kit.animate(
                        kit::key(&format!("preview-{id}")),
                        1.,
                        Some(pm_design::motion::Preset::Show),
                    )
                };
                img(ImageSource::Image(image.clone()))
                    .size_full()
                    .object_fit(ObjectFit::Contain)
                    .opacity(fade)
                    .into_any_element()
            }
            None => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(kit.symbol(symbol, t.metrics.symbol_panel * 1.5, Weight::Regular, t.colors.faint))
                .into_any_element(),
        }
    }
    fn panel_card(
        &mut self,
        card: &PanelCard,
        kit: &Kit,
        width: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = kit.t.clone();
        let c = &t.colors;
        let command = format!("panel:activate:{}", card.id);
        let title = panel_action(
            card.open,
            self.added.get(&card.id).map(std::time::Instant::elapsed),
        );
        let editor = self.editor.as_ref().and_then(WeakEntity::upgrade);
        let owner = editor.as_ref().and_then(|e| {
            e.read(cx)
                .extensions
                .stores
                .get(&card.id)
                .map(|(id, _)| id.clone())
        });
        let (dimmed, metadata) = match (owner.as_ref(), editor.as_ref()) {
            (Some(id), Some(e)) => {
                let state = e.read(cx).kernel.state();
                let entry = state.extensions.get(id);
                let dimmed = entry.is_none_or(|entry| !entry.enabled || !entry.installed);
                let metadata = entry.map(|entry| {
                    let record = entry.source.get("hostRecord").unwrap_or(&entry.source);
                    let name = record
                        .get("name")
                        .and_then(pm_model::Value::as_str)
                        .unwrap_or(id)
                        .to_owned();
                    let icon = entry
                        .source
                        .get("manifest")
                        .and_then(|m| m.get("icon"))
                        .and_then(pm_model::Value::as_str)
                        .map(str::to_owned);
                    (name, icon)
                });
                (dimmed, metadata)
            }
            _ => (false, None),
        };
        let source = match card.origin {
            Origin::BuiltIn => "Built In".to_owned(),
            Origin::Yours => metadata
                .as_ref()
                .map_or_else(|| "Made by You".to_owned(), |(name, _)| name.clone()),
            Origin::Store => metadata
                .as_ref()
                .map_or_else(|| "From the Store".to_owned(), |(name, _)| name.clone()),
        };
        let symbol = pm_shell::symbol::panel_symbol_with_manifest(
            &card.id,
            metadata.as_ref().and_then(|(_, icon)| icon.as_deref()),
        )
        .to_owned();
        let delete_command = owner
            .map(|id| format!("extensionArchive.{id}"))
            .or_else(|| (card.origin == Origin::Yours).then(|| format!("library-delete:{}", card.id)));
        let key = kit::key(&format!("library-card-{}", card.id));
        let hover = if self.focused.as_deref() == Some(&card.id) {
            1.
        } else {
            kit.hovered(key)
        };
        let delete = delete_command.clone().map(|command| {
            kit.destructive_icon_button(
                format!("library-delete-{}", card.id),
                "trash",
                format!("Delete {}", card.title),
            )
            .opacity(hover)
            .rounded(px(t.radius.nested(
                t.radius.nested(pm_shell::scale().settings.group_radius, t.space.sm),
                t.space.xs,
            )))
            .bg(rgb(c.floating))
            .on_click(cx.listener(move |v, _, _, cx| {
                cx.stop_propagation();
                v.send(command.clone(), cx)
            }))
            .into_any_element()
        });
        let action = self.action(kit, command.clone(), title, false, window, cx);
        let cover = self.cover(kit, &card.id, &symbol);
        let keyboard = command.clone();
        let element = self
            .card(kit, &card.id, width, cover, card.title.clone(), source, action, delete, dimmed)
            .track_focus(self.focus.get(&card.id).expect("card focus"))
            .role(Role::Group)
            .aria_label(format!("{}, {}", card.title, title))
            .tab_index(0)
            .on_key_down(cx.listener(move |v, e: &KeyDownEvent, _, cx| {
                if e.keystroke.key == "enter" {
                    v.send(keyboard.clone(), cx);
                    cx.stop_propagation();
                } else if matches!(e.keystroke.key.as_str(), "backspace" | "delete")
                    && let Some(command) = &delete_command
                {
                    v.send(command.clone(), cx);
                    cx.stop_propagation();
                }
            }));
        div()
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(t.space.xs))
            .child(kit.track_hover(element, key))
            .children(self.errors.get(&card.id).map(|error| {
                div()
                    .w(px(width))
                    .text_role(&t, Text::Caption)
                    .text_color(rgb(c.danger))
                    .child(error.clone())
            }))
            .into_any_element()
    }
    fn workspace_card(
        &mut self,
        card: &WorkspaceCard,
        kit: &Kit,
        width: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = kit.t.clone();
        let c = &t.colors;
        let layout = self
            .editor
            .as_ref()
            .and_then(WeakEntity::upgrade)
            .and_then(|e| {
                e.read(cx)
                    .workspace_store()
                    .get(&card.id)
                    .map(workspace::layout)
            })
            .or_else(|| workspace::preset(&card.id).as_ref().map(workspace::layout));
        // The workspace's docks drawn as its map: each panel a tile with its
        // symbol, one inset apart, corners nested in the cover's.
        let names = pm_shell::layout::Panels::new(Vec::new());
        let inset = t.space.xs;
        let cover_corner = t.radius.nested(pm_shell::scale().settings.group_radius, t.space.sm);
        let mut map = div().size_full().flex().gap(px(inset)).p(px(inset));
        if let Some(layout) = layout {
            for dock in &layout.docks {
                let mut col = div().h_full().flex().flex_col().gap(px(inset));
                col = if dock.size.is_some() {
                    col.w(px((width - 2. * t.space.sm) * 0.24))
                } else {
                    col.flex_1()
                };
                for group in &dock.groups {
                    let id = group.active();
                    col = col.child(
                        div()
                            .flex_1()
                            .min_h(px(0.))
                            .overflow_hidden()
                            .flex()
                            .flex_col()
                            .items_center()
                            .justify_center()
                            .gap(px(t.space.hair))
                            .rounded(px(t.radius.nested(cover_corner, inset)))
                            .bg(kit::ink(c.text, t.translucency.hover))
                            .child(kit.symbol(
                                pm_shell::symbol::panel_symbol(id),
                                t.metrics.symbol_panel,
                                Weight::Regular,
                                c.secondary,
                            ))
                            .child(
                                div()
                                    .max_w_full()
                                    .px(px(inset))
                                    .truncate()
                                    .text_role(&t, Text::Caption)
                                    .text_color(rgb(c.secondary))
                                    .child(names.title(id).to_owned()),
                            ),
                    );
                }
                map = map.child(col);
            }
        }
        let command = format!("workspace:{}", card.id);
        let action = self.action(
            kit,
            command.clone(),
            if card.active { "Active" } else { "Activate" },
            card.active,
            window,
            cx,
        );
        let key = kit::key(&format!("workspace-card-{}", card.id));
        let hover = if self.focused.as_deref() == Some(&card.id) {
            1.
        } else {
            kit.hovered(key)
        };
        let (id, name) = (card.id.clone(), card.name.clone());
        let delete = (!card.builtin).then(|| {
            kit.destructive_icon_button(
                format!("workspace-delete-{}", card.id),
                "trash",
                format!("Delete {}", card.name),
            )
            .opacity(hover)
            .rounded(px(t.radius.nested(cover_corner, t.space.xs)))
            .bg(rgb(c.floating))
            .on_click(cx.listener(move |v, _, _, cx| {
                cx.stop_propagation();
                if let Some(e) = v.editor.as_ref().and_then(WeakEntity::upgrade) {
                    e.update(cx, |e, cx| e.confirm_workspace_delete(id.clone(), name.clone(), cx));
                }
            }))
            .into_any_element()
        });
        let source = if card.builtin { "Built In" } else { "Saved by You" };
        let detail = format!("{source} · {} panels", card.panels);
        let keyboard = command.clone();
        let keyboard_delete = (!card.builtin).then(|| (card.id.clone(), card.name.clone()));
        let element = self
            .card(kit, &card.id, width, map.into_any_element(), card.name.clone(), detail, action, delete, false)
            .track_focus(self.focus.get(&card.id).expect("workspace focus"))
            .role(Role::Group)
            .aria_label(card.name.clone())
            .tab_index(0)
            .on_key_down(cx.listener(move |v, e: &KeyDownEvent, _, cx| match e.keystroke.key.as_str() {
                "enter" => {
                    v.send(keyboard.clone(), cx);
                    cx.stop_propagation();
                }
                "backspace" | "delete" => {
                    if let Some((id, name)) = &keyboard_delete
                        && let Some(e) = v.editor.as_ref().and_then(WeakEntity::upgrade)
                    {
                        e.update(cx, |v, cx| v.confirm_workspace_delete(id.clone(), name.clone(), cx));
                        cx.stop_propagation();
                    }
                }
                _ => {}
            }));
        kit.track_hover(element, key).into_any_element()
    }
}
impl Drop for PanelLibrary {
    fn drop(&mut self) {
        self.preview_cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}
impl Render for PanelLibrary {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.refresh_previews(cx);
        self.motion.frame(window);
        let t = self.tokens.clone();
        let kit = Kit::new(&t, &self.motion, Self::motion, window, cx);
        let (c, m, ty) = (&t.colors, &t.metrics, &t.typography);
        let st = pm_shell::scale().settings;
        let (ground, ground_alpha) = t.ground();
        let saves = self.saves_current(cx);
        let (panels, workspaces) = self
            .editor
            .as_ref()
            .and_then(WeakEntity::upgrade)
            .map(|e| {
                let shell = self.shell.as_ref().and_then(WeakEntity::upgrade);
                cards(e.read(cx), shell.as_ref().map(|s| &s.read(cx).layout))
            })
            .unwrap_or_default();
        #[cfg(test)]
        let (panels, workspaces) = self.fixture.clone().unwrap_or((panels, workspaces));
        for id in panels
            .iter()
            .map(|p| &p.id)
            .chain(workspaces.iter().map(|w| &w.id))
        {
            if !self.focus.contains_key(id) {
                let handle = cx.focus_handle();
                let focused = id.clone();
                cx.on_focus(&handle, window, move |v, _, cx| {
                    v.focused = Some(focused.clone());
                    cx.notify();
                })
                .detach();
                cx.on_blur(&handle, window, |v, _, cx| {
                    v.focused = None;
                    cx.notify();
                })
                .detach();
                self.focus.insert(id.clone(), handle);
            }
        }
        // The sidebar: Settings' (its width, its inset, the search on top,
        // quiet group names, the source list's rows).
        let mut sidebar = div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(st.sidebar))
            .h_full()
            .pt(px(m.toolbar))
            .px(px(st.sidebar_inset))
            .pb(px(st.sidebar_inset))
            .bg(pm_shell::view::tint(ground, ground_alpha))
            .children(self.search.clone().map(|field| div().mb(px(t.space.md)).child(field)));
        for page in Page::ALL {
            let heading = match page {
                Page::Panels(None) => Some("Panels"),
                Page::Workspaces(None) => Some("Workspaces"),
                _ => None,
            };
            let count = match page {
                Page::Panels(o) => shown_panels(&panels, o, "").len(),
                Page::Workspaces(b) => shown_workspaces(&workspaces, b, "").len(),
            };
            if count == 0
                && matches!(
                    page,
                    Page::Panels(Some(Origin::Yours | Origin::Store)) | Page::Workspaces(Some(false))
                )
            {
                continue;
            }
            if let Some(heading) = heading {
                sidebar = sidebar.child(
                    div()
                        .pt(px(if page == Page::Panels(None) { 0. } else { t.space.lg }))
                        .pb(px(t.space.xs))
                        .px(px(t.space.md))
                        .text_role(&t, Text::Caption)
                        .font_weight(FontWeight(st.group_weight))
                        .text_color(rgb(c.muted))
                        .child(heading),
                );
            }
            let symbol = match page {
                Page::Panels(None) => "square.grid.2x2",
                Page::Panels(Some(Origin::BuiltIn)) | Page::Workspaces(Some(true)) => "square.stack",
                Page::Panels(Some(Origin::Yours)) => "wand.and.stars",
                Page::Panels(Some(Origin::Store)) => "puzzlepiece.extension",
                Page::Workspaces(None) => "rectangle.3.group",
                Page::Workspaces(Some(false)) => "person.crop.circle",
            };
            sidebar = sidebar.child(
                kit.nav_row(
                    format!("library-page-{page:?}"),
                    symbol,
                    page.title().to_owned(),
                    None,
                    self.page == page,
                )
                .tab_index(0)
                .on_key_down(cx.listener(move |v, e: &KeyDownEvent, _, cx| {
                    if e.keystroke.key == "enter" {
                        v.set_page(page, cx);
                        cx.stop_propagation();
                    }
                }))
                .on_click(cx.listener(move |v, _, _, cx| v.set_page(page, cx))),
            );
        }
        let store = self.action(&kit, "store".into(), "Browse the Store…", false, window, cx);
        sidebar = sidebar.child(div().flex_1()).child(div().flex().justify_center().child(store));
        // The content: the page's title on the traffic lights' line, then
        // the cards, as many to a row as fit, the room shared out.
        let gap = t.space.lg;
        let width = (f32::from(window.viewport_size().width) - st.sidebar - 2. * st.page_inset).max(160.);
        let (columns, card_w) = grid(width, 200., gap);
        let mut cells: Vec<AnyElement> = vec![];
        let empty = match self.page {
            Page::Panels(origin) => {
                let shown = shown_panels(&panels, origin, &self.query);
                for card in &shown {
                    let cell = self.panel_card(card, &kit, card_w, window, cx);
                    cells.push(cell);
                }
                shown.is_empty()
            }
            Page::Workspaces(builtin) => {
                let shown = shown_workspaces(&workspaces, builtin, &self.query);
                for card in &shown {
                    let cell = self.workspace_card(card, &kit, card_w, window, cx);
                    cells.push(cell);
                }
                shown.is_empty()
            }
        };
        let mut grid_el = div()
            .id("library-shelf")
            .flex_none()
            .role(Role::List)
            .aria_label(self.page.title())
            .flex()
            .flex_col()
            .gap(px(gap));
        let mut cells = cells.into_iter().peekable();
        while cells.peek().is_some() {
            let row: Vec<AnyElement> = cells.by_ref().take(columns).collect();
            grid_el = grid_el.child(div().flex_none().flex().items_start().gap(px(gap)).children(row));
        }
        let fade = if t.accessibility.reduce_motion {
            1.
        } else {
            kit.animate(kit::key("library-page-fade"), 1., Some(pm_design::motion::Preset::Show))
        };
        let body = if empty {
            let (symbol, heading, detail) = if !self.query.trim().is_empty() {
                (
                    "magnifyingglass",
                    "No Results",
                    format!("Nothing matches “{}”.", self.query.trim()),
                )
            } else {
                match self.page {
                    Page::Panels(Some(Origin::Yours)) => (
                        "wand.and.stars",
                        "No Panels Yet",
                        "Panels you make with the agent appear here.".into(),
                    ),
                    Page::Panels(Some(Origin::Store)) => (
                        "puzzlepiece.extension",
                        "No Extension Panels",
                        "Extensions from the Store add their panels here.".into(),
                    ),
                    Page::Workspaces(Some(false)) => (
                        "rectangle.3.group",
                        "No Saved Workspaces",
                        "Save the current layout to keep it here.".into(),
                    ),
                    _ => ("square.grid.2x2", "No Panels", String::new()),
                }
            };
            let follow = match self.page {
                Page::Panels(Some(Origin::Store)) if self.query.trim().is_empty() => {
                    Some(self.action(&kit, "store".into(), "Browse the Store…", false, window, cx))
                }
                Page::Workspaces(Some(false)) if self.query.trim().is_empty() => Some(self.action(
                    &kit,
                    "layout:save".into(),
                    "Save Current as New",
                    !saves,
                    window,
                    cx,
                )),
                _ => None,
            };
            kit.empty_state(symbol, heading, &detail)
                .children(follow)
                .flex_1()
                .into_any_element()
        } else {
            div()
                .id("library-body")
                .min_h(px(0.))
                .flex_1()
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .px(px(st.page_inset))
                .pt(px(st.page_top))
                .pb(px(st.page_bottom))
                .opacity(fade)
                .child(grid_el)
                .into_any_element()
        };
        let save = matches!(self.page, Page::Workspaces(_)).then(|| {
            self.action(&kit, "layout:save".into(), "Save Current as New", !saves, window, cx)
        });
        let content = div()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .flex_col()
            .bg(rgb(c.panel))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_between()
                    .gap(px(st.page_inset))
                    .h(px(m.toolbar))
                    .px(px(st.page_inset))
                    .child(
                        div()
                            .text_size(px(ty.heading))
                            .font_weight(FontWeight(st.header_weight))
                            .child(self.page.title()),
                    )
                    .children(save),
            )
            .child(body);
        div()
            .size_full()
            .id("panel-library-root")
            .tab_group()
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "tab" {
                    if event.keystroke.modifiers.shift {
                        window.focus_prev(cx);
                    } else {
                        window.focus_next(cx);
                    }
                    cx.stop_propagation();
                }
            }))
            .flex()
            .text_color(rgb(c.text))
            .text_size(px(ty.body))
            .font_family(pm_design::TEXT_FONT)
            .child(sidebar)
            .child(content)
            .child(
                div()
                    .id("library-announcement")
                    .absolute()
                    .size_0()
                    .overflow_hidden()
                    .role(Role::Status)
                    .aria_live(gpui::accesskit::Live::Polite)
                    .aria_label(self.announcement.clone()),
            )
            .child(kit.settle())
    }
}

/// Preserve unrelated workspace metadata and generated sections when removing one panel.
fn without_generated_panel(record: &serde_json::Value, id: &str) -> Option<serde_json::Value> {
    if !workspace::generated(record).iter().any(|(p, _)| p == id) {
        return None;
    }
    let mut after = record.clone();
    after
        .get_mut("custom")?
        .as_array_mut()?
        .retain(|p| p.get("id").and_then(serde_json::Value::as_str) != Some(id));
    let mut layout = workspace::layout(&after);
    if layout.open_panels().contains(&id) {
        layout.close_panel(id).ok()?;
    }
    Some(workspace::write(&layout, &after))
}

impl Controller {
    /// Generated sections belong to their workspace; delete by stable id after confirmation.
    fn confirm_library_panel_delete(&mut self, id: String, cx: &mut Context<Self>) {
        use crate::alert::{Alert, Button};
        let store = self.workspace_store();
        let Some(record) = store
            .project
            .iter()
            .chain(store.offered())
            .find(|r| workspace::generated(r).iter().any(|(p, _)| p == &id))
            .cloned()
        else {
            return;
        };
        let record_id = workspace::record_id(&record).unwrap_or_default().to_owned();
        let title = workspace::generated(&record)
            .into_iter()
            .find(|(p, _)| p == &id)
            .map(|(_, title)| title)
            .unwrap_or_else(|| id.clone());
        let answer=Alert::new(format!("Delete {title}?"),"This removes the panel from its workspace. Your project and its layers will not be deleted.",vec![Button::new("Cancel",false).default().cancel(),Button::new("Delete Panel",true).destructive()]).sheet(&self.host.borrow().window.clone());
        cx.spawn(async move |this, cx| {
            if answer.recv().await != Ok(Some(true)) {
                return;
            }
            let _ = this.update(cx, |v, cx| {
                let store = v.workspace_store();
                let Some(before) = store
                    .project
                    .iter()
                    .chain(store.offered())
                    .find(|r| workspace::record_id(r) == Some(record_id.as_str()))
                    .cloned()
                else {
                    return;
                };
                let Some(after) = without_generated_panel(&before, &id) else {
                    return;
                };
                let own = store.project.as_ref().and_then(workspace::record_id)
                    == Some(record_id.as_str());
                v.write_workspace(if own {
                    workspace::Write::Active { before, after }
                } else {
                    workspace::Write::Upsert(after)
                });
                cx.notify();
            });
        })
        .detach();
    }
    /// Window ▸ Panel Library, for this editor; brought forward when open.
    /// Opened after this update ends: the new window's first frame reads
    /// this editor.
    pub fn open_panel_library(&mut self, cx: &mut Context<Self>) {
        let editor = cx.entity();
        cx.defer(move |cx| open_library(editor, cx));
    }
}
fn open_library(editor: Entity<Controller>, cx: &mut App) {
    let me = editor.downgrade();
    if let Some(window) = cx.windows().into_iter().find(|w| {
        w.downcast::<PanelLibrary>()
            .and_then(|l| l.read(cx).ok())
            .is_some_and(|l| l.editor.as_ref() == Some(&me))
    }) {
        let _ = window.update(cx, |_, w, _| w.activate());
        return;
    }
    let shell = editor.read(cx).shell.as_ref().and_then(WeakEntity::upgrade);
    let t = pm_shell::tokens();
    let (m, tokens) = (t.metrics, t.clone());
    let st = t.settings;
    let background = if t.translucent() {
        gpui::MacosWindowBackground::Transparent
    } else {
        gpui::MacosWindowBackground::Opaque
    };
    let _ = cx.open_window(
        WindowOptions::new()
            .macos_window_background(background)
            .titlebar(Some(TitlebarOptions {
                title: Some(TITLE.into()),
                appears_transparent: true,
                ..Default::default()
            }))
            // The Settings window family: its width and heights.
            .window_min_size(Some(size(px(st.window_width), px(st.window_height))))
            .window_bounds(Some(WindowBounds::Windowed(Bounds::new(
                point(px(m.toolbar), px(m.toolbar)),
                size(px(st.window_width), px(st.window_default_height)),
            )))),
        |window, cx| {
            if tokens.translucent() {
                pm_shell::window_backdrop(window, &tokens);
            }
            cx.new(|cx| {
                // Follows the editor: panels, workspaces, what's open
                // (the layout lives in the shell's editor view).
                cx.observe(&editor, |_, _, cx| cx.notify()).detach();
                if let Some(shell) = &shell {
                    cx.observe(shell, |_, _, cx| cx.notify()).detach();
                }
                for view in editor
                    .read(cx)
                    .builtins
                    .views
                    .values()
                    .cloned()
                    .collect::<Vec<_>>()
                {
                    cx.observe(&view, |_, _, cx| cx.notify()).detach();
                }
                // The system search field, as Settings' sidebar has.
                let search = cx.new(|cx| {
                    pm_shell::native_control::NativeControl::header_search(
                        "Search",
                        "panel-library-search",
                        "",
                        &tokens,
                        window,
                        cx,
                    )
                });
                cx.subscribe(&search, |v: &mut PanelLibrary, _, event: &serde_json::Value, cx| {
                    if let Some(text) = event.as_str() {
                        v.query = text.to_owned();
                        cx.notify();
                    } else if event["key"] == "Escape" {
                        if let Some(field) = &v.search {
                            field.read(cx).clear_search();
                        }
                        v.query.clear();
                        cx.notify();
                    }
                })
                .detach();
                PanelLibrary {
                    editor: Some(editor.downgrade()),
                    shell: shell.as_ref().map(Entity::downgrade),
                    page: Page::Panels(None),
                    query: String::new(),
                    search: Some(search),
                    tokens,
                    motion: Default::default(),
                    focus: Default::default(),
                    focused: None,
                    scroll: ScrollHandle::new(),
                    buttons: Default::default(),
                    previews: Default::default(),
                    preview_sums: Default::default(),
                    preview_stamp: Default::default(),
                    preview_task: None,
                    preview_cancel: Default::default(),
                    added: Default::default(),
                    errors: Default::default(),
                    announcement: String::new(),
                    #[cfg(test)]
                    fixture: None,
                }
            })
        },
    );
}

#[cfg(test)]
mod tests {
    use super::{Origin, WorkspaceCard, panel_cards, shown_panels, shown_workspaces};
    /// The library draws its covers from a background-executor task, which
    /// runs on a GCD worker thread with a 512 KiB stack. Every built-in
    /// panel's cover renders from such a thread: the panel render's
    /// recursion overflowed it and the app ended when Panels was pressed
    /// (PM-669).
    #[test]
    fn covers_draw_from_a_dispatch_worker_sized_thread() {
        let rig = crate::v4_view_tests::rig("timeline", 8);
        let kernel = rig.kernel.clone();
        let cards = panel_cards(&pm_shell::layout::Panels::new(Vec::new()), &[], |_| false);
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("covers");
        let images = std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn(move || {
                crate::panel_library_preview::capture(
                    kernel,
                    Some(crate::v4_view_tests::PROJECT.into()),
                    cards,
                    Default::default(),
                    pm_design::Tokens::new(pm_design::Scheme::Dark, Default::default()),
                    cache,
                    Default::default(),
                )
            })
            .unwrap()
            .join()
            .unwrap();
        assert!(images.contains_key("timeline"), "timeline cover drawn: {:?}", images.keys());
    }
    #[test]
    fn panel_feedback_goes_from_add_to_added_to_show() {
        use std::time::Duration;
        assert_eq!(super::panel_action(false, None), "Add");
        assert_eq!(super::panel_action(true, Some(Duration::ZERO)), "Added ✓");
        assert_eq!(
            super::panel_action(true, Some(Duration::from_millis(1499))),
            "Added ✓"
        );
        assert_eq!(
            super::panel_action(true, Some(Duration::from_millis(1500))),
            "Show"
        );
        assert_eq!(super::panel_action(true, None), "Show");
        assert_eq!(super::panel_action(false, Some(Duration::ZERO)), "Add");
    }

    #[test]
    fn panels_sort_into_shelves_by_where_they_came_from_and_search_by_title() {
        let panels = pm_shell::layout::Panels::new(vec![
            ("custom-1".into(), "Knobs".into()),
            ("com.example.glow.panel".into(), "Glow".into()),
        ]);
        let cards = panel_cards(&panels, &["viewer", "custom-1"], |id| {
            id == "com.example.glow.panel"
        });
        let origin = |id: &str| cards.iter().find(|c| c.id == id).unwrap().origin;
        assert_eq!(origin("viewer"), Origin::BuiltIn);
        assert_eq!(origin("custom-1"), Origin::Yours);
        assert_eq!(origin("com.example.glow.panel"), Origin::Store);
        assert!(cards.iter().find(|c| c.id == "custom-1").unwrap().open);
        assert!(!cards.iter().find(|c| c.id == "timeline").unwrap().open);
        let titles = |o, q| -> Vec<String> {
            shown_panels(&cards, o, q)
                .into_iter()
                .map(|c| c.title)
                .collect()
        };
        assert_eq!(titles(Some(Origin::Yours), ""), ["Knobs"]);
        assert_eq!(titles(Some(Origin::Store), ""), ["Glow"]);
        assert_eq!(titles(None, "  vIEW "), ["Viewer"]);
        assert!(titles(Some(Origin::Store), "knobs").is_empty());
    }

    #[test]
    fn workspaces_filter_by_built_in_and_name() {
        let card = |id: &str, builtin| WorkspaceCard {
            id: id.into(),
            name: id.to_uppercase(),
            builtin,
            panels: 3,
            active: false,
        };
        let cards = [card("design", true), card("grading", false)];
        let ids = |b, q| -> Vec<String> {
            shown_workspaces(&cards, b, q)
                .into_iter()
                .map(|c| c.id)
                .collect()
        };
        assert_eq!(ids(None, ""), ["design", "grading"]);
        assert_eq!(ids(Some(false), ""), ["grading"]);
        assert_eq!(ids(Some(true), "grad"), Vec::<String>::new());
    }
}

#[cfg(test)]
mod shelf_geometry_tests {
    #[test]
    fn deleting_a_generated_panel_preserves_other_sections_and_workspace_metadata() {
        let mut record = pm_shell::workspace::preset("design").unwrap();
        record["custom"] =
            serde_json::json!([{"id":"custom-a","title":"A"},{"id":"custom-b","title":"B"}]);
        record["reviewMetadata"] = serde_json::json!({"keep":true});
        record["layout"]["docks"][0]["panels"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"id":"custom-a","size":100}));
        let after = super::without_generated_panel(&record, "custom-a").unwrap();
        assert_eq!(
            after["custom"],
            serde_json::json!([{"id":"custom-b","title":"B"}])
        );
        assert_eq!(after["reviewMetadata"], record["reviewMetadata"]);
        assert!(
            !pm_shell::workspace::layout(&after)
                .open_panels()
                .contains(&"custom-a")
        );
        assert!(super::without_generated_panel(&after, "custom-a").is_none());
    }
    use super::grid;
    /// The grid takes as many columns as fit, the cards share the room, and
    /// a narrow or invalid width still gives one column.
    #[test]
    fn the_grid_reflows_to_its_width() {
        assert_eq!(grid(450., 200., 12.), (2, 219.));
        assert_eq!(grid(890., 200., 12.).0, 4);
        assert_eq!(grid(150., 200., 12.), (1, 150.));
        assert_eq!(grid(f32::NAN, 200., 12.), (1, 200.));
    }
}

#[cfg(test)]
mod capture_tests {
    use super::PanelLibrary;
    use gpui::{AppContext, HeadlessAppContext, ScrollHandle, px, size};
    #[test]
    #[ignore = "writes review PNGs; set PM_LIBRARY_CAPTURE"]
    fn capture_panel_library() {
        let dir = std::env::var("PM_LIBRARY_CAPTURE").expect("PM_LIBRARY_CAPTURE");
        std::fs::create_dir_all(&dir).unwrap();
        for (name, scheme, contrast) in [
            ("light", pm_design::Scheme::Light, false),
            ("dark", pm_design::Scheme::Dark, false),
            ("contrast", pm_design::Scheme::Dark, true),
        ] {
            if std::env::var_os("PM_LIBRARY_CAPTURE_REVIEW").is_some() && name == "contrast" {
                continue;
            }
            for (w, h) in [(900., 600.), (1400., 860.)] {
                if std::env::var_os("PM_LIBRARY_CAPTURE_REVIEW").is_some() && w != 900. {
                    continue;
                }
                objc2::rc::autoreleasepool(|_| {
                    let mut cx = HeadlessAppContext::with_platform(
                        std::sync::Arc::new(
                            gpui_parley::ParleyTextSystem::new(gpui_parley::SystemFonts::Load)
                                .with_automatic_optical_sizing(),
                        ),
                        std::sync::Arc::new(()),
                        || {
                            Some(Box::new(
                                gpui_apple::metal_renderer::MetalHeadlessRenderer::new(),
                            ))
                        },
                    );
                    let tokens = pm_design::Tokens::new(
                        scheme,
                        pm_design::Accessibility {
                            increase_contrast: contrast,
                            reduce_transparency: true,
                            reduce_motion: true,
                            ..Default::default()
                        },
                    );
                    let cards = super::panel_cards(
                        &pm_shell::layout::Panels::new(vec![
                            ("custom-notes".into(), "My Notes".into()),
                            ("store-color".into(), "Color Tools".into()),
                        ]),
                        &["viewer", "timeline"],
                        |id| id == "store-color",
                    );
                    let workspaces = pm_shell::workspace::BUILTIN
                        .iter()
                        .filter_map(|id| pm_shell::workspace::preset(id))
                        .map(|r| super::WorkspaceCard {
                            id: pm_shell::workspace::record_id(&r).unwrap().to_owned(),
                            name: pm_shell::workspace::record_name(&r).to_owned(),
                            builtin: true,
                            panels: pm_shell::workspace::layout(&r).open_panels().len(),
                            active: pm_shell::workspace::record_id(&r) == Some("design"),
                        })
                        .collect();
                    let window = cx
                        .open_window(size(px(w), px(h)), |_, cx| {
                            cx.new(|_| PanelLibrary {
                                editor: None,
                                shell: None,
                                page: super::Page::Panels(None),
                                query: std::env::var("PM_LIBRARY_CAPTURE_QUERY")
                                    .unwrap_or_default(),
                                search: None,
                                tokens,
                                motion: Default::default(),
                                focus: Default::default(),
                                focused: None,
                                scroll: ScrollHandle::new(),
                                buttons: Default::default(),
                                previews: Default::default(),
                                preview_sums: Default::default(),
                                preview_stamp: Default::default(),
                                preview_task: None,
                                preview_cancel: Default::default(),
                                added: Default::default(),
                                errors: Default::default(),
                                announcement: String::new(),
                                fixture: Some((cards, workspaces)),
                            })
                        })
                        .unwrap();
                    for _ in 0..40 {
                        cx.run_until_parked();
                        std::thread::sleep(std::time::Duration::from_millis(25));
                    }
                    cx.capture_screenshot(window.into())
                        .unwrap()
                        .save(format!("{dir}/after-{name}-{}x{}.png", w as u32, h as u32))
                        .unwrap();
                    // Capture real accepted panel surfaces through the same
                    // production headless path, using a synthetic document.
                    if w == 900. && name == "dark" {
                        let rig = crate::v4_view_tests::rig("timeline", 8);
                        let cards = super::panel_cards(
                            &pm_shell::layout::Panels::new(Vec::new()),
                            &[],
                            |_| false,
                        );
                        let images = crate::panel_library_preview::capture(
                            rig.kernel.clone(),
                            Some(crate::v4_view_tests::PROJECT.into()),
                            cards,
                            Default::default(),
                            pm_design::Tokens::new(scheme, Default::default()),
                            std::path::PathBuf::from(&dir).join("fixture-cache"),
                            Default::default(),
                        );
                        assert!(
                            images.contains_key("timeline"),
                            "real timeline cover must load"
                        );
                        window
                            .update(&mut cx, |view, _, cx| {
                                view.previews = images
                                    .into_iter()
                                    .map(|(id, png)| {
                                        (
                                            id,
                                            std::sync::Arc::new(gpui::Image::from_bytes(
                                                gpui::ImageFormat::Png,
                                                png,
                                            )),
                                        )
                                    })
                                    .collect();
                                cx.notify();
                            })
                            .unwrap();
                        for _ in 0..20 {
                            cx.run_until_parked();
                            std::thread::sleep(std::time::Duration::from_millis(10));
                        }
                        cx.capture_screenshot(window.into())
                            .unwrap()
                            .save(format!("{dir}/with-previews-dark-900x600.png"))
                            .unwrap();
                    }
                    for (state, elapsed) in [("added", 0), ("show", 2)] {
                        window
                            .update(&mut cx, |view, _, cx| {
                                if let Some((cards, _)) = &mut view.fixture {
                                    cards.iter_mut().find(|c| c.id == "inspector").unwrap().open =
                                        true;
                                }
                                view.added.insert(
                                    "inspector".into(),
                                    std::time::Instant::now()
                                        - std::time::Duration::from_secs(elapsed),
                                );
                                cx.notify();
                            })
                            .unwrap();
                        cx.run_until_parked();
                        cx.capture_screenshot(window.into())
                            .unwrap()
                            .save(format!(
                                "{dir}/{state}-{name}-{}x{}.png",
                                w as u32, h as u32
                            ))
                            .unwrap();
                    }
                    window
                        .update(&mut cx, |view, _, cx| {
                            view.set_page(super::Page::Workspaces(None), cx)
                        })
                        .unwrap();
                    for _ in 0..40 {
                        cx.run_until_parked();
                        std::thread::sleep(std::time::Duration::from_millis(25));
                    }
                    cx.capture_screenshot(window.into())
                        .unwrap()
                        .save(format!(
                            "{dir}/after-workspaces-{name}-{}x{}.png",
                            w as u32, h as u32
                        ))
                        .unwrap();
                });
            }
        }
    }
}

#[cfg(test)]
mod scrolling_tests {
    use super::*;
    use ::core::prelude::v1::test;
    fn fixture(cx: &mut Context<PanelLibrary>) -> PanelLibrary {
        let panels = (0..29)
            .map(|i| PanelCard {
                id: format!("panel-{i}"),
                title: format!("Panel {i}"),
                origin: Origin::BuiltIn,
                open: false,
            })
            .collect();
        let workspaces = (0..46)
            .map(|i| WorkspaceCard {
                id: format!("workspace-{i}"),
                name: format!("Workspace {i}"),
                builtin: true,
                panels: 4,
                active: i == 0,
            })
            .collect();
        let _ = cx;
        PanelLibrary {
            editor: None,
            shell: None,
            page: Page::Panels(None),
            query: String::new(),
            search: None,
            tokens: pm_design::Tokens::new(
                pm_design::Scheme::Dark,
                pm_design::Accessibility {
                    reduce_motion: true,
                    ..Default::default()
                },
            ),
            motion: Default::default(),
            focus: Default::default(),
            focused: None,
            scroll: ScrollHandle::new(),
            buttons: Default::default(),
            previews: Default::default(),
            preview_sums: Default::default(),
            preview_stamp: Default::default(),
            preview_task: None,
            preview_cancel: Default::default(),
            added: Default::default(),
            errors: Default::default(),
            announcement: String::new(),
            fixture: Some((panels, workspaces)),
        }
    }
    #[gpui::test]
    fn library_scroll_wheel_trackpad_momentum_clamping_and_sidebar(cx: &mut TestAppContext) {
        let (view, vcx) = cx.add_window_view(|_, cx| fixture(cx));
        let mut vcx = vcx.clone();
        vcx.simulate_resize(size(px(900.), px(600.)));
        vcx.run_until_parked();
        for page in [Page::Panels(None), Page::Workspaces(None)] {
            view.update(&mut vcx, |v, cx| v.set_page(page, cx));
            vcx.run_until_parked();
            let handle = view.read_with(&vcx, |v, _| v.scroll.clone());
            assert!(
                handle.max_offset().y > px(500.),
                "packed shelves must have their full height"
            );
            for delta in [
                ScrollDelta::Lines(point(0., -3.)),
                ScrollDelta::Pixels(point(px(0.), px(-42.5))),
                ScrollDelta::Pixels(point(px(0.), px(-12.))),
                ScrollDelta::Pixels(point(px(0.), px(-3.))),
            ] {
                let before = handle.offset().y;
                vcx.simulate_event(ScrollWheelEvent {
                    position: point(px(600.), px(300.)),
                    delta,
                    touch_phase: TouchPhase::Moved,
                    ..Default::default()
                });
                vcx.run_until_parked();
                assert!(
                    handle.offset().y < before,
                    "each input including momentum advances monotonically"
                );
            }
            let before = handle.offset();
            vcx.simulate_event(ScrollWheelEvent {
                position: point(px(60.), px(300.)),
                delta: ScrollDelta::Pixels(point(px(0.), px(-100.))),
                ..Default::default()
            });
            vcx.run_until_parked();
            assert_eq!(
                handle.offset(),
                before,
                "sidebar must not scroll the shelves"
            );
            for delta in [-100000., 100000.] {
                vcx.simulate_event(ScrollWheelEvent {
                    position: point(px(600.), px(300.)),
                    delta: ScrollDelta::Pixels(point(px(0.), px(delta))),
                    touch_phase: TouchPhase::Ended,
                    ..Default::default()
                });
                vcx.run_until_parked();
                assert!(handle.offset().y <= px(0.) && handle.offset().y >= -handle.max_offset().y);
            }
            let before = handle.offset();
            vcx.run_until_parked();
            assert_eq!(
                handle.offset(),
                before,
                "momentum cannot continue after input ends"
            );
        }
    }
    #[gpui::test]
    fn library_releases_cover_images_and_cancels_capture_on_close(cx: &mut TestAppContext) {
        let image = std::sync::Arc::new(Image::from_bytes(ImageFormat::Png, Vec::new()));
        let weak = std::sync::Arc::downgrade(&image);
        let cancel = {
            let view = cx.new(fixture);
            view.update(cx, |v, _| {
                assert!(v.previews.get("timeline").is_none());
                v.previews.insert("timeline".into(), image);
                assert!(v.previews.get("timeline").is_some());
                v.preview_cancel.clone()
            })
        };
        cx.update(|_| {});
        cx.run_until_parked();
        assert!(weak.upgrade().is_none());
        assert!(cancel.load(std::sync::atomic::Ordering::Relaxed));
    }
}
