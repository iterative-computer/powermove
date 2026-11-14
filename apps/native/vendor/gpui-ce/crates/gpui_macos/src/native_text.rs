//! Powermove P-TEXT. Real AppKit editors; never translate marked text into GPUI input.
//! Geometry is staged during GPUI paint and applied in the composition transaction.
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, class, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject, Sel},
};
use objc2_app_kit::*;
use objc2_foundation::{NSArray, NSNotification, NSPoint, NSRange, NSRect, NSSize, NSString};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::{Rc, Weak},
};

/// The tag of a field in which Escape cancels the edit; see
/// `NativeTextSlot::set_escape_cancels`.
pub const ESCAPE_CANCELS: isize = 0x504d_4543;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextKind {
    Primary,
    Multiline,
    DenseText,
    DenseNumber,
    /// On-canvas text: an invisible `NSTextView` owns typing, IME, dictation,
    /// emoji and spelling while the host draws the text, caret and selection
    /// from its own layout. Clicks pass through to the host; blurring reports
    /// `Blurred` instead of committing, so the host decides.
    Canvas,
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
    /// Canvas only: the selected UTF-16 range changed.
    Selection {
        start: usize,
        end: usize,
    },
    /// Canvas only: an editing chord the host handles (`commit`, `bold`,
    /// `italic`, `larger`, `smaller`, `larger10`, `smaller10`, `alignLeft`,
    /// `alignCenter`, `alignRight`, `up`, `down`, `shiftUp`, `shiftDown`).
    /// A plain multiline field sends `tab` / `backtab`: focus moves on.
    /// A composer sends `escape`.
    Key(&'static str),
    /// Canvas only: the editor lost first responder.
    Blurred,
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
    /// A multiline field that keeps Tab as a character (a code editor).
    /// Otherwise Tab and Shift-Tab commit and move focus on, as in an
    /// inspector (Option-Tab still types a tab).
    tab_inserts: Cell<bool>,
    focus_pending: Cell<bool>,
    original: RefCell<String>,
    events: RefCell<Vec<TextEvent>>,
    wake: RefCell<Option<Rc<dyn Fn()>>>,
    /// What a paint staged, with that paint's number: placed by the present
    /// that follows that paint, and by no later one.
    geometry: Cell<Option<(u64, NSRect, NSRect)>>,
    staged: Cell<Option<(NSRect, NSRect)>>,
    /// Canvas: the system insertion point, in its own pointer-transparent host.
    caret: Option<(Retained<NSView>, Retained<NSTextInsertionIndicator>)>,
    /// Canvas: the caret rectangle and clip in GPUI points, `None` hides it.
    caret_staged: Cell<Option<(NSRect, NSRect)>>,
    /// Canvas: where input method candidates open (GPUI points): the host's
    /// caret on the visible text, never this view's own unwrapped layout.
    input_rect: Cell<Option<NSRect>>,
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
impl State {
    /// The selected UTF-16 range, start before end.
    fn selection(&self) -> (usize, usize) {
        match &self.editor {
            Editor::Multiline(v) => {
                let r: NSRange = unsafe { msg_send![&**v, selectedRange] };
                (r.location, r.location + r.length)
            }
            Editor::Field(_) => (0, 0),
        }
    }
}
define_class!(
    /// The canvas editor: handles the editing chords a plain text view would
    /// beep at or hand to the menu bar, only while it is first responder.
    #[unsafe(super(NSTextView, NSText, NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMCanvasTextView"]
    #[ivars = RefCell<Weak<State>>]
    struct CanvasTextView;
    unsafe impl NSObjectProtocol for CanvasTextView {}
    impl CanvasTextView {
        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, event: &NSEvent) -> bool {
            self.chord(event) || unsafe { msg_send![super(self), performKeyEquivalent: event] }
        }
        /// Input methods place candidates here (NSTextInputClient).
        #[unsafe(method(firstRectForCharacterRange:actualRange:))]
        fn first_rect(&self, range: NSRange, actual: *mut NSRange) -> NSRect {
            if !actual.is_null() {
                unsafe { *actual = range };
            }
            let staged = self.ivars().borrow().upgrade().and_then(|s| s.input_rect.get());
            match (staged, self.window()) {
                (Some(rect), Some(window)) => {
                    let h = window.contentView().map_or(0., |v| v.bounds().size.height);
                    let flipped = NSRect::new(
                        NSPoint::new(rect.origin.x, h - rect.origin.y - rect.size.height),
                        rect.size,
                    );
                    window.convertRectToScreen(flipped)
                }
                _ => unsafe {
                    msg_send![super(self), firstRectForCharacterRange: range, actualRange: actual]
                },
            }
        }
        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            if !self.chord(event) {
                unsafe { let _: () = msg_send![super(self), keyDown: event]; }
            }
        }
    }
);
impl CanvasTextView {
    fn chord(&self, event: &NSEvent) -> bool {
        let Some(state) = self.ivars().borrow().upgrade() else {
            return false;
        };
        let me = self as *const Self as *const AnyObject;
        let first = self
            .window()
            .and_then(|w| w.firstResponder())
            .is_some_and(|r| std::ptr::eq(Retained::as_ptr(&r) as *const AnyObject, me));
        let marked: bool = unsafe { msg_send![self, hasMarkedText] };
        if !first || marked || event.r#type() != NSEventType::KeyDown {
            return false;
        }
        let flags = event.modifierFlags();
        let has = |f: NSEventModifierFlags| flags.contains(f);
        if !has(NSEventModifierFlags::Command) || has(NSEventModifierFlags::Control) {
            return false;
        }
        let (shift, alt) = (
            has(NSEventModifierFlags::Shift),
            has(NSEventModifierFlags::Option),
        );
        // Key codes, so other layouts reach the same physical keys.
        let key = match (event.keyCode(), shift, alt) {
            (36 | 76, _, _) => "commit",
            (11, false, false) => "bold",
            (34, false, false) => "italic",
            (47, true, false) => "larger",
            (43, true, false) => "smaller",
            (47, true, true) => "larger10",
            (43, true, true) => "smaller10",
            (37, false, true) => "alignLeft",
            (17, false, true) => "alignCenter",
            (15, false, true) => "alignRight",
            _ => return false,
        };
        state.emit(TextEvent::Key(key));
        true
    }
}
define_class!(
    /// Holds the canvas editor without taking any pointer event.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMPassthroughView"]
    struct PassthroughView;
    unsafe impl NSObjectProtocol for PassthroughView {}
    impl PassthroughView {
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, _: NSPoint) -> *mut NSView {
            std::ptr::null_mut()
        }
    }
);
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
        fn text_end(&self, _: &NSNotification) {
            let s = self.ivars();
            if s.kind == TextKind::Canvas {
                s.editing.set(false);
                s.emit(TextEvent::Blurred);
            } else {
                s.finish(false);
            }
        }
        #[unsafe(method(textViewDidChangeSelection:))]
        fn selection_change(&self, _: &NSNotification) {
            let s = self.ivars();
            if s.kind == TextKind::Canvas {
                let (start, end) = s.selection();
                s.emit(TextEvent::Selection { start, end });
            }
        }
        /// A spelling guess chosen from the canvas context menu.
        #[unsafe(method(pmUseGuess:))]
        fn use_guess(&self, item: &NSMenuItem) {
            if let Editor::Multiline(v) = &self.ivars().editor {
                let range: NSRange = unsafe { msg_send![&**v, selectedRange] };
                let title = item.title();
                unsafe {
                    let _: () = msg_send![&**v, insertText: &*title, replacementRange: range];
                }
            }
        }
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
        // A number field steps with Up and Down (Shift: ten steps), as a
        // spin field does; the host owns the value and its step.
        if self.ivars().kind == TextKind::DenseNumber {
            let key = match command {
                c if c == objc2::sel!(moveUp:) => Some("up"),
                c if c == objc2::sel!(moveDown:) => Some("down"),
                c if c == objc2::sel!(moveUpAndModifySelection:) => Some("shiftUp"),
                c if c == objc2::sel!(moveDownAndModifySelection:) => Some("shiftDown"),
                _ => None,
            };
            if let Some(key) = key {
                self.ivars().emit(TextEvent::Key(key));
                return true;
            }
        }
        if self.ivars().kind == TextKind::Canvas {
            let key = match command {
                c if c == objc2::sel!(cancelOperation:) => Some("commit"),
                c if c == objc2::sel!(moveUp:) => Some("up"),
                c if c == objc2::sel!(moveDown:) => Some("down"),
                c if c == objc2::sel!(moveUpAndModifySelection:) => Some("shiftUp"),
                c if c == objc2::sel!(moveDownAndModifySelection:) => Some("shiftDown"),
                // Tab never leaves the text or types into it.
                c if c == objc2::sel!(insertTab:) || c == objc2::sel!(insertBacktab:) => None,
                _ => return false,
            };
            if let Some(key) = key {
                self.ivars().emit(TextEvent::Key(key));
            }
            return true;
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
        // Escape in a composer belongs to its host (close a menu, then the
        // popover around it); AppKit's own cancelOperation: does nothing here.
        if command == objc2::sel!(cancelOperation:) && self.ivars().kind == TextKind::Composer {
            self.ivars().emit(TextEvent::Key("escape"));
            return true;
        }
        if self.ivars().kind == TextKind::Multiline
            && !self.ivars().tab_inserts.get()
            && (command == objc2::sel!(insertTab:) || command == objc2::sel!(insertBacktab:))
        {
            // Commit, give focus back to GPUI, and let the host move it to
            // the next (or previous) control.
            self.ivars().finish(false);
            if let Some(w) = view.window() {
                w.makeFirstResponder(Some(&self.ivars().gpui));
            }
            self.ivars().emit(TextEvent::Key(if command == objc2::sel!(insertTab:) {
                "tab"
            } else {
                "backtab"
            }));
            return true;
        }
        let cancels = match &self.ivars().editor {
            Editor::Field(field) => field.tag() == ESCAPE_CANCELS,
            Editor::Multiline(_) => false,
        };
        if command == objc2::sel!(cancelOperation:) && (self.ivars().kind.dense() || cancels) {
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
    /// Keep Tab as a typed character (a code editor) instead of moving focus.
    pub fn set_tab_inserts(&self, inserts: bool) {
        self.state.tab_inserts.set(inserts);
    }
    pub fn new(gpui: &NSView, kind: TextKind, label: &str, initial: &str) -> Self {
        let mtm = MainThreadMarker::new().expect("text slots require the main thread");
        let mut canvas = None;
        let editor = if kind == TextKind::Canvas {
            let this = CanvasTextView::alloc(mtm).set_ivars(RefCell::new(Weak::new()));
            let v: Retained<CanvasTextView> =
                unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
            canvas = Some(v.clone());
            let v: Retained<NSTextView> = Retained::into_super(v);
            v.setRichText(false);
            v.setImportsGraphics(false);
            v.setAllowsUndo(true);
            v.setContinuousSpellCheckingEnabled(true);
            v.setAutomaticSpellingCorrectionEnabled(false);
            v.setAutomaticQuoteSubstitutionEnabled(false);
            v.setAutomaticDashSubstitutionEnabled(false);
            v.setDrawsBackground(false);
            v.setFont(Some(&NSFont::systemFontOfSize(13.)));
            v.setTextContainerInset(NSSize::new(0., 0.));
            // Lines break only where the text does; the host wraps paragraphs.
            v.setHorizontallyResizable(true);
            v.setVerticallyResizable(true);
            v.setMaxSize(NSSize::new(1e7, 1e7));
            if let Some(container) = unsafe { v.textContainer() } {
                container.setWidthTracksTextView(false);
                container.setContainerSize(NSSize::new(1e7, 1e7));
                container.setLineFragmentPadding(0.);
            }
            Editor::Multiline(v)
        } else if matches!(kind, TextKind::Multiline | TextKind::Composer) {
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
            if kind == TextKind::Composer {
                // A prompt is not prose to rewrite: no Writing Tools badge
                // beside the empty field (NSWritingToolsBehaviorNone).
                unsafe {
                    let supported: bool =
                        msg_send![&*v, respondsToSelector: objc2::sel!(setWritingToolsBehavior:)];
                    if supported {
                        let _: () = msg_send![&*v, setWritingToolsBehavior: -1isize];
                    }
                }
            }
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
        let wrapper = if kind == TextKind::Canvas {
            let this = PassthroughView::alloc(mtm).set_ivars(());
            let v: Retained<PassthroughView> =
                unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
            let v: Retained<NSView> = Retained::into_super(v);
            // Never seen: the host draws what this editor holds.
            v.setAlphaValue(0.);
            v
        } else {
            NSView::initWithFrame(NSView::alloc(mtm), NSRect::ZERO)
        };
        wrapper.setWantsLayer(true);
        if let Some(layer) = wrapper.layer() {
            layer.setMasksToBounds(true);
        }
        wrapper.setHidden(kind.dense());
        let scroll = if kind == TextKind::Canvas {
            wrapper.addSubview(editor.view());
            None
        } else if let Editor::Multiline(v) = &editor {
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
            tab_inserts: Cell::new(false),
            focus_pending: Cell::new(false),
            original: RefCell::new(initial.into()),
            events: RefCell::new(Vec::new()),
            wake: RefCell::new(None),
            geometry: Cell::new(None),
            staged: Cell::new(None),
            caret: (kind == TextKind::Canvas).then(|| {
                let this = PassthroughView::alloc(mtm).set_ivars(());
                let host: Retained<PassthroughView> =
                    unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
                let host: Retained<NSView> = Retained::into_super(host);
                let indicator = NSTextInsertionIndicator::initWithFrame(
                    NSTextInsertionIndicator::alloc(mtm),
                    NSRect::ZERO,
                );
                indicator.setDisplayMode(NSTextInsertionIndicatorDisplayMode::Hidden);
                host.addSubview(&indicator);
                host.setHidden(true);
                (host, indicator)
            }),
            caret_staged: Cell::new(None),
            input_rect: Cell::new(None),
        });
        if let Some(canvas) = &canvas {
            *canvas.ivars().borrow_mut() = Rc::downgrade(&state);
        }
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
            // As a hosted control: it keeps its distance from the window's
            // top edge until the next paint places it.
            let top = objc2_app_kit::NSAutoresizingMaskOptions::ViewMinYMargin;
            state.wrapper.setAutoresizingMask(top);
            content.addSubview_positioned_relativeTo(
                &state.wrapper,
                NSWindowOrderingMode::Above,
                Some(gpui),
            );
            if let Some((host, _)) = &state.caret {
                host.setAutoresizingMask(top);
                content.addSubview_positioned_relativeTo(
                    host,
                    NSWindowOrderingMode::Above,
                    Some(&state.wrapper),
                );
            }
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
    /// Escape in this field cancels the edit (the view hears `Cancelled`:
    /// a search clears itself) instead of committing what was typed.
    pub fn set_escape_cancels(&self, cancels: bool) {
        if let Editor::Field(v) = &self.state.editor {
            v.setTag(if cancels { ESCAPE_CANCELS } else { 0 });
        }
    }
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
        self.state.stage(Some((bounds, clip)));
        self.state.staged.set(Some((bounds, clip)));
    }
    /// A cached GPUI view reuses its paint, so its slot keeps the last geometry.
    pub fn keep(&self) {
        self.state.stage(self.state.staged.get());
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
    pub fn set_value(&self, value: &str) {
        if !self.is_editing() {
            self.state.editor.set(value);
            *self.state.original.borrow_mut() = value.into();
        }
    }
    /// Canvas: where the system insertion point stands (top-left GPUI
    /// points, clipped), or `None` while a range is selected.
    pub fn stage_caret(&self, caret: Option<(NSRect, NSRect)>) {
        self.state.caret_staged.set(caret);
    }
    /// Canvas: the caret rectangle input method candidates open beside
    /// (top-left GPUI points).
    pub fn stage_input_rect(&self, rect: Option<NSRect>) {
        self.state.input_rect.set(rect);
    }
    /// Canvas: the composing (marked) UTF-16 range, if any.
    pub fn marked_range(&self) -> Option<(usize, usize)> {
        let Editor::Multiline(v) = &self.state.editor else {
            return None;
        };
        if !self.has_marked_text() {
            return None;
        }
        let r: NSRange = unsafe { msg_send![&**v, markedRange] };
        (r.location != usize::MAX).then_some((r.location, r.location + r.length))
    }
    /// Canvas: the selected UTF-16 range, start before end.
    pub fn selection(&self) -> (usize, usize) {
        self.state.selection()
    }
    /// Canvas: select a UTF-16 range (clamped to the text).
    pub fn set_selection(&self, start: usize, end: usize) {
        if let Editor::Multiline(v) = &self.state.editor {
            let length = v.string().length();
            let (start, end) = (start.min(length), end.min(length));
            let (start, end) = (start.min(end), start.max(end));
            v.setSelectedRange(NSRange::new(start, end - start));
        }
    }
    /// Replaces the text even while editing (a composer clearing after its
    /// message was sent; on canvas, an outside change such as undo or the
    /// inspector, keeping the selection where it still fits). An input
    /// method's composition is never interrupted.
    pub fn replace_value(&self, value: &str) {
        if self.has_marked_text() || self.value() == value {
            return;
        }
        let (start, end) = self.selection();
        self.state.editor.set(value);
        *self.state.original.borrow_mut() = value.into();
        if self.state.kind == TextKind::Canvas {
            self.set_selection(start, end);
        }
    }
    pub fn has_marked_text(&self) -> bool {
        match &self.state.editor {
            Editor::Multiline(v) => unsafe { msg_send![&**v, hasMarkedText] },
            Editor::Field(_) => false,
        }
    }
    /// The editor is the window's first responder.
    pub fn is_focused(&self) -> bool {
        let view = self.native_view() as *const NSView as *const AnyObject;
        self.state
            .gpui
            .window()
            .and_then(|w| w.firstResponder())
            .is_some_and(|r| std::ptr::eq(Retained::as_ptr(&r) as *const AnyObject, view))
    }
    /// Canvas: the edited text's family and on-screen size, so input method
    /// candidates open beside the glyphs.
    pub fn set_font(&self, family: &str, points: f64) {
        let points = points.clamp(1., 2000.);
        let font = NSFont::fontWithName_size(&NSString::from_str(family), points)
            .unwrap_or_else(|| NSFont::systemFontOfSize(points));
        if let Editor::Multiline(v) = &self.state.editor {
            v.setFont(Some(&font));
        }
    }
    /// Canvas: misspelled UTF-16 ranges, from the system spell checker with
    /// this editor's document tag (so Ignore and Learn apply).
    pub fn misspelled(&self) -> Vec<(usize, usize)> {
        let Editor::Multiline(v) = &self.state.editor else {
            return vec![];
        };
        let text = v.string();
        let length = text.length();
        let mut ranges = vec![];
        unsafe {
            let checker: Retained<AnyObject> =
                msg_send![class!(NSSpellChecker), sharedSpellChecker];
            let tag: isize = msg_send![&**v, spellCheckerDocumentTag];
            let mut from = 0usize;
            while from < length {
                let found: NSRange = msg_send![
                    &checker,
                    checkSpellingOfString: &*text,
                    startingAt: from as isize,
                    language: Option::<&NSString>::None,
                    wrap: false,
                    inSpellDocumentWithTag: tag,
                    wordCount: std::ptr::null_mut::<isize>()
                ];
                if found.length == 0 || found.location == usize::MAX || found.location < from {
                    break;
                }
                ranges.push((found.location, found.location + found.length));
                from = found.location + found.length;
            }
        }
        ranges
    }
    /// Canvas: the editor's own context menu on `word` (selected first), with
    /// spelling guesses on top when it is misspelled.
    pub fn context_menu(&self, word: (usize, usize)) {
        let Editor::Multiline(v) = &self.state.editor else {
            return;
        };
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        self.set_selection(word.0, word.1);
        let app = NSApplication::sharedApplication(mtm);
        let Some(event) = app.currentEvent() else {
            return;
        };
        let menu: Option<Retained<NSMenu>> = unsafe { msg_send![&**v, menuForEvent: &*event] };
        let menu = menu.unwrap_or_else(|| NSMenu::new(mtm));
        if self.misspelled().contains(&word) {
            unsafe {
                let checker: Retained<AnyObject> =
                    msg_send![class!(NSSpellChecker), sharedSpellChecker];
                let tag: isize = msg_send![&**v, spellCheckerDocumentTag];
                let range = NSRange::new(word.0, word.1 - word.0);
                let guesses: Option<Retained<NSArray<NSString>>> = msg_send![
                    &checker,
                    guessesForWordRange: range,
                    inString: &*v.string(),
                    language: Option::<&NSString>::None,
                    inSpellDocumentWithTag: tag
                ];
                let guesses = guesses.map(|g| g.to_vec()).unwrap_or_default();
                if !guesses.is_empty() {
                    menu.insertItem_atIndex(&NSMenuItem::separatorItem(mtm), 0);
                }
                for guess in guesses.iter().take(6).rev() {
                    let item = NSMenuItem::initWithTitle_action_keyEquivalent(
                        NSMenuItem::alloc(mtm),
                        guess,
                        Some(objc2::sel!(pmUseGuess:)),
                        &NSString::new(),
                    );
                    item.setTarget(Some(&*self._delegate));
                    menu.insertItem_atIndex(&item, 0);
                }
            }
        }
        NSMenu::popUpContextMenu_withEvent_forView(&menu, &event, v);
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
        if let Some((host, _)) = &self.state.caret {
            host.removeFromSuperview();
        }
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
impl State {
    fn stage(&self, geometry: Option<(NSRect, NSRect)>) {
        let paint = crate::composition::open_paint(Retained::as_ptr(&self.gpui) as usize);
        self.geometry
            .set(geometry.map(|(bounds, clip)| (paint, bounds, clip)));
    }
}
/// Applies staged text-slot geometry. Returns `Moved` when any slot's frame,
/// clip or visibility changed this frame (P-120 transactional present).
pub(crate) fn present(key: usize) -> crate::composition::SlotChange {
    place(key, true).into()
}
/// Whether [`present`] would move a slot. Nothing is touched.
pub(crate) fn pending(key: usize) -> bool {
    place(key, false)
}
/// Places what the last complete paint staged (`apply`), or only says
/// whether doing so would move anything.
fn place(key: usize, apply: bool) -> bool {
    let mut moved = false;
    let last = crate::composition::last_paint(key);
    let slots = SLOTS.with(|slots| slots.borrow().get(&key).cloned().unwrap_or_default());
    for state in slots.into_iter().filter_map(|s| s.upgrade()) {
        // A canvas editor is invisible and must stay first responder, so a
        // frame that did not repaint its host keeps the last placement.
        let geometry = state
            .geometry
            .get()
            .filter(|(paint, ..)| Some(*paint) == last)
            .map(|(_, bounds, clip)| (bounds, clip))
            .or_else(|| {
                (state.kind == TextKind::Canvas)
                    .then(|| state.staged.get())
                    .flatten()
            });
        if apply {
            state.geometry.set(None);
        }
        let Some((bounds, clip)) = geometry else {
            if !state.wrapper.isHidden() {
                moved = true;
                if apply {
                    state.wrapper.setHidden(true);
                    crate::composition_accessibility::changed(key);
                }
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
            moved = true;
            if apply {
                state.wrapper.setFrame(frame);
            }
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
                moved = true;
                if apply {
                    scroll.setFrame(local);
                }
            }
            let size = scroll.contentSize();
            let view = state.editor.view();
            if apply && view.frame().size.width != size.width {
                view.setFrameSize(NSSize::new(
                    size.width,
                    view.frame().size.height.max(size.height),
                ));
            }
        } else if state.editor.view().frame() != local {
            moved = true;
            if apply {
                state.editor.view().setFrame(local);
            }
        }
        let hidden = visible.size.width <= 0.
            || visible.size.height <= 0.
            || (state.kind.dense() && !state.editing.get());
        if state.wrapper.isHidden() != hidden {
            moved = true;
            if apply {
                state.wrapper.setHidden(hidden);
                crate::composition_accessibility::changed(key);
            }
        }
        if let Some((host, indicator)) = &state.caret {
            let content_h = state
                .gpui
                .window()
                .and_then(|w| w.contentView())
                .map_or(0., |v| v.bounds().size.height);
            let focused = state.editing.get();
            match state.caret_staged.get().filter(|_| focused) {
                Some((rect, clip)) => {
                    let shown = intersection(rect, clip);
                    let frame = NSRect::new(
                        NSPoint::new(rect.origin.x, content_h - rect.origin.y - rect.size.height),
                        rect.size,
                    );
                    let visible = shown.size.width > 0. && shown.size.height > 0.;
                    if host.frame() != frame {
                        moved = true;
                        if apply {
                            host.setFrame(frame);
                            indicator.setFrame(NSRect::new(NSPoint::new(0., 0.), rect.size));
                        }
                    }
                    if host.isHidden() == visible {
                        moved = true;
                        if apply {
                            host.setHidden(!visible);
                        }
                    }
                    if apply {
                        indicator.setDisplayMode(NSTextInsertionIndicatorDisplayMode::Automatic);
                    }
                }
                None => {
                    if !host.isHidden() {
                        moved = true;
                        if apply {
                            indicator.setDisplayMode(NSTextInsertionIndicatorDisplayMode::Hidden);
                            host.setHidden(true);
                        }
                    }
                }
            }
        }
        if apply && state.focus_pending.replace(false) {
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
    moved
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
