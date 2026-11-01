use crate::{
    layout::{self, Divider, Geometry, Layout, LayoutStore, Rect},
    native::{Command, Host},
};
use gpui::{prelude::*, *};
use pm_design::Tokens;
use std::{cell::RefCell, path::PathBuf, rc::Rc};
#[derive(Clone)]
enum Gesture {
    Resize(Divider, Point<Pixels>, Layout),
    Panel(String, Point<Pixels>, Layout),
}
pub struct Editor {
    pub layout: Layout,
    pub tokens: Tokens,
    pub host: Option<Rc<RefCell<Host>>>,
    geometry: Geometry,
    gesture: Option<Gesture>,
    focus: FocusHandle,
    pub status: String,
    store: LayoutStore,
    path: Option<PathBuf>,
    project: String,
    split_focus: Option<Divider>,
}
impl Editor {
    pub fn new(tokens: Tokens, host: Option<Rc<RefCell<Host>>>, cx: &mut Context<Self>) -> Self {
        let path = if host.is_none() {
            None
        } else {
            std::env::var_os("PM_NATIVE_SHELL_STATE")
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var_os("HOME").map(|h| {
                        PathBuf::from(h)
                            .join("Library/Application Support/Powermove Native/Shell/layouts.json")
                    })
                })
        };
        let store = path
            .as_deref()
            .filter(|p| p.exists())
            .map(LayoutStore::load)
            .transpose();
        let (store, status) = match store {
            Ok(s) => (s.unwrap_or_default(), "Ready".into()),
            Err(e) => (
                LayoutStore::default(),
                format!("Layout could not be restored: {e}"),
            ),
        };
        let project = "shell-untitled".to_string();
        let layout = store.get(
            &project,
            store.active.get(&project).map_or("design", String::as_str),
        );
        Self {
            layout,
            tokens,
            host,
            geometry: Geometry::default(),
            gesture: None,
            focus: cx.focus_handle(),
            status,
            store,
            path,
            project,
            split_focus: None,
        }
    }
    pub fn follow_appearance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if std::env::var_os("PM_NATIVE_APPEARANCE").is_none() {
            self.tokens = Tokens::new(
                match window.appearance() {
                    WindowAppearance::Dark | WindowAppearance::VibrantDark => {
                        pm_design::Scheme::Dark
                    }
                    _ => pm_design::Scheme::Light,
                },
                crate::native::settings(),
            );
            if let Some(h) = &self.host {
                h.borrow_mut().update_tokens(self.tokens.clone());
            }
            cx.notify();
        }
    }
    pub fn set_document_metadata(
        &mut self,
        title: &str,
        edited: bool,
        path: Option<&std::path::Path>,
    ) {
        if let Some(host) = &self.host {
            host.borrow().set_document_metadata(title, edited, path);
        }
    }
    pub fn bind_project(&mut self, project: String) {
        self.save();
        self.project = project;
        self.layout = self.store.get(
            &self.project,
            self.store
                .active
                .get(&self.project)
                .map_or("design", String::as_str),
        );
    }
    pub fn save(&mut self) {
        if let Some(path) = &self.path
            && path.exists()
        {
            match LayoutStore::load(path) {
                Ok(latest) => self.store = latest,
                Err(e) => {
                    self.status = format!("Layout could not be saved: {e}");
                    return;
                }
            }
        }
        self.store.put(&self.project, self.layout.clone());
        if let Some(p) = &self.path
            && let Err(e) = self.store.save(p)
        {
            self.status = format!("Layout could not be saved: {e}");
        }
    }
    pub fn command(&mut self, cmd: Command, window: &mut Window, cx: &mut Context<Self>) {
        if cmd.name == "shellReplayComplete" {
            if let (Some(token), Some(host)) = (cmd.token, &self.host) {
                host.borrow_mut().replayed(token);
            }
            return;
        }
        eprintln!(
            "shell command={} args={:?} token={:?}",
            cmd.name, cmd.args, cmd.token
        );
        let mut handled = true;
        match cmd.name.as_str() {
            "palette" => {
                if let Some(h) = &self.host {
                    h.borrow_mut().popup(false);
                }
            }
            "fieldMenu" => {
                if let Some(h) = &self.host {
                    h.borrow_mut().popup(true);
                }
            }
            "toggleSidebar" | "toggleInspector" => {
                let id = if cmd.name == "toggleSidebar" {
                    "left"
                } else {
                    "right"
                };
                if let Some(d) = self.layout.docks.iter_mut().find(|d| d.id == id) {
                    d.collapsed = !d.collapsed;
                    self.save();
                }
            }
            "shellAppearanceChanged" => {
                self.tokens = Tokens::new(self.tokens.scheme, crate::native::settings());
                if let Some(h) = &self.host {
                    h.borrow_mut().update_tokens(self.tokens.clone());
                }
            }
            "settings" => crate::open_settings(cx),
            "newWindow" => crate::open_editor(cx),
            "deselect" | "cancel" => {
                if let Some(g) = self.gesture.take() {
                    self.layout = match g {
                        Gesture::Resize(_, _, l) | Gesture::Panel(_, _, l) => l,
                    };
                } else {
                    handled = false;
                }
            }
            name if name.starts_with("workspace:") => {
                self.save();
                self.layout = self.store.get(&self.project, &name[10..]);
                self.save();
            }
            name => {
                handled = self.host.as_ref().is_some_and(|h| h.borrow().system(name));
                if !handled {
                    self.status = "Command is not available yet".into();
                }
            }
        }
        if let (Some(token), Some(host)) = (cmd.token, &self.host) {
            host.borrow_mut().resolve(token, handled);
        }
        let _ = window;
        cx.notify();
    }
}
fn positioned(r: Rect) -> Div {
    div()
        .absolute()
        .left(px(r.x))
        .top(px(r.y))
        .w(px(r.w))
        .h(px(r.h))
}
impl Render for Editor {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.tokens.clone();
        let m = t.metrics;
        let c = t.colors.clone();
        let viewport = w.viewport_size();
        let top = self
            .host
            .as_ref()
            .map_or(m.toolbar, |h| h.borrow().toolbar_inset());
        self.geometry = self.layout.geometry(
            Rect {
                x: m.gap,
                y: top + m.gap,
                w: (f32::from(viewport.width) - m.gap * 2.).max(0.),
                h: (f32::from(viewport.height) - top - m.status - m.gap * 2.).max(0.),
            },
            &t,
        );
        let viewer = self
            .geometry
            .panels
            .iter()
            .find(|p| self.layout.docks[p.dock].groups[p.group].active() == "viewer")
            .map(|p| Rect {
                x: p.rect.x + m.border,
                y: p.rect.y + m.header,
                w: (p.rect.w - m.border * 2.).max(0.),
                h: (p.rect.h - m.header - m.border).max(0.),
            });
        if let Some(h) = &self.host {
            h.borrow().viewer_rect.set(viewer);
        }
        let native = self.host.is_some();
        let canvas_viewer = viewer;
        let mut root = div()
            .id("editor")
            .role(Role::Group)
            .accessibility_id("shell.editor")
            .aria_label("Powermove editor")
            .size_full()
            .font_family(pm_design::TEXT_FONT)
            .text_size(px(t.typography.body))
            .text_color(rgb(c.text))
            .track_focus(&self.focus)
            .child(
                canvas(
                    move |_, _, _| (),
                    move |b, _, window, _| {
                        let (width, height) = (f32::from(b.size.width), f32::from(b.size.height));
                        let mut rects = vec![];
                        if let Some(v) = canvas_viewer.filter(|_| native) {
                            rects.extend([
                                Rect {
                                    x: 0.,
                                    y: 0.,
                                    w: width,
                                    h: v.y,
                                },
                                Rect {
                                    x: 0.,
                                    y: v.y,
                                    w: v.x,
                                    h: v.h,
                                },
                                Rect {
                                    x: v.x + v.w,
                                    y: v.y,
                                    w: width - v.x - v.w,
                                    h: v.h,
                                },
                                Rect {
                                    x: 0.,
                                    y: v.y + v.h,
                                    w: width,
                                    h: height - v.y - v.h,
                                },
                            ]);
                        } else {
                            rects.push(Rect {
                                x: 0.,
                                y: 0.,
                                w: width,
                                h: height,
                            });
                        }
                        for r in rects {
                            window.paint_quad(
                                PaintQuad::new(Bounds::new(
                                    point(px(r.x), px(r.y)),
                                    gpui::size(px(r.w), px(r.h)),
                                ))
                                .background(rgb(c.window)),
                            );
                        }
                    },
                )
                .absolute()
                .size_full(),
            )
            .on_mouse_move_all({
                let listener = cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                    if !e.dragging() {
                        return;
                    }
                    if let Some(Gesture::Resize(d, start, original)) = &this.gesture {
                        this.layout = original.clone();
                        let delta = if d.horizontal {
                            f32::from(e.position.y - start.y)
                        } else {
                            f32::from(e.position.x - start.x)
                        };
                        this.layout.resize(d, delta, &this.tokens);
                        cx.notify();
                    }
                });
                move |e, phase, _, w, cx| {
                    if phase == DispatchPhase::Bubble {
                        listener(e, w, cx);
                    }
                }
            })
            .on_mouse_up_all({
                let listener = cx.listener(|this, e: &MouseUpEvent, _, cx| {
                    if let Some(g) = this.gesture.take() {
                        if let Gesture::Panel(id, start, _) = g {
                            let dx = f32::from(e.position.x - start.x);
                            let dy = f32::from(e.position.y - start.y);
                            if dx.hypot(dy) >= this.tokens.metrics.drag_threshold
                                && let Some(p) = this.geometry.panels.iter().find(|p| {
                                    p.rect
                                        .contains(f32::from(e.position.x), f32::from(e.position.y))
                                })
                            {
                                let as_tab =
                                    f32::from(e.position.y) < p.rect.y + this.tokens.metrics.header;
                                let group = p.group
                                    + usize::from(
                                        !as_tab
                                            && f32::from(e.position.y) > p.rect.y + p.rect.h / 2.,
                                    );
                                if let Err(err) = this.layout.move_panel(&id, p.dock, group, as_tab)
                                {
                                    this.status = err.to_string();
                                }
                            }
                        }
                        this.save();
                        cx.notify();
                    }
                });
                move |e: &MouseUpEvent, phase, _, w, cx| {
                    if phase == DispatchPhase::Bubble && e.button == MouseButton::Left {
                        listener(e, w, cx);
                    }
                }
            })
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                if e.keystroke.key == "escape" {
                    if let Some(g) = this.gesture.take() {
                        this.layout = match g {
                            Gesture::Resize(_, _, l) | Gesture::Panel(_, _, l) => l,
                        };
                        cx.notify();
                    }
                    return;
                }
                if let Some(d) = this.split_focus.clone() {
                    let key = e.keystroke.key.as_str();
                    let delta = match key {
                        "left" | "up" => -this.tokens.metrics.keyboard_step,
                        "right" | "down" => this.tokens.metrics.keyboard_step,
                        "enter" => {
                            if d.group.is_none() {
                                let dock = &mut this.layout.docks[d.dock];
                                dock.collapsed = !dock.collapsed;
                            }
                            0.
                        }
                        _ => return,
                    };
                    this.layout.resize(&d, delta, &this.tokens);
                    this.save();
                    cx.notify();
                }
            }));
        if self.host.is_none() {
            root = root.child(
                positioned(Rect {
                    x: m.inset,
                    y: 0.,
                    w: f32::from(viewport.width) - m.inset * 2.,
                    h: m.toolbar,
                })
                .flex()
                .items_center()
                .child("Untitled · Edited")
                .child(div().ml_auto().child("Tools   ·   Agent   ·   Export")),
            );
        }
        for p in self.geometry.panels.clone() {
            let group = &self.layout.docks[p.dock].groups[p.group];
            let id = group.active().to_string();
            let is_viewer = id == "viewer";
            let tabs = group.tabs.clone();
            let active = group.active;
            let mut panel = positioned(p.rect)
                .id(format!("group-{}-{}", p.dock, p.group))
                .role(Role::Group)
                .accessibility_id(format!("shell.panel.{id}"))
                .aria_label(layout::panel_title(&id).to_string())
                .rounded(px(m.panel_radius))
                .border(px(m.border))
                .border_color(rgb(c.separator))
                .overflow_hidden();
            if !is_viewer || !native {
                panel = panel.bg(rgb(if is_viewer { c.viewer } else { c.panel }));
            }
            let mut header = div()
                .flex()
                .items_center()
                .gap(px(m.gap))
                .h(px(m.header))
                .px(px(m.inset))
                .bg(rgb(if is_viewer { c.viewer } else { c.panel }));
            for (ti, tab) in tabs.iter().enumerate() {
                let tabid = tab.clone();
                header = header.child(
                    div()
                        .id(format!("tab-{tab}"))
                        .role(Role::Tab)
                        .aria_label(layout::panel_title(tab).to_string())
                        .px(px(m.gap))
                        .py(px(m.gap))
                        .rounded(px(m.control_radius))
                        .bg(rgb(if ti == active && tabs.len() > 1 {
                            c.floating
                        } else if is_viewer {
                            c.viewer
                        } else {
                            c.panel
                        }))
                        .text_color(rgb(if ti == active { c.text } else { c.secondary }))
                        .cursor_pointer()
                        .child(layout::panel_title(tab).to_string())
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                this.layout.docks[p.dock].groups[p.group].active = ti;
                                this.gesture = Some(Gesture::Panel(
                                    tabid.clone(),
                                    e.position,
                                    this.layout.clone(),
                                ));
                                this.split_focus = None;
                                if let Some(h) = &this.host {
                                    h.borrow_mut().splitter_focus = false;
                                }
                                window.focus(&this.focus, cx);
                                this.save();
                                cx.notify();
                            }),
                        ),
                );
            }
            panel = panel.child(header);
            if !is_viewer {
                panel = panel.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .h(px((p.rect.h - m.header).max(0.)))
                        .text_color(rgb(c.muted))
                        .child(id.clone()),
                );
            } else {
                panel = panel.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .h(px((p.rect.h - m.header).max(0.)))
                        .text_color(rgb(c.muted))
                        .child("viewer"),
                );
            }
            root = root.child(panel);
        }
        for (i, d) in self.geometry.dividers.clone().into_iter().enumerate() {
            let divd = d.clone();
            root = root.child(
                positioned(d.rect)
                    .id(format!("splitter-{i}"))
                    .role(Role::Splitter)
                    .aria_label("Resize dock")
                    .cursor(if d.horizontal {
                        CursorStyle::ResizeUpDown
                    } else {
                        CursorStyle::ResizeLeftRight
                    })
                    .hover_style(|s| s.bg(rgb(c.separator)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                            this.split_focus = Some(divd.clone());
                            if let Some(h) = &this.host {
                                h.borrow_mut().splitter_focus = true;
                            }
                            this.gesture = Some(Gesture::Resize(
                                divd.clone(),
                                e.position,
                                this.layout.clone(),
                            ));
                            window.focus(&this.focus, cx);
                            cx.notify();
                        }),
                    ),
            );
        }
        root.child(
            positioned(Rect {
                x: 0.,
                y: f32::from(viewport.height) - m.status,
                w: f32::from(viewport.width),
                h: m.status,
            })
            .flex()
            .items_center()
            .px(px(m.inset))
            .text_size(px(t.typography.minimum))
            .text_color(rgb(c.muted))
            .child(self.status.clone())
            .child(div().ml_auto().child("Shell preview · Paused")),
        )
    }
}
