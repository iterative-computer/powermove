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
    /// A multiline editor that takes its container's whole height (Shader Lab,
    /// Notes) instead of four rows.
    fill: bool,
    /// No well of its own: the field sits inside a control that draws one
    /// (the search field).
    bare: bool,
    /// Source text, in this family: one line least, and the face is kept
    /// when the tokens change.
    code: Option<String>,
    /// The value's ink at rest, when it is not the text ink (a value an
    /// expression drives).
    ink: Option<u32>,
    /// The width the editor was last given, to count its wrapped lines.
    width: Rc<std::cell::Cell<f32>>,
}
impl EventEmitter<TextEvent> for TextField {}
/// Whether `window` can host a native field (an AppKit window, not a
/// headless test window).
pub fn available(window: &Window) -> bool {
    matches!(
        HasWindowHandle::window_handle(window).map(|h| h.as_raw()),
        Ok(RawWindowHandle::AppKit(_))
    )
}
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
            bare: false,
            code: None,
            ink: None,
            width: Rc::default(),
        }
    }
    /// The lines a multi-line field shows: its text as it wraps in the
    /// field's width, between two and [`MULTILINE_MOST`] (a line of source
    /// counts from one). Past the most, the editor scrolls.
    fn lines(&self, window: &mut Window) -> f32 {
        let text = self.slot.value();
        let size = px(self.tokens.typography.body);
        let room = self.width.get() - 2. * MULTILINE_TEXT_ORIGIN.0;
        let mut lines = 0usize;
        for line in text.split('\n') {
            lines += 1;
            if self.code.is_some() {
                // The fixed-width face: every character is this wide.
                let across = (room / (f32::from(size) * MONO_ADVANCE)).floor().max(1.);
                lines += ((line.chars().count() as f32 / across).ceil() as usize).saturating_sub(1);
            } else if room > 0. && !line.is_empty() {
                let style = window.text_style();
                let font = Font {
                    family: pm_design::TEXT_FONT.into(),
                    ..style.font()
                };
                let run = TextRun {
                    len: line.len(),
                    font,
                    color: style.color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                    letter_spacing: None,
                };
                let width = window
                    .text_system()
                    .shape_line(line.to_owned().into(), size, &[run])
                    .width;
                lines += (f32::from(width) / room).floor() as usize;
            }
        }
        let least = if self.code.is_some() { 1 } else { 2 };
        lines.clamp(least, MULTILINE_MOST) as f32
    }
    /// A multiline editor in `family` (code editors use the mono face) that
    /// fills its parent's height.
    pub fn code_editor(&mut self, family: &str) {
        self.set_code(family);
        self.fill = true;
    }
    /// Source text in `family` (the mono face): no spelling marks,
    /// corrections or smart substitutions. The field keeps its own height.
    pub fn set_code(&mut self, family: &str) {
        let size = self.tokens.typography.body as f64;
        // The family's PostScript face when installed, else Menlo (always is).
        let face = [
            format!("{}-Regular", family.replace(' ', "")),
            "Menlo-Regular".into(),
        ]
        .into_iter()
        .find(|name| {
            objc2_app_kit::NSFont::fontWithName_size(
                &objc2_foundation::NSString::from_str(name),
                size,
            )
            .is_some()
        })
        .unwrap_or_default();
        self.slot.set_font(&face, size);
        // Code keeps Tab for indenting.
        self.slot.set_tab_inserts(true);
        // Code: no spelling marks, corrections or smart substitutions.
        if let Some(view) = self
            .slot
            .native_view()
            .downcast_ref::<objc2_app_kit::NSTextView>()
        {
            view.setContinuousSpellCheckingEnabled(false);
            view.setAutomaticSpellingCorrectionEnabled(false);
            view.setAutomaticQuoteSubstitutionEnabled(false);
            view.setAutomaticDashSubstitutionEnabled(false);
            view.setAutomaticTextReplacementEnabled(false);
            view.setGrammarCheckingEnabled(false);
        }
        self.code = Some(family.into());
    }
    /// The value's ink while it is not being edited; `None` is the text ink.
    pub fn set_ink(&mut self, ink: Option<u32>) {
        self.ink = ink;
    }
    /// The editor fills its container's height (a notes pane) instead of
    /// four rows.
    pub fn set_fill(&mut self, fill: bool) {
        self.fill = fill;
    }
    /// Muted hint shown while the field is empty (`metadata.placeholder`).
    /// Draw no well: the enclosing control owns the fill, edge and padding.
    /// Escape cancels the edit (the owner hears `Cancelled`) instead of
    /// committing it: a search field, which clears itself.
    pub fn set_escape_cancels(&mut self, cancels: bool) {
        self.slot.set_escape_cancels(cancels);
    }
    pub fn set_bare(&mut self, bare: bool) {
        self.bare = bare;
    }
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
        let resized = self.tokens.typography.body != tokens.typography.body;
        self.tokens = tokens;
        match self.code.clone() {
            // A source field keeps its face at the new size.
            Some(family) if resized => self.set_code(&family),
            Some(_) => {}
            None => self.slot.set_font_size(self.tokens.typography.body as f64),
        }
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
/// Where a multi-line native editor draws its first glyph, from its own
/// corner: the text container inset plus the line fragment padding.
const MULTILINE_TEXT_ORIGIN: (f32, f32) = (9., 3.);
/// How far under a dense well's top its one-line editor starts, so the
/// text being edited stands on the baseline of the value drawn at rest.
const DENSE_EDIT_DROP: f32 = 3.;
/// A character's advance in the fixed-width face, in ems.
const MONO_ADVANCE: f32 = 0.602;
/// The most lines a multi-line field grows to before it scrolls.
const MULTILINE_MOST: usize = 6;
impl Render for TextField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let slot = self.slot.clone();
        let multiline = (self.kind == TextKind::Multiline && !self.fill && !self.bare)
            .then(|| self.lines(window));
        let seen = self.width.clone();
        let entity = cx.entity().downgrade();
        let dense = matches!(self.kind, TextKind::DenseText | TextKind::DenseNumber);
        let editing = self.slot.is_editing();
        let (t, bare) = (&self.tokens, self.bare);
        let single = !matches!(self.kind, TextKind::Composer | TextKind::Multiline) && !self.fill;
        // A well is a soft ink fill with no outline; while it is edited a
        // neutral ring marks it (never the accent).
        // The ring is inside the well's edge: panels are translucent, and
        // an outer shadow would show through under the whole well.
        let ring = BoxShadow {
            color: crate::kit::ink(t.colors.focus, 1.).into(),
            offset: point(px(0.), px(0.)),
            blur_radius: px(0.),
            spread_radius: px(t.metrics.border),
            inset: true,
        };
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
                } else if bare {
                    // One line, so the control around it can centre the text.
                    (self.tokens.typography.body * 1.35).ceil()
                } else if let Some(lines) = multiline {
                    // As tall as its lines: the text view's line height and
                    // its inset above and below.
                    (lines * (self.tokens.typography.body * 1.3).ceil()
                        + 2. * MULTILINE_TEXT_ORIGIN.1)
                        .max(self.tokens.metrics.field)
                } else {
                    self.tokens.metrics.field
                }))
            })
            // Every value starts on one inset down a column; a well with a
            // unit trades its trailing padding for the value's room.
            .pl(px(if bare { 0. } else { self.tokens.metrics.gap }))
            .pr(px(if bare {
                0.
            } else if self.unit.is_some() {
                self.tokens.space.xs
            } else {
                self.tokens.metrics.gap
            }))
            .when(!bare, |d| {
                d.rounded(px(if single {
                    t.metrics.inner_radius
                } else {
                    t.metrics.control_radius
                }))
                .bg(crate::kit::ink(t.colors.text, t.translucency.hover))
                // A step deeper under the pointer: the well takes input.
                .when(!editing, |d| {
                    let over = crate::kit::ink(t.colors.text, t.translucency.selected);
                    d.hover_style(move |s| s.bg(over))
                })
                .when(editing, |d| d.shadow(vec![ring]))
            })
            .text_color(rgb(self.ink.unwrap_or(self.tokens.colors.text)))
            .text_size(px(self.tokens.typography.body))
            .font_family(pm_design::TEXT_FONT)
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
            // A text view has no placeholder of its own: draw it where its
            // first line starts (the view's container inset and line padding).
            .when(
                matches!(self.kind, TextKind::Composer | TextKind::Multiline)
                    && !self.placeholder.is_empty()
                    && self.slot.value().is_empty(),
                |d| {
                    d.child(
                        div()
                            .absolute()
                            .left(px(MULTILINE_TEXT_ORIGIN.0))
                            .top(px(MULTILINE_TEXT_ORIGIN.1))
                            .text_color(rgb(self.tokens.colors.muted))
                            .child(self.placeholder.clone()),
                    )
                },
            )
            .when(dense && !editing && !self.value.is_empty(), |d| {
                d.overflow_hidden()
                    .child(div().min_w(px(0.)).truncate().child(self.value.clone()))
                    .children(self.unit.clone().map(|unit| {
                        div()
                            .flex_none()
                            .ml(px(self.tokens.space.hair))
                            .text_color(rgb(self.tokens.colors.muted))
                            .child(unit)
                    }))
            })
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, cx| {
                        // A new width wraps the text anew: count again.
                        let width = f32::from(bounds.size.width);
                        if multiline.is_some() && (seen.replace(width) - width).abs() > 0.5 {
                            let _ = entity.update(cx, |_, cx| cx.notify());
                        }
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
                        // A one-line field sets its text at its top; the
                        // value drawn at rest is centred in the well. The
                        // editor stands lower by the difference, so a value
                        // does not jump when it is clicked.
                        let mut bounds = bounds;
                        if dense {
                            bounds.origin.y += px(DENSE_EDIT_DROP);
                            bounds.size.height -= px(DENSE_EDIT_DROP);
                        }
                        slot.stage(rect(bounds), rect(mask));
                    },
                )
                .absolute()
                .inset_0(),
            )
    }
}
