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
    /// A slider. Its one option is its range (`{"min","max","step","log"}`);
    /// without one the value is 0–1.
    Slider,
    /// The system search field: `label` is its placeholder, the value its
    /// text, sent as it is typed. Return, Escape and Down hand the keyboard
    /// back to the window's content and are sent as `{"key": …}` (`Enter`,
    /// `Escape`, `ArrowDown`) for the view to act on.
    Search,
}
/// A slider's value range. A logarithmic one moves its track in ln(value),
/// so each equal pull multiplies the value (brightness, size).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Range {
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub log: bool,
}
impl Range {
    /// From the slider's single option (`{"min","max","step","log"}`).
    pub fn of(options: &[Value]) -> Self {
        let o = options.first().cloned().unwrap_or(Value::Null);
        let n = |k: &str, d: f64| o[k].as_f64().filter(|v| v.is_finite()).unwrap_or(d);
        let (min, max) = (n("min", 0.), n("max", 1.));
        let log = o["log"] == true && min > 0. && max > min;
        Self {
            min,
            max: max.max(min),
            step: n("step", 0.).max(0.),
            log,
        }
    }
    fn position(self, value: f64) -> f64 {
        let v = value.clamp(self.min, self.max);
        if self.log { v.ln() } else { v }
    }
    fn value(self, position: f64) -> f64 {
        let v = if self.log { position.exp() } else { position };
        let v = if self.step > 0. {
            (v / self.step).round() * self.step
        } else {
            // Three significant digits: a pull lands on a value one can read.
            let digits = 2 - v.abs().max(1e-9).log10().floor() as i32;
            let scale = 10f64.powi(digits.max(0));
            (v * scale).round() / scale
        };
        v.clamp(self.min, self.max)
    }
    /// One arrow-key step: the declared step, else 1/20 of the track.
    fn nudge(self, value: f64, up: bool) -> f64 {
        let sign = if up { 1. } else { -1. };
        if self.log {
            let span = (self.max.ln() - self.min.ln()) / 20.;
            return self.value(
                (self.position(value) + sign * span).clamp(self.min.ln(), self.max.ln()),
            );
        }
        let step = if self.step > 0. {
            self.step
        } else {
            (self.max - self.min) / 20.
        };
        self.value((value + sign * step).clamp(self.min, self.max))
    }
}
/// The value VoiceOver reads: up to three significant digits, trailing
/// zeros trimmed.
fn readable(v: f64) -> String {
    let digits = (2 - v.abs().max(1e-9).log10().floor() as i32).clamp(0, 6) as usize;
    let text = format!("{v:.digits$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    }
}
struct TargetState {
    sender: async_channel::Sender<Value>,
    /// The rows the pop-up's menu shows now: what an action's index means.
    options: std::cell::RefCell<Vec<Value>>,
    /// The pop-up's menu is open.
    tracking: std::cell::Cell<bool>,
    /// Rows (and the selected value) that came while the menu was open and
    /// differ in more than their titles: shown once the menu has closed and
    /// its action has gone out.
    pending: std::cell::RefCell<Option<(Vec<Value>, Value)>>,
    popup: std::cell::RefCell<Option<Retained<NSPopUpButton>>>,
    /// Rows the host lists when the menu opens (`NativeControl::fill_rows`):
    /// the pop-up shows only its current row until then.
    fill: std::cell::RefCell<Option<RowSource>>,
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
                Kind::Popup=>{let index:isize=unsafe{msg_send![sender,indexOfSelectedItem]};self.ivars().options.borrow().get(index as usize).map(|o|row_value(o).clone()).unwrap_or(Value::Null)},
                Kind::Checkbox|Kind::Radio|Kind::Switch=>{let state:isize=unsafe{msg_send![sender,state]};json!(state!=0)},
                Kind::Segmented=>{let index:isize=unsafe{msg_send![sender,selectedSegment]};json!(index)},
                Kind::Slider=>{let position:f64=unsafe{msg_send![sender,doubleValue]};json!(Range::of(&self.ivars().options.borrow()).value(position))},
                Kind::Push=>Value::Null,
                Kind::Search=>{let text:Retained<NSString>=unsafe{msg_send![sender,stringValue]};let text=text.to_string();if let Some(field)=sender.downcast_ref::<NSView>(){search_hint(field,text.is_empty());}json!(text)},
            };
            let _=self.ivars().sender.try_send(value);
        }
        #[unsafe(method(menuBegan:))]
        fn menu_began(&self,_:&AnyObject){self.ivars().tracking.set(true);}
        /// The action of the row that was chosen goes out after this, so
        /// the rows it indexes stay until the next turn of the run loop.
        #[unsafe(method(menuEnded:))]
        fn menu_ended(&self,_:&AnyObject){
            self.ivars().tracking.set(false);
            if self.ivars().pending.borrow().is_some(){
                unsafe{let _:()=msg_send![self,performSelector:sel!(showPending),withObject:std::ptr::null::<AnyObject>(),afterDelay:0.0f64];}
            }
        }
        /// The menu is about to show (NSMenuDelegate): a pop-up whose rows
        /// the host lists takes them now, with its current row selected.
        #[unsafe(method(menuNeedsUpdate:))]
        fn menu_needs_update(&self,_:&AnyObject){
            let Some(fill)=self.ivars().fill.borrow().clone() else{return};
            let Some(popup)=self.ivars().popup.borrow().clone() else{return};
            fill_now(self,&popup,&fill);
        }
        #[unsafe(method(showPending))]
        fn show_pending(&self){
            let popup=self.ivars().popup.borrow().clone();
            if self.ivars().tracking.get(){return;}
            if let (Some(popup),Some((options,value)))=(popup,self.ivars().pending.take()){
                replace_rows(&popup,&options);
                if let Some(index)=options.iter().position(|o|row_value(o)==&value){popup.selectItemAtIndex(index as isize);}
                *self.ivars().options.borrow_mut()=options;
            }
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
                if let Some(index) = options.iter().position(|o| row_value(o) == value) {
                    v.selectItemAtIndex(index as isize);
                } else if options.iter().any(|o| o.get("checked").is_some()) {
                    // A settings pull-down has no single value: no row is
                    // "selected", only the checked ones are marked.
                    v.selectItemAtIndex(-1);
                }
                checks(v, options);
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
            Self::Slider(v) => {
                let range = Range::of(options);
                v.setMinValue(range.position(range.min));
                v.setMaxValue(range.position(range.max));
                let n = value.as_f64().unwrap_or(range.min);
                v.setDoubleValue(range.position(n));
                // VoiceOver reads the value, not a logarithmic position.
                v.setAccessibilityValueDescription(Some(&NSString::from_str(&readable(n))));
            }
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
    width: f32,
    height: f32,
    inline: bool,
    /// Fills a slot that may be narrower than `width` (its node has
    /// `flexShrink`); AppKit then truncates the title.
    pub fill: bool,
    /// A segmented control as wide as its row, in equal segments.
    stretch: bool,
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
    /// A push button with the system's liquid glass bezel at the large size
    /// (macOS 26 and later; the plain bezel before): a surface's one raised
    /// action. A segmented control in the system's capsule at the large
    /// size: a header's one mode switch.
    pub glass: bool,
    /// In a row's value column: as high as a value well. A pop-up has no
    /// bezel there and fills the well its row draws under it, so it is the
    /// same well as the fields beside it; a segmented control is the small
    /// size.
    pub well: bool,
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
            glass,
            well,
        } = spec;
        let glass = glass && objc2::runtime::AnyClass::get(c"NSGlassEffectView").is_some();
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
                    tracking: std::cell::Cell::new(false),
                    pending: std::cell::RefCell::new(None),
                    popup: std::cell::RefCell::new(None),
                    fill: std::cell::RefCell::new(None),
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
                watch_menu(&target, &v);
                v.setFont(Some(&NSFont::systemFontOfSize(t.typography.body as f64)));
                if inline || well {
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
                // `glass`: the system's capsule at the large size (macOS 26's
                // liquid glass control), selection still the neutral pill (the
                // system's own lights it in the accent).
                if glass {
                    v.setBorderShape(NSControlBorderShape::Capsule);
                    v.setControlSize(NSControlSize::Large);
                }
                if well {
                    v.setControlSize(NSControlSize::Small);
                }
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
                // One change per pull, so a drag is one Undo.
                v.setContinuous(false);
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
                    v.setBezelStyle(if glass {
                        NSBezelStyle::Glass
                    } else {
                        NSBezelStyle::Push
                    });
                    if glass {
                        v.setControlSize(NSControlSize::Large);
                    }
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
        let height = if well {
            t.metrics.field
        } else {
            t.metrics.control.max(intrinsic.height as f32)
        };
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
            width,
            height,
            inline,
            // A search field is as wide as the place it is given.
            fill: matches!(kind, Kind::Search),
            stretch: well && matches!(kind, Kind::Popup),
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
    pub fn is_popup(&self) -> bool {
        matches!(self.control, Control::Popup(_))
    }
    pub fn is_search(&self) -> bool {
        matches!(self.control, Control::Search(_))
    }
    /// Whether the control takes the whole width of its row.
    pub fn stretches(&self) -> bool {
        self.stretch
    }
    /// A segmented control fills its row with equal segments (a sheet's
    /// destinations), instead of standing at the width of its titles.
    pub fn stretch(&mut self) {
        let Control::Segmented(v) = &self.control else {
            return;
        };
        for i in 0..v.segmentCount() {
            v.setWidth_forSegment(0., i);
        }
        v.setSegmentDistribution(NSSegmentDistribution::FillEqually);
        self.stretch = true;
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
    /// Full Keyboard Access: the panel's focus ring sits on this control and
    /// forwards its keys. Space, Return and the vertical arrows open a
    /// pop-up's menu (AppKit then runs the menu's own keys); Space toggles a
    /// checkbox or switch and presses a button; the arrows step a slider.
    /// Returns whether the key was the control's.
    pub fn key(&self, key: &str) -> bool {
        let press = |view: &NSView| unsafe {
            let _: () = msg_send![view, performClick: std::ptr::null::<AnyObject>()];
        };
        match &self.control {
            Control::Popup(v) if matches!(key, "space" | "enter" | "down" | "up") => {
                press(v.as_ref());
                true
            }
            Control::Button(v) if matches!(key, "space" | "enter") => {
                press(v.as_ref());
                true
            }
            Control::Switch(v) if key == "space" => {
                press(v.as_ref());
                true
            }
            Control::Slider(v) if matches!(key, "left" | "right" | "up" | "down") => {
                let range = Range::of(&self._target.ivars().options.borrow());
                let now = range.value(v.doubleValue());
                let next = range.nudge(now, matches!(key, "right" | "up"));
                if next != now {
                    v.setDoubleValue(range.position(next));
                    let _ = self._target.ivars().sender.try_send(json!(next));
                }
                true
            }
            _ => false,
        }
    }
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
    /// Rows this pop-up's menu lists when it opens, from `source` (a long
    /// list the panel never builds: every layer a layer can follow). The
    /// published `options` stay what the closed button shows.
    pub fn fill_rows(&mut self, source: Option<RowSource>) {
        let Control::Popup(v) = &self.control else {
            return;
        };
        fill_rows_on(&self._target, v, source);
    }
    pub fn update(
        &mut self,
        value: &Value,
        options: &[Value],
        disabled: bool,
        cx: &mut Context<Self>,
    ) {
        // A pop-up's rows follow the tree (the viewer's zoom row names the
        // current zoom).
        if let Control::Popup(v) = &self.control {
            set_rows(&self._target, v, options, value);
        }
        // A slider's one option is its range.
        if let Control::Slider(_) = &self.control {
            let mut range = self._target.ivars().options.borrow_mut();
            if range.as_slice() != options {
                options.clone_into(&mut range);
            }
        }
        let shown = self._target.ivars().options.borrow().clone();
        self.control.value(value, &shown);
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
        let root = if self.stretch || matches!(self.control, Control::Search(_)) {
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
/// The value a row stands for (a plain string row is its own value).
fn row_value(option: &Value) -> &Value {
    option.get("v").unwrap_or(option)
}
fn row_title(option: &Value) -> Retained<NSString> {
    NSString::from_str(
        option
            .get("label")
            .and_then(Value::as_str)
            .or_else(|| option.as_str())
            .unwrap_or(""),
    )
}
/// One menu item a row, in order. (`addItemWithTitle:` would drop an
/// earlier row of the same title, and an action's index would then name
/// another row.)
fn add_items(popup: &NSPopUpButton, options: &[Value]) {
    let Some(menu) = popup.menu() else { return };
    let mtm = MainThreadMarker::new().expect("main thread");
    for option in options {
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &row_title(option),
                None,
                &NSString::from_str(""),
            )
        };
        subtitle(&item, option);
        menu.addItem(&item);
    }
    checks(popup, options);
}
/// A row's second line (a quality's promise, a sky's look): the menu item's
/// own subtitle, under its title, as in System Settings.
fn subtitle(item: &NSMenuItem, option: &Value) {
    let Some(detail) = option.get("detail").and_then(Value::as_str) else {
        return;
    };
    let responds: bool = unsafe { msg_send![item, respondsToSelector: sel!(setSubtitle:)] };
    if responds {
        let _: () = unsafe { msg_send![item, setSubtitle: &*NSString::from_str(detail)] };
    }
}
/// Rows marked `checked` show a checkmark: a pull-down that reports several
/// settings at once (the viewer's collapsed 3D menu).
fn checks(popup: &NSPopUpButton, options: &[Value]) {
    if !options.iter().any(|o| o.get("checked").is_some()) {
        return;
    }
    for (i, option) in options.iter().enumerate() {
        if let Some(item) = popup.itemAtIndex(i as isize) {
            item.setState(if option.get("checked") == Some(&Value::Bool(true)) {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
        }
    }
}
fn replace_rows(popup: &NSPopUpButton, options: &[Value]) {
    popup.removeAllItems();
    add_items(popup, options);
}
/// Brings a pop-up's rows to `next`. Rows that stand for the same values in
/// the same order keep their menu items and take the new titles, open menu
/// or not (the item under the pointer stays the item under the pointer).
/// Any other change replaces the items, and waits for an open menu to close
/// and send its action first: the action names a row by its index.
fn set_rows(target: &Target, popup: &NSPopUpButton, next: &[Value], value: &Value) {
    let state = target.ivars();
    let shown = state.options.borrow().clone();
    let same_rows = shown.len() == next.len()
        && shown
            .iter()
            .zip(next)
            .all(|(a, b)| row_value(a) == row_value(b));
    if same_rows {
        state.pending.take();
        if shown.as_slice() != next {
            for (index, option) in next.iter().enumerate() {
                let title = row_title(option);
                if let Some(item) = popup.itemAtIndex(index as isize) {
                    if item.title() != title {
                        item.setTitle(&title);
                    }
                    subtitle(&item, option);
                }
            }
            *state.options.borrow_mut() = next.to_vec();
        }
    } else if state.tracking.get() {
        *state.pending.borrow_mut() = Some((next.to_vec(), value.clone()));
    } else {
        state.pending.take();
        replace_rows(popup, next);
        *state.options.borrow_mut() = next.to_vec();
    }
}
/// Lists a pop-up's rows when its menu opens.
pub type RowSource = Rc<dyn Fn() -> Vec<Value>>;
fn fill_rows_on(target: &Target, popup: &NSPopUpButton, source: Option<RowSource>) {
    let on = source.is_some();
    *target.ivars().fill.borrow_mut() = source;
    if let Some(menu) = popup.menu() {
        unsafe {
            if on {
                let _: () = msg_send![&*menu, setDelegate: target];
            } else {
                let none: *const AnyObject = std::ptr::null();
                let _: () = msg_send![&*menu, setDelegate: none];
            }
        }
    }
}
/// The rows the source lists now, the selected value kept selected (and
/// listed, should the source no longer offer it).
fn fill_now(target: &Target, popup: &NSPopUpButton, fill: &RowSource) {
    let state = target.ivars();
    let index = popup.indexOfSelectedItem();
    let current = usize::try_from(index)
        .ok()
        .and_then(|i| state.options.borrow().get(i).cloned());
    let mut rows = fill();
    if rows.is_empty() {
        // Nothing to list from (no project): the button's own rows stay.
        return;
    }
    if let Some(current) = &current
        && !rows.iter().any(|r| row_value(r) == row_value(current))
    {
        rows.insert(rows.len().min(1), current.clone());
    }
    if *state.options.borrow() != rows {
        replace_rows(popup, &rows);
        if let Some(current) = &current
            && let Some(at) = rows.iter().position(|r| row_value(r) == row_value(current))
        {
            popup.selectItemAtIndex(at as isize);
        }
        *state.options.borrow_mut() = rows;
    }
}
/// Tells `target` when the pop-up's menu opens and closes.
fn watch_menu(target: &Target, popup: &NSPopUpButton) {
    *target.ivars().popup.borrow_mut() = Some(unsafe {
        Retained::retain(popup as *const NSPopUpButton as *mut NSPopUpButton).unwrap()
    });
    let Some(menu) = popup.menu() else { return };
    let center = objc2_foundation::NSNotificationCenter::defaultCenter();
    unsafe {
        for (selector, name) in [
            (sel!(menuBegan:), NSMenuDidBeginTrackingNotification),
            (sel!(menuEnded:), NSMenuDidEndTrackingNotification),
        ] {
            center.addObserver_selector_name_object(target, selector, Some(name), Some(&menu));
        }
    }
}
/// A pop-up and its rows without a window, for tests of how rows follow a
/// publication while its menu is open.
#[doc(hidden)]
pub struct PopupRows {
    pub popup: Retained<NSPopUpButton>,
    target: Retained<Target>,
    pub chosen: async_channel::Receiver<Value>,
}
impl PopupRows {
    pub fn new(options: &[Value]) -> Self {
        let mtm = MainThreadMarker::new().expect("main thread");
        let (sender, chosen) = async_channel::unbounded();
        let content = NSView::initWithFrame(NSView::alloc(mtm), NSRect::ZERO);
        let target: Retained<Target> = unsafe {
            msg_send![
                super(Target::alloc(mtm).set_ivars(TargetState {
                    sender,
                    options: std::cell::RefCell::new(options.to_vec()),
                    tracking: std::cell::Cell::new(false),
                    pending: std::cell::RefCell::new(None),
                    popup: std::cell::RefCell::new(None),
                    fill: std::cell::RefCell::new(None),
                    kind: Kind::Popup,
                    content,
                })),
                init
            ]
        };
        let popup =
            NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm), NSRect::ZERO, false);
        add_items(&popup, options);
        unsafe {
            popup.setTarget(Some(&target));
            popup.setAction(Some(sel!(act:)));
        }
        watch_menu(&target, &popup);
        Self {
            popup,
            target,
            chosen,
        }
    }
    pub fn update(&self, options: &[Value], value: &Value) {
        set_rows(&self.target, &self.popup, options, value);
    }
    pub fn fill_rows(&self, source: Option<RowSource>) {
        fill_rows_on(&self.target, &self.popup, source);
    }
    /// What AppKit asks the menu's delegate just before the menu shows.
    pub fn menu_will_show(&self) {
        if let Some(menu) = self.popup.menu() {
            let delegate: *mut AnyObject = unsafe { msg_send![&*menu, delegate] };
            if !delegate.is_null() {
                let _: () = unsafe { msg_send![delegate, menuNeedsUpdate: &*menu] };
            }
        }
    }
    /// The value the pop-up's action sends for the selected row.
    pub fn act(&self) {
        unsafe {
            let _: () = msg_send![&*self.target, act: &*self.popup];
        }
    }
}
