//! Real AppKit controls composed into GPUI's native transaction: the system
//! draws them (liquid glass on macOS 26 and later), tracks them, focuses them
//! and dims them. Chrome, sheets, Settings, popovers and panel headers host
//! these; `crate::kit` keeps drawn equivalents only where a view per control
//! cannot work (long scrolling lists, the timeline, the viewer).
//!
//! Each control sits in its own clipping wrapper above the GPUI view
//! (`NativeControlSlot`), staged with the element's bounds and the content
//! mask of the scroll region it is in, so it scrolls and clips with GPUI's
//! frame and never draws outside its panel.
use gpui::{prelude::*, *};
use gpui_macos::native_control::NativeControlSlot;
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained,
    runtime::AnyObject, sel,
};
use objc2_app_kit::*;
use objc2_foundation::{NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};
use serde_json::{Value, json};
use std::rc::Rc;
#[derive(Clone, Copy)]
pub enum Kind {
    Popup,
    Checkbox,
    /// One radio button; its group is the caller's.
    Radio,
    Switch,
    Push,
    /// `options` are the segments (`"label"` or `"symbol"` with a `"label"`
    /// for accessibility); the value is the selected index.
    Segmented,
    /// A continuous slider; the value is 0–1.
    Slider,
    /// The system search field: `label` is its placeholder, the value its
    /// text, sent as it is typed. Return, Escape and Down hand the keyboard
    /// back to the window's content and are sent as `{"key": …}` (`Enter`,
    /// `Escape`, `ArrowDown`) for the view to act on.
    Search,
}
struct TargetState {
    sender: async_channel::Sender<Value>,
    options: std::cell::RefCell<Vec<Value>>,
    kind: Kind,
    /// The window's content view: where the keyboard goes back to.
    content: Retained<NSView>,
}
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind=MainThreadOnly]
    #[name="PMV4ControlTarget"]
    #[ivars=TargetState]
    struct Target;
    unsafe impl NSObjectProtocol for Target{}
    impl Target{
        #[unsafe(method(act:))]
        fn act(&self,sender:&AnyObject){
            let value=match self.ivars().kind{
                Kind::Popup=>{let index:isize=unsafe{msg_send![sender,indexOfSelectedItem]};self.ivars().options.borrow().get(index as usize).map(|o|o.get("v").unwrap_or(o).clone()).unwrap_or(Value::Null)},
                Kind::Checkbox|Kind::Radio|Kind::Switch=>{let state:isize=unsafe{msg_send![sender,state]};json!(state!=0)},
                Kind::Segmented=>{let index:isize=unsafe{msg_send![sender,selectedSegment]};json!(index)},
                Kind::Slider=>{let value:f64=unsafe{msg_send![sender,doubleValue]};json!(value)},
                Kind::Push=>Value::Null,
                Kind::Search=>{let text:Retained<NSString>=unsafe{msg_send![sender,stringValue]};let text=text.to_string();if let Some(field)=sender.downcast_ref::<NSView>(){search_hint(field,text.is_empty());}json!(text)},
            };
            let _=self.ivars().sender.try_send(value);
        }
        /// A search field's editor: Return, Escape and Down give the
        /// keyboard back to the window's content and tell the view.
        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn command(&self,control:&NSControl,_editor:&NSTextView,command:objc2::runtime::Sel)->bool{
            let key=if command==sel!(cancelOperation:){Some("Escape")}
                else if command==sel!(insertNewline:){Some("Enter")}
                else if command==sel!(moveDown:){Some("ArrowDown")}
                else{None};
            if let Some(key)=key{
                if let Some(window)=control.window(){
                    window.makeFirstResponder(Some(&self.ivars().content));
                }
                let _=self.ivars().sender.try_send(json!({"key":key}));
            }
            key.is_some()
        }
    }
);
enum Control {
    Popup(Retained<NSPopUpButton>),
    Button(Retained<NSButton>),
    Switch(Retained<NSSwitch>),
    Segmented(Retained<NSSegmentedControl>),
    Slider(Retained<NSSlider>),
    Search(Retained<NSSearchField>),
}
impl Control {
    fn view(&self) -> &NSView {
        match self {
            Self::Popup(v) => v.as_ref(),
            Self::Button(v) => v.as_ref(),
            Self::Switch(v) => v.as_ref(),
            Self::Segmented(v) => v.as_ref(),
            Self::Slider(v) => v.as_ref(),
            Self::Search(v) => v.as_ref(),
        }
    }
    fn enabled(&self, enabled: bool) {
        match self {
            Self::Popup(v) => v.setEnabled(enabled),
            Self::Button(v) => v.setEnabled(enabled),
            Self::Switch(v) => v.setEnabled(enabled),
            Self::Segmented(v) => v.setEnabled(enabled),
            Self::Slider(v) => v.setEnabled(enabled),
            Self::Search(v) => v.setEnabled(enabled),
        }
    }
    fn value(&self, value: &Value, options: &[Value]) {
        match self {
            Self::Popup(v) => {
                if let Some(index) = options
                    .iter()
                    .position(|o| o.get("v").unwrap_or(o) == value)
                {
                    v.selectItemAtIndex(index as isize);
                }
            }
            Self::Button(v) => v.setState(if value == &Value::Bool(true) {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            }),
            Self::Switch(v) => v.setState(if value == &Value::Bool(true) {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            }),
            Self::Segmented(v) => {
                v.setSelectedSegment(value.as_i64().map_or(-1, |i| i as isize));
            }
            Self::Slider(v) => v.setDoubleValue(value.as_f64().unwrap_or(0.)),
            // The field owns its text while the keyboard is in it (an echo
            // of an earlier keystroke must not replace a later one); at rest
            // it shows the view's text.
            Self::Search(v) => {
                let text = value.as_str().unwrap_or("");
                if v.currentEditor().is_none() && v.stringValue().to_string() != text {
                    v.setStringValue(&NSString::from_str(text));
                }
                search_hint(v, v.stringValue().is_empty());
            }
        }
    }
}
/// A prominent search field's shortcut hint stands at its trailing end while
/// the field is empty (the clear button takes that place once it is not).
const SEARCH_HINT: isize = 0x4846;
fn search_hint(field: &NSView, empty: bool) {
    if let Some(hint) = field.viewWithTag(SEARCH_HINT) {
        hint.setHidden(!empty);
    }
}
pub struct NativeControl {
    control: Control,
    slot: Rc<NativeControlSlot>,
    _target: Retained<Target>,
    options: Vec<Value>,
    width: f32,
    height: f32,
    inline: bool,
    /// Fills a slot that may be narrower than `width` (its node has
    /// `flexShrink`); AppKit then truncates the title.
    pub fill: bool,
    /// The keyboard was asked for before the control was in its window.
    focus: Rc<std::cell::Cell<bool>>,
    /// The last text the view's host set (`host_text`).
    host_revision: Option<u64>,
}
impl EventEmitter<Value> for NativeControl {}
impl NativeControl {
    /// The width the control fits its title in.
    pub fn width(&self) -> f32 {
        self.width
    }
    pub fn height(&self) -> f32 {
        self.height
    }
}

pub struct ControlSpec<'a> {
    pub kind: Kind,
    pub label: &'a str,
    pub identifier: &'a str,
    pub value: &'a Value,
    pub options: Vec<Value>,
    pub disabled: bool,
    pub default: bool,
    pub destructive: bool,
    /// A borderless pop-up sized to its title, for panel bars (the viewer's
    /// zoom and resolution); rows and sheets keep the bezel at `value_column`.
    pub inline: bool,
    /// An inline pop-up titled in the secondary ink: an option under the
    /// content it qualifies (a composer's option bar, a list's sort). A bar
    /// of controls keeps the primary ink, so its titles never read disabled.
    pub quiet: bool,
    /// The view's one accent action: a push button tinted with this colour.
    pub tint: Option<u32>,
    /// A push button's leading SF Symbol. On a search field, its shortcut
    /// (`⌘F`): the field is then the view's one prominent search, drawn large
    /// with the shortcut at its trailing end while it is empty.
    pub symbol: Option<&'a str>,
}

impl NativeControl {
    pub fn new(
        spec: ControlSpec<'_>,
        t: &pm_design::Tokens,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let ControlSpec {
            kind,
            label,
            identifier,
            value,
            options,
            disabled,
            default,
            destructive,
            inline,
            quiet,
            tint,
            symbol,
        } = spec;
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let RawWindowHandle::AppKit(handle) =
            HasWindowHandle::window_handle(window).unwrap().as_raw()
        else {
            unreachable!()
        };
        let gpui: Retained<NSView> =
            unsafe { Retained::retain(handle.ns_view.as_ptr().cast()).unwrap() };
        let mtm = MainThreadMarker::new().unwrap();
        let (tx, rx) = async_channel::bounded(32);
        let target: Retained<Target> = unsafe {
            msg_send![
                super(Target::alloc(mtm).set_ivars(TargetState {
                    sender: tx,
                    options: std::cell::RefCell::new(options.clone()),
                    kind,
                    content: gpui.clone(),
                })),
                init
            ]
        };
        let control = match kind {
            Kind::Popup => {
                let v = NSPopUpButton::initWithFrame_pullsDown(
                    NSPopUpButton::alloc(mtm),
                    NSRect::ZERO,
                    false,
                );
                add_items(&v, &options);
                unsafe {
                    v.setTarget(Some(&target));
                    v.setAction(Some(sel!(act:)));
                }
                v.setFont(Some(&NSFont::systemFontOfSize(t.typography.body as f64)));
                if inline {
                    v.setBordered(false);
                }
                if inline && quiet {
                    // An option under the content it qualifies: a caption,
                    // never the content's own ink.
                    v.setContentTintColor(Some(&NSColor::secondaryLabelColor()));
                }
                v.sizeToFit();
                Control::Popup(v)
            }
            Kind::Segmented => {
                let v =
                    NSSegmentedControl::initWithFrame(NSSegmentedControl::alloc(mtm), NSRect::ZERO);
                v.setSegmentCount(options.len() as isize);
                v.setTrackingMode(NSSegmentSwitchTracking::SelectOne);
                let font = v
                    .font()
                    .unwrap_or_else(|| NSFont::systemFontOfSize(NSFont::systemFontSize()));
                for (i, option) in options.iter().enumerate() {
                    let title = option
                        .get("label")
                        .and_then(Value::as_str)
                        .or_else(|| option.as_str())
                        .unwrap_or("");
                    let glyph = option
                        .get("symbol")
                        .and_then(Value::as_str)
                        .and_then(|name| {
                            NSImage::imageWithSystemSymbolName_accessibilityDescription(
                                &NSString::from_str(name),
                                Some(&NSString::from_str(title)),
                            )
                        })
                        .map(|image| {
                            // A glyph segment: the panel's symbol size in a
                            // segment a row high wide, never scaled to fit.
                            let config =
                                NSImageSymbolConfiguration::configurationWithPointSize_weight(
                                    t.metrics.symbol_panel as f64,
                                    unsafe { NSFontWeightRegular },
                                );
                            image.imageWithSymbolConfiguration(&config).unwrap_or(image)
                        });
                    let width = if glyph.is_some() {
                        v.setToolTip_forSegment(Some(&NSString::from_str(title)), i as isize);
                        (t.metrics.row + t.space.xs) as f64
                    } else {
                        0.
                    };
                    let face = segment_face(title, glyph, &font);
                    let width = if width > 0. {
                        width
                    } else {
                        face.size().width + 2. * t.space.lg as f64
                    };
                    v.setImage_forSegment(Some(&face), i as isize);
                    v.setImageScaling_forSegment(NSImageScaling::ScaleNone, i as isize);
                    v.setWidth_forSegment(width, i as isize);
                }
                unsafe {
                    v.setTarget(Some(&target));
                    v.setAction(Some(sel!(act:)));
                }
                v.setSelectedSegmentBezelColor(Some(&crate::native::selected_color()));
                v.sizeToFit();
                Control::Segmented(v)
            }
            Kind::Slider => {
                let v = unsafe {
                    NSSlider::sliderWithValue_minValue_maxValue_target_action(
                        0.,
                        0.,
                        1.,
                        Some(&target),
                        Some(sel!(act:)),
                        mtm,
                    )
                };
                Control::Slider(v)
            }
            Kind::Checkbox | Kind::Radio | Kind::Push => {
                let v = unsafe {
                    NSButton::buttonWithTitle_target_action(
                        &NSString::from_str(if matches!(kind, Kind::Checkbox | Kind::Radio) {
                            ""
                        } else {
                            label
                        }),
                        Some(&target),
                        Some(sel!(act:)),
                        mtm,
                    )
                };
                if matches!(kind, Kind::Checkbox) {
                    v.setButtonType(NSButtonType::Switch);
                } else if matches!(kind, Kind::Radio) {
                    v.setButtonType(NSButtonType::Radio);
                } else {
                    v.setBezelStyle(NSBezelStyle::Push);
                    v.setHasDestructiveAction(destructive);
                    if let Some(image) = symbol.and_then(|name| {
                        NSImage::imageWithSystemSymbolName_accessibilityDescription(
                            &NSString::from_str(name),
                            None,
                        )
                    }) {
                        v.setImage(Some(&image));
                        v.setImagePosition(NSCellImagePosition::ImageLeading);
                    }
                    if let Some(rgb) = tint {
                        let channel = |shift: u32| ((rgb >> shift) & 255) as f64 / 255.;
                        v.setBezelColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(
                            channel(16),
                            channel(8),
                            channel(0),
                            1.,
                        )));
                    }
                    if default {
                        v.setKeyEquivalent(&NSString::from_str("\r"));
                    }
                }
                v.setFont(Some(&NSFont::systemFontOfSize(t.typography.body as f64)));
                v.sizeToFit();
                Control::Button(v)
            }
            Kind::Search => {
                let v = NSSearchField::initWithFrame(NSSearchField::alloc(mtm), NSRect::ZERO);
                v.setPlaceholderString(Some(&NSString::from_str(label)));
                // The caret and the clear button say where the keyboard is;
                // no accent ring.
                v.setFocusRingType(NSFocusRingType::None);
                v.setFont(Some(&NSFont::systemFontOfSize(t.typography.body as f64)));
                v.setSendsSearchStringImmediately(true);
                v.setSendsWholeSearchString(false);
                unsafe {
                    v.setTarget(Some(&target));
                    v.setAction(Some(sel!(act:)));
                    let _: () = msg_send![&*v, setDelegate: &*target];
                }
                v.sizeToFit();
                // A view's one prominent search (Home): the large field, with
                // its shortcut at the trailing end while it is empty.
                if let Some(shortcut) = symbol {
                    v.setControlSize(NSControlSize::Large);
                    v.setFont(Some(&NSFont::systemFontOfSize(t.typography.title as f64)));
                    v.sizeToFit();
                    let hint = NSTextField::labelWithString(&NSString::from_str(shortcut), mtm);
                    hint.setFont(Some(&NSFont::systemFontOfSize(t.typography.body as f64)));
                    hint.setTextColor(Some(&NSColor::tertiaryLabelColor()));
                    hint.setTag(SEARCH_HINT);
                    hint.setAccessibilityElement(false);
                    hint.sizeToFit();
                    let (field, own) = (v.frame().size, hint.frame().size);
                    hint.setFrameOrigin(NSPoint::new(
                        field.width - own.width - 12.,
                        ((field.height - own.height) / 2.).round(),
                    ));
                    hint.setAutoresizingMask(
                        NSAutoresizingMaskOptions::ViewMinXMargin
                            | NSAutoresizingMaskOptions::ViewMinYMargin
                            | NSAutoresizingMaskOptions::ViewMaxYMargin,
                    );
                    v.addSubview(&hint);
                }
                Control::Search(v)
            }
            Kind::Switch => {
                let v = NSSwitch::initWithFrame(NSSwitch::alloc(mtm), NSRect::ZERO);
                unsafe {
                    v.setTarget(Some(&target));
                    v.setAction(Some(sel!(act:)));
                }
                v.sizeToFit();
                Control::Switch(v)
            }
        };
        control
            .view()
            .setAccessibilityLabel(Some(&NSString::from_str(label)));
        control
            .view()
            .setAccessibilityIdentifier(Some(&NSString::from_str(identifier)));
        control.enabled(!disabled);
        control.value(value, &options);
        let intrinsic = control.view().fittingSize();
        let width = if matches!(kind, Kind::Popup) && !inline {
            t.metrics.value_column.max(intrinsic.width as f32)
        } else if matches!(kind, Kind::Slider | Kind::Search) {
            t.metrics.value_column
        } else {
            (intrinsic.width as f32).max(t.metrics.hit)
        };
        let height = t.metrics.control.max(intrinsic.height as f32);
        let native =
            unsafe { Retained::retain(control.view() as *const NSView as *mut NSView).unwrap() };
        let slot = Rc::new(NativeControlSlot::new(&gpui, native));
        cx.spawn(async move |this, cx| {
            while let Ok(value) = rx.recv().await {
                if this.update(cx, |_, cx| cx.emit(value)).is_err() {
                    break;
                }
            }
        })
        .detach();
        Self {
            control,
            slot,
            _target: target,
            options,
            width,
            height,
            inline,
            // A search field is as wide as the place it is given.
            fill: matches!(kind, Kind::Search),
            focus: Rc::default(),
            host_revision: None,
        }
    }
    /// Puts the keyboard in a search field (⌘F), with its text selected;
    /// one not yet in its window takes it when it is first placed.
    pub fn focus(&self) {
        let Control::Search(v) = &self.control else {
            return;
        };
        match v.window() {
            Some(window) => {
                window.makeFirstResponder(Some(v));
            }
            None => self.focus.set(true),
        }
    }
    /// A text the view's host set (a filter applied from outside) replaces
    /// a search field's text even while it is typed in, once per revision.
    pub fn host_text(&mut self, text: &str, revision: u64) {
        if let Control::Search(v) = &self.control
            && self.host_revision.replace(revision) != Some(revision)
        {
            v.setStringValue(&NSString::from_str(text));
            search_hint(v, text.is_empty());
        }
    }
    /// A search field's placeholder, when it is not its accessibility label.
    pub fn placeholder(&self, text: &str) {
        if let Control::Search(v) = &self.control {
            v.setPlaceholderString(Some(&NSString::from_str(text)));
        }
    }
    pub fn is_search(&self) -> bool {
        matches!(self.control, Control::Search(_))
    }
    /// Whether the keyboard is in this control (a search field's editor).
    pub fn focused(&self) -> bool {
        let Control::Search(v) = &self.control else {
            return false;
        };
        v.currentEditor().is_some()
    }
    pub fn keep(&self) {
        self.slot.keep();
    }
    /// A push button's title follows the panel (Export's action names the
    /// chosen format); its width follows the new title.
    pub fn retitle(&mut self, title: &str, cx: &mut Context<Self>) {
        let Control::Button(button) = &self.control else {
            return;
        };
        let title = NSString::from_str(title);
        if button.title() == title {
            return;
        }
        button.setTitle(&title);
        button.setAccessibilityLabel(Some(&title));
        self.width = (button.fittingSize().width as f32).max(self.height);
        cx.notify();
    }
    /// A pop-up's own title, shown instead of its selected row; the row
    /// keeps its checkmark. `None` shows the selected row.
    pub fn display(&mut self, title: Option<&str>) {
        let Control::Popup(v) = &self.control else {
            return;
        };
        let Some(cell) = v.cell() else { return };
        unsafe {
            let _: () = objc2::msg_send![&*cell, setUsesItemFromMenu: title.is_none()];
            if let Some(title) = title {
                let mtm = MainThreadMarker::new().expect("main thread");
                let item = NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(title),
                    None,
                    &NSString::from_str(""),
                );
                let _: () = objc2::msg_send![&*cell, setMenuItem: &*item];
            }
        }
    }
    pub fn update(
        &mut self,
        value: &Value,
        options: &[Value],
        disabled: bool,
        cx: &mut Context<Self>,
    ) {
        // A pop-up's rows follow the tree (the viewer's zoom row names the
        // current zoom), so a changed list replaces the menu's items.
        if let Control::Popup(v) = &self.control
            && options != self.options.as_slice()
        {
            v.removeAllItems();
            add_items(v, options);
            options.clone_into(&mut self.options);
            options.clone_into(&mut self._target.ivars().options.borrow_mut());
        }
        self.control.value(value, &self.options);
        self.control.enabled(!disabled);
        if self.inline {
            // The title names the current value, so the width follows it.
            self.width = self.control.view().fittingSize().width as f32;
        }
        cx.notify();
    }
}
impl Render for NativeControl {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let slot = self.slot.clone();
        let focus = self.focus.clone();
        let native: Retained<NSView> = unsafe {
            Retained::retain(self.control.view() as *const NSView as *mut NSView).unwrap()
        };
        let root = if matches!(self.control, Control::Search(_)) {
            div().w_full()
        } else if self.fill {
            div().w_full().max_w(px(self.width))
        } else {
            div().w(px(self.width))
        };
        root.relative().h(px(self.height)).child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    let to_rect = |b: Bounds<Pixels>| {
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
                    slot.stage(to_rect(bounds), to_rect(window.content_mask().bounds));
                    if focus.get()
                        && let Some(window) = native.window()
                    {
                        focus.set(false);
                        window.makeFirstResponder(Some(&native));
                    }
                },
            )
            .absolute()
            .inset_0(),
        )
    }
}
/// A segment's content, drawn by us in the label ink. AppKit paints a title
/// or a template glyph white on a custom selected bezel, which only reads on
/// a dark bezel: a heavy pill in the light appearance. A face is an image
/// that is not a template, so the selected segment is the same pale raised
/// neutral as a selected tab, with the content's own ink on it.
fn segment_face(title: &str, glyph: Option<Retained<NSImage>>, font: &NSFont) -> Retained<NSImage> {
    let text = NSString::from_str(title);
    let attributes = objc2_foundation::NSDictionary::<NSString, AnyObject>::from_slices(
        &[unsafe { NSFontAttributeName }, unsafe {
            NSForegroundColorAttributeName
        }],
        &[font as &AnyObject, &*NSColor::labelColor() as &AnyObject],
    );
    let size = match &glyph {
        Some(image) => image.size(),
        None => {
            let size: NSSize = unsafe { msg_send![&*text, sizeWithAttributes: &*attributes] };
            NSSize::new(size.width.ceil(), size.height.ceil())
        }
    };
    let draw = block2::RcBlock::new(move |rect: NSRect| -> objc2::runtime::Bool {
        match &glyph {
            Some(image) => {
                image.drawInRect(rect);
                // The label ink of the appearance being drawn in.
                NSColor::labelColor().set();
                NSRectFillUsingOperation(rect, NSCompositingOperation::SourceAtop);
            }
            None => unsafe {
                let _: () = msg_send![
                    &*text,
                    drawAtPoint: rect.origin,
                    withAttributes: &*attributes
                ];
            },
        }
        objc2::runtime::Bool::YES
    });
    let face = NSImage::imageWithSize_flipped_drawingHandler(size, false, &draw);
    // Drawn afresh each time, so the ink follows the appearance.
    face.setCacheMode(NSImageCacheMode::Never);
    face.setAccessibilityDescription(Some(&NSString::from_str(title)));
    face
}
fn add_items(popup: &NSPopUpButton, options: &[Value]) {
    for option in options {
        popup.addItemWithTitle(&NSString::from_str(
            option
                .get("label")
                .and_then(Value::as_str)
                .or_else(|| option.as_str())
                .unwrap_or(""),
        ));
    }
}
