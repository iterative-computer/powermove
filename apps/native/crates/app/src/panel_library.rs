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
fn shelf_scale(width: f32) -> f32 {
    if !width.is_finite() || width <= 0. {
        0.5
    } else {
        (width / 1700.).clamp(0.4, 0.56)
    }
}
fn preview_bounds((w, h): (f32, f32)) -> (f32, f32) {
    (w.min(960.), h.min(720.))
}
fn cover_size(size: (f32, f32), scale: f32, max_width: f32) -> (f32, f32) {
    let (w, h) = preview_bounds(size);
    let fit = scale.min(420. / h).min(max_width / w);
    let next = if fit.is_finite() && fit > 0. {
        fit
    } else {
        scale
    };
    ((w * next).round(), (h * next).round())
}
fn pack_rows(widths: &[f32], width: f32, gap: f32) -> Vec<Vec<usize>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut used = 0.;
    for (i, w) in widths.iter().enumerate() {
        let next = if row.is_empty() { *w } else { used + gap + w };
        if !row.is_empty() && next > width {
            rows.push(std::mem::take(&mut row));
            used = *w;
        } else {
            used = next;
        }
        row.push(i);
    }
    if !row.is_empty() {
        rows.push(row);
    }
    rows
}
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
    search: Option<Entity<pm_shell::text::TextField>>,
    tokens: pm_design::Tokens,
    motion: pm_shell::motion::Motion<u64>,
    focus: std::collections::HashMap<String, FocusHandle>,
    focused: Option<String>,
    scroll: ScrollHandle,
    packing: std::cell::RefCell<std::collections::HashMap<String, Vec<Vec<usize>>>>,
    previews: std::collections::HashMap<String, std::sync::Arc<Image>>,
    preview_stamp: Vec<(String, u64)>,
    preview_task: Option<Task<()>>,
    preview_cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    #[cfg(test)]
    fixture: Option<(Vec<PanelCard>, Vec<WorkspaceCard>)>,
}
impl PanelLibrary {
    // Geometry is independent of scroll position. Keep the small packed-row
    // model until width/data changes, rather than repacking on every wheel frame.
    fn rows(&self, widths: &[f32], width: f32) -> Vec<Vec<usize>> {
        let key = format!("{width}:{widths:?}");
        let mut cache = self.packing.borrow_mut();
        if !cache.contains_key(&key) {
            if cache.len() >= 8 {
                cache.clear();
            }
            cache.insert(key.clone(), pack_rows(widths, width, 16.));
        }
        cache[&key].clone()
    }
    fn refresh_previews(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.as_ref().and_then(WeakEntity::upgrade) else {
            return;
        };
        let e = editor.read(cx);
        let mut stamp: Vec<_> = e
            .builtins
            .views
            .iter()
            .map(|(id, view)| {
                (
                    format!("{id}@{}", view.read(cx).publication.generation),
                    view.read(cx).shown(),
                )
            })
            .collect();
        if let Some(p) = &e.published {
            stamp.push((format!("document-{}", p.snapshot.doc_id), p.revision()));
        }
        if self.preview_task.is_some() && stamp == self.preview_stamp {
            return;
        }
        self.preview_stamp = stamp;
        let (panels, _) = cards(e, None);
        let kernel = e.kernel.clone();
        let project = e.session.as_ref().map(|s| s.project.clone());
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
                    images
                })
                .await;
            let _ = this.update(cx, |v, cx| {
                v.previews = images
                    .into_iter()
                    .map(|(id, png)| {
                        (
                            id,
                            std::sync::Arc::new(Image::from_bytes(ImageFormat::Png, png)),
                        )
                    })
                    .collect();
                cx.notify();
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
            search.update(cx, |f, cx| f.set_value("", cx));
        }
        cx.notify();
    }
    fn send(&self, command: String, cx: &mut Context<Self>) {
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
    fn panel_card(
        &self,
        card: &PanelCard,
        kit: &Kit,
        scale: f32,
        width: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = &kit.t;
        let c = &t.colors;
        let (w, h) = cover_size(panel_size(&card.id), scale, width);
        let key = kit::key(&format!("library-cover-{}", card.id));
        let hover = if self.focused.as_deref() == Some(&card.id) {
            1.
        } else {
            kit.hovered(key)
        };
        let command = format!("panel:activate:{}", card.id);
        let click = command.clone();
        let keyboard = command.clone();
        let label = format!(
            "{} {} in Workspace",
            if card.open { "Show" } else { "Add" },
            card.title
        );
        let owner = self
            .editor
            .as_ref()
            .and_then(WeakEntity::upgrade)
            .and_then(|e| {
                e.read(cx)
                    .extensions
                    .stores
                    .get(&card.id)
                    .map(|(id, _)| id.clone())
            });
        let dimmed = owner.as_ref().is_some_and(|id| {
            self.editor
                .as_ref()
                .and_then(WeakEntity::upgrade)
                .is_some_and(|e| {
                    e.read(cx)
                        .kernel
                        .state()
                        .extensions
                        .get(id)
                        .is_none_or(|entry| !entry.enabled || !entry.installed)
                })
        });
        let seed = owner.clone().unwrap_or_else(|| card.id.clone());
        let metadata = owner.as_ref().and_then(|id| {
            self.editor
                .as_ref()
                .and_then(WeakEntity::upgrade)
                .and_then(|e| {
                    let state = e.read(cx).kernel.state();
                    let entry = state.extensions.get(id)?;
                    let record = entry.source.get("hostRecord").unwrap_or(&entry.source);
                    let name = record
                        .get("name")
                        .and_then(pm_model::Value::as_str)
                        .unwrap_or(id);
                    let maker = record.get("maker").and_then(|m| {
                        m.get("handle")
                            .and_then(pm_model::Value::as_str)
                            .or_else(|| m.as_str())
                    });
                    let label = maker
                        .map_or_else(|| name.to_owned(), |maker| format!("{name} · by {maker}"));
                    let icon = entry
                        .source
                        .get("manifest")
                        .and_then(|m| m.get("icon"))
                        .and_then(pm_model::Value::as_str)
                        .map(str::to_owned);
                    Some((label, icon))
                })
        });
        let subline = metadata
            .as_ref()
            .map(|(label, _)| label.clone())
            .or_else(|| owner.clone());
        let manifest_icon = metadata.as_ref().and_then(|(_, icon)| icon.as_deref());
        let delete_command = owner
            .map(|id| format!("extensionArchive.{id}"))
            .or_else(|| {
                (card.origin == Origin::Yours).then(|| format!("library-delete:{}", card.id))
            });
        let keyboard_delete = delete_command.clone();
        let art = card.origin != Origin::BuiltIn;
        let (from, to) = crate::store_window::art(&seed);
        let cover = div()
            .id(SharedString::from(format!("library-panel-{}", card.id)))
            .track_focus(self.focus.get(&card.id).expect("cover focus"))
            .role(Role::Button)
            .aria_label(label.clone())
            .tab_index(0)
            .focus_visible_style(|s| {
                s.shadow(vec![BoxShadow {
                    color: rgb(c.focus).into(),
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: px(2.),
                    inset: true,
                }])
            })
            .on_key_down(cx.listener(move |v, e: &KeyDownEvent, _, cx| {
                if e.keystroke.key == "enter" {
                    v.send(keyboard.clone(), cx);
                    cx.stop_propagation();
                } else if matches!(e.keystroke.key.as_str(), "backspace" | "delete") {
                    if let Some(command) = &keyboard_delete {
                        v.send(command.clone(), cx);
                        cx.stop_propagation();
                    }
                }
            }))
            .opacity(if dimmed { 0.62 + hover * 0.18 } else { 1. })
            .relative()
            .w(px(w))
            .h(px(h))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(12.))
            .corner(t, t.radius.lg)
            .bg(rgb(c.panel))
            .active_style(|s| {
                if art {
                    s.bg(linear_gradient(
                        135.,
                        linear_color_stop(rgb(pm_design::color::mix(from, c.text, 0.04)), 0.),
                        linear_color_stop(rgb(pm_design::color::mix(to, c.text, 0.04)), 1.),
                    ))
                } else {
                    s.bg(rgb(pm_design::color::mix(c.panel, c.text, 0.04)))
                }
            })
            .elevated(
                t,
                if hover > 0.1 {
                    pm_design::Elevation::Menu
                } else {
                    pm_design::Elevation::Raised
                },
            )
            .when(art, |d| {
                d.bg(linear_gradient(
                    135.,
                    linear_color_stop(rgb(from), 0.),
                    linear_color_stop(rgb(to), 1.),
                ))
            })
            .child(kit.symbol(
                pm_shell::symbol::panel_symbol_with_manifest(&card.id, manifest_icon),
                26.,
                Weight::Regular,
                if art { 0xffffff } else { c.muted },
            ))
            .child(
                div()
                    .w(px(w * 0.85))
                    .truncate()
                    .text_center()
                    .text_role(t, Text::Body)
                    .font_weight(FontWeight(t.typography.medium_weight))
                    .text_color(if art {
                        kit::ink(0xffffff, 0.94)
                    } else {
                        rgb(c.secondary).into()
                    })
                    .child(card.title.clone()),
            )
            .children(self.previews.get(&card.id).map(|image| {
                let fade = if t.accessibility.reduce_motion {
                    1.
                } else {
                    kit.animate(
                        kit::key(&format!("preview-{}", card.id)),
                        1.,
                        Some(pm_design::motion::Preset::Show),
                    )
                };
                div()
                    .absolute()
                    .inset_0()
                    .overflow_hidden()
                    .corner(t, t.radius.lg)
                    .child(
                        img(ImageSource::Image(image.clone()))
                            .size_full()
                            .object_fit(ObjectFit::Fill)
                            .opacity(fade),
                    )
            }))
            .child(
                div()
                    .absolute()
                    .top(px(8.))
                    .right(px(8.))
                    .opacity(hover)
                    .flex()
                    .gap(px(4.))
                    .children(delete_command.map(|command| {
                        kit.destructive_icon_button(
                            format!("library-delete-{}", card.id),
                            "trash",
                            format!("Delete {}", card.title),
                        )
                        .bg(rgb(c.floating))
                        .on_click(cx.listener(move |v, _, _, cx| {
                            cx.stop_propagation();
                            v.send(command.clone(), cx)
                        }))
                    }))
                    .child(
                        kit.icon_button(
                            format!("library-add-{}", card.id),
                            if card.open { "eye" } else { "plus" },
                            label,
                            false,
                        )
                        .bg(rgb(c.floating))
                        .on_click(cx.listener(move |v, _, _, cx| {
                            cx.stop_propagation();
                            v.send(command.clone(), cx)
                        })),
                    ),
            )
            .on_click(cx.listener(move |v, _, _, cx| v.send(click.clone(), cx)));
        div()
            .flex_none()
            .w(px(w))
            .flex()
            .flex_col()
            .child(kit.track_hover(cover, key))
            .child(self.ledge(kit, card.title.clone(), subline, w))
            .into_any_element()
    }
    fn ledge(&self, kit: &Kit, title: String, detail: Option<String>, width: f32) -> Div {
        let t = &kit.t;
        div()
            .w(px(width))
            .h(px(42.))
            .pl(px(14.))
            .pr(px(10.))
            .flex()
            .flex_col()
            .justify_center()
            .corner(t, t.radius.lg)
            .bg(linear_gradient(
                180.,
                linear_color_stop(kit::ink(t.colors.text, 0.), 0.),
                linear_color_stop(
                    kit::ink(
                        t.colors.text,
                        if t.accessibility.increase_contrast {
                            t.translucency.selected
                        } else {
                            t.translucency.hover
                        },
                    ),
                    0.75,
                ),
            ))
            .child(
                div()
                    .truncate()
                    .text_role(t, Text::Body)
                    .font_weight(FontWeight(t.typography.medium_weight))
                    .child(title),
            )
            .children(detail.map(|s| {
                div()
                    .truncate()
                    .text_role(t, Text::Caption)
                    .text_color(rgb(if s == "Active" {
                        t.colors.accent_text
                    } else {
                        t.colors.muted
                    }))
                    .child(s)
            }))
    }
    fn workspace_card(
        &self,
        card: &WorkspaceCard,
        kit: &Kit,
        scale: f32,
        width: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = &kit.t;
        let c = &t.colors;
        let (w, h) = cover_size((400., 250.), scale, width);
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
        let names = pm_shell::layout::Panels::new(Vec::new());
        let mut map = div().size_full().flex().gap(px(4.)).p(px(6.));
        if let Some(layout) = layout {
            for dock in &layout.docks {
                let mut col = div().h_full().flex().flex_col().gap(px(4.));
                col = if dock.size.is_some() {
                    col.w(px(w * 0.22))
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
                            .items_center()
                            .justify_center()
                            .gap(px(4.))
                            .rounded(px((t.radius.lg - 6.).max(0.)))
                            .bg(kit::ink(c.text, t.translucency.hover))
                            .child(kit.symbol(
                                pm_shell::symbol::panel_symbol(id),
                                14.,
                                Weight::Regular,
                                c.secondary,
                            ))
                            .child(
                                div()
                                    .truncate()
                                    .text_role(t, Text::Caption)
                                    .text_color(rgb(c.secondary))
                                    .child(names.title(id).to_owned()),
                            ),
                    );
                }
                map = map.child(col);
            }
        }
        let activate = format!("workspace:{}", card.id);
        let keyboard_activate = activate.clone();
        let keyboard_delete = (!card.builtin).then(|| (card.id.clone(), card.name.clone()));
        let key = kit::key(&format!("workspace-cover-{}", card.id));
        let (id, name) = (card.id.clone(), card.name.clone());
        let cover = div()
            .id(SharedString::from(format!("library-workspace-{}", card.id)))
            .relative()
            .role(Role::Button)
            .track_focus(self.focus.get(&card.id).expect("workspace focus"))
            .aria_label(format!("Activate {}", card.name))
            .tab_index(0)
            .focus_visible_style(|s| {
                s.shadow(vec![BoxShadow {
                    color: rgb(c.focus).into(),
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: px(2.),
                    inset: true,
                }])
            })
            .on_key_down(cx.listener(move |v, event: &KeyDownEvent, _, cx| {
                match event.keystroke.key.as_str() {
                    "enter" => {
                        v.send(keyboard_activate.clone(), cx);
                        cx.stop_propagation();
                    }
                    "backspace" | "delete" => {
                        if let Some((id, name)) = &keyboard_delete {
                            if let Some(e) = v.editor.as_ref().and_then(WeakEntity::upgrade) {
                                e.update(cx, |v, cx| {
                                    v.confirm_workspace_delete(id.clone(), name.clone(), cx)
                                });
                            }
                            cx.stop_propagation();
                        }
                    }
                    _ => {}
                }
            }))
            .w(px(w))
            .h(px(h))
            .corner(t, t.radius.lg)
            .bg(rgb(c.panel))
            .elevated(t, pm_design::Elevation::Raised)
            .when(card.active, |d| {
                d.shadow(vec![BoxShadow {
                    color: rgb(c.accent).into(),
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: px(2.),
                    inset: true,
                }])
            })
            .child(map)
            .when(!card.builtin, |d| {
                d.child(
                    div()
                        .absolute()
                        .top(px(8.))
                        .right(px(8.))
                        .opacity(if self.focused.as_deref() == Some(&card.id) {
                            1.
                        } else {
                            kit.hovered(key)
                        })
                        .child(
                            kit.destructive_icon_button(
                                format!("workspace-delete-{}", card.id),
                                "trash",
                                format!("Delete {}", card.name),
                            )
                            .bg(rgb(c.floating))
                            .on_click(cx.listener(
                                move |v, _, _, cx| {
                                    cx.stop_propagation();
                                    if let Some(e) = v.editor.as_ref().and_then(WeakEntity::upgrade)
                                    {
                                        e.update(cx, |e, cx| {
                                            e.confirm_workspace_delete(id.clone(), name.clone(), cx)
                                        });
                                    }
                                },
                            )),
                        ),
                )
            })
            .on_click(cx.listener(move |v, _, _, cx| v.send(activate.clone(), cx)));
        div()
            .w(px(w))
            .flex_none()
            .flex()
            .flex_col()
            .child(kit.track_hover(cover, key))
            .child(self.ledge(
                kit,
                card.name.clone(),
                Some(if card.active {
                    "Active".into()
                } else {
                    format!("{} panels", card.panels)
                }),
                w,
            ))
            .into_any_element()
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
        let (ground, ground_alpha) = t.ground();
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
        let mut sidebar = div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(m.border))
            .w(px(m.settings_sidebar))
            .h_full()
            .pt(px(m.toolbar))
            .px(px(m.gap))
            .bg(pm_shell::view::tint(ground, ground_alpha))
            .child(kit.search(self.search.clone(), None).h(px(34.)));
        for page in Page::ALL {
            let heading = match page {
                Page::Panels(None) => Some("PANELS"),
                Page::Workspaces(None) => Some("WORKSPACES"),
                _ => None,
            };
            if let Some(heading) = heading {
                sidebar = sidebar.child(
                    div()
                        .pt(px(m.gap))
                        .px(px(m.gap))
                        .text_role(&t, Text::Caption)
                        .font_weight(FontWeight(ty.semibold_weight))
                        .letter_spacing(px(t.text(Text::Caption).size * 0.05))
                        .text_color(rgb(c.faint))
                        .child(heading),
                );
            }
            let count = match page {
                Page::Panels(o) => shown_panels(&panels, o, "").len(),
                Page::Workspaces(b) => shown_workspaces(&workspaces, b, "").len(),
            };
            if count == 0
                && matches!(
                    page,
                    Page::Panels(Some(Origin::Yours)) | Page::Workspaces(Some(false))
                )
            {
                continue;
            }
            let symbol = match page {
                Page::Panels(None) => "square.grid.2x2",
                Page::Panels(Some(Origin::BuiltIn)) | Page::Workspaces(Some(true)) => {
                    "square.stack"
                }
                Page::Panels(Some(Origin::Yours)) => "wand.and.stars",
                Page::Panels(Some(Origin::Store)) => "basket",
                Page::Workspaces(None) => "rectangle.3.group",
                Page::Workspaces(Some(false)) => "person.crop.circle",
            };
            sidebar = sidebar.child(
                kit.row(format!("library-page-{page:?}"), self.page == page)
                    .role(Role::Button)
                    .aria_label(format!("{}, {count}", page.title()))
                    .tab_index(0)
                    .child(kit.symbol(symbol, m.symbol_panel, Weight::Regular, c.muted))
                    .child(div().flex_1().child(page.title()))
                    .child(kit::shelf_count(&t, count))
                    .on_key_down(cx.listener(move |v, e: &KeyDownEvent, _, cx| {
                        if e.keystroke.key == "enter" {
                            v.set_page(page, cx);
                            cx.stop_propagation();
                        }
                    }))
                    .on_click(cx.listener(move |v, _, _, cx| v.set_page(page, cx))),
            );
        }
        sidebar = sidebar.child(div().flex_1()).child(
            kit.button(
                "library-store",
                "Browse the Store",
                kit::Button::Neutral,
                Some("basket"),
                None,
            )
            .on_click(cx.listener(|v, _, _, cx| v.send("store".into(), cx))),
        );
        let width =
            (f32::from(window.viewport_size().width) - m.settings_sidebar - 2. * (m.inset + m.gap))
                .max(200.);
        let scale = shelf_scale(width);
        let mut shelf = div()
            .id("library-shelf")
            .flex_none()
            .role(Role::List)
            .aria_label(self.page.title())
            .flex()
            .flex_col()
            .gap(px(30.));
        let empty = match self.page {
            Page::Panels(origin) => {
                let shown = shown_panels(&panels, origin, &self.query);
                for source in [Origin::BuiltIn, Origin::Yours, Origin::Store] {
                    if origin.is_some_and(|o| o != source) {
                        continue;
                    }
                    let books: Vec<_> = shown.iter().filter(|p| p.origin == source).collect();
                    if books.is_empty() && source != Origin::Store {
                        continue;
                    }
                    let mut section = div().flex_none().flex().flex_col().gap(px(12.));
                    if origin.is_none() {
                        section = section.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .text_role(&t, Text::Title)
                                .child(Page::Panels(Some(source)).title())
                                .child(kit::shelf_count(&t, books.len())),
                        );
                    }
                    let mut widths: Vec<_> = books
                        .iter()
                        .map(|p| cover_size(panel_size(&p.id), scale, width).0)
                        .collect();
                    if source == Origin::Store && self.query.trim().is_empty() {
                        widths.push(180.);
                    }
                    for row in self.rows(&widths, width) {
                        section = section.child(
                            div()
                                .flex_none()
                                .flex()
                                .items_end()
                                .gap(px(16.))
                                .mb(px(24.))
                                .children(row.into_iter().map(|i| {
                                    if i < books.len() {
                                        self.panel_card(books[i], &kit, scale, width, cx)
                                    } else {
                                        div()
                                            .flex_none()
                                            .w(px(180.))
                                            .flex()
                                            .flex_col()
                                            .child(
                                                kit.row("library-find-store", false)
                                                    .w_full()
                                                    .h(px(160.))
                                                    .flex_col()
                                                    .justify_center()
                                                    .bg(rgb(c.field))
                                                    .child(kit.symbol(
                                                        "basket",
                                                        26.,
                                                        Weight::Regular,
                                                        c.muted,
                                                    ))
                                                    .child(
                                                        div()
                                                            .text_center()
                                                            .child("Find panels in the Store"),
                                                    )
                                                    .on_click(cx.listener(|v, _, _, cx| {
                                                        v.send("store".into(), cx)
                                                    })),
                                            )
                                            .child(div().h(px(42.)))
                                            .into_any_element()
                                    }
                                })),
                        );
                    }
                    shelf = shelf.child(section);
                }
                shown.is_empty()
            }
            Page::Workspaces(builtin) => {
                let shown = shown_workspaces(&workspaces, builtin, &self.query);
                let widths = vec![cover_size((400., 250.), scale, width).0; shown.len()];
                for row in self.rows(&widths, width) {
                    shelf = shelf.child(
                        div().flex_none().flex().items_end().gap(px(16.)).children(
                            row.into_iter()
                                .map(|i| self.workspace_card(&shown[i], &kit, scale, width, cx)),
                        ),
                    );
                }
                shown.is_empty()
            }
        };
        let fade = if t.accessibility.reduce_motion {
            1.
        } else {
            kit.animate(
                kit::key("library-page-fade"),
                1.,
                Some(pm_design::motion::Preset::Show),
            )
        };
        let body = if empty {
            let (symbol, heading, detail) = if !self.query.trim().is_empty() {
                (
                    "magnifyingglass",
                    "No Results",
                    format!("Nothing matches “{}”.", self.query),
                )
            } else {
                match self.page {
                    Page::Panels(Some(Origin::Yours)) => (
                        "wand.and.stars",
                        "No Panels Yet",
                        "Panels you make with the agent appear here.".into(),
                    ),
                    Page::Panels(Some(Origin::Store)) => {
                        ("basket", "Nothing from the Store", String::new())
                    }
                    Page::Workspaces(Some(false)) => {
                        ("rectangle.3.group", "No Saved Workspaces", String::new())
                    }
                    _ => ("square.grid.2x2", "No Panels Yet", String::new()),
                }
            };
            kit.empty_state(symbol, heading, &detail)
                .when(
                    self.query.trim().is_empty()
                        && matches!(self.page, Page::Panels(Some(Origin::Store))),
                    |d| {
                        d.child(
                            kit.button(
                                "empty-store",
                                "Browse the Store",
                                kit::Button::Neutral,
                                Some("basket"),
                                None,
                            )
                            .on_click(cx.listener(|v, _, _, cx| v.send("store".into(), cx))),
                        )
                    },
                )
                .when(
                    self.query.trim().is_empty()
                        && matches!(self.page, Page::Workspaces(Some(false))),
                    |d| {
                        d.child(
                            kit.button(
                                "empty-save",
                                "Save Current as New",
                                kit::Button::Neutral,
                                Some("plus"),
                                None,
                            )
                            .on_click(cx.listener(|v, _, _, cx| v.send("layout:save".into(), cx))),
                        )
                    },
                )
                .flex_1()
                .into_any_element()
        } else {
            div()
                .id("library-body")
                .min_h(px(0.))
                .flex_1()
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .px(px(m.inset + m.gap))
                .pb(px(m.inset))
                .opacity(fade)
                .child(shelf)
                .into_any_element()
        };
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
                    .gap(px(m.inset))
                    .h(px(m.toolbar))
                    .px(px(m.inset + m.gap))
                    .child(
                        div()
                            .text_size(px(ty.heading))
                            .font_weight(FontWeight(ty.semibold_weight))
                            .child(self.page.title()),
                    )
                    .when(matches!(self.page, Page::Workspaces(_)), |d| {
                        d.child(
                            kit.button(
                                "library-save",
                                "Save Current as New",
                                kit::Button::Neutral,
                                Some("plus"),
                                None,
                            )
                            .on_click(cx.listener(|v, _, _, cx| v.send("layout:save".into(), cx))),
                        )
                    }),
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
            .window_min_size(Some(size(px(720.), px(400.))))
            .window_bounds(Some(WindowBounds::Windowed(Bounds::new(
                point(px(m.toolbar), px(m.toolbar)),
                size(
                    px(m.settings_width + m.settings_sidebar),
                    px(m.settings_height),
                ),
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
                let search = cx.new(|cx| {
                    let mut field = pm_shell::text::TextField::new(
                        pm_shell::text::TextKind::Primary,
                        "Search the Panel Library",
                        "",
                        tokens.clone(),
                        window,
                        cx,
                    );
                    field.set_placeholder("Search");
                    field
                });
                search.update(cx, |field, cx| field.begin_editing(cx));
                cx.subscribe(&search, |v: &mut PanelLibrary, _, event, cx| {
                    use pm_shell::text::TextEvent;
                    if let TextEvent::Changed(text)
                    | TextEvent::Committed(text)
                    | TextEvent::Submitted(text) = event
                    {
                        v.query = text.clone();
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
                    packing: Default::default(),
                    previews: Default::default(),
                    preview_stamp: Default::default(),
                    preview_task: None,
                    preview_cancel: Default::default(),
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
    use super::{cover_size, pack_rows, preview_bounds, shelf_scale};
    #[test]
    fn scale_clamps_and_handles_invalid_widths() {
        assert_eq!(shelf_scale(f32::NAN), 0.5);
        assert_eq!(shelf_scale(0.), 0.5);
        assert_eq!(shelf_scale(300.), 0.4);
        assert_eq!(shelf_scale(1700.), 0.56);
    }
    #[test]
    fn covers_crop_then_fit_without_changing_proportions() {
        assert_eq!(preview_bounds((2000., 1000.)), (960., 720.));
        assert_eq!(cover_size((960., 360.), 0.5, 240.), (240., 90.));
        assert_eq!(cover_size((320., 600.), 0.5, 500.), (160., 300.));
    }
    #[test]
    fn rows_pack_greedily_and_allow_oversized_books() {
        assert_eq!(
            pack_rows(&[100., 100., 100.], 220., 16.),
            vec![vec![0, 1], vec![2]]
        );
        assert_eq!(pack_rows(&[500., 40.], 200., 16.), vec![vec![0], vec![1]]);
        assert!(pack_rows(&[], 200., 16.).is_empty());
    }
}

#[cfg(test)]
mod capture_tests {
    use super::PanelLibrary;
    use gpui::{AppContext, HeadlessAppContext, px, size};
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
            for (w, h) in [(900., 600.), (1400., 860.)] {
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
                                query: String::new(),
                                search: None,
                                tokens,
                                motion: Default::default(),
                                focus: Default::default(),
                                focused: None,
                                scroll: ScrollHandle::new(),
                                packing: Default::default(),
                                previews: Default::default(),
                                preview_stamp: Default::default(),
                                preview_task: None,
                                preview_cancel: Default::default(),
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
            packing: Default::default(),
            previews: Default::default(),
            preview_stamp: Default::default(),
            preview_task: None,
            preview_cancel: Default::default(),
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
            let cache_size = view.read_with(&vcx, |v, _| v.packing.borrow().len());
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
            assert_eq!(
                cache_size,
                view.read_with(&vcx, |v, _| v.packing.borrow().len()),
                "scrolling cannot repack rows"
            );
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
        cx.run_until_parked();
        assert!(weak.upgrade().is_none());
        assert!(cancel.load(std::sync::atomic::Ordering::Relaxed));
    }
}
