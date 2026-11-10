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

/// What the library lists for editor `v`.
fn cards(v: &Controller) -> (Vec<PanelCard>, Vec<WorkspaceCard>) {
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
    let panels = panel_cards(
        &pm_shell::layout::Panels::new(contributed),
        &workspace::layout(&record).open_panels(),
        from_store,
    );
    let active = workspace::record_id(&record).map(str::to_owned);
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

pub struct PanelLibrary {
    editor: WeakEntity<Controller>,
    page: Page,
    query: String,
    search: Option<Entity<pm_shell::text::TextField>>,
    tokens: pm_design::Tokens,
}
impl PanelLibrary {
    fn set_page(&mut self, page: Page, cx: &mut Context<Self>) {
        self.page = page;
        cx.notify();
    }
    fn send(&self, command: String, cx: &mut Context<Self>) {
        if let Some(editor) = self.editor.upgrade() {
            editor.read(cx).host.borrow().send_command(&command, vec![]);
        }
    }
    fn panel_card(&self, card: &PanelCard, cx: &mut Context<Self>) -> AnyElement {
        let t = &self.tokens;
        let (c, m, ty) = (&t.colors, &t.metrics, &t.typography);
        let (label, glyph) = if card.open {
            (format!("Show {} in Workspace", card.title), "Show")
        } else {
            (format!("Add {} to Workspace", card.title), "Add")
        };
        let command = format!("panel:activate:{}", card.id);
        div()
            .id(SharedString::from(format!("library-panel-{}", card.id)))
            .role(Role::ListItem)
            .aria_label(card.title.clone())
            .flex()
            .flex_col()
            .gap(px(m.gap))
            .min_w(px(m.thumbnail_min))
            .flex_1()
            .p(px(m.gap))
            .rounded(px(m.control_radius))
            .bg(rgb(c.panel_secondary))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(m.gap))
                    .child(div().min_w_0().truncate().child(card.title.clone()))
                    .child(
                        div()
                            .id(SharedString::from(format!("library-panel-{}-add", card.id)))
                            .role(Role::Button)
                            .aria_label(label)
                            .flex_none()
                            .h(px(m.control))
                            .px(px(m.inset))
                            .flex()
                            .items_center()
                            .rounded(px(m.control_radius))
                            .bg(rgb(c.field))
                            .hover_style(|s| s.bg(rgb(c.hover)))
                            .text_size(px(ty.reading))
                            .child(glyph)
                            .on_click(cx.listener(move |v, _, _, cx| v.send(command.clone(), cx))),
                    ),
            )
            .child(
                div()
                    .text_size(px(ty.reading))
                    .text_color(rgb(c.secondary))
                    .child(if card.open { "In Workspace" } else { "" }),
            )
            .into_any_element()
    }
    fn workspace_card(&self, card: &WorkspaceCard, cx: &mut Context<Self>) -> AnyElement {
        let t = &self.tokens;
        let (c, m, ty) = (&t.colors, &t.metrics, &t.typography);
        let summary = if card.active {
            "Active".to_owned()
        } else if card.panels == 1 {
            "1 panel".to_owned()
        } else {
            format!("{} panels", card.panels)
        };
        let activate = format!("workspace:{}", card.id);
        let (id, name) = (card.id.clone(), card.name.clone());
        div()
            .id(SharedString::from(format!("library-workspace-{}", card.id)))
            .role(Role::ListItem)
            .aria_label(format!("{}, {summary}", card.name))
            .flex()
            .flex_col()
            .gap(px(m.gap))
            .min_w(px(m.thumbnail_min))
            .flex_1()
            .p(px(m.gap))
            .rounded(px(m.control_radius))
            .bg(rgb(c.panel_secondary))
            .hover_style(|s| s.bg(rgb(c.hover)))
            .child(div().truncate().child(card.name.clone()))
            .child(
                div()
                    .text_size(px(ty.reading))
                    .text_color(rgb(c.secondary))
                    .child(summary),
            )
            .on_click(cx.listener(move |v, _, _, cx| v.send(activate.clone(), cx)))
            .when(!card.builtin, |d| {
                // Delete, after S012's alert, as the Workspaces panel does.
                d.on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |v, _: &MouseDownEvent, _, cx| {
                        if let Some(editor) = v.editor.upgrade() {
                            let (id, name) = (id.clone(), name.clone());
                            editor.update(cx, |e, cx| e.confirm_workspace_delete(id, name, cx));
                        }
                    }),
                )
            })
            .into_any_element()
    }
}
impl Render for PanelLibrary {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.tokens.clone();
        let (c, m, ty) = (&t.colors, &t.metrics, &t.typography);
        let (ground, ground_alpha) = t.ground();
        let (line, line_alpha) = t.line();
        let (panels, workspaces) = self
            .editor
            .upgrade()
            .map(|e| cards(e.read(cx)))
            .unwrap_or_default();
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
            .border_r(px(m.border))
            .border_color(pm_shell::view::tint(line, line_alpha));
        for page in Page::ALL {
            let heading = match page {
                Page::Panels(None) => Some("Panels"),
                Page::Workspaces(None) => Some("Workspaces"),
                _ => None,
            };
            if let Some(heading) = heading {
                sidebar = sidebar.child(
                    div()
                        .pt(px(m.gap))
                        .px(px(m.gap))
                        .text_size(px(ty.minimum))
                        .text_color(rgb(c.secondary))
                        .child(heading),
                );
            }
            let label = match page {
                Page::Panels(None) => format!("{} {}", page.title(), panels.len()),
                Page::Workspaces(None) => format!("{} {}", page.title(), workspaces.len()),
                _ => page.title().to_owned(),
            };
            let selected = self.page == page;
            sidebar = sidebar.child(
                div()
                    .id(SharedString::from(format!("library-page-{page:?}")))
                    .role(Role::Button)
                    .aria_label(label.clone())
                    .flex()
                    .items_center()
                    .h(px(m.row))
                    .px(px(m.gap))
                    .rounded(px(m.control_radius))
                    .when(selected, |d| d.bg(rgb(c.selected_fill)))
                    .child(label)
                    .on_click(cx.listener(move |v, _, _, cx| v.set_page(page, cx))),
            );
        }
        let mut shelf = div()
            .id("library-shelf")
            .role(Role::List)
            .aria_label(self.page.title())
            .flex()
            .flex_wrap()
            .gap(px(m.gap));
        let empty = match self.page {
            Page::Panels(origin) => {
                let shown = shown_panels(&panels, origin, &self.query);
                for card in &shown {
                    shelf = shelf.child(self.panel_card(card, cx));
                }
                shown.is_empty()
            }
            Page::Workspaces(builtin) => {
                let shown = shown_workspaces(&workspaces, builtin, &self.query);
                for card in &shown {
                    shelf = shelf.child(self.workspace_card(card, cx));
                }
                shown.is_empty()
            }
        };
        let body = if empty {
            let (heading, line) = if self.query.trim().is_empty() {
                ("Nothing on This Shelf Yet", "")
            } else {
                ("No Results", "There’s nothing here.")
            };
            pm_shell::view::empty_state(&t, heading, line)
                .flex_1()
                .into_any_element()
        } else {
            div()
                .id("library-body")
                .flex_1()
                .overflow_y_scroll()
                .px(px(m.inset + m.gap))
                .pb(px(m.inset))
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
                    .children(
                        self.search
                            .clone()
                            .map(|field| div().w(px(m.settings_sidebar)).child(field)),
                    ),
            )
            .child(body);
        div()
            .size_full()
            .flex()
            .text_color(rgb(c.text))
            .text_size(px(ty.body))
            .font_family(pm_design::TEXT_FONT)
            .child(sidebar)
            .child(content)
    }
}

impl Controller {
    /// Window ▸ Panel Library, for this editor; brought forward when open.
    pub fn open_panel_library(&mut self, cx: &mut Context<Self>) {
        let me = cx.entity().downgrade();
        if let Some(window) = cx.windows().into_iter().find(|w| {
            w.downcast::<PanelLibrary>()
                .and_then(|l| l.read(cx).ok())
                .is_some_and(|l| l.editor == me)
        }) {
            let _ = window.update(cx, |_, w, _| w.activate());
            return;
        }
        let t = pm_shell::tokens();
        let (m, tokens) = (t.metrics, t.clone());
        let background = if t.translucent() {
            gpui::MacosWindowBackground::Transparent
        } else {
            gpui::MacosWindowBackground::Opaque
        };
        let editor = cx.entity();
        let _ = cx.open_window(
            WindowOptions::new()
                .macos_window_background(background)
                .titlebar(Some(TitlebarOptions {
                    title: Some(TITLE.into()),
                    appears_transparent: true,
                    ..Default::default()
                }))
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
                    // Follows the editor: panels, workspaces, what's open.
                    cx.observe(&editor, |_, _, cx| cx.notify()).detach();
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
                        editor: editor.downgrade(),
                        page: Page::Panels(None),
                        query: String::new(),
                        search: Some(search),
                        tokens,
                    }
                })
            },
        );
    }
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
