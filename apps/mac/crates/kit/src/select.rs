use crate::{
    field::{FieldState, field_api, measure},
    input::{InputEvent, TextInput},
};
use gpui::{prelude::*, *};
use std::{fmt::Display, rc::Rc};
#[derive(Clone, Debug)]
pub struct SelectOption<T> {
    pub value: T,
    pub label: SharedString,
    pub disabled: bool,
    pub group: Option<SharedString>,
    pub detail: Option<SharedString>,
    pub icon: Option<SharedString>,
    pub warning: bool,
    pub separator: bool,
    pub checked: Option<bool>,
}
impl<T> SelectOption<T> {
    pub fn new(value: T, label: impl Into<SharedString>) -> Self {
        Self {
            value,
            label: label.into(),
            disabled: false,
            group: None,
            detail: None,
            icon: None,
            warning: false,
            separator: false,
            checked: None,
        }
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    pub fn group(mut self, group: impl Into<SharedString>) -> Self {
        self.group = Some(group.into());
        self
    }
}
pub struct SelectField<T: Clone + PartialEq + Display + 'static> {
    pub state: FieldState<T>,
    pub options: Vec<SelectOption<T>>,
    pub segmented: bool,
    pub searchable: bool,
    pub font_preview: bool,
    open: bool,
    active: usize,
    query: String,
    visible: usize,
    search: Entity<TextInput>,
    bounds: Bounds<Pixels>,
    scroll: ScrollHandle,
    on_select: Option<Rc<dyn Fn(&T, &mut App) -> bool>>,
    menu: bool,
    header: Option<SharedString>,
    typeahead: String,
    navigating: bool,
    pub global_space_shortcut: bool,
    type_time: std::time::Instant,
}
pub type Segmented<T = String> = SelectField<T>;
pub type FontField = SelectField<String>;
pub type PopoverMenu = SelectField<usize>;
pub struct MenuItem {
    pub label: SharedString,
    pub detail: Option<SharedString>,
    pub icon: Option<SharedString>,
    pub warning: bool,
    pub checked: Option<bool>,
    pub disabled: bool,
    pub separator: bool,
    run: Rc<dyn Fn(&mut App)>,
}
impl MenuItem {
    pub fn new(label: impl Into<SharedString>, run: impl Fn(&mut App) + 'static) -> Self {
        Self {
            label: label.into(),
            detail: None,
            icon: None,
            warning: false,
            checked: None,
            disabled: false,
            separator: false,
            run: Rc::new(run),
        }
    }
    pub fn separator() -> Self {
        let mut i = Self::new("", |_| {});
        i.separator = true;
        i.disabled = true;
        i
    }
    pub fn detail(mut self, v: impl Into<SharedString>) -> Self {
        self.detail = Some(v.into());
        self
    }
    pub fn icon(mut self, v: impl Into<SharedString>) -> Self {
        self.icon = Some(v.into());
        self
    }
    pub fn warning(mut self, v: bool) -> Self {
        self.warning = v;
        self
    }
    pub fn checked(mut self, v: bool) -> Self {
        self.checked = Some(v);
        self
    }
    pub fn disabled(mut self, v: bool) -> Self {
        self.disabled = v;
        self
    }
}
impl SelectField<usize> {
    pub fn menu(
        label: impl Into<SharedString>,
        items: Vec<MenuItem>,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let items = Rc::new(items);
        let options = items
            .iter()
            .enumerate()
            .map(|(i, item)| SelectOption {
                value: i,
                label: item.label.clone(),
                detail: item.detail.clone(),
                icon: item.icon.clone(),
                warning: item.warning,
                checked: item.checked,
                separator: item.separator,
                disabled: item.disabled,
                group: None,
            })
            .collect();
        let mut menu = Self::new(usize::MAX, options, w, cx);
        menu.menu = true;
        menu.state.label = label.into();
        menu.on_select = Some(Rc::new(move |i, cx| {
            if let Some(item) = items.get(*i) {
                (item.run)(cx);
            }
            false
        }));
        menu
    }
    pub fn header(mut self, text: impl Into<SharedString>) -> Self {
        self.header = Some(text.into());
        self
    }
}
impl<T: Clone + PartialEq + Display + 'static> SelectField<T> {
    pub fn new(
        value: T,
        options: Vec<SelectOption<T>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let state = FieldState::new(value, "Choose", cx);
        let search = cx.new(|cx| TextInput::new("", "Search", window, cx));
        cx.subscribe_in(&search, window, |this, _, event, _, cx| {
            if let InputEvent::Change(query) = event {
                this.query = query.clone();
                this.navigating = false;
                this.active = 0;
                this.visible = 80;
                this.scroll.set_offset(point(px(0.), px(0.)));
                cx.notify();
            }
        })
        .detach();
        cx.observe_window_activation(window, |this, w, cx| {
            if !w.is_window_active() {
                this.open = false;
                cx.notify();
            }
        })
        .detach();
        Self {
            state,
            options,
            segmented: false,
            searchable: false,
            font_preview: false,
            open: false,
            active: 0,
            query: String::new(),
            visible: 80,
            search,
            bounds: Bounds::default(),
            scroll: ScrollHandle::new(),
            on_select: None,
            menu: false,
            header: None,
            typeahead: String::new(),
            navigating: false,
            global_space_shortcut: false,
            type_time: std::time::Instant::now(),
        }
    }
    field_api!(T);
    pub fn segmented(mut self, segmented: bool) -> Self {
        self.segmented = segmented;
        self
    }
    pub fn searchable(mut self, searchable: bool) -> Self {
        self.searchable = searchable;
        self
    }
    /// Return false for command-like options such as Import… to keep the bound value.
    pub fn on_select(mut self, f: impl Fn(&T, &mut App) -> bool + 'static) -> Self {
        self.on_select = Some(Rc::new(f));
        self
    }
    fn indices(&self) -> Vec<usize> {
        let query = self.query.trim().to_lowercase();
        self.options
            .iter()
            .enumerate()
            .filter(|(_, o)| query.is_empty() || o.label.to_lowercase().contains(&query))
            .map(|(i, _)| i)
            .collect()
    }
    fn close(&mut self, restore: bool, w: &mut Window, cx: &mut Context<Self>) {
        self.open = false;
        if restore {
            self.state.focus.focus(w, cx);
        }
        cx.notify();
    }
    fn show(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if self.state.disabled {
            return;
        }
        if self.open {
            self.close(true, w, cx);
            return;
        }
        self.open = true;
        self.navigating = false;
        self.query.clear();
        self.visible = 80;
        self.active = self
            .options
            .iter()
            .position(|o| o.value == self.state.current(cx))
            .or_else(|| self.options.iter().position(|o| !o.disabled))
            .unwrap_or(0);
        self.visible = self.visible.max((self.active / 80 + 1) * 80);
        if self.searchable {
            self.search.update(cx, |s, cx| {
                s.set_value("", cx);
                s.focus(false, w, cx);
            });
        } else {
            self.state.focus.focus(w, cx);
        }
        self.scroll.scroll_to_item(self.active);
        cx.notify();
    }
    fn choose(&mut self, index: usize, w: &mut Window, cx: &mut Context<Self>) {
        if self.state.disabled {
            return;
        }
        let Some(o) = self.options.get(index) else {
            return;
        };
        if o.disabled {
            return;
        }
        let value = o.value.clone();
        if self.on_select.as_ref().is_none_or(|f| f(&value, cx)) {
            self.state.once(value, cx);
        }
        self.close(true, w, cx);
    }
    fn key(&mut self, e: &KeyDownEvent, w: &mut Window, cx: &mut Context<Self>) {
        if self.state.disabled {
            return;
        }
        let key = e.keystroke.key.as_str();
        let indices = self.indices();
        if key == "escape" || key == "tab" {
            if self.open {
                self.close(true, w, cx);
                cx.stop_propagation();
            }
            return;
        }
        let searching = self.searchable && self.search.read(cx).focus_handle(cx).is_focused(w);
        if key == "space"
            && ((searching && self.open) || (!self.open && self.global_space_shortcut))
        {
            return;
        }
        if key == "enter" || key == "space" {
            if self.open {
                if let Some(i) = indices.get(if searching && !self.navigating {
                    0
                } else {
                    self.active
                }) {
                    self.choose(*i, w, cx);
                }
            } else if !self.segmented {
                self.show(w, cx);
            }
            cx.stop_propagation();
            return;
        }
        if !matches!(key, "left" | "right" | "up" | "down" | "home" | "end") {
            if !self.searchable
                && self.open
                && !e.keystroke.modifiers.platform
                && !e.keystroke.modifiers.control
            {
                if let Some(c) = &e.keystroke.key_char {
                    if self.type_time.elapsed() > std::time::Duration::from_millis(700) {
                        self.typeahead.clear();
                    }
                    self.type_time = std::time::Instant::now();
                    self.typeahead.push_str(&c.to_lowercase());
                    if let Some((i, _)) = self.options.iter().enumerate().find(|(_, o)| {
                        !o.disabled && o.label.to_lowercase().starts_with(&self.typeahead)
                    }) {
                        self.active = i;
                        self.visible = self.visible.max((i / 80 + 1) * 80);
                        self.scroll.scroll_to_item(i);
                    }
                    cx.stop_propagation();
                    cx.notify();
                }
            }
            return;
        }
        if self.searchable
            && self.open
            && matches!(key, "up" | "down")
            && !self.navigating
            && !indices.is_empty()
        {
            self.active = if key == "up" { indices.len() - 1 } else { 0 };
            self.navigating = true;
            self.visible = self.visible.max((self.active / 80 + 1) * 80);
            self.scroll.scroll_to_item(self.active);
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if self.segmented {
            if self.options.is_empty() {
                return;
            }
            let current = self
                .options
                .iter()
                .position(|o| o.value == self.state.current(cx))
                .unwrap_or(0);
            let n = self.options.len();
            let mut index = match key {
                "home" => 0,
                "end" => n - 1,
                "left" | "up" => (current + n - 1) % n,
                _ => (current + 1) % n,
            };
            for _ in 0..n {
                if !self.options[index].disabled {
                    self.choose(index, w, cx);
                    break;
                }
                index = if matches!(key, "left" | "up") {
                    (index + n - 1) % n
                } else {
                    (index + 1) % n
                };
            }
        } else {
            if !self.open {
                self.show(w, cx);
            } else if !indices.is_empty() {
                let n = indices.len();
                let direction = if matches!(key, "left" | "up") { -1 } else { 1 };
                let mut i = match key {
                    "home" => 0,
                    "end" => n - 1,
                    _ => (self.active as isize + direction).clamp(0, n as isize - 1) as usize,
                };
                for _ in 0..n {
                    if !self.options[indices[i]].disabled {
                        break;
                    }
                    let next = (i as isize + direction).clamp(0, n as isize - 1) as usize;
                    if next == i {
                        break;
                    }
                    i = next;
                }
                self.active = i;
                self.visible = self.visible.max((i / 80 + 1) * 80);
                self.scroll.scroll_to_item(i);
            }
        }
        cx.stop_propagation();
        cx.notify();
    }
}
impl SelectField<String> {
    pub fn font(
        value: impl Into<String>,
        fonts: Vec<String>,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let options = fonts
            .into_iter()
            .map(|f| SelectOption::new(f.clone(), f))
            .collect();
        let mut s = Self::new(value.into(), options, w, cx);
        s.searchable = true;
        s.font_preview = true;
        s.state.label = "Font".into();
        s
    }
}
impl<T: Clone + PartialEq + Display + 'static> Focusable for SelectField<T> {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.state.focus.clone()
    }
}
impl<T: Clone + PartialEq + Display + 'static> Render for SelectField<T> {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.state.theme(w);
        let value = self.state.current(cx);
        self.search
            .update(cx, |i, _| i.appearance = self.state.appearance);
        let label = if self.menu {
            self.state.label.clone()
        } else if self.state.mixed {
            "Mixed".into()
        } else {
            self.options
                .iter()
                .find(|o| o.value == value)
                .map(|o| o.label.clone())
                .unwrap_or_else(|| value.to_string().into())
        };
        let mut root = self
            .state
            .well(w)
            .relative()
            .role(if self.segmented {
                Role::TabList
            } else {
                Role::ComboBox
            })
            .aria_expanded(self.open)
            .capture_key_down(cx.listener(Self::key));
        if self.segmented {
            root = root
                .px(px(2.))
                .gap(px(2.))
                .children(self.options.iter().enumerate().map(|(i, o)| {
                    div()
                        .id(i)
                        .role(Role::Tab)
                        .aria_label(o.label.clone())
                        .aria_selected(!self.state.mixed && o.value == value)
                        .flex_1()
                        .flex()
                        .justify_center()
                        .items_center()
                        .h(px(self.state.size.height() - 4.))
                        .rounded(px((self.state.size.radius() - 2.).max(2.)))
                        .when(!self.state.mixed && o.value == value, |d| d.bg(t.ink_3))
                        .when(o.disabled, |d| d.opacity(0.5))
                        .child(o.label.clone())
                        .on_click(cx.listener(move |this, _, w, cx| this.choose(i, w, cx)))
                }));
        } else {
            root = root
                .justify_between()
                .on_click(cx.listener(|this, _, w, cx| this.show(w, cx)))
                .child(
                    div()
                        .overflow_hidden()
                        .text_ellipsis()
                        .when(self.font_preview, |d| d.font_family(label.clone()))
                        .child(label),
                )
                .child("⌄")
                .child(measure(cx.entity(), |this, b| this.bounds = b));
        }
        if self.open {
            let indices = self.indices();
            let mut list = div()
                .id("options")
                .role(if self.menu { Role::Menu } else { Role::ListBox })
                .max_h(px(320.))
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .on_scroll_wheel(cx.listener(|this, _, _, cx| {
                    this.visible += 80;
                    cx.notify();
                }));
            let mut group = None;
            for (position, index) in indices.iter().take(self.visible).copied().enumerate() {
                let o = &self.options[index];
                if o.separator {
                    list = list.child(div().h(px(1.)).my_1().bg(t.line));
                    continue;
                }

                if o.group.is_some() && o.group != group {
                    group = o.group.clone();
                    list = list.child(
                        div()
                            .px_2()
                            .py_1()
                            .text_color(t.text_3)
                            .child(group.clone().unwrap()),
                    );
                }
                list = list.child(
                    div()
                        .id(index)
                        .role(if self.menu {
                            Role::MenuItem
                        } else {
                            Role::ListBoxOption
                        })
                        .aria_label(o.label.clone())
                        .aria_selected(!self.state.mixed && o.value == value)
                        .when(position == self.active, |d| {
                            d.aria_active_descendant().bg(t.ink_2)
                        })
                        .flex()
                        .justify_between()
                        .items_center()
                        .min_h(px(28.))
                        .when(o.detail.is_some(), |d| d.py_1())
                        .px_2()
                        .rounded(px(4.))
                        .when(o.disabled, |d| d.opacity(0.5))
                        .when(self.font_preview, |d| d.font_family(o.label.clone()))
                        .hover(|d| d.bg(t.ink_2))
                        .when(o.warning, |d| d.text_color(t.warning))
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .items_center()
                                .children(o.icon.clone())
                                .child(div().flex().flex_col().child(o.label.clone()).children(
                                    o.detail.clone().map(|detail| {
                                        div().text_size(px(11.)).text_color(t.text_3).child(detail)
                                    }),
                                )),
                        )
                        .child(
                            if o.checked.unwrap_or(!self.state.mixed && o.value == value) {
                                "✓"
                            } else {
                                ""
                            },
                        )
                        .on_mouse_move(cx.listener(move |this, _, _, cx| {
                            this.active = position;
                            this.navigating = true;
                            cx.notify();
                        }))
                        .on_click(cx.listener(move |this, _, w, cx| this.choose(index, w, cx))),
                );
            }
            if indices.is_empty() {
                list = list.child(div().p_2().text_color(t.text_3).child("No matches"));
            }
            let popup = div()
                .id("menu")
                .role(Role::Dialog)
                .aria_label(self.state.label.clone())
                .w(self.bounds.size.width.max(px(248.)))
                .p_1()
                .rounded(px(10.))
                .bg(t.float)
                .border_1()
                .border_color(t.line_strong)
                .shadow_lg()
                .occlude()
                .text_color(t.text)
                .text_size(px(13.))
                .on_mouse_down_out(cx.listener(|this, _, w, cx| this.close(false, w, cx)))
                .when(self.searchable, |d| {
                    d.child(div().px_2().py_1().child(self.search.clone()))
                })
                .children(
                    self.header
                        .clone()
                        .map(|h| div().p_2().text_color(t.text_2).child(h)),
                )
                .child(list);
            root = root.child(
                deferred(
                    anchored()
                        .position(point(self.bounds.right(), self.bounds.bottom() + px(4.)))
                        .anchor(Anchor::TopRight)
                        .snap_to_window_with_margin(px(8.))
                        .child(popup),
                )
                .with_priority(100),
            );
        }
        root
    }
}
