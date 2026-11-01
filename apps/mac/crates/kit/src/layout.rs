//! Small composable layout primitives. All spacing is in logical macOS points.
use crate::theme::{Appearance, Theme};
use gpui::{prelude::*, *};
use std::{collections::HashMap, rc::Rc, time::Duration};
#[derive(IntoElement)]
pub struct Row {
    label: SharedString,
    children: Vec<AnyElement>,
    action: Option<AnyElement>,
    pair: bool,
    full_width: bool,
    appearance: Appearance,
    focus: Option<FocusHandle>,
}
impl Row {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            children: vec![],
            action: None,
            pair: false,
            full_width: false,
            appearance: Appearance::System,
            focus: None,
        }
    }
    pub fn pair(mut self, pair: bool) -> Self {
        self.pair = pair;
        self
    }
    pub fn full_width(mut self, full: bool) -> Self {
        self.full_width = full;
        self
    }
    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.action = Some(action.into_any_element());
        self
    }
    pub fn appearance(mut self, a: Appearance) -> Self {
        self.appearance = a;
        self
    }
    pub fn label_for(mut self, control: &impl Focusable, cx: &App) -> Self {
        self.focus = Some(control.focus_handle(cx));
        self
    }
}
impl ParentElement for Row {
    fn extend(&mut self, e: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(e);
    }
}
impl RenderOnce for Row {
    fn render(self, w: &mut Window, _: &mut App) -> impl IntoElement {
        let t = Theme::for_window(self.appearance, w);
        let focus = self.focus;
        div()
            .flex()
            .when(self.full_width, |d| d.flex_col())
            .items_center()
            .min_h(px(30.))
            .gap_2()
            .w_full()
            .child(
                div()
                    .id(self.label.clone())
                    .role(Role::Label)
                    .aria_label(self.label.clone())
                    .text_size(px(12.))
                    .text_color(t.label)
                    .flex_1()
                    .min_w(px(60.))
                    .child(self.label.clone())
                    .on_mouse_down(MouseButton::Left, move |_, w, cx| {
                        if let Some(focus) = &focus {
                            focus.focus(w, cx);
                        }
                    }),
            )
            .child(
                div()
                    .flex()
                    .gap_1()
                    .items_center()
                    .when(!self.full_width, |d| d.w(px(crate::theme::VALUE_W)))
                    .when(self.full_width, |d| d.w_full())
                    .when(self.pair, |d| d.flex_1())
                    .children(self.children)
                    .children(self.action),
            )
    }
}
#[derive(IntoElement)]
pub struct Section {
    title: SharedString,
    children: Vec<AnyElement>,
    action: Option<AnyElement>,
    appearance: Appearance,
}
impl Section {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            children: vec![],
            action: None,
            appearance: Appearance::System,
        }
    }
    pub fn action(mut self, a: impl IntoElement) -> Self {
        self.action = Some(a.into_any_element());
        self
    }
    pub fn appearance(mut self, a: Appearance) -> Self {
        self.appearance = a;
        self
    }
}
impl ParentElement for Section {
    fn extend(&mut self, e: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(e);
    }
}
impl RenderOnce for Section {
    fn render(self, w: &mut Window, _: &mut App) -> impl IntoElement {
        let t = Theme::for_window(self.appearance, w);
        div()
            .flex()
            .flex_col()
            .w_full()
            .gap_1()
            .pb_3()
            .child(
                div()
                    .id(self.title.clone())
                    .role(Role::Heading)
                    .aria_level(2)
                    .flex()
                    .justify_between()
                    .items_center()
                    .h(px(32.))
                    .border_b_1()
                    .border_color(t.section_line)
                    .text_color(t.text_2)
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(self.title)
                    .children(self.action),
            )
            .children(self.children)
    }
}
#[derive(IntoElement)]
pub struct ActionButton {
    id: ElementId,
    label: SharedString,
    action: Rc<dyn Fn(&mut Window, &mut App)>,
    appearance: Appearance,
    disabled: bool,
    selected: bool,
    scope: Option<FocusHandle>,
    accessible_label: Option<SharedString>,
}
impl ActionButton {
    pub fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            action: Rc::new(action),
            appearance: Appearance::System,
            disabled: false,
            selected: false,
            scope: None,
            accessible_label: None,
        }
    }
    pub fn appearance(mut self, a: Appearance) -> Self {
        self.appearance = a;
        self
    }
    pub fn disabled(mut self, v: bool) -> Self {
        self.disabled = v;
        self
    }
    pub fn in_focus_scope(mut self, focus: FocusHandle) -> Self {
        self.scope = Some(focus);
        self
    }
    pub fn accessible_label(mut self, label: impl Into<SharedString>) -> Self {
        self.accessible_label = Some(label.into());
        self
    }
    pub fn selected(mut self, v: bool) -> Self {
        self.selected = v;
        self
    }
}
impl RenderOnce for ActionButton {
    fn render(self, w: &mut Window, _: &mut App) -> impl IntoElement {
        let t = Theme::for_window(self.appearance, w);
        let action = self.action.clone();
        let disabled = self.disabled;
        let scope = self.scope;
        div()
            .id(self.id)
            .role(Role::Button)
            .aria_label(self.accessible_label.unwrap_or_else(|| self.label.clone()))
            .tab_index(if disabled { -1 } else { 0 })
            .focusable()
            .flex()
            .items_center()
            .justify_center()
            .h(px(24.))
            .px_2()
            .rounded(px(6.))
            .text_size(px(12.))
            .text_color(t.text)
            .bg(if self.selected { t.ink_3 } else { t.ink_1 })
            .when(disabled, |d| d.opacity(0.5))
            .focus(|d| d.border_1().border_color(t.accent))
            .hover(|d| d.bg(t.ink_2))
            .child(self.label)
            .on_click(move |_, w, cx| {
                if !disabled {
                    w.blur(cx);
                    if let Some(f) = &scope {
                        f.focus(w, cx);
                    }
                    action(w, cx);
                }
            })
            .on_key_down(move |e, w, cx| {
                if !disabled && matches!(e.keystroke.key.as_str(), "enter" | "space") {
                    (self.action)(w, cx);
                    cx.stop_propagation();
                }
            })
    }
}
#[derive(IntoElement)]
pub struct BackTitle {
    title: SharedString,
    action: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
    appearance: Appearance,
}
impl BackTitle {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            action: None,
            appearance: Appearance::System,
        }
    }
    pub fn on_back(mut self, f: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.action = Some(Rc::new(f));
        self
    }
    pub fn appearance(mut self, a: Appearance) -> Self {
        self.appearance = a;
        self
    }
}
impl RenderOnce for BackTitle {
    fn render(self, w: &mut Window, _: &mut App) -> impl IntoElement {
        let t = Theme::for_window(self.appearance, w);
        let action = self.action.clone();
        div()
            .id("back-title")
            .role(if action.is_some() {
                Role::Button
            } else {
                Role::Heading
            })
            .aria_label(if action.is_some() {
                format!("Back to {}", self.title)
            } else {
                self.title.to_string()
            })
            .aria_level(1)
            .when(action.is_some(), |d| {
                d.tab_index(0)
                    .focusable()
                    .focus(|d| d.border_1().border_color(t.accent))
            })
            .text_size(px(22.))
            .text_color(t.text)
            .font_weight(FontWeight::MEDIUM)
            .child(if action.is_some() {
                format!("‹ {}", self.title)
            } else {
                self.title.to_string()
            })
            .on_click(move |_, w, cx| {
                if let Some(f) = &action {
                    w.blur(cx);
                    f(w, cx);
                }
            })
            .on_key_down(move |e, w, cx| {
                if matches!(e.keystroke.key.as_str(), "enter" | "space") {
                    if let Some(f) = &self.action {
                        f(w, cx);
                        cx.stop_propagation();
                    }
                }
            })
    }
}
#[derive(Default)]
struct Remembered(HashMap<SharedString, bool>);
impl Global for Remembered {}
#[derive(IntoElement)]
pub struct Disclosure {
    id: SharedString,
    label: SharedString,
    children: Vec<AnyElement>,
    open: bool,
    count: Option<usize>,
    changed: bool,
    remember: bool,
    appearance: Appearance,
}
impl Disclosure {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            children: vec![],
            open: false,
            count: None,
            changed: false,
            remember: false,
            appearance: Appearance::System,
        }
    }
    pub fn open(mut self, v: bool) -> Self {
        self.open = v;
        self
    }
    pub fn count(mut self, v: usize) -> Self {
        self.count = Some(v);
        self
    }
    pub fn changed(mut self, v: bool) -> Self {
        self.changed = v;
        self
    }
    pub fn remember(mut self, v: bool) -> Self {
        self.remember = v;
        self
    }
    pub fn appearance(mut self, a: Appearance) -> Self {
        self.appearance = a;
        self
    }
}
impl ParentElement for Disclosure {
    fn extend(&mut self, e: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(e);
    }
}
impl RenderOnce for Disclosure {
    fn render(self, w: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = Theme::for_window(self.appearance, w);
        if !cx.has_global::<Remembered>() {
            cx.set_global(Remembered::default());
        }
        let initial = if self.remember {
            cx.global::<Remembered>()
                .0
                .get(&self.id)
                .copied()
                .unwrap_or(self.open)
        } else {
            self.open
        };
        let state = w.use_keyed_state(self.id.clone(), cx, |_, _| initial);
        let open = *state.read(cx);
        let key = self.id.clone();
        let remember = self.remember;
        let action = move |_: &mut Window, cx: &mut App| {
            state.update(cx, |open, cx| {
                *open = !*open;
                if remember {
                    let map = &mut cx.global_mut::<Remembered>().0;
                    if map.len() > 1000 {
                        map.clear();
                    }
                    map.insert(key.clone(), *open);
                }
                cx.notify();
            });
        };
        div()
            .flex()
            .flex_col()
            .w_full()
            .child(
                div()
                    .id(self.id)
                    .role(Role::Button)
                    .aria_expanded(open)
                    .aria_label(self.label.clone())
                    .focusable()
                    .tab_index(0)
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(px(28.))
                    .text_size(px(12.))
                    .text_color(t.text_2)
                    .focus(|d| d.border_1().border_color(t.accent).rounded(px(6.)))
                    .child(if open { "⌄" } else { "›" })
                    .child(self.label)
                    .children(
                        self.count
                            .map(|n| div().text_color(t.text_3).child(n.to_string())),
                    )
                    .when(self.changed, |d| {
                        d.child(div().size(px(5.)).rounded_full().bg(t.accent))
                    })
                    .on_click({
                        let action = action.clone();
                        move |_, w, cx| {
                            w.blur(cx);
                            action(w, cx);
                        }
                    })
                    .on_key_down(move |e, w, cx| {
                        if matches!(e.keystroke.key.as_str(), "enter" | "space") {
                            action(w, cx);
                            cx.stop_propagation();
                        }
                    }),
            )
            .when(open, |d| d.children(self.children))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyframeState {
    Unanimated,
    Animated,
    AtPlayhead,
    Expression,
}
#[derive(IntoElement)]
pub struct KeyframeButton {
    state: KeyframeState,
    button: ActionButton,
}
impl KeyframeButton {
    pub fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        state: KeyframeState,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            state,
            button: ActionButton::new(id, label, action),
        }
    }
    pub fn disabled(mut self, v: bool) -> Self {
        self.button = self.button.disabled(v);
        self
    }
    pub fn appearance(mut self, a: Appearance) -> Self {
        self.button = self.button.appearance(a);
        self
    }
}
impl RenderOnce for KeyframeButton {
    fn render(self, _w: &mut Window, _cx: &mut App) -> impl IntoElement {
        let label = self.button.label.clone();
        let mut button = self.button;
        button.accessible_label = Some(label.clone());
        button.label = match self.state {
            KeyframeState::AtPlayhead => "◆",
            KeyframeState::Expression => "ƒ",
            _ => "◇",
        }
        .into();
        let active = matches!(
            self.state,
            KeyframeState::AtPlayhead | KeyframeState::Expression
        );
        div()
            .id("keyframe")
            .role(Role::Group)
            .aria_label(label)
            .child(button.selected(active))
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TipText {
    pub name: String,
    pub hint: String,
    pub key: String,
}
impl TipText {
    pub fn parse(text: &str, key: Option<&str>) -> Self {
        let mut rest = text.trim();
        let mut shortcut = key.unwrap_or("").to_string();
        if shortcut.is_empty() && rest.ends_with(')') {
            if let Some(i) = rest.rfind(" (") {
                let candidate = &rest[i + 2..rest.len() - 1];
                if candidate.chars().any(|c| "⌘⌥⇧⌃".contains(c))
                    || candidate.len() == 1
                    || ["Esc", "Space", "Tab", "Enter", "Home", "End", "Delete"]
                        .contains(&candidate)
                {
                    shortcut = candidate.trim().into();
                    rest = rest[..i].trim();
                }
            }
        }
        let (name, hint) = rest
            .split_once(" — ")
            .or_else(|| rest.split_once(" – "))
            .unwrap_or((rest, ""));
        Self {
            name: name.trim().into(),
            hint: hint.trim().into(),
            key: shortcut,
        }
    }
}
pub struct Tooltip {
    pub text: TipText,
    pub appearance: Appearance,
}
impl Tooltip {
    pub fn new(text: &str) -> Self {
        Self {
            text: TipText::parse(text, None),
            appearance: Appearance::System,
        }
    }
}
impl Render for Tooltip {
    fn render(&mut self, w: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::for_window(self.appearance, w);
        div()
            .id("tooltip")
            .role(Role::Tooltip)
            .px_2()
            .py_1()
            .max_w(px(260.))
            .rounded(px(7.))
            .bg(t.float)
            .text_size(px(12.))
            .text_color(t.text)
            .shadow_lg()
            .child(div().flex().gap_2().child(self.text.name.clone()).when(
                !self.text.key.is_empty(),
                |d| {
                    d.child(
                        div()
                            .px_1()
                            .rounded(px(4.))
                            .bg(t.ink_2)
                            .text_color(t.text_2)
                            .child(self.text.key.clone()),
                    )
                },
            ))
            .when(!self.text.hint.is_empty(), |d| {
                d.child(div().text_color(t.text_3).child(self.text.hint.clone()))
            })
    }
}
pub const TOOLTIP_DELAY: Duration = Duration::from_millis(380);
#[cfg(test)]
mod tests {
    use super::TipText;
    #[test]
    fn tooltip_parts() {
        assert_eq!(
            TipText::parse("Undo — Restore last edit (⌘Z)", None),
            TipText {
                name: "Undo".into(),
                hint: "Restore last edit".into(),
                key: "⌘Z".into()
            }
        );
        assert_eq!(TipText::parse("Zoom (F)", None).key, "F");
    }
}
