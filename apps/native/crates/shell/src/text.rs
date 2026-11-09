//! Native primary/multiline editors and a dense GPUI field with an AppKit edit slot.
use gpui::{prelude::*, *};
use gpui_macos::native_text::NativeTextSlot;
pub use gpui_macos::native_text::{TextEvent, TextKind};
use objc2::rc::Retained;
use objc2_app_kit::NSView;
use objc2_foundation::{NSPoint, NSRect, NSSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::rc::Rc;

pub struct TextField {
    slot: Rc<NativeTextSlot>,
    kind: TextKind,
    value: String,
    placeholder: SharedString,
    /// Drawn after the value in `muted` while not editing (`34 px`).
    unit: Option<SharedString>,
    tokens: pm_design::Tokens,
    /// A composer's last submitted text, cleared once the model empties it.
    submitted: Option<String>,
    /// The newest host replacement this field took (see
    /// [`Self::set_value_from_host`]).
    host_value: u64,
    /// A multiline editor that takes its container's whole height.
    fill: bool,
}
impl EventEmitter<TextEvent> for TextField {}
impl TextField {
    pub fn new(
        kind: TextKind,
        label: &str,
        value: &str,
        tokens: pm_design::Tokens,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let RawWindowHandle::AppKit(handle) =
            HasWindowHandle::window_handle(window).unwrap().as_raw()
        else {
            panic!("Mac-only text slot")
        };
        let view: Retained<NSView> =
            unsafe { Retained::retain(handle.ns_view.as_ptr().cast()).unwrap() };
        let slot = Rc::new(NativeTextSlot::new(&view, kind, label, value));
        slot.set_font_size(tokens.typography.body as f64);
        let (tx, rx) = async_channel::unbounded();
        slot.on_event(Rc::new(move || {
            let _ = tx.try_send(());
        }));
        cx.spawn(async move |this, cx| {
            while rx.recv().await.is_ok() {
                if this
                    .update(cx, |this, cx| {
                        for event in this.slot.take_events() {
                            match &event {
                                TextEvent::Committed(value) => this.value = value.clone(),
                                TextEvent::Submitted(value) => {
                                    this.submitted = Some(value.clone());
                                }
                                TextEvent::Changed(value) if this.kind == TextKind::Composer => {
                                    this.value = value.clone();
                                }
                                _ => {}
                            }
                            cx.emit(event);
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        Self {
            slot,
            kind,
            value: value.into(),
            placeholder: SharedString::default(),
            unit: None,
            tokens,
            submitted: None,
            host_value: 0,
            fill: false,
        }
    }
    /// The editor fills its container's height (a notes pane) instead of
    /// four rows.
    pub fn set_fill(&mut self, fill: bool) {
        self.fill = fill;
    }
    /// Muted hint shown while the field is empty (`metadata.placeholder`).
    pub fn set_unit(&mut self, unit: Option<SharedString>) {
        self.unit = unit;
    }
    pub fn set_placeholder(&mut self, text: &str) {
        if self.placeholder != text {
            self.slot.set_placeholder(text);
            self.placeholder = text.to_owned().into();
        }
    }
    pub fn keep(&self) {
        self.slot.keep();
    }
    pub fn accessibility_item(&self) -> Option<gpui_macos::composition::AccessibilityItem> {
        self.slot.accessibility_item()
    }
    pub fn value(&self) -> &str {
        &self.value
    }
    pub fn begin_editing(&mut self, cx: &mut Context<Self>) {
        self.slot.begin_editing();
        cx.notify();
    }
    /// Focused with all its text selected, so typing replaces it.
    pub fn begin_editing_selecting_all(&mut self, cx: &mut Context<Self>) {
        self.slot.begin_editing();
        self.slot.set_selection(0, usize::MAX);
        cx.notify();
    }
    pub fn set_tokens(&mut self, tokens: pm_design::Tokens) {
        self.slot.set_font_size(tokens.typography.body as f64);
        self.tokens = tokens;
    }
    /// The panel marks a value its host set (`metadata.hostValue`, rising
    /// with each one): that value replaces the text even while the person is
    /// editing, once. Anything else, such as a late answer to their own
    /// typing, never replaces what they typed.
    pub fn set_value_from_host(&mut self, value: &str, revision: u64, cx: &mut Context<Self>) {
        if revision > self.host_value {
            self.host_value = revision;
            if self.slot.is_editing() && self.slot.value() != value {
                self.slot.replace_value(value);
                self.value = value.into();
                cx.notify();
                return;
            }
        }
        self.set_value(value, cx);
    }
    pub fn set_value(&mut self, value: &str, cx: &mut Context<Self>) {
        // A sent message leaves the composer once the model has taken it,
        // unless the person already typed on.
        if value.is_empty()
            && let Some(sent) = self.submitted.take()
            && self.slot.value() == sent
        {
            self.slot.replace_value("");
            self.value.clear();
            cx.notify();
            return;
        }
        self.slot.set_value(value);
        if !self.slot.is_editing() {
            self.value = value.into();
            cx.notify();
        }
    }
}
impl Render for TextField {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let slot = self.slot.clone();
        let dense = matches!(self.kind, TextKind::DenseText | TextKind::DenseNumber);
        let editing = self.slot.is_editing();
        div()
            .id("native-text")
            .relative()
            .w_full()
            .when(self.fill, |d| d.h_full())
            .when(!self.fill, |d| {
                d.h(px(if self.kind == TextKind::Composer {
                    // Grows with its lines, up to the composer's limit.
                    let lines = (self.value.lines().count().max(1) as f32)
                        .min(self.tokens.metrics.composer_max_lines);
                    (lines * self.tokens.typography.body * 1.35 + 10.).max(self.tokens.metrics.hit)
                } else if self.kind == TextKind::Multiline {
                    self.tokens.metrics.row * 4.
                } else {
                    self.tokens.metrics.field
                }))
            })
            // A well with a unit trades side padding for the value's room.
            .px(px(if self.unit.is_some() {
                self.tokens.metrics.inner_radius
            } else {
                self.tokens.metrics.gap
            }))
            .border_1()
            .border_color(rgb(self.tokens.colors.separator))
            .rounded(px(self.tokens.metrics.control_radius))
            .bg(rgb(self.tokens.colors.field))
            .text_color(rgb(self.tokens.colors.text))
            .text_size(px(self.tokens.typography.body))
            .font_family(".AppleSystemUIFont")
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.slot.begin_editing();
                    cx.notify();
                }),
            )
            .flex()
            .items_center()
            .when(dense && !editing && self.value.is_empty(), |d| {
                d.text_color(rgb(self.tokens.colors.muted))
                    .child(self.placeholder.clone())
            })
            .when(dense && !editing && !self.value.is_empty(), |d| {
                d.overflow_hidden()
                    .child(div().min_w(px(0.)).truncate().child(self.value.clone()))
                    .children(self.unit.clone().map(|unit| {
                        div()
                            .flex_none()
                            .ml(px(self.tokens.metrics.inner_radius))
                            .text_color(rgb(self.tokens.colors.muted))
                            .child(unit)
                    }))
            })
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        let mask = window.content_mask().bounds;
                        let rect = |b: Bounds<Pixels>| {
                            NSRect::new(
                                NSPoint::new(
                                    f32::from(b.origin.x) as f64,
                                    f32::from(b.origin.y) as f64,
                                ),
                                NSSize::new(
                                    f32::from(b.size.width) as f64,
                                    f32::from(b.size.height) as f64,
                                ),
                            )
                        };
                        // Same content rectangle and token-sized native font in either mode.
                        slot.stage(rect(bounds), rect(mask));
                    },
                )
                .absolute()
                .inset_0(),
            )
    }
}
