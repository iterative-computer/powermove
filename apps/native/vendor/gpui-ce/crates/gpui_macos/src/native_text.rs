//! Powermove P-TEXT. Real AppKit editors; never translate marked text into GPUI input.
//! Geometry is staged during GPUI paint and applied in the composition transaction.
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{NSObject, NSObjectProtocol, ProtocolObject, Sel},
};
use objc2_app_kit::*;
use objc2_foundation::{NSNotification, NSPoint, NSRect, NSSize, NSString};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::{Rc, Weak},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextKind {
    Primary,
    Multiline,
    DenseText,
    DenseNumber,
    /// A message composer: a multiline text view where Return submits and
    /// Shift- or Option-Return starts a new line (IME, spelling, dictation
    /// and emoji are AppKit's own).
    Composer,
}
impl TextKind {
    fn dense(self) -> bool {
        matches!(self, Self::DenseText | Self::DenseNumber)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum TextEvent {
    Changed(String),
    Committed(String),
    Cancelled,
    /// Return in a composer, with the text as typed.
    Submitted(String),
}
enum Editor {
    Field(Retained<NSTextField>),
    Multiline(Retained<NSTextView>),
}
impl Editor {
    fn view(&self) -> &NSView {
        match self {
            Self::Field(v) => v,
            Self::Multiline(v) => v,
        }
    }
    fn value(&self) -> String {
        match self {
            Self::Field(v) => v.stringValue().to_string(),
            Self::Multiline(v) => v.string().to_string(),
        }
    }
    fn set(&self, value: &str) {
        let s = NSString::from_str(value);
        match self {
            Self::Field(v) => v.setStringValue(&s),
            Self::Multiline(v) => v.setString(&s),
        }
    }
}
struct State {
    kind: TextKind,
    editor: Editor,
    wrapper: Retained<NSView>,
    scroll: Option<Retained<NSScrollView>>,
    gpui: Retained<NSView>,
    editing: Cell<bool>,
    focus_pending: Cell<bool>,
    original: RefCell<String>,
    events: RefCell<Vec<TextEvent>>,
    wake: RefCell<Option<Rc<dyn Fn()>>>,
    geometry: Cell<Option<(NSRect, NSRect)>>,
    staged: Cell<Option<(NSRect, NSRect)>>,
}
impl State {
    fn emit(&self, event: TextEvent) {
        self.events.borrow_mut().push(event);
        let wake = self.wake.borrow().clone();
        if let Some(wake) = wake {
            wake();
        }
    }
    fn finish(&self, cancel: bool) {
        if !self.editing.replace(false) {
            return;
        }
        let value = self.editor.value();
        // Validate only the committed value. Intermediate IME/number strings stay untouched.
        if cancel || (self.kind == TextKind::DenseNumber && !valid_number(&value)) {
            self.editor.set(&self.original.borrow());
            self.emit(TextEvent::Cancelled);
        } else {
            *self.original.borrow_mut() = value.clone();
            self.emit(TextEvent::Committed(value));
        }
        if self.kind.dense() {
            self.wrapper.setHidden(true);
            crate::composition_accessibility::changed(Retained::as_ptr(&self.gpui) as usize);
        }
    }
}
pub fn valid_number(value: &str) -> bool {
    value.trim().parse::<f64>().is_ok_and(f64::is_finite)
}
thread_local! { static SLOTS: RefCell<HashMap<usize, Vec<Weak<State>>>> = RefCell::new(HashMap::new()); }
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMNativeTextDelegate"]
    #[ivars = Rc<State>]
    struct TextDelegate;
    unsafe impl NSObjectProtocol for TextDelegate {}
    unsafe impl NSControlTextEditingDelegate for TextDelegate {}
    unsafe impl NSTextDelegate for TextDelegate {}
    unsafe impl NSTextFieldDelegate for TextDelegate {}
    unsafe impl NSTextViewDelegate for TextDelegate {}
    impl TextDelegate {
        #[unsafe(method(controlTextDidBeginEditing:))]
        fn field_begin(&self, _: &NSNotification) {
            self.ivars().editing.set(true);
            if let Editor::Field(v)=&self.ivars().editor {if let Some(ed)=v.currentEditor() {configure_editor(&ed,self.ivars().kind!=TextKind::DenseNumber);}}
        }
        #[unsafe(method(textDidBeginEditing:))]
        fn text_begin(&self, _: &NSNotification) { self.ivars().editing.set(true); }
        #[unsafe(method(controlTextDidChange:))]
        fn field_change(&self, _: &NSNotification) { self.changed(); }
        #[unsafe(method(textDidChange:))]
        fn text_change(&self, _: &NSNotification) { self.changed(); }
        #[unsafe(method(controlTextDidEndEditing:))]
        fn field_end(&self, _: &NSNotification) { self.ivars().finish(false); }
        #[unsafe(method(textDidEndEditing:))]
        fn text_end(&self, _: &NSNotification) { self.ivars().finish(false); }
        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn field_command(&self, _: &NSControl, view: &NSTextView, command: Sel) -> bool { self.command(view, command) }
        #[unsafe(method(textView:doCommandBySelector:))]
        fn text_command(&self, view: &NSTextView, command: Sel) -> bool { self.command(view, command) }
    }
);
impl TextDelegate {
    fn changed(&self) {
        let s = self.ivars();
        s.emit(TextEvent::Changed(s.editor.value()));
    }
    fn command(&self, view: &NSTextView, command: Sel) -> bool {
        // The input method consumes Escape/Return first when it owns marked text.
        if unsafe { msg_send![view, hasMarkedText] } {
            return false;
        }
        if command == objc2::sel!(insertNewline:) && self.ivars().kind == TextKind::Composer {
            let flags = objc2_app_kit::NSApplication::sharedApplication(MainThreadMarker::from(self))
                .currentEvent()
                .map(|e| e.modifierFlags())
                .unwrap_or(objc2_app_kit::NSEventModifierFlags::empty());
            if flags.intersects(
                objc2_app_kit::NSEventModifierFlags::Shift | objc2_app_kit::NSEventModifierFlags::Option,
            ) {
                // A new line, as typed.
                unsafe { view.insertNewlineIgnoringFieldEditor(None) };
                return true;
            }
            let s = self.ivars();
            s.emit(TextEvent::Submitted(s.editor.value()));
            return true;
        }
        if command == objc2::sel!(cancelOperation:) && self.ivars().kind.dense() {
            self.ivars().finish(true);
            if let Some(w) = view.window() {
                w.makeFirstResponder(Some(&self.ivars().gpui));
            }
            return true;
        }
        false
    }
}
/// Main-thread RAII native slot. Drop removes it and its accessibility parent.
/// Primary/multiline fields remain native; dense fields are hidden until `begin_editing`.
pub struct NativeTextSlot {
    state: Rc<State>,
    _delegate: Retained<TextDelegate>,
}
impl NativeTextSlot {
    pub fn new(gpui: &NSView, kind: TextKind, label: &str, initial: &str) -> Self {
        let mtm = MainThreadMarker::new().expect("text slots require the main thread");
        let editor = if matches!(kind, TextKind::Multiline | TextKind::Composer) {
            let v = NSTextView::initWithFrame(NSTextView::alloc(mtm), NSRect::ZERO);
            v.setRichText(false);
            v.setImportsGraphics(false);
            v.setAllowsUndo(true);
            v.setContinuousSpellCheckingEnabled(true);
            v.setGrammarCheckingEnabled(true);
            v.setAutomaticSpellingCorrectionEnabled(true);
            v.setDrawsBackground(false);
            v.setFont(Some(&NSFont::systemFontOfSize(13.)));
            v.setTextContainerInset(NSSize::new(4., 3.));
            Editor::Multiline(v)
        } else {
            let v = NSTextField::initWithFrame(NSTextField::alloc(mtm), NSRect::ZERO);
            v.setEditable(true);
            v.setSelectable(true);
            v.setBezeled(false);
            v.setBordered(false);
            v.setDrawsBackground(false);
            v.setFont(Some(&NSFont::systemFontOfSize(13.)));
            v.setFocusRingType(NSFocusRingType::None);
            // AppKit configures its shared field editor for this control.
            unsafe {
                let _: () = msg_send![&*v, setAllowsEditingTextAttributes: false];
            }
            Editor::Field(v)
        };
        editor.set(initial);
        editor
            .view()
            .setAccessibilityLabel(Some(&NSString::from_str(label)));
        let wrapper = NSView::initWithFrame(NSView::alloc(mtm), NSRect::ZERO);
        wrapper.setWantsLayer(true);
        if let Some(layer) = wrapper.layer() {
            layer.setMasksToBounds(true);
        }
        wrapper.setHidden(kind.dense());
        let scroll = if let Editor::Multiline(v) = &editor {
            let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), NSRect::ZERO);
            scroll.setHasVerticalScroller(true);
            scroll.setAutohidesScrollers(true);
            scroll.setDrawsBackground(false);
            v.setVerticallyResizable(true);
            v.setHorizontallyResizable(false);
            v.setMaxSize(NSSize::new(f64::MAX, f64::MAX));
            scroll.setDocumentView(Some(v));
            wrapper.addSubview(&scroll);
            Some(scroll)
        } else {
            wrapper.addSubview(editor.view());
            None
        };
        let state = Rc::new(State {
            kind,
            editor,
            wrapper,
            scroll,
            gpui: unsafe { Retained::retain(gpui as *const NSView as *mut NSView).unwrap() },
            editing: Cell::new(false),
            focus_pending: Cell::new(false),
            original: RefCell::new(initial.into()),
            events: RefCell::new(Vec::new()),
            wake: RefCell::new(None),
            geometry: Cell::new(None),
            staged: Cell::new(None),
        });
        let delegate = TextDelegate::alloc(mtm).set_ivars(state.clone());
        let delegate: Retained<TextDelegate> = unsafe { msg_send![super(delegate), init] };
        match &state.editor {
            Editor::Field(v) => unsafe {
                v.setDelegate(Some(ProtocolObject::from_ref(&*delegate)))
            },
            Editor::Multiline(v) => v.setDelegate(Some(ProtocolObject::from_ref(&*delegate))),
        }
        // Writing Tools, dictation, emoji and Services use NSTextView's ordinary responder.
        // No input-context replacement or keyDown override is installed.
        if let Some(content) = gpui.window().and_then(|w| w.contentView()) {
            content.addSubview_positioned_relativeTo(
                &state.wrapper,
                NSWindowOrderingMode::Above,
                Some(gpui),
            );
        }
        SLOTS.with(|slots| {
            slots
                .borrow_mut()
                .entry(gpui as *const NSView as usize)
                .or_default()
                .push(Rc::downgrade(&state))
        });
        crate::composition_accessibility::changed(gpui as *const NSView as usize);
        Self {
            state,
            _delegate: delegate,
        }
    }
    pub fn set_font_size(&self, points: f64) {
        let font = NSFont::systemFontOfSize(points);
        match &self.state.editor {
            Editor::Field(v) => v.setFont(Some(&font)),
            Editor::Multiline(v) => v.setFont(Some(&font)),
        }
    }
    /// Placeholder for single-line fields (AppKit draws it while editing).
    pub fn set_placeholder(&self, text: &str) {
        if let Editor::Field(v) = &self.state.editor {
            v.setPlaceholderString(Some(&NSString::from_str(text)));
        }
    }
    pub fn on_event(&self, wake: Rc<dyn Fn()>) {
        *self.state.wake.borrow_mut() = Some(wake);
    }
    pub fn value(&self) -> String {
        self.state.editor.value()
    }
    pub fn is_editing(&self) -> bool {
        self.state.editing.get()
    }
    pub fn native_view(&self) -> &NSView {
        self.state.editor.view()
    }
    /// Optional explicit reading-order anchor for the host's committed composition.
    pub fn accessibility_item(&self) -> Option<crate::composition::AccessibilityItem> {
        (!self.state.wrapper.isHidden()).then(|| {
            crate::composition::AccessibilityItem::Native(unsafe {
                Retained::retain(self.native_view() as *const NSView as *mut NSView).unwrap()
            })
        })
    }
    /// Top-left point bounds and inherited rectangular content clip, in GPUI coordinates.
    pub fn stage(&self, bounds: NSRect, clip: NSRect) {
        self.state.geometry.set(Some((bounds, clip)));
        self.state.staged.set(Some((bounds, clip)));
    }
    /// A cached GPUI view reuses its paint, so its slot keeps the last geometry.
    pub fn keep(&self) {
        self.state.geometry.set(self.state.staged.get());
    }
    pub fn begin_editing(&self) {
        if !self.state.editing.replace(true) {
            *self.state.original.borrow_mut() = self.value();
        }
        self.state.focus_pending.set(true);
    }
    pub fn take_events(&self) -> Vec<TextEvent> {
        std::mem::take(&mut *self.state.events.borrow_mut())
    }
    /// Replaces the text even while editing, unless an input method holds
    /// marked text (a composer clearing after its message was sent).
    pub fn replace_value(&self, value: &str) {
        if let Editor::Multiline(v) = &self.state.editor
            && unsafe { msg_send![&**v, hasMarkedText] }
        {
            return;
        }
        self.state.editor.set(value);
        *self.state.original.borrow_mut() = value.into();
    }
    pub fn set_value(&self, value: &str) {
        if !self.is_editing() {
            self.state.editor.set(value);
            *self.state.original.borrow_mut() = value.into();
        }
    }
}
impl Drop for NativeTextSlot {
    fn drop(&mut self) {
        if self.state.editing.get() {
            if let Some(w) = self.state.gpui.window() {
                w.makeFirstResponder(Some(&self.state.gpui));
            }
        }
        self.state.wrapper.removeFromSuperview();
        crate::composition_accessibility::changed(Retained::as_ptr(&self.state.gpui) as usize);
        let v = unsafe {
            Retained::retain(self.state.editor.view() as *const NSView as *mut NSView).unwrap()
        };
        crate::composition::clear_parents(&[crate::composition::AccessibilityItem::Native(v)]);
        SLOTS.with(|slots| {
            let mut slots = slots.borrow_mut();
            if let Some(v) = slots.get_mut(&(Retained::as_ptr(&self.state.gpui) as usize)) {
                v.retain(|s| s.upgrade().is_some_and(|s| !Rc::ptr_eq(&s, &self.state)));
            }
        });
    }
}
/// Applies staged text-slot geometry. Returns `Moved` when any slot's frame,
/// clip or visibility changed this frame (P-120 transactional present).
pub(crate) fn present(key: usize) -> crate::composition::SlotChange {
    let mut moved = false;
    let slots = SLOTS.with(|slots| slots.borrow().get(&key).cloned().unwrap_or_default());
    for state in slots.into_iter().filter_map(|s| s.upgrade()) {
        let Some((bounds, clip)) = state.geometry.take() else {
            if !state.wrapper.isHidden() {
                state.wrapper.setHidden(true);
                moved = true;
                crate::composition_accessibility::changed(key);
            }
            continue;
        };
        let h = state
            .gpui
            .window()
            .and_then(|w| w.contentView())
            .map_or(0., |v| v.bounds().size.height);
        let visible = intersection(bounds, clip);
        let frame = NSRect::new(
            NSPoint::new(visible.origin.x, h - visible.origin.y - visible.size.height),
            visible.size,
        );
        if state.wrapper.frame() != frame {
            state.wrapper.setFrame(frame);
            moved = true;
        }
        let local = NSRect::new(
            NSPoint::new(
                bounds.origin.x - visible.origin.x,
                visible.origin.y + visible.size.height - bounds.origin.y - bounds.size.height,
            ),
            bounds.size,
        );
        if let Some(scroll) = &state.scroll {
            if scroll.frame() != local {
                scroll.setFrame(local);
                moved = true;
            }
            let size = scroll.contentSize();
            let view = state.editor.view();
            if view.frame().size.width != size.width {
                view.setFrameSize(NSSize::new(
                    size.width,
                    view.frame().size.height.max(size.height),
                ));
            }
        } else if state.editor.view().frame() != local {
            state.editor.view().setFrame(local);
            moved = true;
        }
        let hidden = visible.size.width <= 0.
            || visible.size.height <= 0.
            || (state.kind.dense() && !state.editing.get());
        if state.wrapper.isHidden() != hidden {
            state.wrapper.setHidden(hidden);
            moved = true;
            crate::composition_accessibility::changed(key);
        }
        if state.focus_pending.replace(false) {
            if let Some(w) = state.gpui.window() {
                if let Editor::Multiline(v) = &state.editor {
                    w.makeFirstResponder(Some(v));
                }
                if let Editor::Field(v) = &state.editor {
                    unsafe {
                        v.selectText(None);
                    }
                    if let Some(ed) = v.currentEditor() {
                        configure_editor(&ed, state.kind != TextKind::DenseNumber);
                    }
                }
            }
        }
    }
    moved.into()
}
pub(crate) fn accessibility(key: usize) -> Vec<crate::composition::AccessibilityItem> {
    SLOTS.with(|slots| {
        slots
            .borrow()
            .get(&key)
            .into_iter()
            .flatten()
            .filter_map(|s| s.upgrade())
            .filter(|s| !s.wrapper.isHidden())
            .map(|s| {
                crate::composition::AccessibilityItem::Native(unsafe {
                    Retained::retain(s.editor.view() as *const NSView as *mut NSView).unwrap()
                })
            })
            .collect()
    })
}
fn intersection(a: NSRect, b: NSRect) -> NSRect {
    let x = a.origin.x.max(b.origin.x);
    let y = a.origin.y.max(b.origin.y);
    NSRect::new(
        NSPoint::new(x, y),
        NSSize::new(
            (a.origin.x + a.size.width)
                .min(b.origin.x + b.size.width)
                .max(x)
                - x,
            (a.origin.y + a.size.height)
                .min(b.origin.y + b.size.height)
                .max(y)
                - y,
        ),
    )
}

pub(crate) fn has_slots(key: usize) -> bool {
    SLOTS.with(|s| {
        s.borrow()
            .get(&key)
            .is_some_and(|v| v.iter().any(|s| s.strong_count() > 0))
    })
}

fn configure_editor(editor: &NSText, spell: bool) {
    unsafe {
        let _: () = msg_send![editor,setContinuousSpellCheckingEnabled:spell];
        let _: () = msg_send![editor,setAutomaticSpellingCorrectionEnabled:spell];
        let supported: bool =
            msg_send![editor,respondsToSelector:objc2::sel!(setWritingToolsBehavior:)];
        if supported {
            let _: () = msg_send![editor,setWritingToolsBehavior:0isize];
        }
    }
}
