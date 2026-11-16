//! The control kit: every GPUI control in the app is one of these recipes.
//!
//! A view owns a [`Motion<u64>`], steps it at the top of `render`, and builds
//! a [`Kit`] for the frame. Controls come back as elements the view attaches
//! its own `on_click` to. Rules the recipes hold for every surface:
//!
//! - Edges are hairline shadows (`Elevation`), never borders.
//! - The on state is the raised neutral surface or a neutral fill, never the accent.
//! - Hover is a spring wash that arrives fast and leaves slowly; press answers
//!   on pointer-down; a gliding selection keeps its velocity when retargeted.
//! - The cursor stays an arrow. Focus rings are neutral and keyboard-only.
use crate::{
    motion::{Motion, animate},
    symbol::{Weight, rasterize},
    text::TextField,
};
use gpui::{prelude::*, *};
use pm_design::{
    Elevation, Text, Tokens,
    motion::{Animated, Preset},
};
use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc};

/// A token colour at an alpha.
pub fn ink(rgb: u32, alpha: f32) -> Rgba {
    rgba((rgb << 8) | (alpha.clamp(0., 1.) * 255.).round() as u32)
}
/// A stable key for a control's animated state.
pub fn key(name: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut hasher);
    hasher.finish()
}
/// The GPUI shadows of an elevation.
pub fn shadows(t: &Tokens, elevation: Elevation) -> Vec<BoxShadow> {
    t.elevation(elevation)
        .into_iter()
        .map(|s| BoxShadow {
            color: ink(s.color, s.alpha).into(),
            offset: point(px(0.), px(s.y)),
            blur_radius: px(s.blur),
            spread_radius: px(s.spread),
            inset: s.inset,
        })
        .collect()
}
/// Token recipes as style calls.
pub trait Recipes: Styled + Sized {
    /// A role of the type scale: size, weight, tracking, leading, figures.
    fn text_role(self, t: &Tokens, role: Text) -> Self {
        let s = t.text(role);
        let mut me = self
            .text_size(px(s.size))
            .font_weight(FontWeight(s.weight))
            .line_height(px((s.size * s.leading).round()));
        if s.tracking != 0. {
            me = me.letter_spacing(px(s.tracking));
        }
        if s.tabular {
            me = me.font_features(FontFeatures(Arc::new(vec![("tnum".into(), 1)])));
        }
        me
    }
    fn elevated(self, t: &Tokens, elevation: Elevation) -> Self {
        self.shadow(shadows(t, elevation))
    }
    /// A corner of the radius scale; `lg` and larger surfaces are continuous.
    fn corner(self, t: &Tokens, radius: f32) -> Self {
        let me = self.rounded(px(radius));
        if radius >= t.radius.lg {
            me.rounded_smoothing(t.radius.smoothing)
        } else {
            me
        }
    }
}
impl<T: Styled> Recipes for T {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    /// The view's one accent action.
    Primary,
    /// The raised neutral push button.
    Neutral,
    /// Text or icon with a hover fill only.
    Quiet,
    /// A neutral button whose label is `danger`.
    Destructive,
}
/// A segment or tab: a symbol (with its accessibility label) or a title.
#[derive(Clone, Copy, Debug)]
pub enum Item<'a> {
    Symbol(&'a str, &'a str),
    Title(&'a str),
}
/// Reports a control (by key) hovered or left.
type Hover = Rc<dyn Fn(u64, bool, &mut App)>;

/// One frame's control builder over a view's tokens and motion.
pub struct Kit {
    pub t: Tokens,
    scale: f32,
    washes: HashMap<u64, f32>,
    values: Rc<RefCell<HashMap<u64, Animated<1>>>>,
    hover: Hover,
}
impl Kit {
    /// `motion` reaches the view's own [`Motion`]; hovers update it and
    /// repaint the view.
    pub fn new<V: 'static>(
        t: &Tokens,
        motion: &Motion<u64>,
        access: fn(&mut V) -> &mut Motion<u64>,
        window: &Window,
        cx: &mut Context<V>,
    ) -> Self {
        let (weak, tokens) = (cx.entity().downgrade(), t.clone());
        Self {
            t: t.clone(),
            scale: window.scale_factor(),
            washes: motion.washes(),
            values: motion.values.clone(),
            hover: Rc::new(move |key, hovered, cx| {
                let _ = weak.update(cx, |view, cx| {
                    access(view).hovers.hover(key, hovered, &tokens);
                    cx.notify();
                });
            }),
        }
    }
    /// An SF Symbol at one of the context sizes (`metrics.symbol_*`).
    pub fn symbol(&self, name: &str, points: f32, weight: Weight, color: u32) -> AnyElement {
        match rasterize(name, points, weight, color, self.scale) {
            Some(image) => img(ImageSource::Render(image))
                .flex_none()
                .size(px(points))
                .into_any_element(),
            None => div().flex_none().size(px(points)).into_any_element(),
        }
    }
    /// The hover wash and pressed fill of a control with corner `radius`.
    pub fn wash(&self, d: Stateful<Div>, key: u64, radius: f32) -> Stateful<Div> {
        let amount = self.washes.get(&key).copied().unwrap_or(0.);
        let (text, tr, hover) = (self.t.colors.text, self.t.translucency, self.hover.clone());
        d.relative()
            .when(amount > 0.004, |d| {
                d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded(px(radius))
                        .bg(ink(text, tr.hover * amount)),
                )
            })
            .on_hover(move |hovered, _, cx| hover(key, *hovered, cx))
    }
    /// How far the hover on `key` has come: 0 at rest, 1 under the pointer,
    /// on the wash's springs (fast in, slow out).
    pub fn hovered(&self, key: u64) -> f32 {
        self.washes.get(&key).copied().unwrap_or(0.)
    }
    /// Follows the pointer over `d` for `hovered`, drawing nothing itself.
    pub fn track_hover(&self, d: Stateful<Div>, key: u64) -> Stateful<Div> {
        let hover = self.hover.clone();
        d.on_hover(move |hovered, _, cx| hover(key, *hovered, cx))
    }
    /// A control the keyboard can reach. It draws no focus ring: owner
    /// decision (2026-11-14), a ring after a click read as a glitch, and
    /// buttons and toggles show no ring at all; fields keep their caret.
    fn focusable(&self, d: Stateful<Div>) -> Stateful<Div> {
        d
    }
    /// A neutral action toast. The owner places it at the window's bottom
    /// centre and attaches its Undo listener and hover lifetime handling.
    /// No focus handle is requested by this recipe.
    pub fn action_toast(
        &self,
        message: impl Into<SharedString>,
        undo: Stateful<Div>,
    ) -> Stateful<Div> {
        let t = &self.t;
        let message = message.into();
        div()
            .id("action-toast")
            .flex()
            .items_center()
            .gap(px(t.space.lg))
            .p(px(t.space.md))
            .bg(rgb(t.colors.floating))
            .text_color(rgb(t.colors.text))
            .text_role(t, Text::Label)
            .corner(t, t.radius.lg)
            .elevated(t, Elevation::Menu)
            .aria_label(message.clone())
            .child(message)
            .child(undo)
    }

    /// A push button: 28 pt, label in the medium weight, an optional leading
    /// symbol and trailing chevron (a pop-up).
    pub fn button(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        kind: Button,
        lead: Option<&str>,
        trail: Option<&str>,
    ) -> Stateful<Div> {
        let (t, id, label) = (&self.t, id.into(), label.into());
        let (c, m) = (&t.colors, &t.metrics);
        let k = key(&id);
        let fg = match kind {
            Button::Primary => c.selected_text,
            Button::Neutral => c.text,
            Button::Quiet => c.secondary,
            Button::Destructive => c.danger,
        };
        let pressed = t.translucency.pressed;
        let d = div()
            .id(id)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(px(t.space.xs))
            .h(px(m.control))
            .px(px(if kind == Button::Quiet {
                t.space.md
            } else {
                t.space.lg
            }))
            .rounded(px(m.control_radius))
            .text_role(t, Text::Label)
            .text_color(rgb(fg))
            .role(Role::Button)
            .aria_label(label.clone())
            .children(lead.map(|s| self.symbol(s, m.symbol_inline, Weight::Medium, fg)))
            .child(label)
            .children(trail.map(|s| self.symbol(s, m.symbol_inline - 2., Weight::Medium, c.muted)));
        let d = match kind {
            Button::Primary => {
                let (accent, edge) = (c.accent, pm_design::mix(c.accent, 0x000000, 0.28));
                d.bg(rgb(accent))
                    .shadow(vec![
                        BoxShadow {
                            color: ink(0xffffff, 0.24).into(),
                            offset: point(px(0.), px(0.5)),
                            blur_radius: px(0.),
                            spread_radius: px(0.),
                            inset: true,
                        },
                        BoxShadow {
                            color: ink(edge, 0.9).into(),
                            offset: point(px(0.), px(0.)),
                            blur_radius: px(0.),
                            spread_radius: px(0.5),
                            inset: false,
                        },
                        BoxShadow {
                            color: ink(edge, 0.35).into(),
                            offset: point(px(0.), px(1.)),
                            blur_radius: px(2.),
                            spread_radius: px(0.),
                            inset: false,
                        },
                    ])
                    .active_style(move |s| s.bg(rgb(pm_design::mix(accent, 0x000000, 0.12))))
            }
            Button::Neutral | Button::Destructive => {
                let (raised, text) = (c.raised, c.text);
                d.bg(rgb(raised))
                    .elevated(t, Elevation::Raised)
                    .active_style(move |s| s.bg(rgb(pm_design::mix(raised, text, pressed))))
            }
            Button::Quiet => {
                let text = c.text;
                d.active_style(move |s| s.bg(ink(text, pressed)))
            }
        };
        self.focusable(self.wash(d, k, m.control_radius))
    }
    /// A square icon button; `selected` is the raised neutral on state.
    pub fn icon_button(
        &self,
        id: impl Into<SharedString>,
        symbol: &str,
        label: impl Into<SharedString>,
        selected: bool,
    ) -> Stateful<Div> {
        self.icon_button_tinted(id, symbol, label, selected, None)
    }
    /// Destructive actions share the icon-button recipe, with danger ink on hover.
    pub fn destructive_icon_button(
        &self,
        id: impl Into<SharedString>,
        symbol: &str,
        label: impl Into<SharedString>,
    ) -> Stateful<Div> {
        self.icon_button_tinted(id, symbol, label, false, Some(self.t.colors.danger))
    }
    fn icon_button_tinted(
        &self,
        id: impl Into<SharedString>,
        symbol: &str,
        label: impl Into<SharedString>,
        selected: bool,
        hover_ink: Option<u32>,
    ) -> Stateful<Div> {
        let (t, id, label) = (&self.t, id.into(), label.into());
        let (c, m) = (&t.colors, &t.metrics);
        let k = key(&id);
        let (text, pressed) = (c.text, t.translucency.pressed);
        let d = div()
            .id(id)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(m.control))
            .rounded(px(m.control_radius))
            .role(Role::Button)
            .aria_label(label.clone())
            .aria_selected(selected)
            .tooltip(tooltip(t, label))
            .child(self.symbol(
                symbol,
                m.symbol_panel,
                Weight::Regular,
                if selected {
                    c.text
                } else if self.hovered(k) > 0.01 {
                    hover_ink.unwrap_or(c.secondary)
                } else {
                    c.secondary
                },
            ))
            .when(selected, |d| {
                d.bg(rgb(c.raised)).elevated(t, Elevation::Raised)
            })
            .when(!selected, |d| {
                d.active_style(move |s| s.bg(ink(text, pressed)))
            });
        let d = if selected {
            d
        } else {
            self.wash(d, k, m.control_radius)
        };
        self.focusable(d)
    }
    /// A search field: a soft fill with no edge, a magnifier, the native
    /// field, and a right-aligned shortcut hint.
    pub fn search(&self, field: Option<Entity<TextField>>, hint: Option<&str>) -> Div {
        let t = &self.t;
        let missing = field.is_none();
        let (c, m) = (&t.colors, &t.metrics);
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(t.space.sm))
            .h(px(m.control))
            .pl(px(t.space.md))
            .pr(px(t.space.md))
            .rounded(px(m.control_radius))
            .bg(ink(c.text, t.translucency.hover))
            .child(self.symbol("magnifyingglass", m.symbol_inline, Weight::Medium, c.muted))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .children(field)
                    .when(missing, |d| {
                        d.child(
                            div()
                                .text_role(t, Text::Body)
                                .text_color(rgb(c.faint))
                                .child("Search"),
                        )
                    }),
            )
            .children(hint.map(|hint| {
                div()
                    .flex_none()
                    .text_role(t, Text::Caption)
                    .text_color(rgb(c.muted))
                    .child(hint.to_string())
            }))
    }
    /// The drawn switch, for places a real view per control cannot work (a
    /// long scrolling list). Everywhere else host the real `NSSwitch`
    /// (`native_control::Kind::Switch`). It keeps the system switch's shape on
    /// macOS 26 (54×28 at regular size: a wide capsule knob in a capsule
    /// track) at the small size, and its knob slides on the layout spring.
    pub fn switch(&self, id: impl Into<SharedString>, on: bool) -> Stateful<Div> {
        let (t, id) = (&self.t, id.into());
        let c = &t.colors;
        let k = key(&id);
        let (w, h, pad): (f32, f32, f32) = (36., 19., 2.);
        let at = animate(&self.values, k, f32::from(on), t.spring(Preset::Layout)).clamp(-0.1, 1.1);
        let values = self.values.clone();
        // The system knob is a capsule about 0.6 of the track wide.
        let (knob, knob_w) = (h - 2. * pad, (w * 0.6).round());
        self.focusable(
            div()
                .id(id)
                .relative()
                .flex_none()
                .w(px(w))
                .h(px(h))
                .rounded_full()
                .bg(rgb(pm_design::mix(
                    pm_design::mix(c.panel, c.text, 0.18),
                    c.text,
                    at.clamp(0., 1.),
                )))
                .role(Role::Switch)
                .aria_toggled(if on { Toggled::True } else { Toggled::False })
                .child(
                    div()
                        .absolute()
                        .top(px(pad))
                        .left(px(pad + at * (w - knob_w - 2. * pad)))
                        .w(px(knob_w))
                        .h(px(knob))
                        .rounded_full()
                        .bg(rgb(if on { c.panel } else { 0xffffff }))
                        .shadow(vec![BoxShadow {
                            color: ink(0x000000, 0.22).into(),
                            offset: point(px(0.), px(1.)),
                            blur_radius: px(2.),
                            spread_radius: px(0.),
                            inset: false,
                        }]),
                )
                .child(
                    canvas(
                        |_, _, _| (),
                        move |_, _, window, _| {
                            if values.borrow().get(&k).is_some_and(Animated::moving) {
                                window.request_animation_frame();
                            }
                        },
                    )
                    .absolute()
                    .size_full(),
                ),
        )
    }
    /// A checkbox (16 pt, 4 pt corner) or, with `round`, a radio button. On
    /// is a `text` ink fill with the mark in the panel colour.
    pub fn check(&self, id: impl Into<SharedString>, on: bool, round: bool) -> Stateful<Div> {
        let (t, id) = (&self.t, id.into());
        let c = &t.colors;
        let size = 16.;
        let d = div()
            .id(id)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(size))
            .rounded(px(if round { size / 2. } else { t.radius.xs }))
            .role(if round {
                Role::RadioButton
            } else {
                Role::CheckBox
            })
            .aria_toggled(if on { Toggled::True } else { Toggled::False });
        let d = if on {
            d.bg(rgb(c.text)).child(if round {
                div()
                    .size(px(6.))
                    .rounded_full()
                    .bg(rgb(c.panel))
                    .into_any_element()
            } else {
                self.symbol("checkmark", 10., Weight::Semibold, c.panel)
            })
        } else {
            d.bg(ink(c.text, t.translucency.hover))
                .shadow(vec![BoxShadow {
                    color: ink(c.text, 0.28).into(),
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: px(0.5),
                    inset: true,
                }])
        };
        self.focusable(d)
    }
    /// A slider at `value` (0–1): a hairline track, the travelled part in
    /// `text` ink, a raised round knob. The view handles the drag.
    pub fn slider(&self, id: impl Into<SharedString>, value: f32) -> Stateful<Div> {
        let (t, c) = (&self.t, &self.t.colors);
        let (knob, value) = (14., value.clamp(0., 1.));
        div()
            .id(id.into())
            .relative()
            .flex()
            .items_center()
            .h(px(t.metrics.field))
            .min_w(px(knob * 3.))
            .role(Role::Slider)
            .aria_numeric_value(f64::from(value))
            .child(
                div()
                    .w_full()
                    .h(px(4.))
                    .rounded_full()
                    .bg(ink(c.text, 0.14))
                    .child(
                        div()
                            .h_full()
                            .w(relative(value))
                            .rounded_full()
                            .bg(ink(c.text, 0.62)),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .left(relative(value))
                    .ml(px(-knob * value))
                    .size(px(knob))
                    .rounded_full()
                    .bg(rgb(if t.scheme == pm_design::Scheme::Dark {
                        c.text
                    } else {
                        c.raised
                    }))
                    .elevated(t, Elevation::Raised),
            )
    }
    /// A disclosure chevron for a section or tree row.
    pub fn disclosure(&self, open: bool) -> AnyElement {
        self.symbol(
            if open {
                "chevron.down"
            } else {
                "chevron.right"
            },
            self.t.metrics.symbol_inline - 2.,
            Weight::Semibold,
            self.t.colors.muted,
        )
    }
    /// A list or tree row: quiet text, a neutral fill when selected, no
    /// divider, glyph tile or count unless the caller's data needs one.
    pub fn row(&self, id: impl Into<SharedString>, selected: bool) -> Stateful<Div> {
        let (t, id) = (&self.t, id.into());
        let (c, m) = (&t.colors, &t.metrics);
        let k = key(&id);
        let d = div()
            .id(id)
            .flex()
            .flex_none()
            .items_center()
            .gap(px(t.space.md))
            .h(px(m.row))
            .px(px(t.space.md))
            .rounded(px(m.inner_radius))
            .text_role(t, Text::Body)
            .text_color(rgb(if selected { c.text } else { c.secondary }))
            .aria_selected(selected)
            .when(selected, |d| d.bg(ink(c.text, t.translucency.selected)));
        if selected {
            d
        } else {
            self.wash(d, k, m.inner_radius)
        }
    }
    /// The surface of a menu or popover drawn inside a window: `floating`
    /// with the menu elevation. (A popover with its own window is framed in
    /// glass by `popover_frame` and stays clear.)
    pub fn menu(&self) -> Div {
        let t = &self.t;
        div()
            .flex()
            .flex_col()
            .relative()
            .p(px(t.space.xs))
            .corner(t, t.radius.lg)
            .bg(rgb(t.colors.floating))
            .elevated(t, Elevation::Menu)
            .text_role(t, Text::Body)
            .text_color(rgb(t.colors.text))
            .occlude()
    }
    /// A menu's one highlight, the first child of [`Kit::menu`]: `top` is the
    /// lit row's offset from the first row, `None` while nothing is lit (as
    /// the menu opens, and once the pointer has left). It appears on the row
    /// it lands on, glides between rows on the layout spring and fades out;
    /// the view keeps which row is lit in a `transient::Picker`.
    pub fn menu_light(&self, name: &str, top: Option<f32>) -> Div {
        let t = &self.t;
        let (lit, at) = (key(&format!("{name}.lit")), key(&format!("{name}.at")));
        let value = |k: u64| self.values.borrow().get(&k).map(|v| v.get());
        let before = value(lit).unwrap_or(0.);
        let fade = if top.is_some() {
            Preset::HoverIn
        } else {
            Preset::HoverOut
        };
        let shown = self
            .animate(lit, f32::from(top.is_some()), Some(fade))
            .clamp(0., 1.);
        let y = match top {
            // It travels only from a row that is lit.
            Some(top) => self.animate(at, top, (before > 0.05).then_some(Preset::Layout)),
            None => value(at).unwrap_or(0.),
        };
        div()
            .absolute()
            .left(px(t.space.xs))
            .right(px(t.space.xs))
            .top(px(t.space.xs + y))
            .h(px(t.metrics.row))
            .rounded(px(t.radius.nested(t.radius.lg, t.space.xs)))
            .bg(ink(t.colors.text, t.translucency.selected * shown))
    }
    /// A section heading in a menu: the caption role in the muted ink, at
    /// the rows' text inset.
    pub fn menu_heading(&self, label: impl Into<SharedString>) -> Div {
        let t = &self.t;
        div()
            .flex()
            .flex_none()
            .items_end()
            .h(px(t.metrics.row - t.space.xs))
            .px(px(t.space.sm))
            .pb(px(t.space.xs))
            .text_role(t, Text::Caption)
            .text_color(rgb(t.colors.muted))
            .child(label.into())
    }
    /// A menu row: 28 pt, concentric in the menu (`lg − xs`). It draws no
    /// highlight of its own: the menu's [`Kit::menu_light`] is lit behind
    /// the row the pointer or the keyboard is on. `checked` reserves the
    /// leading mark column.
    pub fn menu_row(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        checked: Option<bool>,
        shortcut: Option<&str>,
    ) -> Stateful<Div> {
        let (t, id, label) = (&self.t, id.into(), label.into());
        let (c, m) = (&t.colors, &t.metrics);
        div()
            .id(id)
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(t.space.sm))
            .h(px(m.row))
            .pl(px(t.space.sm))
            .pr(px(t.space.md))
            .role(Role::MenuItem)
            .aria_label(label.clone())
            .children(checked.map(|on| {
                div()
                    .flex()
                    .flex_none()
                    .justify_center()
                    .w(px(m.symbol_inline))
                    .when(on, |d| {
                        d.child(self.symbol(
                            "checkmark",
                            m.symbol_inline - 2.,
                            Weight::Semibold,
                            c.text,
                        ))
                    })
            }))
            .child(div().flex_1().min_w(px(0.)).truncate().child(label))
            .children(shortcut.map(|keys| {
                div()
                    .flex_none()
                    .text_role(t, Text::Caption)
                    .text_color(rgb(c.faint))
                    .child(keys.to_string())
            }))
    }
    /// Shared chrome for secondary windows. Content is inset 24 pt; callers
    /// supply rows, actions and content so commands stay owned by the view.
    pub fn utility_window(
        &self,
        title: &str,
        sidebar: Option<AnyElement>,
        actions: Vec<AnyElement>,
        content: AnyElement,
        footer: Vec<AnyElement>,
    ) -> Div {
        let t = &self.t;
        div()
            .size_full()
            .flex()
            .bg(rgb(t.colors.window))
            .text_color(rgb(t.colors.text))
            .text_role(t, Text::Body)
            .font_family(pm_design::TEXT_FONT)
            .children(sidebar)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(self.utility_toolbar(title, actions))
                    .child(self.utility_content().child(content))
                    .when(!footer.is_empty(), |d| {
                        d.child(self.utility_footer().children(footer))
                    }),
            )
    }
    pub fn utility_toolbar(&self, title: &str, actions: Vec<AnyElement>) -> Div {
        let t = &self.t;
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(t.space.md))
            .min_h(px(t.metrics.toolbar))
            .px(px(t.space.xxl))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_role(t, Text::Heading)
                    .child(title.to_owned()),
            )
            .children(actions)
    }
    pub fn utility_content(&self) -> Div {
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .p(px(self.t.space.xxl))
    }
    pub fn utility_sidebar(&self) -> Div {
        let t = &self.t;
        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(t.metrics.settings_sidebar))
            .h_full()
            .p(px(t.space.md))
            .gap(px(t.space.hair))
            .bg(rgb(t.colors.panel))
            .elevated(t, Elevation::Well)
    }
    pub fn utility_footer(&self) -> Div {
        let t = &self.t;
        div()
            .flex()
            .flex_none()
            .justify_end()
            .items_center()
            .gap(px(t.space.md))
            .px(px(t.space.xxl))
            .py(px(t.space.lg))
    }
    /// The same source-list row in Home and utility windows.
    pub fn nav_row(
        &self,
        id: impl Into<SharedString>,
        symbol: &str,
        label: String,
        count: Option<usize>,
        selected: bool,
    ) -> Stateful<Div> {
        let t = &self.t;
        self.row(id, selected)
            .text_size(px(t.typography.reading))
            .role(Role::Button)
            .aria_label(match count {
                Some(n) => format!("{label}, {n}"),
                None => label.clone(),
            })
            .child(self.symbol(
                symbol,
                t.metrics.symbol_panel,
                Weight::Regular,
                if selected {
                    t.colors.text
                } else {
                    t.colors.muted
                },
            ))
            .child(div().flex_1().min_w_0().truncate().child(label))
            .children(count.map(|n| {
                div()
                    .flex_none()
                    .text_role(t, Text::Caption)
                    .font_features(tabular())
                    .text_color(rgb(t.colors.muted))
                    .child(n.to_string())
            }))
    }
    pub fn vertical_separator(&self) -> Div {
        div()
            .flex_none()
            .w(px(1. / self.scale))
            .bg(ink(self.t.colors.text, self.t.translucency.hover))
    }
    /// A physical-pixel hairline, shared by grouped rows.
    pub fn separator(&self) -> Div {
        div()
            .flex_none()
            .h(px(1. / self.scale))
            .bg(ink(self.t.colors.text, self.t.translucency.hover))
    }
    /// A Settings-anatomy card: rows grouped on a soft fill, no outline.
    pub fn card(&self) -> Div {
        let t = &self.t;
        div()
            .flex()
            .flex_col()
            .corner(t, t.metrics.group_radius)
            .bg(ink(t.colors.text, t.translucency.hover))
    }
    /// A card row: title over description on the leading side, one
    /// natural-width control trailing.
    pub fn form_row(&self, title: &str, description: Option<&str>, control: AnyElement) -> Div {
        let t = &self.t;
        div()
            .flex()
            .items_center()
            .gap(px(t.space.xl))
            .min_h(px(t.metrics.form_row))
            .px(px(t.space.lg + t.space.hair))
            .py(px(t.space.md + t.space.hair))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .flex_col()
                    .gap(px(t.space.hair))
                    .child(
                        div()
                            .text_role(t, Text::Title)
                            .text_color(rgb(t.colors.text))
                            .child(title.to_string()),
                    )
                    .children(description.map(|line| {
                        div()
                            .text_role(t, Text::Body)
                            .text_color(rgb(t.colors.muted))
                            .child(line.to_string())
                    })),
            )
            .child(div().flex_none().child(control))
    }
    /// A determinate bar (`Some(0–1)`) or an indeterminate pulse.
    pub fn progress(&self, value: Option<f32>) -> Div {
        let c = &self.t.colors;
        let track = div()
            .w_full()
            .h(px(4.))
            .rounded_full()
            .overflow_hidden()
            .bg(ink(c.text, 0.12));
        match value {
            Some(value) => track.child(
                div()
                    .h_full()
                    .w(relative(value.clamp(0., 1.)))
                    .rounded_full()
                    .bg(rgb(c.accent)),
            ),
            None => track.child(
                div().size_full().bg(ink(c.text, 0.5)).with_animation(
                    "progress",
                    Animation::new(std::time::Duration::from_millis(1200))
                        .repeat()
                        .with_easing(pulsating_between(0.25, 0.9)),
                    |this, delta| this.opacity(delta),
                ),
            ),
        }
    }
    /// A centred empty state: one quiet glyph, a title, one line, and the
    /// caller may append a single action.
    pub fn empty_state(&self, symbol: &str, title: &str, detail: &str) -> Div {
        empty_state_at_scale(&self.t, self.scale, symbol, title, detail)
    }
    /// A toast or inline banner: a raised strip with a state glyph.
    pub fn banner(&self, symbol: &str, tint: u32, text: impl Into<SharedString>) -> Div {
        let t = &self.t;
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(t.space.md))
            .min_h(px(t.metrics.control + t.space.md))
            .px(px(t.space.lg))
            .corner(t, t.radius.lg)
            .bg(rgb(t.colors.floating))
            .elevated(t, Elevation::Menu)
            .text_role(t, Text::Body)
            .text_color(rgb(t.colors.text))
            .child(self.symbol(symbol, t.metrics.symbol_panel, Weight::Medium, tint))
            .child(text.into())
    }
}

struct Tip {
    text: SharedString,
    tokens: Tokens,
}
impl Render for Tip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let t = &self.tokens;
        // GPUI anchors a tooltip at the pointer: sit just below it.
        div().pt(px(t.space.md)).pl(px(t.space.xs)).child(
            div()
                .px(px(t.space.md))
                .py(px(t.space.xs))
                .rounded(px(t.radius.sm))
                .bg(rgb(t.colors.floating))
                .elevated(t, Elevation::Menu)
                .text_role(t, Text::Caption)
                .text_color(rgb(t.colors.text))
                .font_family(pm_design::TEXT_FONT)
                .child(self.text.clone()),
        )
    }
}
/// A tooltip builder naming a control's action.
pub fn tooltip(
    t: &Tokens,
    text: impl Into<SharedString>,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let (tokens, text) = (t.clone(), text.into());
    move |_, cx| {
        cx.new(|_| Tip {
            text: text.clone(),
            tokens: tokens.clone(),
        })
        .into()
    }
}
impl Kit {
    /// The number at `key` heading for `target` on `preset`, as it is now.
    /// `None` places it at once (a layout change rather than a move).
    pub fn animate(&self, key: u64, target: f32, preset: Option<Preset>) -> f32 {
        animate(
            &self.values,
            key,
            target,
            preset.map_or(pm_design::motion::Spring::SNAP, |p| self.t.spring(p)),
        )
    }
    /// Keeps frames coming while any [`Kit::animate`] value still moves. A view
    /// that animates values puts one of these in its tree.
    pub fn settle(&self) -> impl IntoElement + use<> {
        let values = self.values.clone();
        canvas(
            |_, _, _| (),
            move |_, _, window, _| {
                if values.borrow().values().any(Animated::moving) {
                    window.request_animation_frame();
                }
            },
        )
        .absolute()
        .size(px(0.))
    }
}
impl Kit {
    /// A control that cannot be used now: dimmed, with its events left off
    /// by the caller.
    pub fn disabled<E: Styled>(&self, element: E) -> E {
        element.opacity(0.45)
    }
}
/// Development builds: Home shows the kit gallery instead of the projects
/// (`--open reviewKit`).
#[cfg(debug_assertions)]
pub static GALLERY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Tabular figures for counts, sizes and times.
pub fn tabular() -> FontFeatures {
    FontFeatures(Arc::new(vec![("tnum".into(), 1)]))
}

/// Shared empty-state anatomy for callers without a frame's Kit.
pub fn empty_state(t: &Tokens, symbol: &str, title: &str, detail: &str) -> Div {
    empty_state_at_scale(t, 2., symbol, title, detail)
}
fn empty_symbol(scale: f32, name: &str, points: f32, weight: Weight, color: u32) -> AnyElement {
    match rasterize(name, points, weight, color, scale) {
        Some(image) => img(ImageSource::Render(image))
            .flex_none()
            .size(px(points))
            .into_any_element(),
        None => div().size(px(points)).into_any_element(),
    }
}
fn empty_state_at_scale(t: &Tokens, scale: f32, symbol: &str, title: &str, detail: &str) -> Div {
    let c = &t.colors;
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(t.space.xs))
        .child(div().mb(px(t.space.md)).child(empty_symbol(
            scale,
            symbol,
            28.,
            Weight::Regular,
            c.faint,
        )))
        .child(
            div()
                .text_role(t, Text::Heading)
                .text_color(rgb(c.text))
                .child(title.to_string()),
        )
        .child(
            div()
                .max_w(px(t.metrics.sheet_width - 2. * t.metrics.sheet_inset))
                .text_center()
                .text_role(t, Text::Reading)
                .text_color(rgb(c.muted))
                .child(detail.to_string()),
        )
}

